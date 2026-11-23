use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::Relaxed;
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
const SETTLE: Duration = Duration::from_millis(700);
const LAUNCH: Duration = Duration::from_millis(1800);
const YIELD_POLL: Duration = Duration::from_millis(20);
const YIELD_MAX: Duration = Duration::from_secs(4);
const SCREEN: &str = "\n\nThe screen now, as a picture and as a list:\n";
const STOP: &str = "stop";
const TYPED_CHARS: usize = 400;
const MOVED: f64 = 0.8;
const SAME: &str = "Nothing on the screen changed, so that did nothing. Do not repeat it. Try a different way, like a key, a menu item, another control, or grid.";
const IDLE: &str = "nothing is being done on their computer right now";
const HALTED: &str = "stopped what was being done on their computer";
pub const BUSY: &str = "something else is still being done on their computer; it has to finish or be stopped first";

const PROMPT: &str = "You operate a person's Mac for them. They approved the plan below, and you carry it out one action at a time. You work in one app's window in the background while they use their computer, so you never need the window to be in front.

After each action you get a picture of the window and a list of its controls. In the picture, each control has a box with a number. The same numbers are in the list, in square brackets, with the control's name. Lines in the list without a number are text in the window. Look at the picture to understand what is on the screen and where things are, and use the list to be sure of a control's name.

Rules:
- Call exactly one tool at a time.
- Numbers change after every action. Only use numbers from the latest screen.
- Do what the plan says and nothing else. Do not add steps of your own.
- open brings up an app by name or a website by address, and makes it the window you work in. Start with open unless the window you need is already the one shown.
- click presses a control by its number. type types text, into a control if you give its number, otherwise where the cursor is. Typing adds to what is already there.
- press sends keys to the app, like return, escape, tab, down, pagedown, cmd+n or cmd+f. Press pagedown to see further down a page.
- menu picks from the app's menu bar, like File > New Window. It is the surest way to do something an app has a menu item for.
- When the thing you need is in the picture but has no number, call grid. It shows the picture divided into squares named A1 to F4. Call grid again with a square to get closer, then call click with the square that the thing is in the middle of.
- look shows the screen again. Use it when something is still loading.
- If an action fails or the screen did not change the way you expected, try another way.
- Before you finish, check that the latest picture shows the plan is done. Typing in a search box or seeing a result listed is not the same as having opened it. If the picture does not show it, keep going or say it did not work.
- If you cannot finish after trying, stop and say what got in the way.
- When the plan is done, do not call a tool. Write one or two short plain sentences that Milo will say out loud: what was done, or what you found if they asked for information. Take every fact from the screen.";

static RUNNING: AtomicBool = AtomicBool::new(false);
static HALT: Notify = Notify::const_new();

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
    let number = json!({ "type": "integer", "description": "The number of a control on the latest screen." });
    let cell = json!({ "type": "string", "description": "A square of the grid, from A1 to F4." });
    json!([
        tool(
            "open",
            "Bring up an app or a website, and work in its window.",
            json!({ "what": { "type": "string", "description": "An app name like Mail, or a web address like mail.google.com." } }),
            &["what"],
        ),
        tool(
            "click",
            "Press a control by its number, or the middle of a grid square.",
            json!({ "number": number, "cell": cell }),
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
        tool(
            "grid",
            "Show the picture divided into named squares, to click something that has no number. Give a square to get a closer view of it.",
            json!({ "cell": cell }),
            &[],
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

async fn screen() -> Seen {
    match control(json!({ "op": "look" })).await {
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

async fn click(call: &Call, before: &Seen) -> (Result<String, String>, Seen) {
    let cell = call.args.get("cell").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let modes: &[&str] = if cell.is_empty() { &["", "quiet", "real"] } else { &["quiet", "real"] };
    let mut did = Err("it could not be clicked".to_string());
    for mode in modes {
        let request = match cell.is_empty() {
            true => json!({ "op": "click", "number": whole(&call.args), "mode": mode }),
            false => json!({ "op": "click", "cell": cell, "mode": mode }),
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
        tokio::time::sleep(SETTLE).await;
        let after = screen().await;
        if !after.same(before) {
            return (did, after);
        }
    }
    (did, screen().await)
}

async fn step(call: &Call, before: &Seen) -> (String, Seen) {
    let text = |key: &str| call.args.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let name = call.name.as_str();
    let (did, after) = match name {
        "look" => (Ok("looked again".to_string()), screen().await),
        "grid" => match control(json!({ "op": "grid", "cell": text("cell") })).await {
            Ok(answer) => (Ok("showing the grid".to_string()), Seen::from(&answer)),
            Err(e) => (Err(e), screen().await),
        },
        "click" => click(call, before).await,
        "open" => {
            let what = text("what");
            let mut did = tools::launch(&what, true).await.map_err(|e| format!("{e:#}"));
            tokio::time::sleep(LAUNCH).await;
            if did.is_ok()
                && let Err(e) = act(json!({ "op": "aim", "what": what })).await
            {
                did = Err(e);
            }
            (did, screen().await)
        }
        "type" | "press" | "menu" => {
            let request = match name {
                "type" => json!({ "op": "type", "number": whole(&call.args), "text": text("text") }),
                "press" => json!({ "op": "press", "keys": text("keys") }),
                _ => json!({ "op": "menu", "path": text("path") }),
            };
            let did = act(request).await;
            tokio::time::sleep(SETTLE).await;
            (did, screen().await)
        }
        _ => (Err("there is no such tool".to_string()), screen().await),
    };
    let mut did = did.unwrap_or_else(|e| format!("{name} failed: {e}"));
    if !matches!(name, "look" | "grid") && after.same(before) {
        did = format!("{did}. {SAME}");
    }
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
    let first = control(json!({ "op": "look" })).await.map_err(|e| anyhow!("{e}"))?;
    let mut before = Seen::from(&first);
    let plan: Vec<String> = steps.iter().enumerate().map(|(i, s)| format!("{}. {s}", i + 1)).collect();
    let schema = schema();
    let lead = format!("What they asked for: {task}\n\nThe plan they approved:\n{}", plan.join("\n"));
    let mut messages = vec![Message::system(PROMPT), Seen::from(&first).message(&lead)];
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
        let (did, now) = step(&call, &before).await;
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
