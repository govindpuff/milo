use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::Ordering::{Acquire, Relaxed};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use rtrb::Consumer;

use crate::resample::Resampler;
use crate::{Clock, Hdr, Status};

const RATE: u32 = 16_000;
const BLOCK: usize = 320;
const MAX_LEAD: f64 = 0.040;
const LEAD_MARGIN: f64 = 0.002;
const SLIP_UP: i64 = 16;
const SLIP_DOWN: i64 = 64;
const REF_HISTORY: i64 = 800;
const REF_KEEP: usize = 4000;
const MAX_REF_WAIT: Duration = Duration::from_millis(60);
const OUTPUT_STALL: Duration = Duration::from_millis(100);

pub(crate) struct Tap {
    samples: Consumer<f32>,
    hdrs: Consumer<Hdr>,
    rate: u32,
    rs: Resampler,
    clock: Clock,
    buf: VecDeque<f32>,
    start: i64,
    end: i64,
    seg_pos: u64,
    seg_idx: i64,
    expect: Option<u64>,
    last: Option<(u64, f64)>,
    delay: f64,
    tmp: Vec<f32>,
    out: Vec<f32>,
    gaps: u64,
    last_data: Instant,
}

impl Tap {
    pub fn new(samples: Consumer<f32>, hdrs: Consumer<Hdr>, rate: u32, clock: Clock) -> Result<Tap> {
        Ok(Tap {
            samples,
            hdrs,
            rate,
            rs: Resampler::new(rate, RATE)?,
            clock,
            buf: VecDeque::with_capacity(RATE as usize * 2),
            start: 0,
            end: 0,
            seg_pos: 0,
            seg_idx: 0,
            expect: None,
            last: None,
            delay: 0.0,
            tmp: Vec::with_capacity(8192),
            out: Vec::with_capacity(4096),
            gaps: 0,
            last_data: Instant::now(),
        })
    }

    fn pump(&mut self) {
        while let Ok(h) = self.hdrs.pop() {
            let n = h.frames as usize;
            self.tmp.clear();
            match self.samples.read_chunk(n) {
                Ok(c) => {
                    let (a, b) = c.as_slices();
                    self.tmp.extend_from_slice(a);
                    self.tmp.extend_from_slice(b);
                    c.commit_all();
                }
                Err(_) => {
                    self.expect = None;
                    continue;
                }
            }
            if self.expect != Some(h.pos) {
                if self.expect.is_some() {
                    self.gaps += 1;
                }
                self.rs.reset();
                self.seg_pos = h.pos;
                self.seg_idx = self.end;
            }
            self.expect = Some(h.pos + n as u64);
            self.last = Some((h.pos, self.clock.secs(h.t_ns)));
            self.delay = (h.t_ns as f64 - h.cb_ns as f64).abs() / 1e9;
            self.out.clear();
            self.rs.process(&self.tmp, &mut self.out);
            self.buf.extend(&self.out);
            self.end += self.out.len() as i64;
            self.last_data = Instant::now();
        }
    }

    fn time_of(&self, idx: i64) -> Option<f64> {
        let (hp, ht) = self.last?;
        let pos = self.seg_pos as f64 + (idx - self.seg_idx) as f64 * self.rate as f64 / RATE as f64;
        Some(ht + (pos - hp as f64) / self.rate as f64)
    }

    fn index_at(&self, t: f64) -> Option<f64> {
        let (hp, ht) = self.last?;
        let pos = hp as f64 + (t - ht) * self.rate as f64;
        Some(self.seg_idx as f64 + (pos - self.seg_pos as f64) * RATE as f64 / self.rate as f64)
    }

    fn drop_before(&mut self, idx: i64) {
        let k = (idx - self.start).clamp(0, self.buf.len() as i64);
        self.buf.drain(..k as usize);
        self.start += k;
    }

    fn get(&self, idx: i64) -> f32 {
        if idx >= self.start && idx < self.end { self.buf[(idx - self.start) as usize] } else { 0.0 }
    }
}

pub struct Capture {
    mic: Tap,
    refs: Tap,
    status: Arc<Status>,
    off: Option<i64>,
    ready_at: Option<Instant>,
    gaps_seen: (u64, u64),
    errors_seen: u64,
    errors_logged: Option<Instant>,
    warned_idle: bool,
}

