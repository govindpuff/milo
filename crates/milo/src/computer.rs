use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use browser::Browser;
use llm::{Call, Llm, Message, Role};
use serde_json::{Map, Value, json};
use tokio::sync::mpsc::UnboundedSender;

use crate::controller::Event;
use crate::skills;
use crate::tools;

const MAX_STEPS: usize = 40;
const SETTLE: Duration = Duration::from_millis(250);
const QUICK: Duration = Duration::from_millis(110);
const PLAIN: Duration = Duration::from_millis(500);
const ARRIVE: Duration = Duration::from_secs(5);
const STILL: Duration = Duration::from_millis(300);
const SETTLED: Duration = Duration::from_secs(3);
const LOADED: Duration = Duration::from_secs(12);
const SCREEN: &str = "What is there now, as a list:\n";
const PICTURED: &str = "What is there now, as a picture and as a list:\n";
const SPARSE: usize = 5;
const REFUSALS: usize = 2;
const NAMED: usize = 24;
const NEAR: usize = 4;
const STOP: &str = "stop";
const MOVED: f64 = 0.8;
const SPOT: u8 = 8;
const STARTED: Duration = Duration::from_secs(15);
const FOLLOWED: Duration = Duration::from_secs(6);
const WAITED: Duration = Duration::from_secs(1);
pub const READ: &str = "the text of the page";
pub const LEAD: &str = "You are working on their computer now, on the plan you gave for";
const LOGGED: usize = 400;
const TABS: usize = 30;
const SAME: &str = "Nothing on the screen changed, so that did nothing. Do not repeat it. Try a different way, like a key, a menu item or another control.";
const PACE: &str = "Carry it out an action at a time, and look at what each one did. To save time, when the next few actions are all on things you can see now and none of them needs the result of another, like filling in several fields, or pressing a key several times, call them together in one turn, in the order they should happen. An action that sends, buys, deletes or submits something always goes alone, after you have seen that everything before it is right.";
const UNPLANNED: &str =
    "nothing was done: you are not working on their computer yet. Call computer first, with a short plan";
const TIRED: &str = "nothing was done: this has taken too many actions. Stop here and tell them what got in the way";
const FRONT: &str = "The window in front of them is shown below, as a picture and as a list";

const POINT: &str = "This is a picture of an app window. Give the bounding box of the thing described below as [ymin, xmin, ymax, xmax], normalized to 0-1000. Reply with only the box. If it is not in the picture, reply with only the word none.\n\n";
const UNMOVED: &str =
    ". The list of what is in the place is the same as before, so check the picture to see whether that did anything";
const STUCK: usize = 3;
const STRAY: &str = "<|\"|>";
const LOOPING: &str = " You have done exactly this before and ended up in the same place. Do not do it again. Do something different, or stop and say what got in the way.";
const LISTED: usize = 8;
const WHY: &str = "Write this first. One or two short sentences: which place you are in and what it shows now, which step of the plan you are on, and why this action is the next one.";
const GIVEN_UP: &str = " You have done this several times and ended up in the same place every time. Stop here and tell them what got in the way.";
const SKIPPED: &str = "nothing was done: the action before this one did not work, so this one was not tried. Look at what is there now and choose again";
const SHIFTED: &str = "nothing was done: the list changed after the action before this one, and that number is something else now. Use a number from the list below";
const UNOPENED: &str = "no app has been opened yet";
const UNSEEN: &str = "that could not be found in the picture; describe it another way, or use a number from the list";
const NOWHERE: &str = "You are not in any tab or window yet. Go to one with go";
const LOST: &str = "The place you were in is gone.";
const HOSTING: &str = " (Milo runs in this app)";
const SPRUNG: &str = ". It opened in a new tab, and you are in that tab now";
const UNLINKED: &str = "your own browser cannot be reached; work in a browser's window with go, click and press";
const OFF: &str = "none of the browsers that milo can run is installed (Chrome, Brave, Edge, Vivaldi or Chromium), so websites are read off the screen, which is slower and less exact";
const OWN: &str = " (your own browser, where websites open)";

static CURRENT: AtomicU64 = AtomicU64::new(0);
static BLIND: AtomicBool = AtomicBool::new(false);
static FRESH: AtomicBool = AtomicBool::new(true);
static EPOCH: AtomicU64 = AtomicU64::new(0);
static SHOWN: Mutex<Option<(String, Option<String>)>> = Mutex::new(None);
static SHOT: AtomicUsize = AtomicUsize::new(0);
static SESSION: tokio::sync::Mutex<Session> = tokio::sync::Mutex::const_new(Session {
    browser: None,
    place: Place::None,
    mine: None,
    tabs: Vec::new(),
    warned: false,
    refused: 0,
    lent: None,
    trail: Vec::new(),
    task: None,
    seen: None,
    paced: None,
    round: None,
});

#[derive(Clone, PartialEq)]
enum Place {
    None,
    Window,
    Tab(String),
}

struct Session {
    browser: Option<Browser>,
    place: Place,
    mine: Option<String>,
    tabs: Vec<String>,
    warned: bool,
    refused: usize,
    lent: Option<i64>,
    trail: Vec<skills::Step>,
    task: Option<Task>,
    seen: Option<Seen>,
    paced: Option<Instant>,
    round: Option<(Seen, bool)>,
}

struct Task {
    id: u64,
    name: String,
    show: bool,
    steps: usize,
    done: HashMap<String, Vec<String>>,
    briefed: Vec<String>,
}

#[derive(Clone)]
struct Seen {
    text: String,
    image: Option<String>,
    print: Vec<u8>,
    web: bool,
}

impl Seen {
    fn from(answer: &Value) -> Seen {
        let print = answer["print"].as_str().unwrap_or_default();
        Seen {
            text: answer["text"].as_str().unwrap_or_default().to_string(),
            image: answer["image"].as_str().map(str::to_string),
            print: (0..print.len() / 2).filter_map(|i| u8::from_str_radix(&print[i * 2..i * 2 + 2], 16).ok()).collect(),
            web: answer["web"] == true,
        }
    }

    fn plain(text: String) -> Seen {
        Seen { text, image: None, print: Vec::new(), web: false }
    }

    fn same(&self, other: &Seen) -> bool {
        if self.text != other.text {
            return false;
        }
        if self.print.is_empty() || self.print.len() != other.print.len() {
            return true;
        }
        let apart: u64 = self.print.iter().zip(&other.print).map(|(a, b)| a.abs_diff(*b) as u64).sum();
        let most = self.print.iter().zip(&other.print).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        (apart as f64 / self.print.len() as f64) < MOVED && most < SPOT
    }
}

