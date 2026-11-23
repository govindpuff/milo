use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use llm::{Call, Llm, Message};
use serde_json::{Map, Value, json};
use tokio::sync::Notify;
use tokio::sync::mpsc::UnboundedSender;

use crate::controller::Event;
use crate::tools;

const MAX_STEPS: usize = 30;
const TOTAL: Duration = Duration::from_secs(240);
const SETTLE: Duration = Duration::from_millis(700);
const LAUNCH: Duration = Duration::from_millis(1800);
const YIELD_POLL: Duration = Duration::from_millis(20);
const YIELD_MAX: Duration = Duration::from_secs(4);
const SCREEN: &str = "\n\nThe screen now:\n";
const STOP: &str = "stop";
const TYPED_CHARS: usize = 400;
const SAME: &str = "Nothing on the screen changed, so that did nothing. Do not repeat it. Try a different way, like a key, a menu item or another control.";
const IDLE: &str = "nothing is being done on their computer right now";
const HALTED: &str = "stopped what was being done on their computer";
pub const BUSY: &str = "something else is still being done on their computer; it has to finish or be stopped first";

const PROMPT: &str = "You operate a person's Mac for them. They approved the plan below, and you carry it out one action at a time.

You see the window in front as a list. A line that starts with a number in square brackets is a control you can use. The other lines are text on the screen.

Rules:
- Call exactly one tool at a time. After each call you get the screen as it is then.
- Numbers change after every call. Only use numbers from the latest screen.
- Do what the plan says and nothing else. Do not add steps of your own.
- click presses a control. type types text, into a control if you give its number, otherwise where the cursor is. Typing adds to what is already there.
- press sends keys, like return, escape, tab, down, pagedown, cmd+n or cmd+shift+g. Press pagedown to see further down a page.
- menu picks from the menu bar of the app in front, like File > New Window. It is the surest way to do something an app has a menu item for.
- open brings up an app by name or a website by address. Use it to switch apps.
- look shows the screen again. Use it when something is still loading.
- If an action fails or the screen did not change the way you expected, try another way, like a menu item or a key instead of a click.
- If you cannot finish after trying, stop and say what got in the way.
- Before you finish, check that the latest screen shows the plan is done. Typing in a search box or seeing a result listed is not the same as having opened it. If the screen does not show it, keep going or say it did not work.
- When the plan is done, do not call a tool. Write one or two short plain sentences that Milo will say out loud: what was done, or what you found if they asked for information. Take every fact from the screen.";

static RUNNING: AtomicBool = AtomicBool::new(false);
static HALT: Notify = Notify::const_new();

struct Running;

impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.store(false, Relaxed);
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
    let number =
        json!({ "type": "integer", "description": "The number in square brackets of a control on the latest screen." });
    json!([
        tool(
            "click",
            "Press a control: a button, link, checkbox, tab, row or field.",
            json!({ "number": number }),
            &["number"]
        ),
        tool(
            "type",
            "Type text into a control, or where the cursor is when no number is given.",
            json!({ "text": { "type": "string", "description": "The exact text to type." }, "number": number }),
            &["text"],
        ),
        tool(
            "press",
            "Press a key or a key combination.",
            json!({ "keys": { "type": "string", "description": "Like return, escape, tab, down, pagedown, cmd+n, cmd+shift+g." } }),
            &["keys"],
        ),
        tool(
            "menu",
            "Choose an item from the menu bar of the app in front.",
            json!({ "path": { "type": "string", "description": "The menu and the item, like File > New Window." } }),
            &["path"],
        ),
        tool(
            "open",
            "Bring up an app or a website.",
            json!({ "what": { "type": "string", "description": "An app name like Mail, or a web address like youtube.com." } }),
            &["what"],
        ),
        tool("look", "Show the screen again.", json!({}), &[]),
    ])
}

async fn control(request: Value) -> Result<String, String> {
    tokio::task::spawn_blocking(move || control::call(&request))
        .await
        .unwrap_or_else(|_| Err("it did not work".to_string()))
}

async fn screen() -> String {
    match control(json!({ "op": "look" })).await {
        Ok(text) => text,
        Err(text) => format!("the screen could not be read: {text}"),
    }
}

fn whole(args: &Map<String, Value>) -> Option<u64> {
    match args.get("number")? {
        Value::Number(n) => n.as_u64().or_else(|| n.as_f64().map(|f| f as u64)),
        Value::String(s) => s.trim().trim_matches(['[', ']']).parse().ok(),
        _ => None,
    }
}

async fn step(call: &Call, before: &str) -> (String, String) {
    let text = |key: &str| call.args.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let mut wait = SETTLE;
    let did = match call.name.as_str() {
        "look" => Ok("looked again".to_string()),
        "click" => control(json!({ "op": "click", "number": whole(&call.args) })).await,
        "type" => control(json!({ "op": "type", "number": whole(&call.args), "text": text("text") })).await,
        "press" => control(json!({ "op": "press", "keys": text("keys") })).await,
        "menu" => control(json!({ "op": "menu", "path": text("path") })).await,
        "open" => {
            wait = LAUNCH;
            tools::launch(&text("what")).await.map_err(|e| format!("{e:#}"))
        }
        _ => Err("there is no such tool".to_string()),
    };
    let failed = did.is_err();
    let mut did = did.unwrap_or_else(|e| format!("{} failed: {e}", call.name));
    tokio::time::sleep(wait).await;
    let mut after = screen().await;
    if call.name == "click" && !failed && after == before {
        let real = control(json!({ "op": "click", "number": whole(&call.args), "real": true })).await;
        if real.is_ok() {
            tokio::time::sleep(wait).await;
            after = screen().await;
        }
    }
    if call.name != "look" && after == before {
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
    let first = control(json!({ "op": "look" })).await.map_err(|e| anyhow!("{e}"))?;
    let plan: Vec<String> = steps.iter().enumerate().map(|(i, s)| format!("{}. {s}", i + 1)).collect();
    let schema = schema();
    let mut messages = vec![
        Message::system(PROMPT),
        Message::user(format!(
            "What they asked for: {task}\n\nThe plan they approved:\n{}{SCREEN}{first}",
            plan.join("\n")
        )),
    ];
    let mut typed: Vec<String> = Vec::new();
    let mut before = first.clone();
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
        messages.push(Message::called(std::slice::from_ref(&call)));
        let (did, now) = step(&call, &before).await;
        let result = format!("{did}{SCREEN}{now}");
        before = now;
        if call.name == "type" && !did.contains("failed") {
            typed.extend(call.args.get("text").and_then(Value::as_str).map(str::to_string));
        }
        notch::revise(id, &card(task, did.split(". Nothing").next().unwrap_or(&did)));
        for m in messages.iter_mut() {
            if let Some(at) = m.content.find(SCREEN) {
                m.content.truncate(at);
            }
        }
        messages.push(Message::tool(&call.id, result));
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
