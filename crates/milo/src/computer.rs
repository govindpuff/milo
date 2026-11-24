use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use llm::{Call, Llm, Message, Role};
use serde_json::{Map, Value, json};
use tokio::sync::Notify;
use tokio::sync::mpsc::UnboundedSender;

use crate::controller::Event;
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
const SCREEN: &str = "\n\nThe screen now, as a picture and as a list:\n";
const STOP: &str = "stop";
const TYPED_CHARS: usize = 400;
const MOVED: f64 = 0.8;
const SAME: &str = "Nothing on the screen changed, so that did nothing. Do not repeat it. Try a different way, like a key, a menu item or another control.";
const IDLE: &str = "nothing is being done on their computer right now";
const HALTED: &str = "stopped what was being done on their computer";
pub const BUSY: &str = "something else is still being done on their computer; it has to finish or be stopped first";

const PROMPT: &str = "You operate a person's Mac for them. They approved the plan below, and you carry it out one action at a time. You work in one app's window in the background while they use their computer, so you never need the window to be in front.

After each action you get a picture of the window and a list of what is in it. Lines in the list that start with a number in square brackets are controls you can use. The other lines are text in the window. The picture shows what the window looks like right now. Trust the picture over what you expect.

Rules:
- Call exactly one tool at a time.
- Numbers change after every action. Only use numbers from the latest list.
- Do what the plan says and nothing else. Do not add steps of your own.
- open brings up an app by name or a website by address, and makes it the window you work in. Start with open unless the window you need is already the one shown. When a web address can take you straight to what the plan needs, like a site's search results for some words, open that address instead of clicking through the site.
- click presses a control. Give its number from the list. When the thing you need is in the picture but not in the list, give what instead: a few words that say what it looks like and where it is.
- type types text, into a control if you give its number, otherwise where the cursor is. Typing into a text field replaces what was in it. Typing into a text area adds to what is there. The result tells you what the control contains afterward.
- press sends keys to the app, like return, escape, tab, down, pagedown, cmd+n or cmd+f. Press pagedown to see further down a page.
- menu picks from the app's menu bar, like File > New Window. It is the surest way to do something an app has a menu item for.
- look shows the screen again. Use it when something is still loading.
- If an action fails or the screen did not change the way you expected, try another way.
- You may close a banner or a pop-up that is in the way. But when something asks for a permission, a password, a payment, or to delete or send something the plan does not cover, do not answer it. Stop and say what it is asking.
- Before you type or send anything meant for a person, check that the picture and the list show that person's conversation is the one open, by its name. A name that is merely listed in a sidebar is not open. If the wrong conversation is open or you cannot tell, do not type: open the right one first, or stop and say you could not find it.
- When the plan says me, myself or my own, it means the person you work for. Their name is under Known about the person when it is known.
- Before you finish, check that the latest picture shows the plan is done. Typing in a search box or seeing a result listed is not the same as having opened it. If the picture does not show it, keep going or say it did not work.
- If you cannot finish after trying, stop and say what got in the way.
- When the plan is done, do not call a tool. Write one or two short plain sentences that Milo will say out loud: what was done, or what you found if they asked for information. Take every fact from the screen.";

const POINT: &str = "This is a picture of an app window. Give the bounding box of the thing described below as [ymin, xmin, ymax, xmax], normalized to 0-1000. Reply with only the box. If it is not in the picture, reply with only the word none.\n\n";
const UNOPENED: &str = "no app has been opened yet";
const UNSEEN: &str = "that could not be found in the picture; describe it another way, or use a number from the list";

static RUNNING: AtomicBool = AtomicBool::new(false);
static HALT: Notify = Notify::const_new();
static SHOT: AtomicUsize = AtomicUsize::new(0);

struct Running;

impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.store(false, Relaxed);
    }
}

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

    fn same(&self, other: &Seen) -> bool {
        if self.text != other.text {
            return false;
        }
        if self.print.is_empty() || self.print.len() != other.print.len() {
            return true;
        }
        let apart: u64 = self.print.iter().zip(&other.print).map(|(a, b)| a.abs_diff(*b) as u64).sum();
        (apart as f64 / self.print.len() as f64) < MOVED
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

fn schema() -> Value {
    let tool = |name: &str, description: &str, properties: Value, required: &[&str]| {
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
    json!([
        tool(
            "open",
            "Bring up an app or a website, and work in its window.",
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
            json!({ "text": { "type": "string", "description": "The exact text to type." }, "number": number }),
            &["text"],
        ),
        tool(
            "press",
            "Press a key or a key combination in the app.",
            json!({ "keys": { "type": "string", "description": "Like return, escape, tab, down, pagedown, cmd+n, cmd+f." } }),
            &["keys"],
        ),
        tool(
            "menu",
            "Choose an item from the app's menu bar.",
            json!({ "path": { "type": "string", "description": "The menu and the item, like File > New Window." } }),
            &["path"],
        ),
        tool("look", "Show the screen again.", json!({}), &[]),
    ])
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

async fn settle(first: Duration, most: Duration) {
    tokio::time::sleep(first).await;
    let until = Instant::now() + most;
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

async fn screen() -> Seen {
    match control(json!({ "op": "look", "save": next() })).await {
        Ok(answer) => Seen::from(&answer),
        Err(text) => Seen { text: format!("the screen could not be read: {text}"), image: None, print: Vec::new() },
    }
}

fn whole(args: &Map<String, Value>) -> Option<u64> {
    match args.get("number")? {
        Value::Number(n) => n.as_u64().or_else(|| n.as_f64().map(|f| f as u64)),
        Value::String(s) => s.trim().trim_matches(['[', ']']).parse().ok(),
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

async fn find(llm: &Llm, what: &str, image: &str) -> Option<(f64, f64)> {
    let question = format!("{POINT}{what}");
    let (x, y) = llm.see(&question, image).await.ok().as_deref().and_then(boxed)?;
    let Ok(near) = control(json!({ "op": "closer", "x": x, "y": y })).await else { return Some((x, y)) };
    let (Some(piece), Some(left), Some(top), Some(share)) =
        (near["image"].as_str(), near["left"].as_f64(), near["top"].as_f64(), near["share"].as_f64())
    else {
        return Some((x, y));
    };
    match llm.see(&question, piece).await.ok().as_deref().and_then(boxed) {
        Some((nx, ny)) => Some((left + nx * share, top + ny * share)),
        None => Some((x, y)),
    }
}

async fn click(llm: &Llm, call: &Call, before: &Seen) -> (Result<String, String>, Seen) {
    let what = call.args.get("what").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let mut spot = None;
    if whole(&call.args).is_none() && !what.is_empty() {
        let Some(image) = &before.image else {
            return (Err("there is no picture to find it in".to_string()), screen().await);
        };
        spot = find(llm, &what, image).await;
        if spot.is_none() {
            return (Err(UNSEEN.to_string()), screen().await);
        }
    }
    let mut did = Err("it could not be clicked".to_string());
    for mode in ["", "quiet", "real"] {
        let request = match spot {
            Some((x, y)) => json!({ "op": "click", "x": x, "y": y, "mode": mode }),
            None => json!({ "op": "click", "number": whole(&call.args), "mode": mode }),
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
        settle(SETTLE, SETTLED).await;
        let after = screen().await;
        if !after.same(before) {
            return (did, after);
        }
    }
    (did, screen().await)
}

async fn step(llm: &Llm, call: &Call, before: &Seen) -> (String, Seen) {
    let text = |key: &str| call.args.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let name = call.name.as_str();
    let (did, after) = match name {
        "look" => (Ok("looked again".to_string()), screen().await),
        "click" => click(llm, call, before).await,
        "open" => {
            let what = text("what");
            let was = control(json!({ "op": "glance" })).await.ok().map(|answer| Seen::from(&answer));
            let site = what.contains('.') && !what.contains(' ');
            let mut did = match site {
                true => act(json!({ "op": "visit", "what": what })).await,
                false => Err(String::new()),
            };
            if did.is_err() {
                let _ = act(json!({ "op": "hold" })).await;
                did = tools::launch(&what, true).await.map_err(|e| format!("{e:#}"));
                if did.is_ok()
                    && let Err(e) = act(json!({ "op": "aim", "what": what })).await
                {
                    did = Err(e);
                }
            }
            let until = Instant::now() + ARRIVE;
            while Instant::now() < until {
                tokio::time::sleep(STILL).await;
                let now = control(json!({ "op": "glance" })).await.ok().map(|answer| Seen::from(&answer));
                match (&was, &now) {
                    (Some(was), Some(now)) if was.same(now) => {}
                    _ => break,
                }
            }
            settle(STILL, LOADED).await;
            (did, screen().await)
        }
        "type" | "press" | "menu" => {
            let request = match name {
                "type" => json!({ "op": "type", "number": whole(&call.args), "text": text("text") }),
                "press" => json!({ "op": "press", "keys": text("keys") }),
                _ => json!({ "op": "menu", "path": text("path") }),
            };
            let did = act(request).await;
            settle(SETTLE, SETTLED).await;
            (did, screen().await)
        }
        _ => (Err("there is no such tool".to_string()), screen().await),
    };
    let mut did = did.unwrap_or_else(|e| format!("{name} failed: {e}"));
    if name != "look" && after.same(before) {
        did = format!("{did}. {SAME}");
    }
    keep(&after, &format!("{name} {} -> {did}", Value::Object(call.args.clone())));
    (did, after)
}

fn card(task: &str, doing: &str) -> notch::Card {
    notch::Card {
        title: format!("Working: {task}"),
        body: doing.to_string(),
        actions: vec![notch::Action { id: STOP.into(), label: "Stop".into(), primary: false }],
    }
}

async fn drive(
    llm: &Llm,
    talking: &AtomicBool,
    events: &UnboundedSender<Event>,
    task: &str,
    steps: &[String],
    id: u64,
) -> Result<String> {
    act(json!({ "op": "start" })).await.map_err(|e| anyhow!("{e}"))?;
    let _ = std::fs::remove_dir_all(saved());
    let _ = std::fs::create_dir_all(saved());
    SHOT.store(0, Relaxed);
    let mut before = match control(json!({ "op": "look", "save": next() })).await {
        Ok(answer) => Seen::from(&answer),
        Err(text) if text.starts_with(UNOPENED) => Seen { text, image: None, print: Vec::new() },
        Err(text) => bail!("{text}"),
    };
    let plan: Vec<String> = steps.iter().enumerate().map(|(i, s)| format!("{}. {s}", i + 1)).collect();
    let schema = schema();
    let mut lead = format!("What they asked for: {task}\n\nThe plan they approved:\n{}", plan.join("\n"));
    let known = tools::memory();
    if !known.is_empty() {
        lead.push_str(&format!("\n\nKnown about the person:\n- {}", known.join("\n- ")));
    }
    keep(&before, &lead);
    let first = Seen { text: before.text.clone(), image: before.image.clone(), print: Vec::new() };
    let mut messages = vec![Message::system(PROMPT), first.message(&lead)];
    let mut typed: Vec<String> = Vec::new();
    for _ in 0..MAX_STEPS {
        let waited = Instant::now();
        while talking.load(Relaxed) && waited.elapsed() < YIELD_MAX {
            tokio::time::sleep(YIELD_POLL).await;
        }
        let reply = llm.act(&messages, &schema).await?;
        let Some(call) = reply.calls.into_iter().next() else {
            let said = reply.text.trim().trim_matches('"').trim();
            let said = if said.is_empty() { format!("finished: {task}") } else { tools::clip(said) };
            let wrote: String = typed.join(" ").split_whitespace().collect::<Vec<_>>().join(" ");
            if wrote.is_empty() {
                return Ok(said);
            }
            let wrote: String = wrote.chars().take(TYPED_CHARS).collect();
            return Ok(format!("{said} What you typed: \"{wrote}\""));
        };
        let _ = events.send(Event::Doing(format!("{} {}", call.name, Value::Object(call.args.clone()))));
        let (did, now) = step(llm, &call, &before).await;
        log::info!("{} {} -> {did}", call.name, Value::Object(call.args.clone()));
        if call.name == "type" && !did.contains("failed") {
            typed.extend(call.args.get("text").and_then(Value::as_str).map(str::to_string));
        }
        notch::revise(id, &card(task, did.split(". Nothing").next().unwrap_or(&did)));
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
    match tokio::time::timeout(TOTAL, drive(llm, &talking, &events, task, &plan, 0)).await {
        Ok(out) => out,
        Err(_) => Err(anyhow!("it took too long and was stopped before finishing {task}")),
    }
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
) -> Result<Option<String>> {
    let (id, stopped) = notch::show(&card(task, "Starting"));
    let out = tokio::select! {
        out = tokio::time::timeout(TOTAL, drive(llm, talking, events, task, steps, id)) => match out {
            Ok(out) => out.map(Some),
            Err(_) => Err(anyhow!("it took too long and was stopped before finishing {task}")),
        },
        _ = stopped => Ok(None),
        _ = HALT.notified() => Ok(None),
    };
    notch::dismiss(id);
    out
}
