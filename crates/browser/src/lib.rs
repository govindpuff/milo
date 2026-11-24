use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

const PAGE: &str = include_str!("page.js");
const HOMES: [(&str, &str); 7] = [
    ("Google Chrome", "Google/Chrome"),
    ("Brave Browser", "BraveSoftware/Brave-Browser"),
    ("Microsoft Edge", "Microsoft Edge"),
    ("Arc", "Arc/User Data"),
    ("Vivaldi", "Vivaldi"),
    ("Chromium", "Chromium"),
    ("Google Chrome Beta", "Google/Chrome Beta"),
];
const PROBE: Duration = Duration::from_millis(300);
const ANSWER: Duration = Duration::from_secs(15);
const SHOT: Duration = Duration::from_secs(4);
const QUICK: Duration = Duration::from_millis(1500);
const BEAT: Duration = Duration::from_millis(40);
const TURNED: Duration = Duration::from_millis(350);
const SENT: f64 = 640_000.0;
const QUALITY: u32 = 60;
const MINE: (u32, u32) = (1180, 860);
const PIECE: f64 = 3.0;
const SCROLL: f64 = 0.8;
const NOTE_CHARS: usize = 200;
pub const GONE: &str = "that number is not in the latest list";

type Waiting = Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>;

pub struct Tab {
    pub id: String,
    pub title: String,
    pub url: String,
    pub parent: Option<String>,
}

pub struct Seen {
    pub text: String,
    pub image: Option<String>,
}

pub struct Browser {
    name: String,
    out: mpsc::UnboundedSender<String>,
    waiting: Waiting,
    next: AtomicU64,
    live: Arc<AtomicBool>,
    sessions: Mutex<HashMap<String, String>>,
    notes: Arc<Mutex<Vec<String>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

fn homes() -> Vec<(String, u16, String)> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return Vec::new() };
    let mut out = Vec::new();
    for (name, folder) in HOMES {
        let file = home.join("Library/Application Support").join(folder).join("DevToolsActivePort");
        let Ok(text) = std::fs::read_to_string(file) else { continue };
        let mut lines = text.lines();
        let port = lines.next().and_then(|line| line.trim().parse::<u16>().ok());
        let path = lines.next().map(str::trim).unwrap_or_default();
        if let Some(port) = port
            && path.starts_with('/')
        {
            out.push((name.to_string(), port, path.to_string()));
        }
    }
    out
}

fn open(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), PROBE).is_ok()
}

pub fn found() -> Option<String> {
    homes().into_iter().find(|(_, port, _)| open(*port)).map(|(name, _, _)| name)
}

struct Key {
    key: String,
    code: String,
    number: u32,
    text: String,
}

fn key(name: &str) -> Option<Key> {
    let made = |key: &str, code: &str, number: u32, text: &str| {
        Some(Key { key: key.into(), code: code.into(), number, text: text.into() })
    };
    match name {
        "return" | "enter" => made("Enter", "Enter", 13, "\r"),
        "tab" => made("Tab", "Tab", 9, ""),
        "escape" | "esc" => made("Escape", "Escape", 27, ""),
        "space" => made(" ", "Space", 32, " "),
        "delete" | "backspace" => made("Backspace", "Backspace", 8, ""),
        "forwarddelete" => made("Delete", "Delete", 46, ""),
        "left" => made("ArrowLeft", "ArrowLeft", 37, ""),
        "up" => made("ArrowUp", "ArrowUp", 38, ""),
        "right" => made("ArrowRight", "ArrowRight", 39, ""),
        "down" => made("ArrowDown", "ArrowDown", 40, ""),
        "pageup" => made("PageUp", "PageUp", 33, ""),
        "pagedown" => made("PageDown", "PageDown", 34, ""),
        "home" => made("Home", "Home", 36, ""),
        "end" => made("End", "End", 35, ""),
        _ => {
            let mut letters = name.chars();
            let (Some(c), None) = (letters.next(), letters.next()) else { return None };
            let code = match c {
                'a'..='z' => format!("Key{}", c.to_ascii_uppercase()),
                '0'..='9' => format!("Digit{c}"),
                _ => String::new(),
            };
            made(&c.to_string(), &code, c.to_ascii_uppercase() as u32, &c.to_string())
        }
    }
}