pub fn running() -> bool {
    CURRENT.load(Relaxed) != 0
}

pub fn schema() -> Vec<Value> {
    let tool = |name: &str, description: &str, properties: Value, required: &[&str]| {
        let mut all = Map::new();
        all.insert("why".to_string(), json!({ "type": "string", "description": WHY }));
        all.extend(properties.as_object().cloned().unwrap_or_default());
        let properties = Value::Object(all);
        let required: Vec<&str> = std::iter::once("why").chain(required.iter().copied()).collect();
        json!({
            "type": "function",
            "function": {
                "name": name,
                "description": description,
                "parameters": {
                    "type": "object",
                    "properties": properties,
                    "required": required,
                    "additionalProperties": false,
                },
            },
        })
    };
    let number = json!({ "type": "integer", "description": "The number of a control in the latest list." });
    let tab = json!({ "type": "integer", "description": "The number of a tab in the list of open tabs." });
    let mut all = vec![
        tool(
            "go",
            "While you work on their computer: go to an app or a website, and work there.",
            json!({ "what": { "type": "string", "description": "An app name like Mail, or a web address like mail.google.com." } }),
            &["what"],
        ),
        tool(
            "click",
            "While you work on their computer: press a control by its number, or something in the picture by describing it.",
            json!({ "number": number, "what": { "type": "string", "description": "What the thing looks like and where it is, like the blue Send button at the bottom right." } }),
            &[],
        ),
        tool(
            "type",
            "While you work on their computer: type text into a control, or where the cursor is when no number is given.",
            json!({
                "text": { "type": "string", "description": "The exact text to type." },
                "number": number,
                "enter": { "type": "boolean", "description": "True to press return after the text." },
            }),
            &["text"],
        ),
        tool(
            "press",
            "While you work on their computer: press a key or a key combination.",
            json!({ "keys": { "type": "string", "description": "Like return, escape, tab, down, cmd+n, cmd+f." } }),
            &["keys"],
        ),
        tool(
            "scroll",
            "While you work on their computer: move down or up to see more.",
            json!({ "direction": { "type": "string", "enum": ["down", "up"] } }),
            &["direction"],
        ),
        tool("read", "While you work on their computer: get all the text of the web page you are on.", json!({}), &[]),
        tool(
            "menu",
            "While you work on their computer: choose an item from an app's menu bar.",
            json!({ "path": { "type": "string", "description": "The menu and the item, like File > New Window." } }),
            &["path"],
        ),
        tool(
            "look",
            "While you work on their computer: wait a moment and show the place again, with a picture of it.",
            json!({}),
            &[],
        ),
    ];
    all.push(tool(
        "tab",
        "While you work on their computer: go to a browser tab that is already open, and work there.",
        json!({ "number": tab }),
        &["number"],
    ));
    all
}

async fn control(request: Value) -> Result<Value, String> {
    tokio::task::spawn_blocking(move || control::call(&request))
        .await
        .unwrap_or_else(|_| Err("it did not work".to_string()))
}

async fn act(request: Value) -> Result<String, String> {
    control(request).await.map(|answer| answer["text"].as_str().unwrap_or_default().to_string())
}

fn saved() -> std::path::PathBuf {
    std::env::temp_dir().join("milo-computer")
}

fn next() -> String {
    let at = SHOT.fetch_add(1, Relaxed);
    saved().join(format!("{at:02}.jpg")).to_string_lossy().into_owned()
}

fn keep(seen: &Seen, did: &str) {
    let at = SHOT.load(Relaxed).saturating_sub(1);
    let _ = std::fs::write(saved().join(format!("{at:02}.txt")), format!("{did}\n\n{}\n", seen.text));
}

fn site(address: &str) -> &str {
    let rest = address.split_once("://").map_or(address, |(_, rest)| rest);
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    host.strip_prefix("www.").unwrap_or(host)
}

async fn front() -> Option<(i64, String)> {
    let apps = control(json!({ "op": "around" })).await.ok()?;
    let front = apps["apps"].as_array()?.iter().find(|app| app["front"] == true)?;
    Some((front["pid"].as_i64()?, front["name"].as_str().unwrap_or_default().to_string()))
}

async fn forward(pid: i64) {
    let _ = act(json!({ "op": "front", "pid": pid })).await;
}

async fn here(s: &Session) -> String {
    match &s.place {
        Place::None => String::new(),
        Place::Window => act(json!({ "op": "start" })).await.unwrap_or_default(),
        Place::Tab(tab) => {
            let Some(browser) = &s.browser else { return String::new() };
            let tabs = browser.tabs().await.unwrap_or_default();
            tabs.iter().find(|have| have.id == *tab).map(|have| site(&have.url).to_string()).unwrap_or_default()
        }
    }
}

async fn link(s: &mut Session) {
    if s.browser.as_ref().is_some_and(Browser::alive) {
        return;
    }
    s.browser = None;
    if s.refused >= REFUSALS {
        return;
    }
    if browser::found().is_none() {
        if !s.warned {
            log::warn!("{OFF}");
            s.warned = true;
        }
        return;
    }
    let back = front().await;
    let got = Browser::connect(STARTED).await;
    if let Some((back, _)) = back
        && front().await.is_some_and(|(ahead, _)| ahead != back)
    {
        forward(back).await;
    }
    match got {
        Ok(browser) => {
            log::info!("milo's own browser is {}", browser.name());
            s.refused = 0;
            s.browser = Some(browser);
        }
        Err(e) => {
            s.refused += 1;
            let after = if s.refused >= REFUSALS { ", and milo will not try again until it starts again" } else { "" };
            log::warn!(
                "milo's own browser could not be started, so websites are read off the screen instead{after}: {e:#}"
            );
        }
    }
}

