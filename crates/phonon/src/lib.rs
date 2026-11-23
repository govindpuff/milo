use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

use anyhow::{Context, Result, ensure};
use ptts::checkpoint::{Checkpoint, DEFAULT_MODEL, ResolveOptions};
use ptts::preprocess::Lang;
use ptts::synth::{DeviceKind, Quant, Session, SpeechOptions};
use ptts::voice_catalog::{DEFAULT_VOICE_CATALOG, VoiceCatalog};

pub const VOICE: &str = "Easton";
pub const QUANT: Quant = Quant::F32;
const THREADS: usize = 4;

pub struct Tts {
    session: Session,
    sample_rate: u32,
}

impl Tts {
    pub fn load() -> Result<Tts> {
        if std::env::var_os("RAYON_NUM_THREADS").is_none() {
            let cpus = std::thread::available_parallelism().map_or(1, |n| n.get());
            xn::set_num_threads(THREADS.min(cpus));
        }
        let options = ResolveOptions { quant: QUANT, weights: None };
        let checkpoint =
            Checkpoint::locate(DEFAULT_MODEL, options).with_context(|| format!("load Phonon model {DEFAULT_MODEL}"))?;
        let mut synth = checkpoint.builder(Lang::En).device(DeviceKind::Cpu).build()?;
        checkpoint.register_voices(&mut synth);
        let name = VoiceCatalog::new(DEFAULT_VOICE_CATALOG)?.register(&mut synth, VOICE)?;
        ensure!(
            synth.voices().contains(&name),
            "no voice '{name}': the model has {}, and the voice catalog has none by that name",
            synth.voices().join(", ")
        );
        let session = synth.session_default(&SpeechOptions::default().voice(name))?;
        Ok(Tts { session, sample_rate: synth.sample_rate() })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn synthesize(
        &self,
        text: &str,
        stop: &AtomicBool,
        on_pcm: &mut (dyn FnMut(&[f32]) -> Result<()> + Send),
    ) -> Result<()> {
        if text.trim().is_empty() || stop.load(Relaxed) {
            return Ok(());
        }
        let mut stream = self.session.stream(text)?;
        while !stop.load(Relaxed) {
            let Some(pcm) = stream.next() else { break };
            let pcm = pcm?;
            if stop.load(Relaxed) {
                break;
            }
            on_pcm(&pcm)?;
        }
        Ok(())
    }
}
