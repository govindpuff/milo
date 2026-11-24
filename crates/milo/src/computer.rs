use std::collections::HashMap;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use browser::Browser;
use llm::{Call, Llm, Message, Role};
use serde_json::{Map, Value, json};
use tokio::sync::Notify;
use tokio::sync::mpsc::UnboundedSender;

use crate::controller::Event;
use crate::skills;
use crate::tools;

const MAX_STEPS: usize = 30;
const TOTAL: Duration = Duration::from_secs(360);
const SETTLE: Duration = Duration::from_millis(250);
const ARRIVE: Duration = Duration::from_secs(5);
const STILL: Duration = Duration::from_millis(300);
const SETTLED: Duration = Duration::from_secs(3);
const LOADED: Duration = Duration::from_secs(12);
const YIELD_POLL: Duration = Duration::from_millis(20);
const YIELD_MAX: Duration = Duration::from_secs(4);
const SCREEN: &str = "\n\nThe place you are working in now, as a picture and as a list:\n";
const STOP: &str = "stop";
const TYPED_CHARS: usize = 400;
const MOVED: f64 = 0.8;
const SPOT: u8 = 8;
const ALLOW: Duration = Duration::from_secs(40);
const ASKED: Duration = Duration::from_millis(1500);
const PEEK: Duration = Duration::from_secs(5);
const FOLLOWED: Duration = Duration::from_secs(6);
const WAITED: Duration = Duration::from_secs(1);
const PAST: usize = 4;
const BUDGET: usize = 6500;
const PICTURE: usize = 300;
const OLD: usize = 300;
const READ: &str = "the text of the page";
const RETURNS: usize = 5;
const LOGGED: usize = 400;
const TABS: usize = 30;
const SAME: &str = "Nothing on the screen changed, so that did nothing. Do not repeat it. Try a different way, like a key, a menu item or another control.";
const IDLE: &str = "no plan is being carried out right now, so there was nothing to stop. If they want a video, music or something else in an app stopped, call computer for that";
const HALTED: &str = "stopped what was being done on their computer";
pub const BUSY: &str = "something else is still being done on their computer; it has to finish or be stopped first";

const PROMPT: &str = "You operate a person's Mac for them. They approved the plan below, and you carry it out one action at a time. You work in one place at a time: a tab in their browser, or the window of an app. A browser tab never needs to be in front.

After each action you get a picture of the place and a list of what is in it. Lines in the list that start with a number in square brackets are controls you can use. The other lines are text. Trust the list and the picture over what you expect.

