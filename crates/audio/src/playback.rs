use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::Result;
use cpal::{FromSample, OutputCallbackInfo};
use rtrb::{Consumer, Producer};

use crate::resample::Resampler;
use crate::{Clock, Hdr, Status};

const FADE_MS: u64 = 5;
const ONSET_GAP_MS: u64 = 250;
const SEGMENT: usize = 8192;
const REBUFFER_MS: u64 = 300;
const REBUFFER_WAIT_MS: u64 = 180;

#[derive(Default)]
pub(crate) struct PlayShared {
    flush_gen: AtomicU64,
    flush_upto: AtomicU64,
    written: AtomicU64,
    consumed: AtomicU64,
    played: AtomicU64,
    onset_ns: AtomicU64,
    gaps: AtomicU64,
    gap_frames: AtomicU64,
}

struct Writer {
    prod: Producer<f32>,
    rs: Resampler,
    scratch: Vec<f32>,
}

struct Inner {
    shared: Arc<PlayShared>,
    status: Arc<Status>,
    writer: Mutex<Writer>,
    tts_rate: u32,
    dev_rate: u32,
    clock: Clock,
}

#[derive(Clone)]
pub struct Playback {
    inner: Arc<Inner>,
}

impl Playback {
    pub(crate) fn new(
        prod: Producer<f32>,
        shared: Arc<PlayShared>,
        status: Arc<Status>,
        tts_rate: u32,
        dev_rate: u32,
        clock: Clock,
    ) -> Result<Playback> {
        let writer = Writer { prod, rs: Resampler::new(tts_rate, dev_rate)?, scratch: Vec::with_capacity(16384) };
        Ok(Playback {
            inner: Arc::new(Inner { shared, status, writer: Mutex::new(writer), tts_rate, dev_rate, clock }),
        })
    }

    fn writer(&self) -> MutexGuard<'_, Writer> {
        self.inner.writer.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn write(&self, pcm: &[f32]) {
        if pcm.is_empty() {
            return;
        }
        let s = &self.inner.shared;
        let mut w = self.writer();
        let gen0 = s.flush_gen.load(Relaxed);
        let Writer { prod, rs, scratch } = &mut *w;
        scratch.clear();
        rs.process(pcm, scratch);
        let (done, rest) = prod.push_partial_slice(scratch);
        s.written.fetch_add(done.len() as u64, Release);
        if rest.is_empty() {
            return;
        }
        let mut rest = rest.to_vec();
        drop(w);
        loop {
            std::thread::sleep(Duration::from_millis(5));
            if !self.inner.status.alive.load(Acquire) {
                return;
            }
            let mut w = self.writer();
            if s.flush_gen.load(Relaxed) != gen0 {
                return;
            }
            let n = w.prod.push_partial_slice(&rest).0.len();
            s.written.fetch_add(n as u64, Release);
            rest.drain(..n);
            if rest.is_empty() {
                return;
            }
        }
    }

    pub fn flush(&self) {
        let s = &self.inner.shared;
        let mut w = self.writer();
        w.rs.reset();
        s.flush_upto.store(s.written.load(Relaxed), Release);
        s.flush_gen.fetch_add(1, Release);
    }

    pub fn queued_ms(&self) -> u32 {
        let s = &self.inner.shared;
        let written = s.written.load(Acquire);
        let gone = s.consumed.load(Acquire).max(s.flush_upto.load(Acquire));
        (written.saturating_sub(gone) * 1000 / self.inner.dev_rate as u64) as u32
    }

    pub fn played_samples(&self) -> u64 {
        let played = self.inner.shared.played.load(Acquire) as u128;
        (played * self.inner.tts_rate as u128 / self.inner.dev_rate as u128) as u64
    }

    pub fn gaps(&self) -> (u64, u64) {
        let s = &self.inner.shared;
        (s.gaps.load(Relaxed), s.gap_frames.load(Relaxed) * 1000 / self.inner.dev_rate as u64)
    }

    pub fn last_onset(&self) -> Option<Instant> {
        match self.inner.shared.onset_ns.load(Acquire) {
            0 => None,
            t => Some(self.inner.clock.instant(t)),
        }
    }
}

pub(crate) struct Render {
    cons: Consumer<f32>,
    ref_prod: Producer<f32>,
    ref_hdr: Producer<Hdr>,
    shared: Arc<PlayShared>,
    channels: usize,
    rate: u64,
    mono: Vec<f32>,
    seen_gen: u64,
    skip_to: u64,
    fade_left: u64,
    fade_total: u64,
    fade_len: u64,
    consumed: u64,
    played: u64,
    idle: u64,
    gap: u64,
    pos: u64,
    playing: bool,
    wait: u64,
    fade_in: u64,
}

