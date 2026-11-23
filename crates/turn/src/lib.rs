mod mel;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use ort::environment::Environment;
use ort::session::Session;
use ort::value::Tensor;

use crate::mel::Mel;

const VAD_URL: &str =
    "https://huggingface.co/onnx-community/silero-vad/resolve/e71cae966052b992a7eca6b17738916ce0eca4ec/onnx/model.onnx";
const TURN_URL: &str = "https://huggingface.co/pipecat-ai/smart-turn-v3/resolve/f766f81d3cfdf7737ac64aad813d91bbfd56bf93/smart-turn-v3.2-cpu.onnx";
const FRAME: usize = 512;
const CONTEXT: usize = 64;
const FRAME_MS: u32 = 32;
const START_FRAMES: u32 = 2;
const MAX_UTTERANCE: usize = 30 * 16000;
const EARLY_MS: u32 = 128;
const VAD_THRESHOLD: f32 = 0.5;
const STOP_MS: u32 = 200;

#[derive(Clone, Copy, Debug)]
pub struct TurnConfig {
    pub threshold: f32,
}

#[derive(Clone, Debug)]
pub enum TurnEvent {
    Start {
        seq: u64,
    },
    End {
        seq: u64,
        prob: Option<f32>,
        complete: Option<bool>,
        refining: bool,
        voiced_ms: u32,
        speech_end: Instant,
        at: Instant,
    },
    Refined {
        seq: u64,
        text: Option<String>,
    },
}

pub type Refine = Box<dyn Fn(&[f32]) -> Option<String> + Send>;

type Handler = Arc<dyn Fn(TurnEvent) + Send + Sync>;

struct Job {
    seq: u64,
    audio: Arc<Vec<f32>>,
    refining: bool,
    voiced_ms: u32,
    speech_end: Instant,
}

#[derive(Clone)]
pub struct Reset(Arc<AtomicBool>);

impl Reset {
    pub fn reset(&self) {
        self.0.store(true, Relaxed);
    }
}

pub struct Detector {
    vad: Session,
    state: Vec<f32>,
    context: [f32; CONTEXT],
    pending: Vec<f32>,
    history: Vec<f32>,
    speaking: bool,
    loud: u32,
    quiet: u32,
    last_speech: Instant,
    seq: u64,
    reset: Arc<AtomicBool>,
    truncated: bool,
    refine_sent: bool,
    voiced: u32,
    jobs: Sender<Job>,
    refine: Option<Sender<(u64, Arc<Vec<f32>>)>>,
    on_event: Handler,
}

fn fetch(url: &str, name: &str) -> Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set"))?;
    let dir = PathBuf::from(home).join(".cache/milo");
    let path = dir.join(name);
    if path.metadata().is_ok_and(|m| m.len() > 0) {
        return Ok(path);
    }
    std::fs::create_dir_all(&dir)?;
    log::info!("downloading {name}");
    let tmp = dir.join(format!("{name}.part"));
    let mut reader = ureq::get(url).call().with_context(|| format!("download {url}"))?.into_reader();
    let mut file = std::fs::File::create(&tmp)?;
    std::io::copy(&mut reader, &mut file).with_context(|| format!("download {url}"))?;
    drop(file);
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

fn session(env: &Environment, path: &Path) -> Result<Session> {
    let load = || -> std::result::Result<Session, String> {
        let builder = Session::builder(env).map_err(|e| e.to_string())?;
        let builder = builder.with_intra_threads(1).map_err(|e| e.to_string())?;
        let mut builder = builder.with_inter_threads(1).map_err(|e| e.to_string())?;
        builder.commit_from_file(path).map_err(|e| e.to_string())
    };
    load().map_err(|e| anyhow!("load {}: {e}", path.display()))
}

fn predict(session: &mut Session, mel: &Mel, audio: &[f32]) -> Result<f32> {
    let features = mel.features(audio);
    let input = Tensor::from_array(([1usize, mel::MELS, mel::FRAMES], features)).map_err(|e| anyhow!("{e}"))?;
    let outputs = session.run(ort::inputs!["input_features" => input]).map_err(|e| anyhow!("{e}"))?;
    let (_, data) = outputs[0].try_extract_tensor::<f32>().map_err(|e| anyhow!("{e}"))?;
    data.first().copied().ok_or_else(|| anyhow!("smart turn returned nothing"))
}

