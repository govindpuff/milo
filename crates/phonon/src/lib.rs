use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Mutex, PoisonError};

use anyhow::{Context, Result, ensure};
use ptts::checkpoint::{Checkpoint, DEFAULT_MODEL, ResolveOptions};
use ptts::preprocess::Lang;
use ptts::synth::{DeviceKind, Session, SpeechOptions, Synth};
use ptts::voice_catalog::{DEFAULT_VOICE_CATALOG, VoiceCatalog};

pub use ptts::synth::Quant;

const DEFAULT_THREADS: usize = 4;

#[derive(Debug, Clone)]
pub struct TtsConfig {
    pub quant: Quant,
    pub threads: Option<usize>,
}

pub struct Tts {
    synth: Mutex<Synth>,
    catalog: VoiceCatalog,
    sample_rate: u32,
}

pub struct Voice(Session);

impl Tts {
    pub fn load(cfg: &TtsConfig) -> Result<Tts> {
        let cpus = std::thread::available_parallelism().map_or(1, |n| n.get());
        match cfg.threads {
            Some(n) => xn::set_num_threads(n.max(1)),
            None if std::env::var_os("RAYON_NUM_THREADS").is_none() => xn::set_num_threads(DEFAULT_THREADS.min(cpus)),
            None => {}
        }
        let options = ResolveOptions { quant: cfg.quant, weights: None };
        let checkpoint =
            Checkpoint::locate(DEFAULT_MODEL, options).with_context(|| format!("load Phonon model {DEFAULT_MODEL}"))?;
        let mut synth = checkpoint.builder(Lang::En).device(DeviceKind::Cpu).build()?;
        checkpoint.register_voices(&mut synth);
        let catalog = VoiceCatalog::new(DEFAULT_VOICE_CATALOG)?;
        Ok(Tts { sample_rate: synth.sample_rate(), synth: Mutex::new(synth), catalog })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn voice(&self, source: &str) -> Result<Voice> {
        let mut synth = self.synth.lock().unwrap_or_else(PoisonError::into_inner);
        let name = if source.ends_with(".safetensors") {
            synth.add_voice_file(source, Path::new(source))?;
            source.to_string()
        } else {
            self.catalog.register(&mut synth, source)?
        };
        ensure!(
            synth.voices().contains(&name),
            "no voice '{name}': the model has {}, and the voice catalog has none by that name",
            synth.voices().join(", ")
        );
        Ok(Voice(synth.session_default(&SpeechOptions::default().voice(name))?))
    }

    pub fn synthesize(
        &self,
        voice: &Voice,
        text: &str,
        stop: &AtomicBool,
        on_pcm: &mut (dyn FnMut(&[f32]) -> Result<()> + Send),
    ) -> Result<()> {
        if text.trim().is_empty() || stop.load(Relaxed) {
            return Ok(());
        }
        let mut stream = voice.0.stream(text)?;
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
