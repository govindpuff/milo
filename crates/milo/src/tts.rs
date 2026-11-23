use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use anyhow::Result;
use audio::Playback;
use phonon::Tts;
use tokio::sync::mpsc::UnboundedSender;

use crate::controller::Event;

const TAIL_MS: u32 = 10;

pub enum Engine {
    Phonon(Tts, phonon::Voice),
    Sidecar(voice::Voice),
}

impl Engine {
    pub fn sounds(&self) -> &'static [&'static str] {
        match self {
            Engine::Phonon(..) => &[],
            Engine::Sidecar(sidecar) => sidecar.kind().sounds(),
        }
    }

    pub fn sample_rate(&self) -> u32 {
        match self {
            Engine::Phonon(tts, _) => tts.sample_rate(),
            Engine::Sidecar(sidecar) => sidecar.sample_rate(),
        }
    }

    pub fn synthesize(
        &self,
        text: &str,
        instruct: Option<&str>,
        session: Option<u64>,
        lead: Option<usize>,
        stop: &AtomicBool,
        on_pcm: &mut (dyn FnMut(&[f32]) -> Result<()> + Send),
    ) -> Result<()> {
        match self {
            Engine::Phonon(tts, voice) => tts.synthesize(voice, &strip_tags(text), stop, on_pcm),
            Engine::Sidecar(sidecar) => sidecar.synthesize(text, instruct, session, lead, stop, on_pcm),
        }
    }

    pub fn lead(&self, id: usize, text: &str, pcm: &[f32]) {
        if let Engine::Sidecar(sidecar) = self
            && let Err(e) = sidecar.lead(id, text, pcm)
        {
            log::debug!("{e:#}");
        }
    }
}

fn strip_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0u32;
    for c in text.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            c if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub enum Job {
    Say { turn: u64, text: String, instruct: Option<String>, lead: Option<usize>, stop: Arc<AtomicBool> },
    End { turn: u64 },
}

pub struct Gate {
    playback: Playback,
    rate: u32,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    turn: u64,
    committed: bool,
    held: Vec<f32>,
    samples: u64,
    base: Option<u64>,
    chunks: Vec<(u64, String)>,
}

impl State {
    fn reset(&mut self, turn: u64) {
        self.turn = turn;
        self.committed = false;
        self.held.clear();
        self.samples = 0;
        self.base = None;
        self.chunks.clear();
    }

    fn heard(&self, played: u64) -> String {
        let Some(base) = self.base else { return String::new() };
        let n = played.saturating_sub(base);
        self.chunks.iter().filter(|(at, _)| *at < n).map(|(_, t)| t.as_str()).collect::<Vec<_>>().join(" ")
    }
}

impl Gate {
    pub fn new(playback: Playback, rate: u32) -> Gate {
        Gate { playback, rate, state: Mutex::new(State::default()) }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self, s: &mut State, pcm: &[f32]) {
        if pcm.is_empty() {
            return;
        }
        if s.base.is_none() {
            let queued = self.playback.queued_ms() as u64 * self.rate as u64 / 1000;
            s.base = Some(self.playback.played_samples() + queued);
        }
        self.playback.write(pcm);
    }

    pub fn open(&self, turn: u64) {
        self.state().reset(turn);
    }

    pub fn close(&self, turn: u64) {
        let mut s = self.state();
        if s.turn == turn {
            s.reset(0);
        }
    }

    pub fn commit(&self, turn: u64) {
        let mut s = self.state();
        if s.turn != turn {
            return;
        }
        s.committed = true;
        let mut held = std::mem::take(&mut s.held);
        self.write(&mut s, &held);
        held.clear();
        s.held = held;
    }

    pub fn interrupt(&self, turn: u64) -> String {
        let mut s = self.state();
        let heard = if s.turn == turn { s.heard(self.playback.played_samples()) } else { String::new() };
        s.reset(0);
        self.playback.flush();
        heard
    }

    fn chunk(&self, turn: u64, text: &str) {
        let mut s = self.state();
        if s.turn == turn {
            let at = s.samples;
            s.chunks.push((at, text.to_owned()));
        }
    }

    fn push(&self, turn: u64, pcm: &[f32]) -> bool {
        let mut s = self.state();
        if s.turn != turn || pcm.is_empty() {
            return false;
        }
        let first = s.samples == 0;
        s.samples += pcm.len() as u64;
        if s.committed {
            self.write(&mut s, pcm);
        } else {
            s.held.extend_from_slice(pcm);
        }
        first
    }

    fn finish(&self, turn: u64, tail: &[f32]) {
        let mut s = self.state();
        if s.turn != turn || s.samples == 0 {
            return;
        }
        if s.committed {
            self.write(&mut s, tail);
        } else {
            s.held.extend_from_slice(tail);
        }
    }
}

pub fn spawn(
    engine: Engine,
    gate: Arc<Gate>,
    events: UnboundedSender<Event>,
) -> std::io::Result<(Sender<Job>, JoinHandle<()>)> {
    let (tx, rx) = mpsc::channel::<Job>();
    let tail = vec![0.0f32; (engine.sample_rate() * TAIL_MS / 1000) as usize];
    let handle = thread::Builder::new().name("tts".into()).spawn(move || {
        for job in rx {
            match job {
                Job::Say { turn, text, instruct, lead, stop } => {
                    if stop.load(Relaxed) {
                        continue;
                    }
                    gate.chunk(turn, &text);
                    let mut on_pcm = |pcm: &[f32]| -> Result<()> {
                        let at = Instant::now();
                        if gate.push(turn, pcm) {
                            let _ = events.send(Event::TtsFirst { turn, at });
                        }
                        Ok(())
                    };
                    if let Err(e) = engine.synthesize(&text, instruct.as_deref(), Some(turn), lead, &stop, &mut on_pcm)
                    {
                        log::warn!("tts failed on {text:?}: {e:#}");
                    }
                }
                Job::End { turn } => {
                    gate.finish(turn, &tail);
                    let _ = events.send(Event::TtsEnd { turn });
                }
            }
        }
    })?;
    Ok((tx, handle))
}