Rules:
- Call exactly one tool at a time.
- Numbers change after every action. Only use numbers from the latest list.
- Do what the plan says and nothing else. Do not add steps of your own.
- First make sure you are in the right place. The first message lists the apps and the browser tabs that are open, and says which place you are in, if any. When the plan is about something that is already open, like that video, this tab or the page they are on, go to the tab or the app that holds it instead of opening it again. It is usually the place you worked in last, or the one in front. If the place you are in is not where the plan belongs, change place before you do anything else. Never click, type or press keys in a place the plan is not about.
- open brings up an app by name or a website by address, and makes it the place you work in. An app comes to the front while you work in it. A website opens in a browser window of your own, so their tabs stay as they are. To use a website, open its address: do not open the browser by its name, and do not go through a search engine when the plan names the site. When a web address can take you straight to what the plan needs, like a site's search results for some words, open that address instead of clicking through the site.
- click presses a control. Give its number from the list. The number in square brackets is where the control is in the list, not what it is called: [5] button \"7\" is control 5, and it is the key 7. To press a control by what it is called, leave the number out and give the exact name as what, like 7 or Multiply. When the thing you need is in the picture but not in the list, leave the number out and give what instead: a few words that say what it is and where, like the word Español in the list of languages. Never guess a number.
- type types text, into a text field or a text area if you give its number, otherwise where the cursor is. Never give the number of a button or a link to type. Typing into a field replaces what was in it. Typing into a large writing area, like a document or the body of a message, adds to what is there. Set enter to true to press return after the text, which is how a search, an address or a chat message is confirmed. To pick from a pop-up menu that lists its choices, type the choice into it. The result tells you what the control contains afterward.
- press sends keys, like return, escape, tab, down, cmd+n or cmd+f.
- scroll moves down or up to show more of a page or a window.
- read gives you all the text of a web page, including what is off screen. Use it when the plan asks for something to be found or read out and the list does not show all of it.
- Type only when the result of your last action or the list shows that the cursor is in the right place. If typed text went into the wrong place, undo it with cmd+z before you go on.
- In a save or open sheet of an app, keys do not work. Type the file name into the Save As field by its number, and click the Save button. To choose the folder, click the Where pop-up and pick the folder there. If the folder the plan names is not offered, save where the sheet already is, and say in your report which folder the file went to.
- menu picks from an app's menu bar, like File > New Window. In an app it is the surest way to do something there is a menu item for. A web page has no menu bar.
- look waits a moment and shows the place again. Use it when something is still loading.
- You work behind their other windows. When they asked to see what you did, it is brought to the front for them after you finish, so never try to do that yourself.
- The result of each action says what left the place and what is new in it. Read it to see what your action did. When something you meant to remove is listed as gone, that part is done.
- If an action fails or the screen did not change the way you expected, try another way.
- The first message can hold notes from earlier work in an app or a site. They say what got things done there before, and what did nothing. Carry out each step of the plan the way the notes give for it, and do not try what they say does not work. When the screen shows that a note is wrong, trust the screen.
- Only work in the app or the site the plan names. If it will not open, stop and say so. Never use a different app in its place.
- You may close an advertisement or a cookie banner that is in the way. But when something asks you to allow, approve, confirm, restore, sign in, pay, delete or send something the plan does not cover, do not answer it in any way, not even to cancel it. Stop and say what it is asking.
- Before you type or send anything meant for a person, check that the picture and the list show that person's conversation is the one open, by its name. A name that is merely listed in a sidebar is not open. If the wrong conversation is open or you cannot tell, do not type: open the right one first, or stop and say you could not find it.
- When the plan says me, myself or my own, it means the person you work for. Their name is under Known about the person when it is known.
- Before you finish, check that the latest list or picture shows the plan is done. Typing in a search box or seeing a result listed is not the same as having opened it. If it does not show it, keep going or say it did not work.
- If you cannot finish after trying, stop and say what got in the way.
- When the plan asks you to find, read or tell something, and the list or the picture already shows it, you are done: do not click it or open it. Answer from what is shown.
- When the plan is done, do not call a tool. Write one or two short plain sentences that Milo will say out loud: what was done, or what you found if they asked for information. Take every fact from the screen.";

const TABBED: &str = "\n- tab moves you to a browser tab that is already open, by its number in the list of tabs. close_tab closes a tab: the one you are in, or another by its number. Keys like cmd+t, cmd+l and cmd+w do not reach the browser, so use open, tab and close_tab instead.";

const POINT: &str = "This is a picture of an app window. Give the bounding box of the thing described below as [ymin, xmin, ymax, xmax], normalized to 0-1000. Reply with only the box. If it is not in the picture, reply with only the word none.\n\n";
const UNMOVED: &str =
    ". The list of what is in the place is the same as before, so check the picture to see whether that did anything";