async fn facing(s: &Session) -> (Vec<Value>, Option<String>) {
    let apps = match control(json!({ "op": "around" })).await {
        Ok(answer) => answer["apps"].as_array().cloned().unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    let Some(browser) = &s.browser else { return (apps, None) };
    let own = browser.pid();
    let title =
        apps.iter().find(|app| own.is_some() && app["pid"].as_i64() == own).and_then(|app| app["title"].as_str());
    let Some(title) = title.filter(|title| !title.is_empty()) else { return (apps, None) };
    let tabs = browser.tabs().await.unwrap_or_default();
    let shown = tabs
        .iter()
        .filter(|tab| !tab.title.is_empty() && title.starts_with(&tab.title))
        .max_by_key(|tab| tab.title.len());
    (apps, shown.map(|tab| tab.id.clone()))
}

async fn around(s: &mut Session) -> String {
    let (apps, shown) = facing(s).await;
    let mut out = String::from("Open on their Mac:");
    for app in &apps {
        out.push_str(&format!("\n- {}", app["name"].as_str().unwrap_or_default()));
        if app["front"] == true {
            out.push_str(" (in front)");
        }
        if app["host"] == true {
            out.push_str(HOSTING);
        }
        if s.browser.as_ref().is_some_and(|browser| browser.pid().is_some() && browser.pid() == app["pid"].as_i64()) {
            out.push_str(OWN);
        }
        if s.place == Place::Window && app["aimed"] == true {
            out.push_str(" (you are here)");
        }
    }
    let Some(browser) = &s.browser else { return out };
    let Ok(tabs) = browser.tabs().await else { return out };
    for tab in &tabs {
        if !s.tabs.contains(&tab.id) {
            s.tabs.push(tab.id.clone());
        }
    }
    out.push_str("\nTabs open in your own browser:");
    let mut any = 0;
    for (at, id) in s.tabs.iter().enumerate() {
        let Some(tab) = tabs.iter().find(|tab| tab.id == *id) else { continue };
        if any >= TABS {
            out.push_str("\n- and more");
            break;
        }
        any += 1;
        let title = if tab.title.is_empty() { "untitled" } else { tab.title.as_str() };
        out.push_str(&format!("\n- tab {}: {}, {}", at + 1, tools::clip(title), site(&tab.url)));
        if shown.as_deref() == Some(id.as_str()) {
            out.push_str(" (showing in its front window)");
        }
        if s.mine.as_deref() == Some(id.as_str()) {
            out.push_str(" (in the window you opened)");
        }
        if s.place == Place::Tab(id.clone()) {
            out.push_str(" (you are here)");
        }
    }
    if any == 0 {
        out.push_str("\n- none");
    }
    out
}

async fn screen(s: &mut Session) -> Seen {
    let path = next();
    match s.place.clone() {
        Place::Tab(tab) => {
            let got = match &s.browser {
                Some(browser) => browser.look(&tab).await.ok(),
                None => None,
            };
            match got {
                Some(seen) => {
                    if let Some(bytes) = seen.image.as_ref().and_then(|image| STANDARD.decode(image).ok()) {
                        let _ = std::fs::write(path, bytes);
                    }
                    Seen { text: seen.text, image: seen.image, print: Vec::new(), web: false }
                }
                None => {
                    s.place = Place::None;
                    Seen::plain(format!("{LOST} {NOWHERE}.\n\n{}", around(s).await))
                }
            }
        }
        Place::Window => match control(json!({ "op": "look", "save": path })).await {
            Ok(answer) => Seen::from(&answer),
            Err(text) if text.starts_with(UNOPENED) => {
                s.place = Place::None;
                Seen::plain(format!("{LOST} {NOWHERE}.\n\n{}", around(s).await))
            }
            Err(text) => Seen::plain(format!("the screen could not be read: {text}")),
        },
        Place::None => Seen::plain(format!("{NOWHERE}.\n\n{}", around(s).await)),
    }
}

async fn glimpse(s: &Session) -> Option<(Seen, bool)> {
    match &s.place {
        Place::None => None,
        Place::Tab(tab) => {
            let (print, loaded) = s.browser.as_ref()?.glance(tab).await.ok()?;
            Some((Seen::plain(print), loaded))
        }
        Place::Window => {
            let answer = control(json!({ "op": "glance" })).await.ok()?;
            Some((Seen::from(&answer), answer["loaded"].as_bool().unwrap_or(true)))
        }
    }
}

async fn settle(s: &Session, was: Option<&Seen>, most: Duration) {
    let began = Instant::now();
    let mut last: Option<Seen> = None;
    while began.elapsed() < most {
        tokio::time::sleep(QUICK).await;
        let Some((now, loaded)) = glimpse(s).await else { return };
        let moved = was.is_some_and(|was| !was.same(&now)) || began.elapsed() >= PLAIN;
        if loaded && moved && last.as_ref().is_some_and(|last| last.same(&now)) {
            return;
        }
        last = Some(now);
    }
}

fn whole(args: &Map<String, Value>) -> Option<u64> {
    match args.get("number")? {
        Value::Number(n) => n.as_u64().or_else(|| n.as_f64().map(|f| f as u64)),
        Value::String(s) => s.trim().trim_matches(['[', ']']).trim_start_matches("tab").trim().parse().ok(),
        _ => None,
    }
}

fn boxed(reply: &str) -> Option<(f64, f64)> {
    let inner = reply.split_once('[')?.1.split_once(']')?.0;
    let parts: Vec<f64> = inner.split(',').filter_map(|part| part.trim().parse().ok()).collect();
    let &[top, left, bottom, right] = parts.as_slice() else { return None };
    let within = |value: f64| (0.0..=1000.0).contains(&value);
    (within(top) && within(left) && within(bottom) && within(right))
        .then_some(((left + right) / 2000.0, (top + bottom) / 2000.0))
}

async fn closer(s: &Session, x: f64, y: f64) -> Option<(String, f64, f64, f64)> {
    match &s.place {
        Place::Tab(tab) => s.browser.as_ref()?.piece(tab, x, y).await.ok(),
        _ => {
            let near = control(json!({ "op": "closer", "x": x, "y": y })).await.ok()?;
            Some((
                near["image"].as_str()?.to_string(),
                near["left"].as_f64()?,
                near["top"].as_f64()?,
                near["share"].as_f64()?,
            ))
        }
    }
}

async fn find(llm: &Llm, s: &Session, what: &str, image: &str) -> Option<(f64, f64)> {
    let question = format!("{POINT}{what}");
    let (x, y) = llm.see(&question, image).await.ok().as_deref().and_then(boxed)?;
    let Some((piece, left, top, share)) = closer(s, x, y).await else { return Some((x, y)) };
    match llm.see(&question, &piece).await.ok().as_deref().and_then(boxed) {
        Some((nx, ny)) => Some((left + nx * share, top + ny * share)),
        None => Some((x, y)),
    }
}

fn line(seen: &Seen, number: u64) -> String {
    let tag = format!("[{number}] ");
    seen.text.lines().find_map(|line| line.strip_prefix(&tag)).unwrap_or("it").to_string()
}

fn stale<T>(got: &Result<T>) -> bool {
    got.as_ref().is_err_and(|e| format!("{e:#}").contains(browser::GONE))
}

fn stem(line: &str) -> &str {
    match line.rfind('"') {
        Some(at) => &line[..=at],
        None => line.split(" (").next().unwrap_or(line),
    }
}

async fn again(browser: &Browser, tab: &str, before: &Seen, number: u64) -> Option<u64> {
    let held = line(before, number);
    let wanted = stem(&held);
    let now = browser.look(tab).await.ok()?;
    let mut hits = now.text.lines().filter_map(|line| {
        let (number, rest) = line.strip_prefix('[')?.split_once("] ")?;
        (stem(rest) == wanted).then(|| number.parse::<u64>().ok()).flatten()
    });
    let first = hits.next()?;
    hits.next().is_none().then_some(first)
}

fn called(seen: &Seen, what: &str) -> Option<u64> {
    let wanted = what.trim().trim_matches('"').to_lowercase();
    if wanted.is_empty() {
        return None;
    }
    let mut hits = seen.text.lines().filter_map(|line| {
        let (number, rest) = line.strip_prefix('[')?.split_once("] ")?;
        let name = rest.split_once('"')?.1.split_once('"')?.0;
        (name.to_lowercase() == wanted).then(|| number.parse::<u64>().ok()).flatten()
    });
    let first = hits.next()?;
    hits.next().is_none().then_some(first)
}

async fn sprung(s: &mut Session, from: &str) -> bool {
    let Some(browser) = &s.browser else { return false };
    let Ok(tabs) = browser.tabs().await else { return false };
    let Some(new) = tabs.iter().find(|tab| tab.parent.as_deref() == Some(from) && !s.tabs.contains(&tab.id)) else {
        return false;
    };
    s.tabs.push(new.id.clone());
    s.place = Place::Tab(new.id.clone());
    true
}

async fn click(
    llm: &Llm,
    s: &mut Session,
    call: &Call,
    before: &Seen,
    was: Option<&Seen>,
) -> (Result<String, String>, Seen) {
    if s.place == Place::None {
        return (Err(format!("{NOWHERE} first")), screen(s).await);
    }
    let what = call.args.get("what").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let mut spot = None;
    let number = whole(&call.args).filter(|number| {
        let found = line(before, *number).to_lowercase();
        what.is_empty() || found.contains(&what.to_lowercase())
    });
    let number = number.or_else(|| called(before, &what));
    if number.is_none() && !what.is_empty() {
        let Some(image) = &before.image else {
            return (Err("there is no picture to find it in".to_string()), screen(s).await);
        };
        spot = find(llm, s, &what, image).await;
        if spot.is_none() {
            return (Err(UNSEEN.to_string()), screen(s).await);
        }
    }
    if number.is_none() && spot.is_none() {
        return (Err("give the number of a control, or say what to click".to_string()), screen(s).await);
    }
    if let Place::Tab(tab) = s.place.clone() {
        let Some(browser) = &s.browser else { return (Err(UNLINKED.to_string()), screen(s).await) };
        let began = Instant::now();
        let did = match (spot, number) {
            (Some((x, y)), _) => browser.point(&tab, x, y).await.map(|hit| match hit.is_empty() {
                true => format!("clicked {what}"),
                false => format!("clicked {hit}"),
            }),
            (None, Some(number)) => {
                let mut got = browser.click(&tab, number).await;
                if stale(&got)
                    && let Some(moved) = again(browser, &tab, before, number).await
                {
                    got = browser.click(&tab, moved).await;
                }
                got.map(|_| format!("clicked {}", line(before, number)))
            }
            (None, None) => Err(anyhow!("nothing to click")),
        };
        let mut did = did.map_err(|e| format!("{e:#}"));
        settle(s, was, FOLLOWED).await;
        tokio::time::sleep(STILL.saturating_sub(began.elapsed())).await;
        if did.is_ok() && sprung(s, &tab).await {
            did = did.map(|text| format!("{text}{SPRUNG}"));
            settle(s, None, FOLLOWED).await;
        }
        return (did, screen(s).await);
    }
    let mut did = Err("it could not be clicked".to_string());
    for mode in ["", "quiet", "real"] {
        let request = match spot {
            Some((x, y)) => json!({ "op": "click", "x": x, "y": y, "mode": mode }),
            None => json!({ "op": "click", "number": number, "mode": mode }),
        };
        match act(request).await {
            Ok(text) => did = Ok(text),
            Err(text) => {
                if did.is_err() {
                    did = Err(text);
                }
                break;
            }
        }
        settle(s, was, SETTLED).await;
        let after = screen(s).await;
        if !after.same(before) {
            return (did, after);
        }
    }
    (did, screen(s).await)
}

fn brief(did: &str) -> &str {
    let cut = [". Nothing", ". The list of", ". Gone from", ". New in", ". Most of", " You have done"]
        .iter()
        .filter_map(|mark| did.find(mark))
        .min();
    &did[..cut.unwrap_or(did.len())]
}

fn bare(line: &str) -> &str {
    match line.strip_prefix('[').and_then(|rest| rest.split_once("] ")) {
        Some((number, rest)) if number.chars().all(|c| c.is_ascii_digit()) => rest,
        _ => line,
    }
}

fn changed(before: &Seen, after: &Seen) -> String {
    let only = |from: &Seen, other: &Seen| -> Vec<String> {
        let mut have: HashMap<&str, i32> = HashMap::new();
        for line in other.text.lines().map(bare) {
            *have.entry(line).or_default() += 1;
        }
        let mut out = Vec::new();
        for line in from.text.lines().map(bare) {
            let left = have.entry(line).or_default();
            *left -= 1;
            if *left < 0 && !line.trim().is_empty() {
                out.push(line.to_string());
            }
        }
        out
    };
    let gone = only(before, after);
    let new = only(after, before);
    if gone.len() + new.len() == 0 {
        return UNMOVED.to_string();
    }
    if gone.len() > LISTED || new.len() > LISTED {
        let title = after.text.lines().find(|line| line.starts_with("Window: ") || line.starts_with("Tab: "));
        return format!(". Most of what is shown changed. {}", title.unwrap_or_default());
    }
    let mut out = String::new();
    if !gone.is_empty() {
        out.push_str(&format!(". Gone from the window: {}", gone.join("; ")));
    }
    if !new.is_empty() {
        out.push_str(&format!(". New in the window: {}", new.join("; ")));
    }
    out
}

async fn exists(s: &Session, tab: &str) -> bool {
    match &s.browser {
        Some(browser) => browser.tabs().await.is_ok_and(|tabs| tabs.iter().any(|have| have.id == tab)),
        None => false,
    }
}

fn plain(address: &str) -> String {
    let address = address.split('#').next().unwrap_or(address);
    let address = address.split_once("://").map_or(address, |(_, rest)| rest);
    address.strip_prefix("www.").unwrap_or(address).trim_end_matches('/').to_lowercase()
}

async fn visit(s: &mut Session, what: &str) -> Result<String> {
    let address = if what.contains("://") { what.to_string() } else { format!("https://{what}") };
    let kept = match s.mine.clone() {
        Some(tab) if exists(s, &tab).await => Some(tab),
        _ => None,
    };
    let Some(browser) = &s.browser else { bail!(UNLINKED) };
    let open = browser.tabs().await.unwrap_or_default();
    if let Some(have) = open.iter().find(|have| plain(&have.url) == plain(&address)) {
        if !s.tabs.contains(&have.id) {
            s.tabs.push(have.id.clone());
        }
        s.place = Place::Tab(have.id.clone());
        return Ok(format!("{what} was already open, and you are in that tab now: {}", tools::clip(&have.title)));
    }
    let tab = match kept {
        Some(tab) => tab,
        None => browser.create().await?,
    };
    s.mine = Some(tab.clone());
    if !s.tabs.contains(&tab) {
        s.tabs.push(tab.clone());
    }
    browser.goto(&tab, &address).await?;
    s.place = Place::Tab(tab);
    Ok(format!("opened {what} in a browser window of your own"))
}

async fn launch(s: &mut Session, what: &str, web: bool) -> Result<String, String> {
    let was = control(json!({ "op": "glance" })).await.ok().map(|answer| Seen::from(&answer));
    let mut did = match web {
        true => act(json!({ "op": "visit", "what": what })).await,
        false => Err(String::new()),
    };
    if did.is_err() {
        let _ = act(json!({ "op": "hold" })).await;
        did = act(json!({ "op": "aim", "what": what, "wait": false })).await;
    }
    if did.is_err() {
        let name = match web {
            true => what.to_string(),
            false => act(json!({ "op": "installed", "what": what })).await.unwrap_or_else(|_| what.to_string()),
        };
        did = tools::launch(&name, true).await.map_err(|e| format!("{e:#}"));
        if did.is_ok()
            && let Err(e) = act(json!({ "op": "aim", "what": name })).await
        {
            did = Err(e);
        }
    }
    s.place = match act(json!({ "op": "start" })).await {
        Ok(name) if !name.is_empty() => {
            let ahead = front().await;
            if ahead.as_ref().is_none_or(|(_, ahead)| *ahead != name) {
                if s.lent.is_none() {
                    s.lent = ahead.map(|(pid, _)| pid);
                }
                let _ = act(json!({ "op": "raise" })).await;
            }
            Place::Window
        }
        _ => Place::None,
    };
    let until = Instant::now() + ARRIVE;
    while s.place == Place::Window && Instant::now() < until {
        tokio::time::sleep(STILL).await;
        let now = control(json!({ "op": "glance" })).await.ok().map(|answer| Seen::from(&answer));
        match (&was, &now) {
            (Some(was), Some(now)) if was.same(now) => {}
            _ => break,
        }
    }
    did
}

async fn enter(s: &mut Session, what: &str) -> Result<String, String> {
    let web = what.contains("://") || (what.contains('.') && !what.contains(' '));
    if web {
        link(s).await;
        if s.browser.is_some() {
            return visit(s, what).await.map_err(|e| format!("{e:#}"));
        }
    }
    launch(s, what, web).await
}

async fn step(llm: &Llm, s: &mut Session, call: &Call, before: &Seen) -> (String, Seen, bool) {
    let text = |key: &str| call.args.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let name = call.name.as_str();
    let nowhere = || Err(format!("{NOWHERE} first"));
    let place = s.place.clone();
    let was = match name {
        "click" | "type" | "press" | "scroll" | "menu" => glimpse(s).await.map(|(seen, _)| seen),
        _ => None,
    };
    let was = was.as_ref();
    let (did, after) = match name {
        "look" => {
            tokio::time::sleep(WAITED).await;
            (Ok("looked again".to_string()), screen(s).await)
        }
        "click" => click(llm, s, call, before, was).await,
        "go" => {
            let did = enter(s, &text("what")).await;
            settle(s, None, LOADED).await;
            (did, screen(s).await)
        }
        "tab" | "close_tab" => {
            let chosen = whole(&call.args).and_then(|number| s.tabs.get(number.saturating_sub(1) as usize).cloned());
            let chosen = match (chosen, &place, name) {
                (Some(tab), _, _) => Some(tab),
                (None, Place::Tab(tab), "close_tab") if whole(&call.args).is_none() => Some(tab.clone()),
                _ => None,
            };
            let did = match (chosen, &s.browser) {
                (_, None) => Err(UNLINKED.to_string()),
                (None, _) => Err("give the number of a tab from the list of open tabs".to_string()),
                (Some(tab), Some(browser)) => {
                    let all = browser.tabs().await.unwrap_or_default();
                    match all.iter().find(|have| have.id == tab) {
                        None => Err("that tab is no longer open".to_string()),
                        Some(have) if name == "tab" => {
                            let said = format!("now in the tab {}", tools::clip(&have.title));
                            s.place = Place::Tab(tab);
                            Ok(said)
                        }
                        Some(have) => {
                            let said = format!(
                                "closed the tab {}. That tab is gone now. Do not close another tab unless they asked for more than one",
                                tools::clip(&have.title)
                            );
                            let done = browser.close(&tab).await.map(|_| said).map_err(|e| format!("{e:#}"));
                            if done.is_ok() {
                                if s.place == Place::Tab(tab.clone()) {
                                    s.place = Place::None;
                                }
                                if s.mine.as_deref() == Some(tab.as_str()) {
                                    s.mine = None;
                                }
                            }
                            done
                        }
                    }
                }
            };
            settle(s, None, SETTLED).await;
            (did, screen(s).await)
        }
        "type" => {
            let (words, number, enter) =
                (text("text"), whole(&call.args), call.args.get("enter") == Some(&json!(true)));
            let did = match (&place, &s.browser) {
                (Place::None, _) => nowhere(),
                (Place::Tab(tab), Some(browser)) => {
                    let into =
                        number.map(|number| line(before, number)).unwrap_or_else(|| "where the cursor is".to_string());
                    let mut wrote = browser.write(tab, number, &words).await;
                    if stale(&wrote)
                        && let Some(number) = number
                        && let Some(moved) = again(browser, tab, before, number).await
                    {
                        wrote = browser.write(tab, Some(moved), &words).await;
                    }
                    let mut did = wrote.map(|held| match held {
                        Some(held) => format!("typed \"{words}\" into {into}. It now contains \"{held}\""),
                        None => format!("typed \"{words}\" into {into}"),
                    });
                    if did.is_ok() && enter {
                        tokio::time::sleep(SETTLE).await;
                        did = browser
                            .press(tab, "return")
                            .await
                            .and(did)
                            .map(|text| format!("{text}, and pressed return"));
                    }
                    did.map_err(|e| format!("{e:#}"))
                }
                (Place::Tab(_), None) => Err(UNLINKED.to_string()),
                (Place::Window, _) => {
                    let mut did = act(json!({ "op": "type", "number": number, "text": words })).await;
                    if did.is_ok() && enter {
                        tokio::time::sleep(SETTLE).await;
                        did = act(json!({ "op": "press", "keys": "return" }))
                            .await
                            .and(did)
                            .map(|text| format!("{text}, and pressed return"));
                    }
                    did
                }
            };
            settle(s, was, if enter { FOLLOWED } else { SETTLED }).await;
            (did, screen(s).await)
        }
        "press" | "scroll" | "menu" | "read" => {
            let keys = text("keys");
            let down = text("direction") != "up";
            let did = match (&place, &s.browser) {
                (Place::None, _) => nowhere(),
                (Place::Tab(_), None) => Err(UNLINKED.to_string()),
                (Place::Tab(tab), Some(browser)) => {
                    let combo = keys.to_lowercase().replace(' ', "").replace("command", "cmd");
                    let did = match name {
                        "press" if ["cmd+t", "cmd+n", "cmd+l", "cmd+w"].contains(&combo.as_str()) => {
                            Err(anyhow!("that key does not reach the browser; use go, tab or close_tab instead"))
                        }
                        "press" => browser.press(tab, &keys).await.map(|_| format!("pressed {keys}")),
                        "scroll" => browser
                            .scroll(tab, down)
                            .await
                            .map(|_| format!("scrolled {}", if down { "down" } else { "up" })),
                        "read" => browser.read(tab).await.map(|all| format!("{READ}:\n{all}\n")),
                        _ => Err(anyhow!("a web page has no menu bar; use the controls on the page")),
                    };
                    did.map_err(|e| format!("{e:#}"))
                }
                (Place::Window, _) => match name {
                    "press" => act(json!({ "op": "press", "keys": keys })).await,
                    "scroll" => act(json!({ "op": "press", "keys": if down { "pagedown" } else { "pageup" } }))
                        .await
                        .map(|_| format!("scrolled {}", if down { "down" } else { "up" })),
                    "read" => Ok("everything that can be read in this window is already in the list".to_string()),
                    _ => act(json!({ "op": "menu", "path": text("path") })).await,
                },
            };
            if name != "read" {
                settle(s, was, SETTLED).await;
            }
            (did, screen(s).await)
        }
        _ => (Err("there is no such tool".to_string()), screen(s).await),
    };
    let mut after = after;
    let quiet = ["look", "read"].contains(&name);
    let failed = did.is_err();
    let mut did = match did {
        Ok(text) if !quiet && s.place == Place::Window && place == Place::Window && after.same(before) => {
            if s.lent.is_none() {
                s.lent = front().await.map(|(pid, _)| pid);
            }
            let _ = act(json!({ "op": "raise" })).await;
            settle(s, None, SETTLED).await;
            after = screen(s).await;
            text
        }
        Ok(text) => text,
        Err(e) => format!("{name} failed: {e}"),
    };
    let went = name == "type" && did.contains("It now contains");
    let chosen = name == "click"
        && whole(&call.args).is_some_and(|number| line(before, number).contains("(selected)"))
        && after.same(before);
    if chosen {
        did = format!("{did}. It was already selected, so nothing changed. Go on with the next thing");
    }
    let went = went || chosen;
    if !quiet && !went && after.same(before) {
        did = format!("{did}. {SAME}");
    } else if !quiet && place == s.place {
        did.push_str(&changed(before, &after));
    }
    keep(&after, &format!("{name} {} -> {did}", Value::Object(call.args.clone())));
    let wasted = failed || (!quiet && !went && after.same(before));
    (did, after, wasted)
}

fn card(task: &str, doing: &str) -> notch::Card {
    notch::Card {
        title: format!("Working: {task}"),
        body: doing.to_string(),
        actions: vec![notch::Action { id: STOP.into(), label: "Stop".into(), primary: false }],
    }
}

async fn arrive(s: &mut Session) {
    s.tabs.clear();
    s.place = match s.place.clone() {
        Place::Tab(tab) if exists(s, &tab).await => Place::Tab(tab),
        Place::Window if act(json!({ "op": "start" })).await.is_ok_and(|name| !name.is_empty()) => Place::Window,
        _ => Place::None,
    };
    if s.place != Place::Window {
        let _ = act(json!({ "op": "drop" })).await;
    }
}

fn squeeze(text: &str) -> String {
    let mut recent: VecDeque<(&str, &str)> = VecDeque::new();
    let mut out = Vec::new();
    for line in text.lines() {
        let number = line.strip_prefix('[').and_then(|rest| rest.split_once("] ")).map(|(number, _)| number);
        let name =
            line.find('"').zip(line.rfind('"')).filter(|(from, to)| to > from).map(|(from, to)| &line[from..=to]);
        let (Some(number), Some(name)) = (number, name) else {
            out.push(line.to_string());
            continue;
        };
        match recent.iter().find(|(_, had)| *had == name) {
            Some((first, _)) if name.chars().count() >= NAMED => {
                out.push(line.replacen(name, &format!("same name as [{first}]"), 1));
            }
            _ => out.push(line.to_string()),
        }
        recent.push_back((number, name));
        if recent.len() > NEAR {
            recent.pop_front();
        }
    }
    out.join("\n")
}

fn titled(seen: &Seen) -> Vec<&str> {
    seen.text
        .lines()
        .filter(|line| ["Tab: ", "Window: ", "App: "].iter().any(|start| line.starts_with(start)))
        .collect()
}

fn show(seen: &Seen, pictured: bool) {
    let sparse = seen.text.lines().filter(|line| line.starts_with('[')).count() < SPARSE;
    let image = seen.image.clone().filter(|_| pictured || sparse || seen.web);
    *SHOWN.lock().unwrap_or_else(|e| e.into_inner()) = Some((squeeze(&seen.text), image));
}

fn hide() {
    *SHOWN.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

pub fn blind() {
    BLIND.store(true, Relaxed);
}

pub fn epoch() -> u64 {
    EPOCH.load(Relaxed)
}

pub fn compose(history: &[Message]) -> Vec<Message> {
    let mut out = history.to_vec();
    let Some((text, image)) = SHOWN.lock().unwrap_or_else(|e| e.into_inner()).clone() else { return out };
    let image = image.filter(|_| !BLIND.load(Relaxed));
    let lead = if image.is_some() { PICTURED } else { SCREEN };
    match out.last_mut() {
        Some(last) if last.role == Role::User && last.image.is_none() => {
            last.content.push_str(&format!("\n\n{lead}{text}"));
            last.image = image;
        }
        _ => out.push(Message::seen(format!("{lead}{text}"), image)),
    }
    out
}

pub async fn begin(events: &UnboundedSender<Event>, task: &str, steps: &[String], shown: bool) -> Result<String> {
    let mut held = SESSION.lock().await;
    let s = &mut *held;
    if let Some(old) = s.task.take() {
        notch::dismiss(old.id);
    }
    act(json!({ "op": "start" })).await.map_err(|e| anyhow!("{e}"))?;
    let _ = std::fs::remove_dir_all(saved());
    let _ = std::fs::create_dir_all(saved());
    SHOT.store(0, Relaxed);
    s.trail.clear();
    s.lent = None;
    let (id, stopped) = notch::show(&card(task, "Starting"));
    let events = events.clone();
    tokio::spawn(async move {
        if stopped.await.is_ok() {
            let _ = events.send(Event::Halt);
        }
    });
    link(s).await;
    arrive(s).await;
    let mut lead = format!("{LEAD} {task}. {PACE}");
    let before = screen(s).await;
    if s.place != Place::None {
        lead.push_str(&format!("\n\n{}", around(s).await));
    }
    let mut briefed: Vec<String> = Vec::new();
    let asked = format!("{task}\n{}", steps.join("\n"));
    for place in std::iter::once(here(s).await).chain(skills::named(&asked)) {
        if briefed.contains(&skills::slug(&place)) {
            continue;
        }
        if let Some(notes) = skills::brief(&place) {
            lead.push_str(&notes);
            briefed.push(skills::slug(&place));
        }
    }
    keep(&before, &lead);
    show(&before, true);
    s.seen = Some(before);
    s.task = Some(Task { id, name: task.to_string(), show: shown, steps: 0, done: HashMap::new(), briefed });
    s.paced = Some(Instant::now());
    EPOCH.fetch_add(1, Relaxed);
    CURRENT.store(id, Relaxed);
    Ok(lead)
}

pub async fn perform(llm: &Llm, mut call: Call) -> String {
    let mut held = SESSION.lock().await;
    let s = &mut *held;
    let Some(task) = s.task.as_mut() else { return UNPLANNED.to_string() };
    task.steps += 1;
    if task.steps > MAX_STEPS {
        return TIRED.to_string();
    }
    let (id, name) = (task.id, task.name.clone());
    for value in call.args.values_mut() {
        if let Value::String(text) = value
            && text.contains(STRAY)
        {
            *text = text.replace(STRAY, "");
        }
    }
    if let Some(why) = call.args.remove("why").as_ref().and_then(Value::as_str) {
        log::info!("why: {why}");
    }
    let before = match s.seen.clone() {
        Some(seen) => seen,
        None => screen(s).await,
    };
    let began = Instant::now();
    let thought = s.paced.map_or(0, |paced| began.duration_since(paced).as_millis());
    if FRESH.swap(false, Relaxed) {
        s.round = None;
    }
    let numbered = !["tab", "close_tab"].contains(&call.name.as_str());
    let refused = match (&s.round, whole(&call.args).filter(|_| numbered)) {
        (Some((_, false)), _) => Some(SKIPPED),
        (Some((first, _)), Some(number)) if stem(&line(first, number)) != stem(&line(&before, number)) => Some(SHIFTED),
        _ => None,
    };
    if let Some(refused) = refused {
        s.round = Some((before, false));
        return refused.to_string();
    }
    let first = s.round.take().map_or_else(|| before.clone(), |(first, _)| first);
    let (mut did, now, wasted) = step(llm, s, &call, &before).await;
    s.round = Some((first, !wasted));
    log::info!("{} took {} ms, {thought} ms after the step before it", call.name, began.elapsed().as_millis());
    let key = format!("{} {}", call.name, Value::Object(call.args.clone()));
    let place = here(s).await;
    let mut again = 0;
    if let Some(task) = s.task.as_mut() {
        let ended = task.done.entry(key.clone()).or_default();
        again = ended.iter().filter(|text| **text == now.text).count();
        ended.push(now.text.clone());
        if (again >= STUCK && call.name != "look") || (call.name == "type" && ended.len() > STUCK) {
            did.push_str(GIVEN_UP);
        } else if again >= 1 {
            did.push_str(LOOPING);
        }
        if !place.is_empty() && !task.briefed.contains(&skills::slug(&place)) {
            task.briefed.push(skills::slug(&place));
            if let Some(notes) = skills::brief(&place) {
                did.push_str(&notes);
            }
        }
    }
    let short: String = did.lines().next().unwrap_or_default().chars().take(LOGGED).collect();
    s.trail.push(skills::Step {
        place,
        did: format!("{key} -> {short}"),
        wasted: wasted || again >= 1,
        acted: !["go", "look", "read", "tab"].contains(&call.name.as_str()),
    });
    notch::revise(id, &card(&name, brief(&did)));
    show(&now, wasted || call.name == "look" || did.contains(UNMOVED) || titled(&before) != titled(&now));
    s.seen = Some(now);
    s.paced = Some(Instant::now());
    did
}

pub fn round() {
    FRESH.store(true, Relaxed);
}

pub async fn refresh() {
    let mut held = SESSION.lock().await;
    let s = &mut *held;
    if s.task.is_none() {
        return;
    }
    settle(s, None, SETTLED).await;
    let now = screen(s).await;
    show(&now, false);
    s.seen = Some(now);
}

pub async fn see() -> Result<String> {
    let mut held = SESSION.lock().await;
    let s = &mut *held;
    if s.task.is_some() {
        let now = screen(s).await;
        show(&now, true);
        s.seen = Some(now);
        return Ok("The place you are working in is shown below again".to_string());
    }
    let (apps, shown) = facing(s).await;
    let front = apps.iter().find(|app| app["front"] == true);
    let hosting = front.is_some_and(|app| app["host"] == true);
    let own = s.browser.as_ref().and_then(Browser::pid);
    let browsing = front.is_some_and(|app| own.is_some() && app["pid"].as_i64() == own);
    let tab = match (&s.place, shown) {
        (Place::Tab(tab), _) if hosting => Some(tab.clone()),
        (_, Some(tab)) if browsing => Some(tab),
        _ => None,
    };
    let seen = match (tab, &s.browser) {
        (Some(tab), Some(browser)) => {
            let seen = browser.look(&tab).await?;
            Seen { text: seen.text, image: seen.image, print: Vec::new(), web: false }
        }
        _ => {
            let op = if hosting && s.place == Place::Window { "look" } else { "peek" };
            Seen::from(&control(json!({ "op": op })).await.map_err(|e| anyhow!("{e}"))?)
        }
    };
    if seen.image.is_none() && seen.text.trim().is_empty() {
        bail!("there is no window to look at");
    }
    show(&seen, true);
    Ok(FRONT.to_string())
}

async fn end(llm: Llm, id: u64, ended: String, done: bool) {
    let mut held = SESSION.lock().await;
    let s = &mut *held;
    if s.task.as_ref().is_none_or(|task| task.id != id) {
        return;
    }
    let Some(task) = s.task.take() else { return };
    s.seen = None;
    if task.show && done {
        match (&s.place, &s.browser) {
            (Place::Tab(tab), Some(browser)) => {
                let _ = browser.show(tab).await;
                if let Some(pid) = browser.pid() {
                    forward(pid).await;
                }
                s.lent = None;
            }
            (Place::Window, _) => {
                let _ = act(json!({ "op": "raise" })).await;
                s.lent = None;
            }
            _ => {}
        }
    }
    if let Some(back) = s.lent.take() {
        forward(back).await;
    }
    notch::dismiss(task.id);
    let trail = std::mem::take(&mut s.trail);
    drop(held);
    skills::learn(llm, task.name, trail, ended, done).await;
}

pub fn rest(llm: &Llm, ended: &str, done: bool) {
    hide();
    let id = CURRENT.swap(0, Relaxed);
    if id != 0 {
        tokio::spawn(end(llm.clone(), id, ended.to_string(), done));
    }
}

pub async fn finish(llm: &Llm, ended: &str, done: bool) {
    hide();
    let id = CURRENT.swap(0, Relaxed);
    if id != 0 {
        end(llm.clone(), id, ended.to_string(), done).await;
    }
}

pub async fn quit() {
    let leave = async {
        if let Some(browser) = &SESSION.lock().await.browser {
            browser.quit().await;
        }
    };
    let _ = tokio::time::timeout(WAITED, leave).await;
}

pub async fn installed(app: &str) -> Option<String> {
    act(json!({ "op": "installed", "what": app })).await.ok()
}

pub async fn media(action: &str) -> Result<String> {
    let held = SESSION.lock().await;
    let s = &*held;
    if let Some(browser) = &s.browser
        && (action == "pause" || action == "play")
    {
        let hit = browser.media(action == "play").await;
        if !hit.is_empty() {
            let did = if action == "play" { "resumed" } else { "paused" };
            return Ok(format!("{did} what was playing in the tab {}", tools::clip(&hit.join(", "))));
        }
    }
    let key = if action == "pause" { "play" } else { action };
    act(json!({ "op": "media", "key": key })).await.map_err(|e| anyhow!("{e}"))?;
    Ok(match action {
        "pause" | "play" => {
            "pressed the play and pause key, which pauses or resumes whatever app is playing".to_string()
        }
        other => format!("pressed the {other} track key"),
    })
}

pub async fn pick(which: &str) -> Result<(String, String)> {
    let mut held = SESSION.lock().await;
    let s = &mut *held;
    let Some(browser) = &s.browser else { bail!("you have no tabs open in your own browser") };
    let tabs = browser.tabs().await?;
    let (_, shown) = facing(s).await;
    let here = match &s.place {
        Place::Tab(tab) => Some(tab.clone()),
        _ => None,
    };
    let wanted: Vec<String> = which
        .to_lowercase()
        .split_whitespace()
        .filter(|word| *word != "tab" && *word != "the")
        .map(str::to_string)
        .collect();
    let fits = |tab: &browser::Tab| {
        let both = format!("{} {}", tab.title, tab.url).to_lowercase();
        !wanted.is_empty() && wanted.iter().all(|word| both.contains(word))
    };
    let mut hits: Vec<&browser::Tab> = tabs.iter().filter(|tab| fits(tab)).collect();
    if wanted.is_empty() || hits.len() != 1 && wanted.len() <= 1 {
        let near =
            |tab: &&browser::Tab| here.as_deref() == Some(tab.id.as_str()) || shown.as_deref() == Some(tab.id.as_str());
        let pool: Vec<&browser::Tab> = if hits.is_empty() { tabs.iter().collect() } else { hits.clone() };
        let close: Vec<&browser::Tab> = pool.into_iter().filter(near).collect();
        let last = close.iter().find(|tab| here.as_deref() == Some(tab.id.as_str())).or(close.first());
        if let Some(tab) = last {
            hits = vec![tab];
        }
    }
    match hits.as_slice() {
        [tab] => Ok((tab.id.clone(), tab.title.clone())),
        [] => bail!(
            "no open tab matches that; the open tabs are: {}",
            tabs.iter().map(|tab| tab.title.as_str()).collect::<Vec<_>>().join("; ")
        ),
        many => bail!(
            "more than one tab matches, so ask them which: {}",
            many.iter().map(|tab| tab.title.as_str()).collect::<Vec<_>>().join("; ")
        ),
    }
}

pub async fn shut(tab: &str, show: bool) -> Result<()> {
    let mut held = SESSION.lock().await;
    let s = &mut *held;
    let Some(browser) = &s.browser else { bail!("{UNLINKED}") };
    if show {
        browser.show(tab).await?;
        if let Some(pid) = browser.pid() {
            forward(pid).await;
        }
        s.place = Place::Tab(tab.to_string());
        return Ok(());
    }
    browser.close(tab).await?;
    if s.place == Place::Tab(tab.to_string()) {
        s.place = Place::None;
    }
    if s.mine.as_deref() == Some(tab) {
        s.mine = None;
    }
    Ok(())
}
