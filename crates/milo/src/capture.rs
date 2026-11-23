use std::collections::VecDeque;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use audio::Capture;
use speech::Speech;
use tokio::sync::mpsc::UnboundedSender;
use turn::Detector;

use crate::controller::Event;

const CHECKPOINTS: usize = 1500;

pub struct AudioClock {
    ring: Mutex<VecDeque<(u64, Instant)>>,
}

impl AudioClock {
    pub fn new() -> AudioClock {
        AudioClock { ring: Mutex::new(VecDeque::with_capacity(CHECKPOINTS)) }
    }

    fn record(&self, samples: u64, at: Instant) {
        let mut ring = self.ring.lock().unwrap_or_else(|e| e.into_inner());
        if ring.len() == CHECKPOINTS {
            ring.pop_front();
        }
        ring.push_back((samples, at));
    }

    pub fn wall(&self, ms: u64) -> Option<Instant> {
        self.at(ms * 16)
    }

    fn at(&self, target: u64) -> Option<Instant> {
        let ring = self.ring.lock().unwrap_or_else(|e| e.into_inner());
        let i = ring.partition_point(|&(s, _)| s < target);
        let &(s, t) = ring.get(i).or(ring.back())?;
        if s >= target { t.checked_sub(samples(s - target)) } else { t.checked_add(samples(target - s)) }
    }
}

fn samples(n: u64) -> Duration {
    Duration::from_micros(n * 1000 / 16)
}

pub fn spawn(
    mut capture: Capture,
    speech: Arc<Speech>,
    clock: Arc<AudioClock>,
    mut detector: Option<Detector>,
    cleaned: Option<Receiver<Vec<f32>>>,
    events: UnboundedSender<Event>,
) -> std::io::Result<JoinHandle<()>> {
    if let Some(cleaned) = cleaned
        && let Some(mut detector) = detector.take()
    {
        let clock = clock.clone();
        thread::Builder::new().name("detect".into()).spawn(move || {
            let mut heard = 0u64;
            for audio in cleaned {
                heard += audio.len() as u64;
                detector.push(&audio, clock.at(heard).unwrap_or_else(Instant::now));
            }
        })?;
    }
    thread::Builder::new().name("capture".into()).spawn(move || {
        let mut mic = Vec::with_capacity(1024);
        let mut reference = Vec::with_capacity(1024);
        let mut pushed = 0u64;
        loop {
            match capture.read(&mut mic, &mut reference) {
                Ok(true) => {
                    let at = Instant::now();
                    speech.push(&mic, &reference);
                    pushed += mic.len() as u64;
                    clock.record(pushed, at);
                    if let Some(detector) = &mut detector {
                        detector.push(&mic, at);
                    }
                }
                Ok(false) => break,
                Err(e) => {
                    let _ = events.send(Event::Fatal(format!("{e:#}")));
                    break;
                }
            }
        }
    })
}