const STUCK: usize = 3;
const LOOPING: &str = " You have done exactly this before and ended up in the same place. Do not do it again. Do something different, or stop and say what got in the way.";
const LISTED: usize = 8;
const GLIMPSE: &str = "This is a picture of what is on a person's Mac, with a list of what it holds. Answer the question in one to three short plain sentences, only from what is shown.\n\n";
const WHY: &str = "Write this first. One or two short sentences: which place you are in and what it shows now, which step of the plan you are on, and why this action is the next one. If the place already shows what the plan asks for, do not call a tool at all: write the answer.";
const UNOPENED: &str = "no app has been opened yet";
const UNSEEN: &str = "that could not be found in the picture; describe it another way, or use a number from the list";
const NOWHERE: &str = "You are not in any tab or window yet. Go to one with open";
const LOST: &str = "The place you were in is gone.";
const HOSTING: &str = " (Milo itself runs in this app; never work in it)";
const SPRUNG: &str = ". It opened in a new tab, and you are in that tab now";
const UNLINKED: &str = "the browser's tabs cannot be reached; work in the browser's window with open, click and press";
const OFF: &str = "no browser allows remote debugging, so websites are read off the screen, which is slower and less exact. To fix that, open chrome://inspect/#remote-debugging in the browser and turn on Allow remote debugging for this browser instance";
const CUT: &str = "the person stopped it before it finished";
const KNOCK: &str = "Connecting to the browser. If it asks whether to allow remote debugging, click Allow.";

static RUNNING: AtomicBool = AtomicBool::new(false);
static HALT: Notify = Notify::const_new();
static SHOT: AtomicUsize = AtomicUsize::new(0);
static SESSION: tokio::sync::Mutex<Session> = tokio::sync::Mutex::const_new(Session {
    browser: None,
    place: Place::None,
    mine: None,
    tabs: Vec::new(),
    past: Vec::new(),
    warned: false,
    lent: None,
    trail: Vec::new(),
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
    past: Vec<String>,
    warned: bool,
    lent: Option<String>,
    trail: Vec<skills::Step>,
}

struct Running;

impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.store(false, Relaxed);
    }
}

#[derive(Clone)]
struct Seen {
    text: String,
    image: Option<String>,
    print: Vec<u8>,
}

impl Seen {
    fn from(answer: &Value) -> Seen {
        let print = answer["print"].as_str().unwrap_or_default();
        Seen {
            text: answer["text"].as_str().unwrap_or_default().to_string(),
            image: answer["image"].as_str().map(str::to_string),
            print: (0..print.len() / 2).filter_map(|i| u8::from_str_radix(&print[i * 2..i * 2 + 2], 16).ok()).collect(),
        }
    }

    fn plain(text: String) -> Seen {
        Seen { text, image: None, print: Vec::new() }
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

    fn message(self, lead: &str) -> Message {
        Message::seen(format!("{lead}{SCREEN}{}", self.text), self.image)
    }
}

pub fn running() -> bool {
    RUNNING.load(Relaxed)
}

pub fn halt() -> Result<String> {
    if !running() {
        bail!(IDLE);
    }
    HALT.notify_waiters();
    Ok(HALTED.to_string())
}

fn schema(linked: bool) -> Value {
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
            "open",
            "Bring up an app or a website, and work there.",
            json!({ "what": { "type": "string", "description": "An app name like Mail, or a web address like mail.google.com." } }),
            &["what"],
        ),
        tool(
            "click",
            "Press a control by its number, or something in the picture by describing it.",
            json!({ "number": number, "what": { "type": "string", "description": "What the thing looks like and where it is, like the blue Send button at the bottom right." } }),
            &[],
        ),
        tool(
            "type",
            "Type text into a control, or where the cursor is when no number is given.",
            json!({
                "text": { "type": "string", "description": "The exact text to type." },
                "number": number,
                "enter": { "type": "boolean", "description": "True to press return after the text." },
            }),
            &["text"],
        ),
        tool(
            "press",
            "Press a key or a key combination.",
            json!({ "keys": { "type": "string", "description": "Like return, escape, tab, down, cmd+n, cmd+f." } }),
            &["keys"],
        ),
        tool(
            "scroll",
            "Move down or up to see more.",
            json!({ "direction": { "type": "string", "enum": ["down", "up"] } }),
            &["direction"],
        ),
        tool("read", "Get all the text of the web page you are on.", json!({}), &[]),
        tool(
            "menu",
            "Choose an item from an app's menu bar.",
            json!({ "path": { "type": "string", "description": "The menu and the item, like File > New Window." } }),
            &["path"],
        ),
        tool("look", "Wait a moment and show the place again.", json!({}), &[]),
    ];
    if linked {
        all.push(tool("tab", "Go to a browser tab that is already open.", json!({ "number": tab }), &["number"]));
        all.push(tool(
            "close_tab",
            "Close a browser tab: the one you are in, or another by its number.",
            json!({ "number": tab }),
            &[],
        ));
    }
    Value::Array(all)
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

