use std::fs::File;
use std::io::IsTerminal;
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::Relaxed;
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;

use crate::config::{self, Config};
use crate::controller::Event;
use crate::tools;

const RELEASES: &str = "https://api.github.com/repos/govindpuff/milo/releases/latest";
const WAIT: Duration = Duration::from_millis(300);
const CHECK: Duration = Duration::from_secs(10);
const RELAUNCH: &str = "while kill -0 \"$0\" 2>/dev/null; do sleep 0.1; done; exec /usr/bin/open \"$1\"";

static RESTART: AtomicBool = AtomicBool::new(false);
static STOPPED: AtomicBool = AtomicBool::new(false);
static QUIT: Mutex<Option<UnboundedSender<Event>>> = Mutex::new(None);

fn bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let app = exe.parent()?.parent()?.parent()?;
    (app.extension()? == "app").then(|| app.to_path_buf())
}

pub fn bundled() -> bool {
    bundle().is_some() && !std::io::stderr().is_terminal()
}

pub fn settle() -> Result<()> {
    let home = PathBuf::from(std::env::var("HOME").context("HOME is not set")?);
    let logs = home.join("Library/Logs/Milo");
    std::fs::create_dir_all(&logs).with_context(|| format!("create {}", logs.display()))?;
    let log = File::create(logs.join("milo.log")).context("create the log file")?;
    unsafe {
        libc::dup2(log.as_raw_fd(), libc::STDOUT_FILENO);
        libc::dup2(log.as_raw_fd(), libc::STDERR_FILENO);
    }
    let path = std::env::var("PATH").unwrap_or_default();
    let local = home.join(".local/bin");
    unsafe { std::env::set_var("PATH", format!("/opt/homebrew/bin:/usr/local/bin:{}:{path}", local.display())) };
    Ok(())
}

pub fn setup(names: &[&String]) -> Result<()> {
    if !bundled() {
        return Ok(());
    }
    let mut shown = false;
    loop {
        let config = Config::load()?;
        let wanted = names.iter().find_map(|name| config.wants(name));
        if wanted.is_some() || (!shown && voice::fresh()) {
            notch::post(json!({ "type": "setup", "provider": wanted }));
            shown = true;
        }
        let Some(id) = wanted else { return Ok(()) };
        while !Config::load()?.keyed(&id) {
            std::thread::sleep(WAIT);
        }
    }
}

pub fn listening(quit: UnboundedSender<Event>) -> bool {
    *QUIT.lock().unwrap() = Some(quit);
    !RESTART.load(Relaxed)
}

pub fn stopped() {
    STOPPED.store(true, Relaxed);
}

pub fn finish() {
    if RESTART.load(Relaxed) {
        again();
    }
}

fn restart() {
    RESTART.store(true, Relaxed);
    if STOPPED.load(Relaxed) {
        again();
    }
    if let Some(quit) = QUIT.lock().unwrap().as_ref() {
        let _ = quit.send(Event::Quit);
    }
}

fn again() -> ! {
    let exe = std::env::current_exe().unwrap_or_default();
    match bundle().filter(|_| bundled()) {
        Some(app) => {
            let waits = std::process::Command::new("/bin/sh")
                .args(["-c", RELAUNCH, &std::process::id().to_string()])
                .arg(app)
                .spawn();
            std::process::exit(if waits.is_ok() { 0 } else { 1 });
        }
        None => {
            let failed = std::process::Command::new(exe).args(std::env::args_os().skip(1)).exec();
            eprintln!("Error: could not start again: {failed}");
            std::process::exit(1);
        }
    }
}

fn state() -> Result<Value> {
    let config = Config::load()?;
    let (inputs, outputs) = audio::names().unwrap_or_default();
    Ok(json!({
        "type": "settings",
        "settings": config.settings(),
        "providers": config.hosted(),
        "inputs": inputs,
        "outputs": outputs,
        "voice": config::VOICE,
    }))
}

fn answer(message: &Value, port: u16) -> Result<()> {
    let id = message["provider"].as_str().unwrap_or_default();
    match message["type"].as_str() {
        Some("settings") => notch::post(state()?),
        Some("key") => {
            let (state, detail) = match config::enter(id, message["key"].as_str().unwrap_or_default(), port) {
                Ok(true) => ("saved", String::new()),
                Ok(false) => ("refused", String::new()),
                Err(e) => ("failed", format!("{e:#}")),
            };
            notch::post(json!({ "type": "key", "provider": id, "state": state, "detail": detail }));
            notch::post(self::state()?);
        }
        Some("models") => {
            let models = config::offered(id, port).unwrap_or_default();
            notch::post(json!({ "type": "models", "provider": id, "models": models }));
        }
        Some("save") => {
            let Some(settings) = message["settings"].as_object() else { return Ok(()) };
            let changed = config::update(settings)?;
            tools::unasked(Config::load()?.flag("auto_approve"));
            notch::post(state()?);
            if changed {
                restart();
            }
        }
        Some("restart") => match QUIT.lock().unwrap().is_some() {
            true => restart(),
            false => again(),
        },
        Some("quit") => match QUIT.lock().unwrap().as_ref().filter(|_| !STOPPED.load(Relaxed)) {
            Some(quit) => drop(quit.send(Event::Quit)),
            None => std::process::exit(0),
        },
        _ => {}
    }
    Ok(())
}

async fn update() -> Option<Value> {
    let http = reqwest::Client::builder().user_agent("milo").timeout(CHECK).build().ok()?;
    let latest: Value = http.get(RELEASES).send().await.ok()?.json().await.ok()?;
    let version = latest["tag_name"].as_str()?.trim_start_matches('v');
    let parts = |version: &str| version.split('.').map(|part| part.parse().unwrap_or(0)).collect::<Vec<u64>>();
    if parts(version) <= parts(env!("CARGO_PKG_VERSION")) {
        return None;
    }
    let files = latest["assets"].as_array().into_iter().flatten();
    let dmg = files.filter_map(|file| file["browser_download_url"].as_str()).find(|url| url.ends_with(".dmg"));
    Some(json!({ "type": "update", "version": version, "url": dmg.or(latest["html_url"].as_str())? }))
}

pub fn serve(port: u16) {
    let asked = notch::requests();
    std::thread::spawn(move || {
        for message in asked {
            if let Err(e) = answer(&message, port) {
                log::warn!("{e:#}");
            }
        }
    });
    if !bundled() {
        return;
    }
    std::thread::spawn(|| {
        let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
        if let Some(found) = rt.block_on(update()) {
            notch::post(found);
        }
    });
}
