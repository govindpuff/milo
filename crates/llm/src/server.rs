use std::fs::File;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

const START_TIMEOUT: Duration = Duration::from_secs(20 * 60);
const STOP_GRACE: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(200);

const GLOBAL_FLAGS: &[&str] = &["--no-banner", "--no-telemetry", "--disable-version-check"];
const SERVE_FLAGS: &[&str] =
    &["--host", "127.0.0.1", "--disable-disk-caches", "--stream-interval", "1", "--hybrid-cache-entries", "16"];

const CACHE_ENV: &str = "RAPID_MLX_PREFIX_CACHE_MAX_BYTES";
const CACHE_GB_ENV: &str = "APC_MEMORY_MAX_GB";
const CACHE_SHARE: u64 = 5;
const RESERVE_ENV: &str = "APC_MEMORY_RESERVE_GB";
const RESERVE_GB: &str = "1";
const MEMORY_FALLBACK: u64 = 16 << 30;
const GIB: f64 = (1u64 << 30) as f64;

fn memory() -> u64 {
    let mut bytes: u64 = 0;
    let mut len = std::mem::size_of::<u64>();
    let ok = unsafe {
        libc::sysctlbyname(c"hw.memsize".as_ptr(), (&raw mut bytes).cast(), &mut len, std::ptr::null_mut(), 0) == 0
    };
    if ok && bytes > 0 { bytes } else { MEMORY_FALLBACK }
}

pub struct ServerGuard {
    child: Child,
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let pgid = self.child.id() as libc::pid_t;
        signal_group(pgid, libc::SIGTERM);
        let deadline = Instant::now() + STOP_GRACE;
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                _ => return,
            }
        }
        log::warn!("rapid-mlx did not exit after SIGTERM, killing");
        signal_group(pgid, libc::SIGKILL);
        let _ = self.child.wait();
    }
}

fn signal_group(pgid: libc::pid_t, sig: libc::c_int) {
    unsafe {
        libc::kill(-pgid, sig);
    }
}

pub fn ensure_running(model: &str, port: u16) -> Result<Option<ServerGuard>> {
    match probe(port) {
        Probe::Ready => return Ok(None),
        Probe::Busy => {
            log::info!("server on port {port} is not ready yet, waiting");
            wait_ready(port, None)?;
            return Ok(None);
        }
        Probe::Down => {}
    }

    let bin = find_binary()?;
    let log_path = std::env::temp_dir().join(format!("milo-rapid-mlx-{port}.log"));
    let log = File::create(&log_path).with_context(|| format!("create {}", log_path.display()))?;
    let port_arg = port.to_string();
    let ppid = std::process::id().to_string();
    let budget = memory() / CACHE_SHARE;
    let mut cmd = Command::new(&bin);
    cmd.args(GLOBAL_FLAGS)
        .arg("serve")
        .arg(model)
        .args(["--port", &port_arg])
        .args(SERVE_FLAGS)
        .args(["--watchdog-ppid", &ppid])
        .env("RAPID_MLX_TELEMETRY", "0")
        .env(CACHE_ENV, std::env::var(CACHE_ENV).unwrap_or_else(|_| budget.to_string()))
        .env(RESERVE_ENV, std::env::var(RESERVE_ENV).unwrap_or_else(|_| RESERVE_GB.to_string()))
        .env(CACHE_GB_ENV, std::env::var(CACHE_GB_ENV).unwrap_or_else(|_| format!("{:.2}", budget as f64 / GIB)))
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .process_group(0);
    log::info!("starting {} serve {model} on port {port}, log {}", bin.display(), log_path.display());
    let child = cmd.spawn().with_context(|| format!("spawn {}", bin.display()))?;
    let mut guard = ServerGuard { child };
    wait_ready(port, Some((&mut guard.child, &log_path)))?;
    Ok(Some(guard))
}

fn wait_ready(port: u16, mut spawned: Option<(&mut Child, &Path)>) -> Result<()> {
    let start = Instant::now();
    let mut last_note = start;
    loop {
        if let Probe::Ready = probe(port) {
            log::info!("rapid-mlx ready on port {port} after {:.1}s", start.elapsed().as_secs_f32());
            return Ok(());
        }
        if let Some((child, log_path)) = &mut spawned
            && let Some(status) = child.try_wait()?
        {
            bail!("rapid-mlx exited ({status}) before becoming ready:\n{}", tail(log_path, 30));
        }
        if start.elapsed() > START_TIMEOUT {
            bail!("rapid-mlx on port {port} not ready after {}s", START_TIMEOUT.as_secs());
        }
        if last_note.elapsed() > Duration::from_secs(15) {
            log::info!("waiting for rapid-mlx on port {port} ({:.0}s)", start.elapsed().as_secs_f32());
            last_note = Instant::now();
        }
        std::thread::sleep(POLL);
    }
}

enum Probe {
    Ready,
    Busy,
    Down,
}

fn probe(port: u16) -> Probe {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(500)) else {
        return Probe::Down;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let req = format!("GET /v1/models HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    if stream.write_all(req.as_bytes()).is_err() {
        return Probe::Busy;
    }
    let mut head = [0u8; 16];
    let mut n = 0;
    while n < 12 {
        match stream.read(&mut head[n..]) {
            Ok(0) | Err(_) => break,
            Ok(k) => n += k,
        }
    }
    if n >= 12 && head.starts_with(b"HTTP/") && head[9] == b'2' { Probe::Ready } else { Probe::Busy }
}

fn find_binary() -> Result<PathBuf> {
    const NAME: &str = "rapid-mlx";
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let p = dir.join(NAME);
            if p.is_file() {
                return Ok(p);
            }
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let p = PathBuf::from(home).join(".local/bin").join(NAME);
        if p.is_file() {
            return Ok(p);
        }
    }
    bail!(
        "{NAME} not found on PATH or in ~/.local/bin; install it with `uv tool install rapid-mlx==1.1.0`, or pick a hosted model with --model"
    )
}

fn tail(path: &Path, lines: usize) -> String {
    let text = std::fs::read(path).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}