async fn front() -> Option<String> {
    let apps = control(json!({ "op": "around" })).await.ok()?;
    let front = apps["apps"].as_array()?.iter().find(|app| app["front"] == true)?;
    front["name"].as_str().map(str::to_string)
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

async fn link(s: &mut Session, wait: Duration, id: Option<(u64, &str)>) {
    if s.browser.as_ref().is_some_and(Browser::alive) {
        return;
    }
    s.browser = None;
    let Some(name) = browser::found() else {
        if !s.warned {
            log::warn!("{OFF}");
            s.warned = true;
        }
        return;
    };
    if let Some((id, task)) = id {
        notch::revise(id, &card(task, KNOCK));
    }
    let back = front().await.filter(|front| *front != name);
    let connect = Browser::connect(wait);
    tokio::pin!(connect);
    let got = tokio::select! {
        got = &mut connect => got,
        _ = tokio::time::sleep(ASKED) => {
            log::info!("{name} is asking whether to allow remote debugging: click Allow there");
            let _ = tools::launch(&name, false).await;
            connect.await
        }
    };
    if let Some(back) = back {
        for _ in 0..RETURNS {
            tokio::time::sleep(STILL).await;
            if front().await.as_deref() != Some(back.as_str()) {
                let _ = tools::launch(&back, false).await;
            }
        }
    }
    match got {
        Ok(browser) => {
            log::info!("connected to {}", browser.name());
            s.browser = Some(browser);
        }
        Err(e) => log::warn!("the browser could not be reached, so websites are read off the screen instead: {e:#}"),
    }
}

pub async fn warm() {
    let mut held = SESSION.lock().await;
    link(&mut held, ALLOW, None).await;
}

async fn facing(s: &Session) -> (Vec<Value>, Option<String>) {
    let apps = match control(json!({ "op": "around" })).await {
        Ok(answer) => answer["apps"].as_array().cloned().unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    let Some(browser) = &s.browser else { return (apps, None) };
    let title = apps.iter().find(|app| app["name"] == browser.name()).and_then(|app| app["title"].as_str());
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
    out.push_str(&format!("\nTabs open in {}:", browser.name()));
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
            out.push_str(" (showing in the browser's front window)");
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
                    Seen { text: seen.text, image: seen.image, print: Vec::new() }
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

async fn settle(s: &Session, first: Duration, most: Duration) {
    tokio::time::sleep(first).await;
    let until = Instant::now() + most;
    match &s.place {
        Place::None => {}
        Place::Tab(tab) => {
            let Some(browser) = &s.browser else { return };
            let mut last = None;
            while Instant::now() < until {
                let Ok((print, loaded)) = browser.glance(tab).await else { return };
                if loaded && last.as_ref() == Some(&print) {
                    return;
                }
                last = Some(print);
                tokio::time::sleep(STILL).await;
            }
        }
        Place::Window => {
            let mut last: Option<Seen> = None;
            while Instant::now() < until {
                let Ok(answer) = control(json!({ "op": "glance" })).await else { return };
                let now = Seen::from(&answer);
                let loaded = answer["loaded"].as_bool().unwrap_or(true);
                if loaded && last.as_ref().is_some_and(|last| last.same(&now)) {
                    return;
                }
                last = Some(now);
                tokio::time::sleep(STILL).await;
            }
        }
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

async fn click(llm: &Llm, s: &mut Session, call: &Call, before: &Seen) -> (Result<String, String>, Seen) {
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
        tokio::time::sleep(STILL).await;
        if did.is_ok() && sprung(s, &tab).await {
            did = did.map(|text| format!("{text}{SPRUNG}"));
        }
        settle(s, SETTLE, FOLLOWED).await;
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
        settle(s, SETTLE, SETTLED).await;
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

async fn visit(s: &mut Session, what: &str) -> Result<String> {
    let address = if what.contains("://") { what.to_string() } else { format!("https://{what}") };
    let kept = match s.mine.clone() {
        Some(tab) if exists(s, &tab).await => Some(tab),
        _ => None,
    };
    let Some(browser) = &s.browser else { bail!(UNLINKED) };
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
            if ahead.as_deref() != Some(name.as_str()) {
                if s.lent.is_none() {
                    s.lent = ahead;
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
        link(s, PEEK, None).await;
        if s.browser.is_some() {
            return visit(s, what).await.map_err(|e| format!("{e:#}"));
        }
    }
    let did = launch(s, what, web).await;
    if did.is_err() || s.place != Place::Window {
        return did;
    }
    if web {
        link(s, PEEK, None).await;
    }
    let named = s.browser.as_ref().map(|browser| browser.name().to_lowercase());
    let lower = what.to_lowercase();
    if web || named.is_some_and(|name| name.contains(&lower) || lower.contains(&name)) {
        let (_, shown) = facing(s).await;
        let first = match (&shown, &s.browser) {
            (None, Some(browser)) => browser.tabs().await.ok().and_then(|tabs| tabs.first().map(|tab| tab.id.clone())),
            _ => None,
        };
        if let Some(tab) = shown.or(first) {
            s.place = Place::Tab(tab);
        }
    }
    did
}

async fn step(llm: &Llm, s: &mut Session, call: &Call, before: &Seen) -> (String, Seen, bool) {
    let text = |key: &str| call.args.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let name = call.name.as_str();
    let nowhere = || Err(format!("{NOWHERE} first"));
    let place = s.place.clone();
    let (did, after) = match name {
        "look" => {
            tokio::time::sleep(WAITED).await;
            (Ok("looked again".to_string()), screen(s).await)
        }
        "click" => click(llm, s, call, before).await,
        "open" => {
            let did = enter(s, &text("what")).await;
            settle(s, STILL, LOADED).await;
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
                                "closed the tab {}. That tab is gone now. Do not close another tab unless the plan names more than one",
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
            settle(s, SETTLE, SETTLED).await;
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
            settle(s, SETTLE, if enter { FOLLOWED } else { SETTLED }).await;
            (did, screen(s).await)
        }
        "press" | "scroll" | "menu" | "read" | "show" => {
            let keys = text("keys");
            let down = text("direction") != "up";
            let did = match (&place, &s.browser) {
                (Place::None, _) => nowhere(),
                (Place::Tab(_), None) => Err(UNLINKED.to_string()),
                (Place::Tab(tab), Some(browser)) => {
                    let combo = keys.to_lowercase().replace(' ', "").replace("command", "cmd");
                    let did = match name {
                        "press" if ["cmd+t", "cmd+n", "cmd+l", "cmd+w"].contains(&combo.as_str()) => {
                            Err(anyhow!("that key does not reach the browser; use open, tab or close_tab instead"))
                        }
                        "press" => browser.press(tab, &keys).await.map(|_| format!("pressed {keys}")),
                        "scroll" => browser
                            .scroll(tab, down)
                            .await
                            .map(|_| format!("scrolled {}", if down { "down" } else { "up" })),
                        "read" => browser.read(tab).await.map(|all| format!("{READ}:\n{all}\n")),
                        "show" => {
                            let shown = browser.show(tab).await;
                            let _ = tools::launch(browser.name(), false).await;
                            shown.map(|_| "brought the tab to the front of their screen".to_string())
                        }
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
                    "show" => act(json!({ "op": "raise" })).await,
                    _ => act(json!({ "op": "menu", "path": text("path") })).await,
                },
            };
            settle(s, SETTLE, SETTLED).await;
            (did, screen(s).await)
        }
        _ => (Err("there is no such tool".to_string()), screen(s).await),
    };
    let mut after = after;
    let quiet = ["look", "read", "show"].contains(&name);
    let failed = did.is_err();
    let mut did = match did {
        Ok(text) if !quiet && s.place == Place::Window && place == Place::Window && after.same(before) => {
            if s.lent.is_none() {
                s.lent = front().await;
            }
            let _ = act(json!({ "op": "raise" })).await;
            settle(s, STILL, SETTLED).await;
            after = screen(s).await;
            text
        }
        Ok(text) => text,
        Err(e) => format!("{name} failed: {e}"),
    };
    if name == "show" {
        s.lent = None;
    }
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

fn written(text: &str, schema: &Value) -> Option<Call> {
    let first = text.trim().lines().next()?.trim();
    let (name, rest) = first.split_at(first.find(['{', '(']).unwrap_or(first.len()));
    let name = name.trim().trim_end_matches(':').trim();
    if !schema.as_array()?.iter().any(|tool| tool["function"]["name"] == name) {
        return None;
    }
    let rest = rest.trim().trim_start_matches('(').trim_end_matches(')').trim();
    let args = match rest.is_empty() {
        true => Map::new(),
        false => serde_json::from_str::<Value>(rest).ok()?.as_object().cloned()?,
    };
    Some(Call { id: "written".to_string(), name: name.to_string(), args })
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

async fn drive(
    llm: &Llm,
    talking: &AtomicBool,
    events: &UnboundedSender<Event>,
    task: &str,
    steps: &[String],
    id: u64,
    show: bool,
) -> Result<String> {
    let mut held = SESSION.lock().await;
    let s = &mut *held;
    s.lent = None;
    let out = work(llm, talking, events, task, steps, id, s).await;
    if show && out.is_ok() {
        match (&s.place, &s.browser) {
            (Place::Tab(tab), Some(browser)) => {
                let _ = browser.show(tab).await;
                let _ = tools::launch(browser.name(), false).await;
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
        let _ = tools::launch(&back, false).await;
    }
    out
}

async fn work(
    llm: &Llm,
    talking: &AtomicBool,
    events: &UnboundedSender<Event>,
    task: &str,
    steps: &[String],
    id: u64,
    s: &mut Session,
) -> Result<String> {
    act(json!({ "op": "start" })).await.map_err(|e| anyhow!("{e}"))?;
    let _ = std::fs::remove_dir_all(saved());
    let _ = std::fs::create_dir_all(saved());
    SHOT.store(0, Relaxed);
    s.trail.clear();
    link(s, ALLOW, Some((id, task))).await;
    arrive(s).await;
    let plan: Vec<String> = steps.iter().enumerate().map(|(i, s)| format!("{}. {s}", i + 1)).collect();
    let schema = schema(s.browser.is_some());
    let mut lead = format!("What they asked for: {task}\n\nThe plan they approved:\n{}", plan.join("\n"));
    let known = tools::memory();
    if !known.is_empty() {
        lead.push_str(&format!("\n\nKnown about the person:\n- {}", known.join("\n- ")));
    }
    if !s.past.is_empty() {
        lead.push_str(&format!(
            "\n\nWhat you did for them earlier on this call, oldest first:\n- {}",
            s.past.join("\n- ")
        ));
    }
    let mut before = screen(s).await;
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
    let first = Seen { text: before.text.clone(), image: before.image.clone(), print: Vec::new() };
    let prompt = if s.browser.is_some() { format!("{PROMPT}{TABBED}") } else { PROMPT.to_string() };
    let mut messages = vec![Message::system(&prompt), first.message(&lead)];
    let mut typed: Vec<String> = Vec::new();
    let mut done: HashMap<String, Vec<String>> = HashMap::new();
    for _ in 0..MAX_STEPS {
        let waited = Instant::now();
        while talking.load(Relaxed) && waited.elapsed() < YIELD_MAX {
            tokio::time::sleep(YIELD_POLL).await;
        }
        let spent = |messages: &[Message]| {
            let text: usize = messages.iter().map(|m| m.content.chars().count()).sum();
            let pictures = messages.iter().filter(|m| m.image.is_some()).count();
            (text + schema.to_string().len()) / 3 + pictures * PICTURE
        };
        if spent(&messages) > BUDGET {
            messages.iter_mut().for_each(|m| m.image = None);
        }
        let reply = match llm.act(&messages, &schema).await {
            Err(e) if messages.iter().any(|m| m.image.is_some()) && format!("{e:#}").contains("400") => {
                messages.iter_mut().for_each(|m| m.image = None);
                llm.act(&messages, &schema).await?
            }
            other => other?,
        };
        let Some(mut call) = reply.calls.into_iter().next().or_else(|| written(&reply.text, &schema)) else {
            let said = reply.text.trim().trim_matches('"').trim();
            let said = if said.is_empty() { format!("finished: {task}") } else { tools::clip(said) };
            s.past.push(format!("{task}: {said}"));
            if s.past.len() > PAST {
                s.past.remove(0);
            }
            let wrote: String = typed.join(" ").split_whitespace().collect::<Vec<_>>().join(" ");
            if wrote.is_empty() {
                return Ok(said);
            }
            let wrote: String = wrote.chars().take(TYPED_CHARS).collect();
            return Ok(format!("{said} What you typed: \"{wrote}\""));
        };
        if let Some(why) = call.args.remove("why").as_ref().and_then(Value::as_str) {
            log::info!("why: {why}");
        }
        let _ = events.send(Event::Doing(format!("{} {}", call.name, Value::Object(call.args.clone()))));
        let (mut did, now, wasted) = step(llm, s, &call, &before).await;
        let key = format!("{} {}", call.name, Value::Object(call.args.clone()));
        let ended = done.entry(key.clone()).or_default();
        let again = ended.iter().filter(|text| **text == now.text).count();
        ended.push(now.text.clone());
        if again >= STUCK && call.name != "look" {
            bail!("it kept repeating the same step without getting anywhere and was stopped before finishing {task}");
        }
        if again >= 1 {
            did.push_str(LOOPING);
        }
        if call.name == "type" && ended.len() > STUCK {
            bail!("it kept typing the same thing and was stopped before finishing {task}");
        }
        let short: String = did.lines().next().unwrap_or_default().chars().take(LOGGED).collect();
        log::info!("{key} -> {short}");
        let place = here(s).await;
        s.trail.push(skills::Step {
            place: place.clone(),
            did: format!("{key} -> {short}"),
            wasted: wasted || again >= 1,
            acted: !["open", "look", "read", "tab"].contains(&call.name.as_str()),
        });
        if call.name == "type" && !did.contains("failed") {
            typed.extend(call.args.get("text").and_then(Value::as_str).map(str::to_string));
        }
        notch::revise(id, &card(task, brief(&did)));
        let mut index = 0;
        messages.retain_mut(|m| {
            index += 1;
            if index == 2 {
                m.image = None;
                if let Some(at) = m.content.find(SCREEN) {
                    m.content.truncate(at);
                }
            }
            index <= 2 || m.role != Role::User
        });
        for old in messages.iter_mut().filter(|m| m.role == Role::Tool) {
            if old.content.chars().count() > OLD && !old.content.starts_with(READ) {
                old.content = old.content.chars().take(OLD).collect::<String>() + "…";
            }
        }
        if !place.is_empty() && !briefed.contains(&skills::slug(&place)) {
            briefed.push(skills::slug(&place));
            if let Some(notes) = skills::brief(&place) {
                messages[1].content.push_str(&notes);
            }
        }
        messages.push(Message::called(std::slice::from_ref(&call)));
        messages.push(Message::tool(&call.id, did));
        messages.push(Seen { text: now.text.clone(), image: now.image.clone(), print: Vec::new() }.message(""));
        before = now;
    }
    bail!("it ran out of steps before finishing {task}")
}

pub async fn solo(llm: &Llm, task: &str, steps: &[String]) -> Result<String> {
    let (events, mut seen) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move { while seen.recv().await.is_some() {} });
    let talking = AtomicBool::new(false);
    let plan = if steps.is_empty() { vec![task.to_string()] } else { steps.to_vec() };
    let out = match tokio::time::timeout(TOTAL, drive(llm, &talking, &events, task, &plan, 0, false)).await {
        Ok(out) => out,
        Err(_) => Err(anyhow!("it took too long and was stopped before finishing {task}")),
    };
    let ended = match &out {
        Ok(said) => said.clone(),
        Err(e) => format!("{e:#}"),
    };
    let trail = std::mem::take(&mut SESSION.lock().await.trail);
    skills::learn(llm.clone(), task.to_string(), trail, ended, out.is_ok()).await;
    out
}

pub async fn glimpse(llm: &Llm, question: &str) -> Result<String> {
    if running() {
        bail!(BUSY);
    }
    let mut held = SESSION.lock().await;
    let s = &mut *held;
    link(s, PEEK, None).await;
    let (apps, shown) = facing(s).await;
    let front = apps.iter().find(|app| app["front"] == true);
    let hosting = front.is_some_and(|app| app["host"] == true);
    let browsing = front.is_some_and(|app| s.browser.as_ref().is_some_and(|browser| app["name"] == browser.name()));
    let tab = match (&s.place, shown) {
        (Place::Tab(tab), _) if hosting => Some(tab.clone()),
        (_, Some(tab)) if browsing => Some(tab),
        _ => None,
    };
    let seen = match (tab, &s.browser) {
        (Some(tab), Some(browser)) => {
            let seen = browser.look(&tab).await?;
            Seen { text: seen.text, image: seen.image, print: Vec::new() }
        }
        _ => {
            let op = if hosting && s.place == Place::Window { "look" } else { "peek" };
            Seen::from(&control(json!({ "op": op })).await.map_err(|e| anyhow!("{e}"))?)
        }
    };
    let Some(image) = &seen.image else { bail!("there is no window to look at") };
    let said =
        llm.see(&format!("{GLIMPSE}{question}\n\nWhat the window holds, as a list:\n{}", seen.text), image).await?;
    Ok(tools::clip(said.trim()))
}

pub async fn installed(app: &str) -> Option<String> {
    act(json!({ "op": "installed", "what": app })).await.ok()
}

pub async fn media(action: &str) -> Result<String> {
    if running() {
        bail!(BUSY);
    }
    let mut held = SESSION.lock().await;
    let s = &mut *held;
    link(s, PEEK, None).await;
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
    if running() {
        bail!(BUSY);
    }
    let mut held = SESSION.lock().await;
    let s = &mut *held;
    link(s, PEEK, None).await;
    let Some(browser) = &s.browser else { bail!("{UNLINKED}") };
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
        let _ = tools::launch(browser.name(), false).await;
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

pub fn claim() -> Result<impl Drop> {
    if RUNNING.swap(true, Relaxed) {
        bail!(BUSY);
    }
    Ok(Running)
}

pub async fn run(
    llm: &Llm,
    talking: &AtomicBool,
    events: &UnboundedSender<Event>,
    task: &str,
    steps: &[String],
    show: bool,
) -> Result<Option<String>> {
    let (id, stopped) = notch::show(&card(task, "Starting"));
    let out = tokio::select! {
        out = tokio::time::timeout(TOTAL, drive(llm, talking, events, task, steps, id, show)) => match out {
            Ok(out) => out.map(Some),
            Err(_) => Err(anyhow!("it took too long and was stopped before finishing {task}")),
        },
        _ = stopped => Ok(None),
        _ = HALT.notified() => Ok(None),
    };
    notch::dismiss(id);
    let ended = match &out {
        Ok(Some(said)) => said.clone(),
        Ok(None) => CUT.to_string(),
        Err(e) => format!("{e:#}"),
    };
    let trail = std::mem::take(&mut SESSION.lock().await.trail);
    tokio::spawn(skills::learn(llm.clone(), task.to_string(), trail, ended, matches!(out, Ok(Some(_)))));
    out
}