impl Browser {
    pub async fn connect(wait: Duration) -> Result<Browser> {
        let Some((name, port, path)) = homes().into_iter().find(|(_, port, _)| open(*port)) else {
            bail!("no browser allows remote debugging");
        };
        let address = format!("ws://127.0.0.1:{port}{path}");
        let (socket, _) = tokio::time::timeout(wait, tokio_tungstenite::connect_async(address))
            .await
            .map_err(|_| anyhow!("{name} did not allow the connection in time"))??;
        let (mut sink, mut stream) = socket.split();
        let (out, mut queued) = mpsc::unbounded_channel::<String>();
        let waiting: Waiting = Arc::default();
        let live = Arc::new(AtomicBool::new(true));
        let notes: Arc<Mutex<Vec<String>>> = Arc::default();
        tokio::spawn(async move {
            while let Some(text) = queued.recv().await {
                if sink.send(Message::text(text)).await.is_err() {
                    break;
                }
            }
        });
        let (answers, alive, said, back) = (waiting.clone(), live.clone(), notes.clone(), out.clone());
        tokio::spawn(async move {
            while let Some(Ok(message)) = stream.next().await {
                let Message::Text(text) = message else { continue };
                let Ok(value) = serde_json::from_str::<Value>(&text) else { continue };
                if let Some(id) = value["id"].as_u64() {
                    if let Some(to) = lock(&answers).remove(&id) {
                        let _ = to.send(value);
                    }
                    continue;
                }
                if value["method"] == "Page.javascriptDialogOpening" {
                    let asked: String =
                        value["params"]["message"].as_str().unwrap_or_default().chars().take(NOTE_CHARS).collect();
                    let kind = value["params"]["type"].as_str().unwrap_or_default();
                    let accept = kind == "alert" || kind == "beforeunload";
                    if kind != "beforeunload" {
                        let how = if accept { "closed" } else { "dismissed without answering" };
                        lock(&said).push(format!("The page put up a message that said \"{asked}\". It was {how}."));
                    }
                    let _ = back.send(
                        json!({
                            "id": u64::MAX - 1,
                            "method": "Page.handleJavaScriptDialog",
                            "params": { "accept": accept },
                            "sessionId": value["sessionId"],
                        })
                        .to_string(),
                    );
                }
            }
            alive.store(false, Relaxed);
            lock(&answers).clear();
        });
        Ok(Browser { name, out, waiting, next: AtomicU64::new(1), live, sessions: Mutex::default(), notes })
    }