impl Render {
    pub fn new(
        cons: Consumer<f32>,
        ref_prod: Producer<f32>,
        ref_hdr: Producer<Hdr>,
        shared: Arc<PlayShared>,
        channels: usize,
        rate: u32,
    ) -> Render {
        let rate = rate as u64;
        let gap = rate * ONSET_GAP_MS / 1000;
        Render {
            cons,
            ref_prod,
            ref_hdr,
            shared,
            channels: channels.max(1),
            rate,
            mono: vec![0.0; SEGMENT],
            seen_gen: 0,
            skip_to: 0,
            fade_left: 0,
            fade_total: 0,
            fade_len: (rate * FADE_MS / 1000).max(1),
            consumed: 0,
            played: 0,
            idle: gap,
            gap,
            pos: 0,
            playing: false,
            wait: 0,
            fade_in: 0,
        }
    }

    pub fn render<T>(&mut self, out: &mut [T], info: &OutputCallbackInfo)
    where
        T: cpal::SizedSample + FromSample<f32>,
    {
        let ts = info.timestamp();
        let t0 = ts.playback.as_nanos() as u64;
        let cb = ts.callback.as_nanos() as u64;
        let ch = self.channels;
        let mut done = 0u64;
        for seg in out.chunks_mut(SEGMENT * ch) {
            let n = seg.len() / ch;
            let t = t0 + done * 1_000_000_000 / self.rate;
            self.fill(n, t);
            let mono = &self.mono[..n];
            for (frame, &s) in seg.chunks_exact_mut(ch).zip(mono) {
                frame.fill(T::from_sample_(s.clamp(-1.0, 1.0)));
            }
            if n > 0 && self.ref_prod.slots() >= n && self.ref_hdr.slots() >= 1 {
                let _ = self.ref_prod.push_entire_slice(mono);
                let _ = self.ref_hdr.push(Hdr { pos: self.pos, t_ns: t, cb_ns: cb, frames: n as u32 });
            }
            self.pos += n as u64;
            done += n as u64;
        }
        self.shared.consumed.store(self.consumed, Release);
        self.shared.played.store(self.played, Release);
    }

    fn fill(&mut self, n: usize, t: u64) {
        let mut f = 0;
        while f < n {
            if self.consumed < self.skip_to {
                if self.fade_left > 0 {
                    match self.cons.pop() {
                        Ok(s) => {
                            self.mono[f] = s * self.fade_left as f32 / (self.fade_total + 1) as f32;
                            self.fade_left -= 1;
                            self.consumed += 1;
                            self.played += 1;
                            self.playing = self.fade_left > 0;
                            f += 1;
                        }
                        Err(_) => self.skip_to = self.consumed,
                    }
                } else {
                    let d = (self.skip_to - self.consumed).min(self.cons.slots() as u64) as usize;
                    if let Ok(c) = self.cons.read_chunk(d) {
                        c.commit_all();
                    }
                    self.consumed += d as u64;
                    self.skip_to = self.consumed;
                    self.playing = false;
                }
                continue;
            }
            let avail = self.cons.slots();
            if self.check_flush() {
                continue;
            }
            let mut k = avail.min(n - f);
            let resuming = k > 0 && !self.playing && self.idle > 0 && self.idle < self.gap;
            if resuming
                && (avail as u64) < self.rate * REBUFFER_MS / 1000
                && self.wait < self.rate * REBUFFER_WAIT_MS / 1000
            {
                self.wait += (n - f) as u64;
                k = 0;
            }
            if k == 0 {
                self.mono[f..n].fill(0.0);
                self.idle = self.idle.saturating_add((n - f) as u64);
                self.playing = false;
                break;
            }
            if self.idle >= self.gap {
                let onset = t + f as u64 * 1_000_000_000 / self.rate;
                self.shared.onset_ns.store(onset.max(1), Release);
            }
            if resuming {
                self.shared.gaps.fetch_add(1, Relaxed);
                self.shared.gap_frames.fetch_add(self.idle, Relaxed);
                self.fade_in = self.fade_len;
            }
            self.wait = 0;
            self.idle = 0;
            self.playing = true;
            if let Ok(c) = self.cons.read_chunk(k) {
                let (a, b) = c.as_slices();
                self.mono[f..f + a.len()].copy_from_slice(a);
                self.mono[f + a.len()..f + k].copy_from_slice(b);
                c.commit_all();
            }
            for s in &mut self.mono[f..f + k] {
                if self.fade_in == 0 {
                    break;
                }
                *s *= 1.0 - self.fade_in as f32 / self.fade_len as f32;
                self.fade_in -= 1;
            }
            f += k;
            self.consumed += k as u64;
            self.played += k as u64;
        }
    }

    fn check_flush(&mut self) -> bool {
        let g = self.shared.flush_gen.load(Acquire);
        if g == self.seen_gen {
            return false;
        }
        self.seen_gen = g;
        let upto = self.shared.flush_upto.load(Acquire);
        if upto < self.consumed {
            return false;
        }
        self.idle = self.gap;
        if upto == self.consumed {
            return false;
        }
        self.skip_to = upto;
        self.fade_total = if self.playing { (upto - self.consumed).min(self.fade_len) } else { 0 };
        self.fade_left = self.fade_total;
        true
    }
}