impl Capture {
    pub(crate) fn new(mic: Tap, refs: Tap, status: Arc<Status>) -> Capture {
        Capture {
            mic,
            refs,
            status,
            off: None,
            ready_at: None,
            gaps_seen: (0, 0),
            errors_seen: 0,
            errors_logged: None,
            warned_idle: false,
        }
    }

    pub fn read(&mut self, mic: &mut Vec<f32>, reference: &mut Vec<f32>) -> Result<bool> {
        loop {
            if !self.status.alive.load(Acquire) {
                return Ok(false);
            }
            if let Some(e) = self.status.fatal() {
                bail!("audio stream failed: {e}");
            }
            self.mic.pump();
            self.refs.pump();
            self.report();
            if self.mic.buf.len() >= BLOCK {
                let i0 = self.mic.start;
                self.align(i0);
                if let Some(off) = self.off {
                    let need = i0 + off + BLOCK as i64;
                    let ready = *self.ready_at.get_or_insert_with(Instant::now);
                    if self.refs.end < need
                        && ready.elapsed() < MAX_REF_WAIT
                        && self.refs.last_data.elapsed() < OUTPUT_STALL
                    {
                        std::thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                }
                self.ready_at = None;
                mic.clear();
                mic.extend(self.mic.buf.drain(..BLOCK));
                self.mic.start += BLOCK as i64;
                reference.clear();
                match self.off {
                    Some(off) => {
                        let j0 = i0 + off;
                        reference.extend((j0..j0 + BLOCK as i64).map(|j| self.refs.get(j)));
                        self.refs.drop_before(j0 + BLOCK as i64 - REF_HISTORY);
                    }
                    None => reference.resize(BLOCK, 0.0),
                }
                let excess = self.refs.buf.len().saturating_sub(REF_KEEP) as i64;
                self.refs.drop_before(self.refs.start + excess);
                return Ok(true);
            }
            if self.mic.last_data.elapsed() > Duration::from_secs(2) && !self.warned_idle {
                log::warn!("no audio from the input device for 2 s");
                self.warned_idle = true;
            }
            let missing_ms = (BLOCK - self.mic.buf.len()) as u64 * 1000 / RATE as u64;
            std::thread::sleep(Duration::from_millis(if missing_ms > 6 { missing_ms - 4 } else { 1 }));
        }
    }

    fn align(&mut self, i0: i64) {
        let lead = (self.mic.delay + self.refs.delay + LEAD_MARGIN).min(MAX_LEAD);
        let Some(tm) = self.mic.time_of(i0) else { return };
        let Some(j) = self.refs.index_at(tm + lead) else { return };
        let want = j.round() as i64 - i0;
        match self.off {
            Some(off) if want - off <= SLIP_UP && off - want <= SLIP_DOWN => {}
            Some(off) => {
                log::debug!("echo reference realigned by {} samples (lead {:.1} ms)", want - off, lead * 1000.0);
                self.off = Some(want);
            }
            None => {
                log::info!(
                    "echo reference aligned: lead {:.1} ms (input delay {:.1} ms, output delay {:.1} ms)",
                    lead * 1000.0,
                    self.mic.delay * 1000.0,
                    self.refs.delay * 1000.0
                );
                self.off = Some(want);
            }
        }
    }

    fn report(&mut self) {
        if (self.mic.gaps, self.refs.gaps) != self.gaps_seen {
            log::warn!("audio ring overflow (input gaps {}, echo reference gaps {})", self.mic.gaps, self.refs.gaps);
            self.gaps_seen = (self.mic.gaps, self.refs.gaps);
        }
        let errors = self.status.errors.load(Relaxed);
        if errors != self.errors_seen && self.errors_logged.is_none_or(|t| t.elapsed() >= Duration::from_secs(10)) {
            log::warn!(
                "audio: {} CoreAudio overloads or stream errors since the last report",
                errors - self.errors_seen
            );
            self.errors_seen = errors;
            self.errors_logged = Some(Instant::now());
        }
    }
}