    pub fn alive(&self) -> bool {
        self.live.load(Relaxed)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    async fn send(&self, method: &str, params: Value, session: Option<&str>) -> Result<Value> {
        let id = self.next.fetch_add(1, Relaxed);
        let (to, from) = oneshot::channel();
        lock(&self.waiting).insert(id, to);
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session {
            message["sessionId"] = json!(session);
        }
        self.out.send(message.to_string()).map_err(|_| anyhow!("the browser connection closed"))?;
        let answer = match tokio::time::timeout(ANSWER, from).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(_)) => bail!("the browser connection closed"),
            Err(_) => {
                lock(&self.waiting).remove(&id);
                bail!("the browser did not answer");
            }
        };
        if let Some(error) = answer.get("error") {
            bail!("{}", error["message"].as_str().unwrap_or("the browser refused"));
        }
        Ok(answer["result"].clone())
    }

    async fn session(&self, tab: &str) -> Result<String> {
        if let Some(have) = lock(&self.sessions).get(tab) {
            return Ok(have.clone());
        }
        let got = self.send("Target.attachToTarget", json!({ "targetId": tab, "flatten": true }), None).await?;
        let session = got["sessionId"].as_str().ok_or_else(|| anyhow!("that tab is gone"))?.to_string();
        self.send("Page.enable", json!({}), Some(&session)).await?;
        let _ = self.send("Emulation.setFocusEmulationEnabled", json!({ "enabled": true }), Some(&session)).await;
        lock(&self.sessions).insert(tab.to_string(), session.clone());
        Ok(session)
    }

    async fn on(&self, tab: &str, method: &str, params: Value) -> Result<Value> {
        let session = self.session(tab).await?;
        let out = self.send(method, params, Some(&session)).await;
        if out.is_err() {
            lock(&self.sessions).remove(tab);
        }
        out
    }

    async fn page(&self, tab: &str, call: &str) -> Result<Value> {
        let expression = format!("({}).{call}", PAGE.trim());
        let got = self.on(tab, "Runtime.evaluate", json!({ "expression": expression, "returnByValue": true })).await?;
        if got.get("exceptionDetails").is_some() {
            bail!("the page could not be read");
        }
        Ok(got["result"]["value"].clone())
    }

    pub async fn tabs(&self) -> Result<Vec<Tab>> {
        let got = self.send("Target.getTargets", json!({}), None).await?;
        let mut out = Vec::new();
        for info in got["targetInfos"].as_array().into_iter().flatten() {
            let url = info["url"].as_str().unwrap_or_default();
            let web = ["http://", "https://", "file://", "about:blank", "chrome://newtab"];
            if info["type"] != "page" || !web.iter().any(|start| url.starts_with(start)) {
                continue;
            }
            out.push(Tab {
                id: info["targetId"].as_str().unwrap_or_default().to_string(),
                title: info["title"].as_str().unwrap_or_default().to_string(),
                url: url.to_string(),
                parent: info["openerId"].as_str().map(str::to_string),
            });
        }
        Ok(out)
    }

    pub async fn create(&self) -> Result<String> {
        let made = self
            .send("Target.createTarget", json!({ "url": "about:blank", "newWindow": true, "background": true }), None)
            .await?;
        let tab = made["targetId"].as_str().ok_or_else(|| anyhow!("the browser did not open a window"))?.to_string();
        if let Ok(held) = self.send("Browser.getWindowForTarget", json!({ "targetId": tab }), None).await {
            let bounds = json!({ "width": MINE.0, "height": MINE.1 });
            let _ = self
                .send("Browser.setWindowBounds", json!({ "windowId": held["windowId"], "bounds": bounds }), None)
                .await;
        }
        Ok(tab)
    }

    pub async fn close(&self, tab: &str) -> Result<()> {
        lock(&self.sessions).remove(tab);
        self.send("Target.closeTarget", json!({ "targetId": tab }), None).await.map(|_| ())
    }

    pub async fn show(&self, tab: &str) -> Result<()> {
        self.send("Target.activateTarget", json!({ "targetId": tab }), None).await.map(|_| ())
    }

    pub async fn goto(&self, tab: &str, address: &str) -> Result<()> {
        let got = self.on(tab, "Page.navigate", json!({ "url": address })).await?;
        match got["errorText"].as_str() {
            Some(text) if !text.is_empty() && text != "net::ERR_ABORTED" => bail!("the address did not load: {text}"),
            _ => Ok(()),
        }
    }

    pub async fn glance(&self, tab: &str) -> Result<(String, bool)> {
        let print = self.page(tab, "glance()").await?.as_str().unwrap_or_default().to_string();
        let loaded = print.split('|').nth(1) == Some("complete");
        Ok((print, loaded))
    }

    async fn picture(&self, tab: &str, clip: Value) -> Option<String> {
        let ask = self.on(tab, "Page.captureScreenshot", json!({ "format": "jpeg", "quality": QUALITY, "clip": clip }));
        let got = tokio::time::timeout(SHOT, ask).await.ok()?.ok()?;
        got["data"].as_str().map(str::to_string)
    }

    async fn size(&self, tab: &str) -> Result<[f64; 5]> {
        let got = self.page(tab, "size()").await?;
        let at = |i: usize| got[i].as_f64().unwrap_or(1.0).max(if i < 3 { 1.0 } else { 0.0 });
        Ok([at(0), at(1), at(2), at(3), at(4)])
    }

    pub async fn look(&self, tab: &str) -> Result<Seen> {
        let got = self.page(tab, "look()").await?;
        let (wide, tall) =
            (got["width"].as_f64().unwrap_or(1.0).max(1.0), got["height"].as_f64().unwrap_or(1.0).max(1.0));
        let ratio = got["ratio"].as_f64().unwrap_or(1.0).max(0.1);
        let mut text = format!(
            "Tab: {}\nAddress: {}",
            got["title"].as_str().unwrap_or_default(),
            got["url"].as_str().unwrap_or_default()
        );
        for note in lock(&self.notes).drain(..) {
            text.push('\n');
            text.push_str(&note);
        }
        let lines: Vec<&str> = got["lines"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        if lines.is_empty() {
            text.push_str(
                "\nNothing on this page could be listed. Use the picture, and click what you see by describing it.",
            );
        }
        for line in lines {
            text.push('\n');
            text.push_str(line);
        }
        let [_, _, _, left, top] = self.size(tab).await.unwrap_or([wide, tall, ratio, 0.0, 0.0]);
        let scale = (SENT / (wide * tall)).sqrt().min(1.0) / ratio;
        let clip = json!({ "x": left, "y": top, "width": wide, "height": tall, "scale": scale });
        Ok(Seen { text, image: self.picture(tab, clip).await })
    }

    pub async fn piece(&self, tab: &str, x: f64, y: f64) -> Result<(String, f64, f64, f64)> {
        let [wide, tall, ratio, left, top] = self.size(tab).await?;
        let share = 1.0 / PIECE;
        let (x, y) = ((x - share / 2.0).clamp(0.0, 1.0 - share), (y - share / 2.0).clamp(0.0, 1.0 - share));
        let (width, height) = (wide * share, tall * share);
        let scale = (SENT / (width * height)).sqrt().min(PIECE) / ratio;
        let clip =
            json!({ "x": left + x * wide, "y": top + y * tall, "width": width, "height": height, "scale": scale });
        let image = self.picture(tab, clip).await.ok_or_else(|| anyhow!("no picture"))?;
        Ok((image, x, y, share))
    }

    async fn mouse(&self, tab: &str, x: f64, y: f64) -> Result<()> {
        for kind in ["mouseMoved", "mousePressed", "mouseReleased"] {
            let buttons = if kind == "mousePressed" { 1 } else { 0 };
            let event = json!({ "type": kind, "x": x, "y": y, "button": if kind == "mouseMoved" { "none" } else { "left" }, "buttons": buttons, "clickCount": if kind == "mouseMoved" { 0 } else { 1 } });
            self.on(tab, "Input.dispatchMouseEvent", event).await?;
            tokio::time::sleep(BEAT).await;
        }
        Ok(())
    }

    async fn spot(&self, tab: &str, number: u64) -> Result<(f64, f64, bool)> {
        let got = self.page(tab, &format!("spot({number})")).await?;
        let (Some(x), Some(y)) = (got["x"].as_f64(), got["y"].as_f64()) else { bail!(GONE) };
        Ok((x, y, got["list"].as_bool().unwrap_or(false)))
    }

    pub async fn click(&self, tab: &str, number: u64) -> Result<()> {
        let (x, y, list) = self.spot(tab, number).await?;
        if list {
            bail!("that is a pop-up menu; to pick from it, use type with its number and the choice you want");
        }
        self.mouse(tab, x, y).await
    }

    pub async fn point(&self, tab: &str, x: f64, y: f64) -> Result<String> {
        let [wide, tall, ..] = self.size(tab).await?;
        let (x, y) = (x * wide, y * tall);
        let what = self.page(tab, &format!("under({x}, {y})")).await.ok();
        self.mouse(tab, x, y).await?;
        Ok(what.as_ref().and_then(Value::as_str).unwrap_or_default().to_string())
    }

    pub async fn write(&self, tab: &str, number: Option<u64>, text: &str) -> Result<Option<String>> {
        if let Some(number) = number {
            let (x, y, list) = self.spot(tab, number).await?;
            if list {
                let chosen = self.page(tab, &format!("choose({number}, {})", json!(text))).await?;
                return match chosen.as_str() {
                    Some(name) if !name.is_empty() => Ok(Some(name.to_string())),
                    _ => Err(anyhow!("that pop-up menu has no choice like {text}")),
                };
            }
            self.mouse(tab, x, y).await?;
            tokio::time::sleep(TURNED).await;
            if self.page(tab, &format!("ready({number})")).await?.as_str() == Some("gone") {
                self.spot(tab, number).await?;
            }
        }
        if text.is_empty() {
            self.stroke(tab, "delete", 0).await?;
        } else {
            self.on(tab, "Input.insertText", json!({ "text": text })).await?;
        }
        tokio::time::sleep(BEAT).await;
        let mut holds = self.page(tab, "holds()").await?;
        let landed = |holds: &Value| {
            holds.as_str().is_none_or(|held| held.contains(text.chars().take(20).collect::<String>().trim()))
        };
        if !landed(&holds) && self.page(tab, &format!("force({})", json!(text))).await?.as_bool() == Some(true) {
            holds = self.page(tab, "holds()").await?;
        }
        Ok(holds.as_str().map(str::to_string))
    }

    async fn stroke(&self, tab: &str, name: &str, modifiers: u32) -> Result<()> {
        let Some(key) = key(name) else { bail!("there is no key called {name}") };
        let held = modifiers & 6 != 0;
        let mut event = json!({
            "type": if held || key.text.is_empty() { "rawKeyDown" } else { "keyDown" },
            "key": key.key,
            "code": key.code,
            "modifiers": modifiers,
            "windowsVirtualKeyCode": key.number,
            "nativeVirtualKeyCode": key.number,
        });
        if !held && !key.text.is_empty() {
            event["text"] = json!(key.text);
        }
        let command = match (modifiers, name) {
            (4, "a") => "selectAll",
            (4, "c") => "copy",
            (4, "v") => "paste",
            (4, "x") => "cut",
            (4, "z") => "undo",
            (12, "z") => "redo",
            _ => "",
        };
        if !command.is_empty() {
            event["commands"] = json!([command]);
        }
        self.on(tab, "Input.dispatchKeyEvent", event.clone()).await?;
        event["type"] = json!("keyUp");
        event.as_object_mut().map(|all| {
            all.remove("text");
            all.remove("commands")
        });
        self.on(tab, "Input.dispatchKeyEvent", event).await.map(|_| ())
    }

    pub async fn press(&self, tab: &str, keys: &str) -> Result<()> {
        let lower = keys.to_lowercase().replace(' ', "");
        let mut modifiers = 0;
        let mut name = "";
        for part in lower.split('+').filter(|part| !part.is_empty()) {
            match part {
                "alt" | "option" | "opt" => modifiers |= 1,
                "ctrl" | "control" => modifiers |= 2,
                "cmd" | "command" | "meta" => modifiers |= 4,
                "shift" => modifiers |= 8,
                other => name = other,
            }
        }
        let script = match (modifiers, name) {
            (4, "[") | (4, "left") => "history.back()",
            (4, "]") | (4, "right") => "history.forward()",
            (4, "r") => "location.reload()",
            _ => "",
        };
        if !script.is_empty() {
            return self.on(tab, "Runtime.evaluate", json!({ "expression": script })).await.map(|_| ());
        }
        self.stroke(tab, name, modifiers).await
    }

    pub async fn scroll(&self, tab: &str, down: bool) -> Result<()> {
        let [wide, tall, ..] = self.size(tab).await?;
        let by = tall * SCROLL * if down { 1.0 } else { -1.0 };
        let (before, _) = self.glance(tab).await?;
        let event = json!({ "type": "mouseWheel", "x": wide / 2.0, "y": tall / 2.0, "deltaX": 0, "deltaY": by });
        let _ = self.on(tab, "Input.dispatchMouseEvent", event).await;
        tokio::time::sleep(TURNED).await;
        if self.glance(tab).await?.0 == before {
            self.page(tab, &format!("wheel({by})")).await?;
        }
        Ok(())
    }

    pub async fn peek(&self, tab: &str, expression: &str) -> Option<Value> {
        let attach = self.send("Target.attachToTarget", json!({ "targetId": tab, "flatten": true }), None);
        let got = tokio::time::timeout(QUICK, attach).await.ok()?.ok()?;
        let session = got["sessionId"].as_str()?.to_string();
        let ask = json!({ "expression": expression, "returnByValue": true, "userGesture": true });
        let out = tokio::time::timeout(QUICK, self.send("Runtime.evaluate", ask, Some(&session))).await;
        let _ = self.send("Target.detachFromTarget", json!({ "sessionId": session }), None).await;
        Some(out.ok()?.ok()?["result"]["value"].clone())
    }

    pub async fn media(&self, play: bool) -> Vec<String> {
        let script = match play {
            true => {
                "(() => { let n = 0; for (const m of document.querySelectorAll('video,audio')) if (m.dataset.miloPaused) { delete m.dataset.miloPaused; m.play(); n++; } return n; })()"
            }
            false => {
                "(() => { let n = 0; for (const m of document.querySelectorAll('video,audio')) if (!m.paused && !m.ended) { m.pause(); m.dataset.miloPaused = '1'; n++; } return n; })()"
            }
        };
        let tabs = self.tabs().await.unwrap_or_default();
        let asked = tabs.iter().filter(|tab| tab.url.starts_with("http")).map(|tab| async move {
            let hit = self.peek(&tab.id, script).await.and_then(|n| n.as_u64()).unwrap_or(0);
            (hit > 0).then(|| tab.title.clone())
        });
        futures_util::future::join_all(asked).await.into_iter().flatten().collect()
    }

    pub async fn read(&self, tab: &str) -> Result<String> {
        Ok(self.page(tab, "read()").await?.as_str().unwrap_or_default().to_string())
    }
}
