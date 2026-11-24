use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString, c_char};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::oneshot;

const APPROVE: &str = "approve";
const DECLINE: &str = "decline";

unsafe extern "C" {
    fn milo_notch_run(reply: extern "C" fn(*const c_char));
    fn milo_notch_post(message: *const c_char);
    fn milo_notch_level(source: i32, value: f32);
}

static ON: AtomicBool = AtomicBool::new(false);
static MUTED: AtomicBool = AtomicBool::new(false);
static SHOWN: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Mutex::default);
static NEXT: AtomicU64 = AtomicU64::new(1);
static WAITING: LazyLock<Mutex<HashMap<u64, oneshot::Sender<String>>>> = LazyLock::new(Mutex::default);

pub struct Action {
    pub id: String,
    pub label: String,
    pub primary: bool,
}

pub struct Card {
    pub title: String,
    pub body: String,
    pub actions: Vec<Action>,
}

extern "C" fn reply(message: *const c_char) {
    if message.is_null() {
        return;
    }
    let text = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    let Ok(value) = serde_json::from_str::<Value>(&text) else { return };
    if value["type"] == "mute" {
        MUTED.store(value["on"].as_bool().unwrap_or(false), Relaxed);
        return;
    }
    let (Some(id), Some(action)) = (value["id"].as_u64(), value["action"].as_str()) else { return };
    if let Some(tx) = WAITING.lock().unwrap().remove(&id) {
        let _ = tx.send(action.to_string());
    }
}

fn post(value: Value) {
    if !on() {
        return;
    }
    if let Ok(text) = CString::new(value.to_string()) {
        unsafe { milo_notch_post(text.as_ptr()) };
    }
}

pub fn run(work: impl FnOnce() + Send + 'static) {
    ON.store(true, Relaxed);
    std::thread::spawn(work);
    unsafe { milo_notch_run(reply) };
}

pub fn on() -> bool {
    ON.load(Relaxed)
}

fn level(source: i32, samples: &[f32]) {
    if !on() || samples.is_empty() {
        return;
    }
    let power = samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32;
    unsafe { milo_notch_level(source, power.sqrt()) };
}

pub fn heard(samples: &[f32]) {
    level(0, samples);
}

pub fn spoke(samples: &[f32]) {
    level(1, samples);
}

pub fn waiting() -> Vec<String> {
    let shown = SHOWN.lock().unwrap();
    let mut titles: Vec<String> = shown.iter().filter_map(|key| key.lines().next()).map(str::to_string).collect();
    titles.sort();
    titles
}

pub fn muted() -> bool {
    MUTED.load(Relaxed)
}

pub fn mute(on: bool) {
    post(json!({ "type": "mute", "on": on }));
}

pub fn status(state: &str) {
    post(json!({ "type": "status", "state": state }));
}

fn card(id: u64, card: &Card) -> Value {
    let actions: Vec<Value> =
        card.actions.iter().map(|a| json!({ "id": a.id, "label": a.label, "primary": a.primary })).collect();
    json!({
        "type": "card",
        "card": { "id": id, "title": card.title, "body": card.body, "actions": actions },
    })
}

pub fn show(shown: &Card) -> (u64, oneshot::Receiver<String>) {
    let id = NEXT.fetch_add(1, Relaxed);
    let (tx, rx) = oneshot::channel();
    WAITING.lock().unwrap().insert(id, tx);
    post(card(id, shown));
    (id, rx)
}

pub fn revise(id: u64, shown: &Card) {
    if WAITING.lock().unwrap().contains_key(&id) {
        post(card(id, shown));
    }
}

pub fn dismiss(id: u64) {
    WAITING.lock().unwrap().remove(&id);
    post(json!({ "type": "dismiss", "id": id }));
}

pub async fn ask(card: Card, wait: Duration) -> Option<String> {
    ask_unless(card, wait, std::future::pending()).await
}

pub async fn ask_unless(card: Card, wait: Duration, gone: impl Future<Output = ()>) -> Option<String> {
    if !on() {
        return None;
    }
    let key = format!("{}\n{}", card.title, card.body);
    if !SHOWN.lock().unwrap().insert(key.clone()) {
        return None;
    }
    let (id, rx) = show(&card);
    let answer = tokio::select! {
        answer = tokio::time::timeout(wait, rx) => answer.ok().and_then(Result::ok),
        _ = gone => None,
    };
    SHOWN.lock().unwrap().remove(&key);
    if answer.is_none() {
        dismiss(id);
    }
    answer
}

pub async fn approve_unless(title: &str, body: &str, wait: Duration, gone: impl Future<Output = ()>) -> Option<bool> {
    let card = Card {
        title: title.to_string(),
        body: body.to_string(),
        actions: vec![
            Action { id: DECLINE.into(), label: "Decline".into(), primary: false },
            Action { id: APPROVE.into(), label: "Approve".into(), primary: true },
        ],
    };
    ask_unless(card, wait, gone).await.map(|action| action == APPROVE)
}

pub async fn approve(title: &str, body: &str, wait: Duration) -> Option<bool> {
    let card = Card {
        title: title.to_string(),
        body: body.to_string(),
        actions: vec![
            Action { id: DECLINE.into(), label: "Decline".into(), primary: false },
            Action { id: APPROVE.into(), label: "Approve".into(), primary: true },
        ],
    };
    ask(card, wait).await.map(|action| action == APPROVE)
}
