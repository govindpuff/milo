use std::fs::File;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};

const SCRIPT: &str = include_str!("../../../sidecar/voice_server.py");
const NEEDS: &str = include_str!("../../../sidecar/requirements.txt");
const PYTHON: &str = "3.12";
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

pub fn fresh() -> bool {
    !root().is_ok_and(|root| root.join("venv/bin/python").is_file())
}

fn uv(args: &[&std::ffi::OsStr]) -> Result<()> {
    let beside = std::env::current_exe().ok().map(|exe| exe.with_file_name("uv")).filter(|uv| uv.is_file());
    let uv = beside.unwrap_or_else(|| PathBuf::from("uv"));
    let status = Command::new(&uv)
        .args(args)
        .stdin(Stdio::null())
        .status()
        .with_context(|| format!("could not run {}; run ./setup.sh", uv.display()))?;
    if !status.success() {
        bail!("{} {} failed ({status})", uv.display(), args[0].to_string_lossy());
    }
    Ok(())
}

pub fn prepare() -> Result<()> {
    install(&root()?)
}

fn install(root: &Path) -> Result<()> {
    static ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let venv = root.join("venv");
    let python = venv.join("bin/python");
    let have = venv.join("requirements.txt");
    if python.is_file() && std::fs::read_to_string(&have).is_ok_and(|have| have == NEEDS) {
        return Ok(());
    }
    log::info!("installing the voice in {}", venv.display());
    std::fs::create_dir_all(root).with_context(|| format!("create {}", root.display()))?;
    let needs = root.join("requirements.txt");
    std::fs::write(&needs, NEEDS).with_context(|| format!("write {}", needs.display()))?;
    if !python.is_file() {
        uv(&["venv".as_ref(), "--python".as_ref(), PYTHON.as_ref(), venv.as_os_str()])?;
    }
    uv(&[
        "pip".as_ref(),
        "install".as_ref(),
        "--python".as_ref(),
        python.as_os_str(),
        "-r".as_ref(),
        needs.as_os_str(),
    ])?;
    std::fs::write(&have, NEEDS).with_context(|| format!("write {}", have.display()))?;
    Ok(())
}

fn tail(path: &Path, lines: usize) -> String {
    let text = std::fs::read(path).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

fn named(cfg: &VoiceConfig) -> String {
    match (&cfg.ref_wav, &cfg.ref_text) {
        (Some(wav), Some(text)) => format!("{}|{text}", wav.display()),
        _ => cfg.design.clone(),
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_millis(500)).build()
}

fn spawn(cfg: &VoiceConfig) -> Result<(Child, PathBuf)> {
    let root = root()?;
    let python = root.join("venv/bin/python");
    install(&root).context("the voice could not be installed")?;
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
    Ok((child, log_path))
}

fn stop(mut child: Child) {
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

fn serving(cfg: &VoiceConfig, held: &mut Option<Child>) -> Result<serde_json::Value> {
    let agent = agent();
    let health = format!("http://127.0.0.1:{}/health", cfg.port);
    if let Ok(response) = agent.get(&health).call() {
        return Ok(response.into_json().unwrap_or_default());
    }
    let (child, log_path) = spawn(cfg)?;
    let child = held.insert(child);
    let start = Instant::now();
    let mut noted = start;
    loop {
        if let Ok(response) = agent.get(&health).call() {
            log::info!("voice ready after {:.1}s", start.elapsed().as_secs_f32());
            return Ok(response.into_json().unwrap_or_default());
        }
        let ended = child.try_wait()?;
        let late = start.elapsed() > START_TIMEOUT;
        if ended.is_some() || late {
            if let Some(child) = held.take() {
                stop(child);
            }
            match ended {
                Some(status) => {
                    bail!("the {ENGINE} voice exited ({status}) before becoming ready:\n{}", tail(&log_path, 30))
                }
                None => bail!("the {ENGINE} voice was not ready after {}s", START_TIMEOUT.as_secs()),
            }
        }
        if noted.elapsed() > Duration::from_secs(15) {
            let last = tail(&log_path, 1);
            log::info!("waiting for the voice ({:.0}s) {last}", start.elapsed().as_secs_f32());
            noted = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

static HELD: std::sync::Mutex<Option<Child>> = std::sync::Mutex::new(None);

pub fn preview(cfg: &VoiceConfig, design: &str) -> Result<PathBuf> {
    let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
    serving(cfg, &mut held)?;
    let made = agent()
        .post(&format!("http://127.0.0.1:{}/design", cfg.port))
        .send_json(serde_json::json!({ "design": design }))
        .map_err(|e| anyhow!("the voice could not be designed: {e}"))?;
    let made: serde_json::Value = made.into_json().unwrap_or_default();
    made["path"].as_str().map(PathBuf::from).ok_or_else(|| anyhow!("the voice could not be designed"))
}

impl Voice {
    pub fn start(cfg: &VoiceConfig) -> Result<Voice> {
        let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
        let state = serving(cfg, &mut held)?;
        let base = format!("http://127.0.0.1:{}", cfg.port);
        if state["engine"].as_str() != Some(ENGINE) {
            bail!("port {} is already serving a different voice; stop it or pass another --tts-port", cfg.port);
        }
        let agent = agent();
        if state["voice"].as_str() != Some(named(cfg).as_str()) {
            let wav = cfg.ref_wav.as_ref().map(|wav| wav.display().to_string());
            agent
                .post(&format!("{base}/voice"))
                .send_json(serde_json::json!({ "design": cfg.design, "ref_wav": wav, "ref_text": cfg.ref_text }))
                .map_err(|e| anyhow!("the voice on port {} could not be changed: {e}", cfg.port))?;
        }
        Ok(Voice { agent, url: format!("{base}/speak"), child: held.take() })
    }

    pub fn remote(&self) -> Voice {
        Voice { agent: self.agent.clone(), url: self.url.clone(), child: None }
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
        if let Some(child) = self.child.take() {
            stop(child);
        }
    }
}
