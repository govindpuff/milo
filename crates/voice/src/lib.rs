use std::fs::File;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};

const SCRIPT: &str = include_str!("../../../sidecar/voice_server.py");
const ENGINE: &str = "breeze";
const START_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const SAMPLE_RATE: u32 = 24000;
pub const SOUNDS: &[&str] = &["laugh", "sigh", "cough", "clears throat"];

#[derive(Clone, Debug)]
pub struct VoiceConfig {
    pub design: String,
    pub port: u16,
    pub ref_wav: Option<PathBuf>,
    pub ref_text: Option<String>,
}

pub struct Voice {
    agent: ureq::Agent,
    url: String,
    child: Option<Child>,
}

fn root() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set"))?;
    Ok(PathBuf::from(home).join(".cache/milo"))
}

fn tail(path: &Path, lines: usize) -> String {
    let text = std::fs::read(path).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

impl Voice {
    pub fn start(cfg: &VoiceConfig) -> Result<Voice> {
        let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_millis(500)).build();
        let base = format!("http://127.0.0.1:{}", cfg.port);
        let health = format!("{base}/health");
        let url = format!("{base}/speak");
        if let Ok(response) = agent.get(&health).call() {
            let body: serde_json::Value = response.into_json().unwrap_or_default();
            if body["engine"].as_str() != Some(ENGINE) {
                bail!("port {} is already serving a different voice; stop it or pass another --tts-port", cfg.port);
            }
            return Ok(Voice { agent, url, child: None });
        }

        let root = root()?;
        let venv = root.join("venv");
        let python = venv.join("bin/python");
        if !python.is_file() {
            bail!("the voice environment {} is missing; run ./setup.sh", venv.display());
        }
        let reference = match (&cfg.ref_wav, &cfg.ref_text) {
            (Some(wav), Some(text)) => Some((wav, text)),
            (None, None) => None,
            _ => bail!("a reference voice needs both the clip and its exact transcript"),
        };
        if let Some((wav, _)) = &reference
            && !wav.is_file()
        {
            bail!("reference voice clip {} not found", wav.display());
        }
        let script = root.join("voice_server.py");
        std::fs::write(&script, SCRIPT).with_context(|| format!("write {}", script.display()))?;
        let log_path = std::env::temp_dir().join(format!("milo-voice-{}.log", cfg.port));
        let log = File::create(&log_path).with_context(|| format!("create {}", log_path.display()))?;
        let mut cmd = Command::new(&python);
        cmd.arg(&script)
            .args(["--port", &cfg.port.to_string()])
            .args(["--ppid", &std::process::id().to_string()])
            .arg("--voices-dir")
            .arg(root.join("voices"))
            .args(["--design", &cfg.design]);
        if let Some((wav, text)) = reference {
            cmd.arg("--ref-wav").arg(wav).arg("--ref-text").arg(text);
        }
        cmd.env("PYTHONUNBUFFERED", "1").stdin(Stdio::null()).stdout(log.try_clone()?).stderr(log).process_group(0);
        log::info!("starting the {ENGINE} voice on port {}, log {}", cfg.port, log_path.display());
        let child = cmd.spawn().with_context(|| format!("spawn {}", python.display()))?;
        let mut voice = Voice { agent, url, child: Some(child) };

        let start = Instant::now();
        let mut noted = start;
        loop {
            if voice.agent.get(&health).call().is_ok() {
                log::info!("voice ready after {:.1}s", start.elapsed().as_secs_f32());
                return Ok(voice);
            }
            if let Some(child) = voice.child.as_mut()
                && let Some(status) = child.try_wait()?
            {
                bail!("the {ENGINE} voice exited ({status}) before becoming ready:\n{}", tail(&log_path, 30));
            }
            if start.elapsed() > START_TIMEOUT {
                bail!("the {ENGINE} voice was not ready after {}s", START_TIMEOUT.as_secs());
            }
            if noted.elapsed() > Duration::from_secs(15) {
                let last = tail(&log_path, 1);
                log::info!("waiting for the voice ({:.0}s) {last}", start.elapsed().as_secs_f32());
                noted = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    pub fn sample_rate(&self) -> u32 {
        SAMPLE_RATE
    }

    pub fn lead(&self, id: usize, text: &str, pcm: &[f32]) -> Result<()> {
        let bytes: Vec<u8> = pcm.iter().flat_map(|s| ((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()).collect();
        self.agent
            .post(&self.url.replace("/speak", "/lead"))
            .query("id", &id.to_string())
            .query("text", text)
            .send_bytes(&bytes)
            .map_err(|e| anyhow!("voice lead failed: {e}"))?;
        Ok(())
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
        if text.trim().is_empty() || stop.load(Relaxed) {
            return Ok(());
        }
        let body =
            serde_json::json!({ "text": text, "instruct": instruct.unwrap_or(""), "session": session, "lead": lead });
        let response = match self.agent.post(&self.url).send_json(body.clone()) {
            Ok(r) => r,
            Err(_) => self.agent.post(&self.url).send_json(body).map_err(|e| anyhow!("voice request failed: {e}"))?,
        };
        let mut reader = response.into_reader();
        let mut buf = [0u8; 4096];
        let mut carry: Option<u8> = None;
        let mut pcm = Vec::with_capacity(2048);
        while !stop.load(Relaxed) {
            let n = reader.read(&mut buf).context("voice stream failed")?;
            if n == 0 {
                break;
            }
            pcm.clear();
            let mut bytes = buf[..n].iter().copied();
            if let Some(low) = carry.take()
                && let Some(high) = bytes.next()
            {
                pcm.push(i16::from_le_bytes([low, high]) as f32 / 32768.0);
            }
            while let Some(low) = bytes.next() {
                match bytes.next() {
                    Some(high) => pcm.push(i16::from_le_bytes([low, high]) as f32 / 32768.0),
                    None => carry = Some(low),
                }
            }
            if stop.load(Relaxed) {
                break;
            }
            if !pcm.is_empty() {
                on_pcm(&pcm)?;
            }
        }
        Ok(())
    }
}

impl Drop for Voice {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else { return };
        let pgid = child.id() as libc::pid_t;
        unsafe { libc::kill(-pgid, libc::SIGTERM) };
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if !matches!(child.try_wait(), Ok(None)) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        unsafe { libc::kill(-pgid, libc::SIGKILL) };
        let _ = child.wait();
    }
}