impl Detector {
    pub fn load(
        cfg: TurnConfig,
        on_event: Box<dyn Fn(TurnEvent) + Send + Sync>,
        refine: Option<Refine>,
    ) -> Result<(Detector, Reset)> {
        let env = ort::init().with_name("milo").build().map_err(|e| anyhow!("onnx runtime: {e}"))?;
        let vad = session(&env, &fetch(VAD_URL, "silero-vad.onnx")?)?;
        let mut turn = session(&env, &fetch(TURN_URL, "smart-turn-v3.2-cpu.onnx")?)?;
        let mel = Mel::new();
        predict(&mut turn, &mel, &[0.0; 16000])?;

        let on_event: Handler = Arc::from(on_event);
        let (jobs, rx) = mpsc::channel::<Job>();
        let emit = on_event.clone();
        thread::Builder::new().name("turn".into()).spawn(move || {
            for job in rx {
                let prob = match predict(&mut turn, &mel, &job.audio) {
                    Ok(p) => Some(p),
                    Err(e) => {
                        log::warn!("smart turn failed: {e:#}");
                        None
                    }
                };
                emit(TurnEvent::End {
                    seq: job.seq,
                    prob,
                    complete: prob.map(|p| p > cfg.threshold),
                    refining: job.refining,
                    voiced_ms: job.voiced_ms,
                    speech_end: job.speech_end,
                    at: Instant::now(),
                });
            }
        })?;

        let refine = match refine {
            Some(run) => {
                let (tx, rx) = mpsc::channel::<(u64, Arc<Vec<f32>>)>();
                let emit = on_event.clone();
                thread::Builder::new().name("refine".into()).spawn(move || {
                    for (seq, audio) in rx {
                        emit(TurnEvent::Refined { seq, text: run(&audio) });
                    }
                })?;
                Some(tx)
            }
            None => None,
        };

        let reset = Arc::new(AtomicBool::new(false));
        let now = Instant::now();
        let detector = Detector {
            vad,
            state: vec![0.0; 2 * 128],
            context: [0.0; CONTEXT],
            pending: Vec::with_capacity(2 * FRAME),
            history: Vec::with_capacity(MAX_UTTERANCE + 2 * FRAME),
            truncated: false,
            refine_sent: false,
            voiced: 0,
            speaking: false,
            loud: 0,
            quiet: 0,
            last_speech: now,
            seq: 0,
            reset: reset.clone(),
            jobs,
            refine,
            on_event,
        };
        Ok((detector, Reset(reset)))
    }

    pub fn push(&mut self, mic16k: &[f32], at: Instant) {
        if self.reset.swap(false, Relaxed) {
            self.history.clear();
            self.truncated = false;
        }
        self.pending.extend_from_slice(mic16k);
        while self.pending.len() >= FRAME {
            let behind = (self.pending.len() - FRAME) as u64;
            let frame_end = at.checked_sub(Duration::from_micros(behind * 1000 / 16)).unwrap_or(at);
            let frame: Vec<f32> = self.pending.drain(..FRAME).collect();
            self.frame(&frame, frame_end);
        }
    }

    fn frame(&mut self, frame: &[f32], at: Instant) {
        self.history.extend_from_slice(frame);
        if self.history.len() > MAX_UTTERANCE + FRAME {
            let cut = self.history.len() - MAX_UTTERANCE;
            self.history.drain(..cut);
            self.truncated = true;
        }
        let prob = match self.vad_prob(frame) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("vad failed: {e:#}");
                return;
            }
        };
        if prob >= VAD_THRESHOLD {
            self.loud += 1;
            self.quiet = 0;
            self.last_speech = at;
            if !self.speaking && self.loud >= START_FRAMES {
                self.speaking = true;
                self.voiced = self.loud;
                self.seq += 1;
                (self.on_event)(TurnEvent::Start { seq: self.seq });
            } else if self.speaking {
                self.voiced += 1;
                if self.refine_sent {
                    self.refine_sent = false;
                    self.seq += 1;
                    (self.on_event)(TurnEvent::Start { seq: self.seq });
                }
            }
        } else {
            self.loud = 0;
            if !self.speaking || prob >= VAD_THRESHOLD - 0.15 {
                return;
            }
            self.quiet += 1;
            let mut audio = None;
            if !self.refine_sent
                && !self.truncated
                && self.quiet * FRAME_MS >= EARLY_MS
                && let Some(tx) = &self.refine
            {
                let snapshot = Arc::new(self.history.clone());
                self.refine_sent = tx.send((self.seq, snapshot.clone())).is_ok();
                audio = Some(snapshot);
            }
            if self.quiet * FRAME_MS >= STOP_MS {
                self.speaking = false;
                self.quiet = 0;
                let job = Job {
                    seq: self.seq,
                    audio: audio.unwrap_or_else(|| Arc::new(self.history.clone())),
                    refining: self.refine_sent,
                    voiced_ms: self.voiced * FRAME_MS,
                    speech_end: self.last_speech,
                };
                self.refine_sent = false;
                let _ = self.jobs.send(job);
            }
        }
    }

    fn vad_prob(&mut self, frame: &[f32]) -> Result<f32> {
        let input = [&self.context[..], frame].concat();
        self.context.copy_from_slice(&frame[FRAME - CONTEXT..]);
        let input = Tensor::from_array(([1usize, CONTEXT + FRAME], input)).map_err(|e| anyhow!("{e}"))?;
        let state = Tensor::from_array(([2usize, 1, 128], self.state.clone())).map_err(|e| anyhow!("{e}"))?;
        let sr = Tensor::from_array(((), vec![16000i64])).map_err(|e| anyhow!("{e}"))?;
        let outputs =
            self.vad.run(ort::inputs!["input" => input, "state" => state, "sr" => sr]).map_err(|e| anyhow!("{e}"))?;
        let (_, prob) = outputs["output"].try_extract_tensor::<f32>().map_err(|e| anyhow!("{e}"))?;
        let prob = prob.first().copied().unwrap_or(0.0);
        let (_, next) = outputs["stateN"].try_extract_tensor::<f32>().map_err(|e| anyhow!("{e}"))?;
        if next.len() == self.state.len() {
            self.state.copy_from_slice(next);
        }
        Ok(prob)
    }
}
