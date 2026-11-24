use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use llm::{Call, Llm, Message};
use serde_json::{Map, Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::computer;
use crate::controller::Event;

const HTTP_TIMEOUT: Duration = Duration::from_secs(8);
const RUN_TIMEOUT: Duration = Duration::from_secs(20);
const APPROVE_TIMEOUT: Duration = Duration::from_secs(45);
const ASKED: &str = "asked them to approve it on their screen; it runs if they approve";
const RESULT_CHARS: usize = 700;
const FILE_SHOWN: usize = 900;
static PLANS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static NEWER: tokio::sync::Notify = tokio::sync::Notify::const_new();
const DONE_KEPT: usize = 4;
const SCRIPTED: Duration = Duration::from_secs(20);
const PEOPLE_LISTED: usize = 80;
const CHATS: &str = "500";
const IMSG: [&str; 3] = ["/opt/homebrew/bin/imsg", "/usr/local/bin/imsg", "imsg"];
const NO_CHATS: &str = "imsg could not read any conversations. The terminal app that runs Milo needs Full Disk Access, under Privacy & Security in System Settings";
const NO_NAMES: &str = "imsg could read the conversations but not the names of the people in them. The terminal app that runs Milo needs access to Contacts, or Full Disk Access, under Privacy & Security in System Settings";
const NO_IMSG: &str = "nothing was sent: the imsg command is not installed on this Mac. They can install it with: brew install steipete/tap/imsg";
const DONE_CHARS: usize = 90;
const NOTHING: &str = "nothing was done: the helper has no way to do what they asked. All it can do is: {abilities}. If what they asked is not one of those, tell them plainly that you can't do that. If it is one of those and a detail is missing, ask for the detail";
const MAX_CALLS: usize = 2;
const MAX_STEPS: usize = 4;
const MEMORY_LINES: usize = 60;
const TURNED: &str =
    "the helper can do what they just asked after all, and has put it on their screen for them to approve";
const CONFUSED: &str = "the helper failed: it could not work out the request; ask them to say it again";
const MAX_TIMER: f64 = 24.0 * 3600.0;
const YIELD_POLL: Duration = Duration::from_millis(20);
const YIELD_MAX: Duration = Duration::from_secs(4);

const DOER_PROMPT: &str = "You work silently beside Milo, who is on a live voice call with a person. You never speak. \
Your only job is to notice when the person asks for something one of your tools does, and call that tool.

Rules:
- Milo cannot do anything itself. Nothing ever happens unless you call a tool, no matter what Milo says in the call. You alone decide whether something can be done.
- Use the most direct tool. media pauses, resumes and skips what is playing. close_tab and show_tab handle a browser tab. file writes text into a file, and opens the file in an app when you name one, so when they want something written, call file alone. message sends a text message to someone, so when they want someone texted, call message alone. open brings up an app or a site. Use computer only for what none of those does.
- When they correct or change what they just asked, call the tool once more with the corrected request in full. It replaces the earlier one.
- Act on the person's latest message. The earlier lines are only there so you can work out what they mean, like which city or which app.
- Call a tool when they ask for something a tool does, or ask a question a tool answers, like what the weather is. A request can be casual: can you open chrome, check boston, remind me in five.
- Already done lists what you did before. If they ask for the same thing again, or say it did not happen, do it again.
- The words come from speech recognition, so expect small mistakes. Write names out in full: SF is San Francisco.
- If they are only chatting, thinking out loud, describing how Milo works, or asking for something no tool does, do nothing.
- Fill arguments only from what they said. Leave out anything they did not say.
- Memory: when they tell you a lasting fact about themselves or their life, like their name, where they live, their people and pets, their work, what they like, or a plan, call remember even though they did not ask. Also call it when they ask you to remember, note or write something down. Do not remember small talk, passing moods, questions, or anything already listed under Known about the person. If they ask to write something down but have not said what yet, do nothing.
- Being asked whether you can do something is not a request to do it. Can you take notes is a question; do nothing.
- To do nothing, reply with exactly: none

After a tool answers:
- Check that it is really what they asked for. If it is about the wrong person, place or thing, or it failed in a way you can fix, call a tool again with something better, like the full correct name. Do not give up after one bad result, and never pass on a result that is about the wrong thing.
- When you have what they asked for, stop calling tools and write one or two short plain sentences that Milo will say to them: only the facts they asked for, taken from the tool results, nothing else.
- If the results do not contain what they asked for, say that in one sentence. Never fill a gap from memory.
- If it could not be done, say what went wrong in one sentence.";

#[derive(Debug)]
struct Skipped;

impl std::fmt::Display for Skipped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("they did not approve it on their screen")
    }
}

impl std::error::Error for Skipped {}

enum Kind {
    Weather,
    Timer,
    Open,
    Remember,
    Forget,
    Computer,
    Halt,
    Message,
    See,
    Media,
    CloseTab,
    ShowTab,
    File,
    Command(Vec<String>),
}

struct Tool {
    name: String,
    description: String,
    can: String,
    parameters: Value,
    kind: Kind,
}

fn tool(name: &str, description: &str, can: &str, properties: Value, required: &[&str], kind: Kind) -> Tool {
    Tool {
        name: name.into(),
        description: description.into(),
        can: can.into(),
        parameters: json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        }),
        kind,
    }
}

pub struct Tools {
    list: Vec<Tool>,
    http: reqwest::Client,
    events: UnboundedSender<Event>,
    llm: Llm,
    talking: Arc<AtomicBool>,
}

impl Tools {
    pub fn load(events: UnboundedSender<Event>, llm: Llm, talking: Arc<AtomicBool>) -> Tools {
        let mut list = vec![
            tool(
                "weather",
                "Get the current weather and the forecast for today and tomorrow in a city.",
                "check the weather",
                json!({
                    "place": {
                        "type": "string",
                        "description": "Full city name, like Boston or San Francisco. Leave out if they did not name a place.",
                    },
                }),
                &[],
                Kind::Weather,
            ),
            tool(
                "timer",
                "Start a countdown timer or a timed reminder. Milo tells the person when it finishes.",
                "set a timer",
                json!({
                    "minutes": { "type": "number", "description": "Length in minutes. Use this or seconds." },
                    "seconds": { "type": "number", "description": "Length in seconds. Use this or minutes." },
                    "label": { "type": "string", "description": "What the timer or reminder is for, if they said." },
                }),
                &[],
                Kind::Timer,
            ),
            tool(
                "open",
                "Open an app or a website on the person's computer.",
                "open an app or a website on their computer",
                json!({
                    "what": {
                        "type": "string",
                        "description": "An app name like Spotify, or a web address like youtube.com.",
                    },
                }),
                &["what"],
                Kind::Open,
            ),
            tool(
                "remember",
                "Save something to long-term memory: a lasting fact about the person, or anything they ask to remember, note or write down.",
                "remember things for them, from this call and earlier ones",
                json!({
                    "fact": {
                        "type": "string",
                        "description": "One short sentence in the third person, like Their name is Sam. or They need to buy milk.",
                    },
                }),
                &["fact"],
                Kind::Remember,
            ),
            tool(
                "forget",
                "Remove something from long-term memory when they ask to forget or delete it.",
                "forget something they asked you to remember",
                json!({ "what": { "type": "string", "description": "A few words from the memory to remove." } }),
                &["what"],
                Kind::Forget,
            ),
        ];
        if notch::on() {
            list.push(tool(
                "computer",
                "Do something on the person's computer by using its apps and its browser for them: clicking, typing and choosing from menus. Use it for anything that takes more than opening an app or a website, like sending an email, writing a note, searching a site, changing a setting, playing, pausing or stopping a song or a video, closing or switching a tab, or reading something off a page. It opens the apps and websites it needs, so do not call open as well. It remembers where it worked last, so it can go on with that video, that tab or that page. If they have not said something the task needs, like what an email should say or who it is for, do not call this: Milo will ask them.",
                "do things in their apps and their browser for them, like clicking a button, typing, playing or pausing a video, closing a tab, filling in a form or reading a page",
                json!({
                    "task": {
                        "type": "string",
                        "description": "A short name for what they asked, like Send Priya a message or Play some jazz.",
                    },
                    "steps": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "The plan, as one to six short steps in order, like Open Messages. Each step names the app, the site or the tab it is about, and the exact words to type, taken from what they said. Never make up words to type.",
                    },
                    "show": {
                        "type": "boolean",
                        "description": "True only when they asked to see, watch or be shown it, like play a video or pull up a page. Then it is brought to the front of their screen when the plan is done. Leave it out when they just want something done.",
                    },
                }),
                &["task", "steps"],
                Kind::Computer,
            ));
            list.push(tool(
                "see",
                "Look at the window on the person's screen and answer a question about it. Use it when they ask what is on their screen, ask you to look at it, or say that something there is wrong.",
                "look at the window on their screen and say what is there",
                json!({ "question": { "type": "string", "description": "What to find out, like What is in this window and is anything wrong." } }),
                &[],
                Kind::See,
            ));
            list.push(tool(
                "stop",
                "Stop the plan that computer is carrying out right now. Use it only while a plan is still being carried out, when they say to stop, cancel or never mind, or that it is not working. It does not stop a video, music or anything else that is playing: use computer for that.",
                "stop a plan that is being carried out on their computer",
                json!({}),
                &[],
                Kind::Halt,
            ));
        }
        if notch::on() {
            list.push(tool(
                "media",
                "Pause, stop, resume or skip whatever video or music is playing on the person's computer, in the browser or in an app. Use it whenever they want something that is playing to stop, pause, go on or skip, instead of computer or stop.",
                "pause, resume or skip a video or music that is playing",
                json!({ "action": { "type": "string", "enum": ["pause", "play", "next", "previous"] } }),
                &["action"],
                Kind::Media,
            ));
            list.push(tool(
                "close_tab",
                "Close a tab in the person's browser. Use it instead of computer when all they want is a tab closed.",
                "close a browser tab",
                json!({ "which": { "type": "string", "description": "A word or two from the tab's title or address, like youtube. Leave it out when they say that tab, this tab or the tab, which means the one that was just worked in or that is showing." } }),
                &[],
                Kind::CloseTab,
            ));
            list.push(tool(
                "show_tab",
                "Bring a tab that is already open in the person's browser to the front.",
                "switch to a browser tab that is open",
                json!({ "which": { "type": "string", "description": "A word or two from the tab's title or address, like gmail." } }),
                &["which"],
                Kind::ShowTab,
            ));
            list.push(tool(
                "file",
                "Write a text file on the person's computer, like a note, a list or a short program, and open it in an app if they want. Use it instead of computer whenever they ask for something to be written into a file: it writes the exact text, so nothing has to be typed in a window. You write the whole content yourself.",
                "write a text file, like a note or a short program, and open it in an app",
                json!({
                    "path": { "type": "string", "description": "Where the file goes, under their home folder, like Desktop/hello.py or dev/notes.txt." },
                    "content": { "type": "string", "description": "The whole text of the file." },
                    "app": { "type": "string", "description": "An app to open the file in, like Visual Studio Code or TextEdit. Leave it out to only write the file." },
                }),
                &["path", "content"],
                Kind::File,
            ));
            list.push(tool(
                "message",
                "Send a text message to someone through the Messages app, as an iMessage or an SMS. Use it instead of computer whenever they want someone texted or messaged. It finds the person by name among the people they have conversations with. If they have not said who it is for or what it should say, do not call this: Milo will ask them.",
                "send a text message to someone through Messages",
                json!({
                    "to": { "type": "string", "description": "Who it is for, by the name they are saved under, like Priya Shah. When Known about the person says what someone is saved as, use that name." },
                    "text": { "type": "string", "description": "The exact words of the message, taken from what they said." },
                }),
                &["to", "text"],
                Kind::Message,
            ));
        }
        match custom() {
            Ok(more) => {
                for t in more {
                    if list.iter().any(|have| have.name == t.name) {
                        log::warn!("tool {:?} in tools.json has the name of an existing tool, skipping", t.name);
                    } else {
                        list.push(t);
                    }
                }
            }
            Err(e) => log::warn!("tools.json ignored: {e:#}"),
        }
        let http = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .connect_timeout(Duration::from_secs(4))
            .user_agent("milo/0.1")
            .build()
            .expect("reqwest client");
        Tools { list, http, events, llm, talking }
    }

    pub fn abilities(&self) -> String {
        self.list.iter().map(|t| t.can.as_str()).collect::<Vec<_>>().join(", ")
    }

    fn schema(&self) -> Value {
        Value::Array(
            self.list
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "function": { "name": t.name, "description": t.description, "parameters": t.parameters },
                    })
                })
                .collect(),
        )
    }

    fn direct(&self, name: &str) -> bool {
        self.list.iter().any(|t| t.name == name && !matches!(t.kind, Kind::Command(_)))
    }

    fn quiet(&self, name: &str) -> bool {
        self.list.iter().any(|t| t.name == name && matches!(t.kind, Kind::Remember))
    }

    fn gated(&self, name: &str) -> bool {
        notch::on()
            && self.list.iter().any(|t| {
                t.name == name
                    && matches!(
                        t.kind,
                        Kind::Open | Kind::Computer | Kind::CloseTab | Kind::File | Kind::Message | Kind::Command(_)
                    )
            })
    }

    async fn attempt(&self, call: &Call) -> Result<String> {
        let Some(t) = self.list.iter().find(|t| t.name == call.name) else { bail!("there is no such tool") };
        match &t.kind {
            Kind::Weather => self.weather(&call.args).await,
            Kind::Timer => self.timer(&call.args),
            Kind::Open => open(&call.args).await,
            Kind::Remember => remember(&call.args),
            Kind::Forget => forget(&call.args),
            Kind::Computer => self.computer(&call.args).await,
            Kind::Halt => computer::halt(),
            Kind::See => {
                let question = text(&call.args, "question").unwrap_or_else(|| "What is in this window?".to_string());
                computer::glimpse(&self.llm, &question).await
            }
            Kind::Media => computer::media(&text(&call.args, "action").unwrap_or_else(|| "pause".to_string())).await,
            Kind::ShowTab => {
                let (tab, title) = computer::pick(&text(&call.args, "which").unwrap_or_default()).await?;
                computer::shut(&tab, true).await?;
                Ok(format!("brought the tab {} to the front", clip(&title)))
            }
            Kind::CloseTab => {
                let (tab, title) = computer::pick(&text(&call.args, "which").unwrap_or_default()).await?;
                allow("Close this tab?", &title).await?;
                computer::shut(&tab, false).await?;
                Ok(format!("closed the tab {}", clip(&title)))
            }
            Kind::File => file(&call.args).await,
            Kind::Message => message(&call.args).await,
            Kind::Command(argv) => command(t, argv, &call.args).await,
        }
    }

    async fn run(&self, call: &Call) -> (String, bool) {
        match self.attempt(call).await {
            Ok(text) => (clip(&text), true),
            Err(e) => (format!("{} failed: {}", call.name, clip(&format!("{e:#}"))), false),
        }
    }

    fn later(self: Arc<Self>, call: Call) {
        tokio::spawn(async move {
            match self.attempt(&call).await {
                Ok(text) => {
                    let _ = self.events.send(Event::Aside(clip(&text)));
                }
                Err(e) if e.is::<Skipped>() => {}
                Err(e) => {
                    let _ =
                        self.events.send(Event::Aside(format!("{} failed: {}", call.name, clip(&format!("{e:#}")))));
                }
            }
        });
    }

    async fn get(&self, url: &str) -> Result<Value> {
        let resp = self.http.get(url).send().await.map_err(|_| anyhow!("could not reach the service"))?;
        if !resp.status().is_success() {
            bail!("the service answered {}", resp.status().as_u16());
        }
        resp.json().await.map_err(|_| anyhow!("the service sent something unreadable"))
    }

    async fn weather(&self, args: &Map<String, Value>) -> Result<String> {
        let Some(place) = text(args, "place") else {
            bail!("they did not say which city; ask them which city");
        };
        let city = place.split(',').next().unwrap_or(&place).trim();
        let city = match city.to_lowercase().replace('.', "").as_str() {
            "sf" | "san fran" | "frisco" => "San Francisco",
            "nyc" | "new york city" => "New York",
            "la" => "Los Angeles",
            "dc" | "washington dc" => "Washington",
            "philly" => "Philadelphia",
            "vegas" => "Las Vegas",
            _ => city,
        };
        let found = self
            .get(&format!(
                "https://geocoding-api.open-meteo.com/v1/search?name={}&count=1&language=en&format=json",
                encode(city)
            ))
            .await?;
        let hit = found["results"].get(0).ok_or_else(|| anyhow!("could not find a place called {place}"))?;
        let (lat, lon) = (hit["latitude"].as_f64().unwrap_or(0.0), hit["longitude"].as_f64().unwrap_or(0.0));
        let name = match (hit["name"].as_str(), hit["admin1"].as_str()) {
            (Some(n), Some(a)) if n != a => format!("{n}, {a}"),
            (Some(n), _) => n.to_string(),
            _ => place.clone(),
        };
        let w = self
            .get(&format!(
                "https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}\
                 &current=temperature_2m,apparent_temperature,weather_code,wind_speed_10m\
                 &daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max\
                 &temperature_unit=fahrenheit&wind_speed_unit=mph&timezone=auto&forecast_days=2"
            ))
            .await?;
        let num = |v: &Value| v.as_f64().map(|f| f.round() as i64);
        let now = &w["current"];
        let temp = num(&now["temperature_2m"]).ok_or_else(|| anyhow!("the forecast came back empty"))?;
        let mut out = format!("weather in {name}: {temp}°F and {} right now", sky(now["weather_code"].as_i64()));
        if let Some(feels) = num(&now["apparent_temperature"]).filter(|f| (f - temp).abs() >= 4) {
            out.push_str(&format!(", feels like {feels}"));
        }
        if let Some(wind) = num(&now["wind_speed_10m"]).filter(|w| *w >= 15) {
            out.push_str(&format!(", wind {wind} mph"));
        }
        out.push('.');
        let d = &w["daily"];
        for (i, day) in ["Today", "Tomorrow"].iter().enumerate() {
            let (Some(hi), Some(lo)) = (num(&d["temperature_2m_max"][i]), num(&d["temperature_2m_min"][i])) else {
                continue;
            };
            out.push_str(&format!(" {day}: {}, high {hi}, low {lo}", sky(d["weather_code"][i].as_i64())));
            if let Some(rain) = num(&d["precipitation_probability_max"][i]) {
                out.push_str(&format!(", {rain}% chance of rain"));
            }
            out.push('.');
        }
        Ok(out)
    }

    async fn computer(&self, args: &Map<String, Value>) -> Result<String> {
        let task = text(args, "task").ok_or_else(|| anyhow!("they did not say what to do"))?;
        let show = args.get("show").is_some_and(|show| show == &json!(true) || show == &json!("true"));
        let mut steps: Vec<String> = match args.get("steps") {
            Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).map(str::to_string).collect(),
            Some(Value::String(lines)) => lines.lines().map(str::to_string).collect(),
            _ => Vec::new(),
        };
        steps.retain_mut(|s| {
            *s = s
                .trim()
                .trim_start_matches(|c: char| c.is_ascii_digit() || matches!(c, '.' | ')' | '-'))
                .trim()
                .to_string();
            !s.is_empty()
        });
        if steps.is_empty() {
            steps.push(task.clone());
        }
        if computer::running() {
            bail!(computer::BUSY);
        }
        let plan: Vec<String> = steps.iter().enumerate().map(|(i, s)| format!("{}. {s}", i + 1)).collect();
        let mine = PLANS.fetch_add(1, Relaxed) + 1;
        NEWER.notify_waiters();
        let newer = async {
            while PLANS.load(Relaxed) == mine {
                NEWER.notified().await;
            }
        };
        if notch::on() {
            match notch::approve_unless(&task, &plan.join("\n"), APPROVE_TIMEOUT, newer).await {
                Some(true) => {}
                _ => return Err(Skipped.into()),
            }
        }
        let _claim = computer::claim()?;
        match computer::run(&self.llm, &self.talking, &self.events, &task, &steps, show).await? {
            Some(said) => Ok(said),
            None => Err(Skipped.into()),
        }
    }

    fn timer(&self, args: &Map<String, Value>) -> Result<String> {
        let secs = number(args, "minutes").unwrap_or(0.0) * 60.0 + number(args, "seconds").unwrap_or(0.0);
        if !(1.0..=MAX_TIMER).contains(&secs) {
            bail!("they did not say a usable length; ask them how long");
        }
        let length = span(secs.round() as u64);
        let what = match text(args, "label") {
            Some(label) => format!("{length} timer for {label}"),
            None => format!("{length} timer"),
        };
        let (events, ring) = (self.events.clone(), format!("the {what} just finished"));
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs_f64(secs)).await;
            let _ = events.send(Event::Aside(ring));
        });
        Ok(format!("{what} started"))
    }
}

fn opener(what: &str, behind: bool) -> Result<tokio::process::Command> {
    let what = what.trim().to_string();
    if what.is_empty() {
        bail!("nothing to open");
    }
    let lower = what.to_lowercase();
    let site =
        lower.starts_with("http://") || lower.starts_with("https://") || (what.contains('.') && !what.contains(' '));
    let mut cmd = tokio::process::Command::new("/usr/bin/open");
    if behind {
        cmd.arg("-g");
    }
    if site {
        if lower.contains("://") && !lower.starts_with("http://") && !lower.starts_with("https://") {
            bail!("only web addresses and apps can be opened");
        }
        cmd.arg(if lower.contains("://") { what.clone() } else { format!("https://{what}") });
    } else {
        if what.contains('/') || what.starts_with('-') {
            bail!("only web addresses and apps can be opened");
        }
        cmd.args(["-a", &what]);
    }
    Ok(cmd)
}

pub async fn launch(what: &str, behind: bool) -> Result<String> {
    let mut cmd = opener(what, behind)?;
    let what = what.trim();
    let out = tokio::time::timeout(RUN_TIMEOUT, cmd.output()).await.map_err(|_| anyhow!("it took too long"))??;
    if !out.status.success() {
        bail!("could not find anything called {what} to open");
    }
    Ok(format!("opened {what}"))
}

async fn file(args: &Map<String, Value>) -> Result<String> {
    let given = text(args, "path").ok_or_else(|| anyhow!("they did not say where the file goes"))?;
    let content = args.get("content").and_then(Value::as_str).unwrap_or_default().to_string();
    if content.trim().is_empty() {
        bail!("there is nothing to write; ask them what the file should say");
    }
    let inner = given.trim().trim_start_matches("~/").trim_start_matches('/');
    let home = home_dir()?;
    let inner = inner.strip_prefix(&format!("{}/", home.to_string_lossy().trim_start_matches('/'))).unwrap_or(inner);
    let parts: Vec<&str> = inner.split('/').collect();
    if parts.iter().any(|part| part.is_empty() || *part == ".." || part.starts_with('.')) {
        bail!("the file has to be a plain path under their home folder");
    }
    let path = home.join(inner);
    let title = if path.exists() { format!("Replace ~/{inner}?") } else { format!("Write ~/{inner}?") };
    let shown: String = content.chars().take(FILE_SHOWN).collect();
    allow(&title, &shown).await?;
    if let Some(folder) = path.parent() {
        tokio::fs::create_dir_all(folder).await?;
    }
    tokio::fs::write(&path, &content).await?;
    let mut said = format!("wrote ~/{inner}");
    if let Some(app) = text(args, "app") {
        let name = computer::installed(&app).await.unwrap_or(app);
        let opened = tokio::process::Command::new("/usr/bin/open").args(["-a", &name]).arg(&path).output().await;
        match opened {
            Ok(out) if out.status.success() => said.push_str(&format!(" and opened it in {name}")),
            _ => said.push_str(&format!(", but could not open it in {name}")),
        }
    }
    Ok(said)
}

async fn imsg(args: &[&str]) -> Result<String> {
    let bin = IMSG.iter().find(|path| !path.contains('/') || std::path::Path::new(path).exists()).unwrap_or(&IMSG[2]);
    let run = tokio::process::Command::new(bin).args(args).stdin(std::process::Stdio::null()).output();
    let out = match tokio::time::timeout(SCRIPTED, run).await {
        Err(_) => bail!("nothing was sent: Messages did not answer in time"),
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => bail!(NO_IMSG),
        Ok(out) => out?,
    };
    if !out.status.success() {
        let said = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let said = if said.is_empty() { String::from_utf8_lossy(&out.stdout).trim().to_string() } else { said };
        bail!("nothing was sent: imsg failed: {}", clip(&said));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

struct Chat {
    id: i64,
    name: String,
    to: String,
    service: String,
}

async fn message(args: &Map<String, Value>) -> Result<String> {
    let to = text(args, "to").ok_or_else(|| anyhow!("they did not say who it is for; ask them"))?;
    let words = text(args, "text").ok_or_else(|| anyhow!("they did not say what it should say; ask them"))?;
    let listed = imsg(&["chats", "--limit", CHATS, "--json"]).await?;
    let mut chats: Vec<Chat> = Vec::new();
    for line in listed.lines() {
        let Ok(chat) = serde_json::from_str::<Value>(line) else { continue };
        if chat["is_group"] == true {
            continue;
        }
        let Some(id) = chat["id"].as_i64() else { continue };
        let field = |key: &str| chat[key].as_str().unwrap_or_default().trim().to_string();
        let to = field("identifier");
        let name = [field("contact_name"), field("display_name"), field("name")]
            .into_iter()
            .find(|name| !name.is_empty() && *name != to)
            .unwrap_or_default();
        if !name.is_empty() {
            chats.push(Chat { id, name, to, service: field("service") });
        }
    }
    if chats.is_empty() {
        let seen = listed.lines().filter(|line| line.trim_start().starts_with('{')).count();
        bail!("nothing was sent: {}", if seen == 0 { NO_CHATS } else { NO_NAMES });
    }
    let split = |name: &str| -> Vec<String> {
        name.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_string)
            .collect()
    };
    let wanted = to.to_lowercase();
    let asked = split(&to);
    let exact: Vec<&Chat> = chats.iter().filter(|chat| chat.name.to_lowercase() == wanted).collect();
    let hits: Vec<&Chat> = match exact.is_empty() {
        false => exact,
        true => chats
            .iter()
            .filter(|chat| {
                let have = split(&chat.name);
                !asked.is_empty() && asked.iter().all(|word| have.contains(word))
            })
            .collect(),
    };
    let mut names: Vec<&str> = hits.iter().map(|chat| chat.name.as_str()).collect();
    names.sort();
    names.dedup();
    let name = match names.as_slice() {
        [name] => *name,
        [] => {
            let mut all: Vec<&str> = Vec::new();
            for chat in &chats {
                if !all.contains(&chat.name.as_str()) {
                    all.push(&chat.name);
                }
            }
            all.truncate(PEOPLE_LISTED);
            bail!(
                "nothing was sent: none of their conversations in Messages is with someone saved as {to}. The people they have conversations with, newest first: {}. If one of them is plainly the person they mean, call message again with that exact name. Otherwise ask them what the person is saved as",
                all.join("; ")
            );
        }
        many => bail!("nothing was sent: more than one person matches {to}, so ask them which: {}", many.join("; ")),
    };
    let mine: Vec<&&Chat> = hits.iter().filter(|chat| chat.name == name).collect();
    let pick = mine.iter().find(|chat| chat.service.eq_ignore_ascii_case("imessage")).or(mine.first());
    let Some(chat) = pick else { bail!("nothing was sent: {to} could not be found") };
    allow(&format!("Message {name}?"), &format!("{words}\n\nTo {} by {}", chat.to, chat.service)).await?;
    imsg(&["send", "--chat-id", &chat.id.to_string(), "--text", &words]).await?;
    Ok(format!("sent the text \"{}\" to {name}", clip(&words)))
}

async fn open(args: &Map<String, Value>) -> Result<String> {
    let what = text(args, "what").ok_or_else(|| anyhow!("nothing to open"))?;
    allow(&format!("Open {what}?"), &shown(opener(&what, false)?.as_std())).await?;
    launch(&what, false).await
}

fn shown(cmd: &std::process::Command) -> String {
    let mut parts = vec![cmd.get_program().to_string_lossy().into_owned()];
    parts.extend(cmd.get_args().map(|a| a.to_string_lossy().into_owned()));
    parts.join(" ")
}

async fn allow(title: &str, detail: &str) -> Result<()> {
    if !notch::on() {
        return Ok(());
    }
    match notch::approve(title, detail, APPROVE_TIMEOUT).await {
        Some(true) => Ok(()),
        _ => Err(Skipped.into()),
    }
}

fn memory_path() -> Result<PathBuf> {
    let dir = home_dir()?.join(".config/milo");
    std::fs::create_dir_all(&dir).context("could not make the memory folder")?;
    Ok(dir.join("memory.md"))
}

pub fn memory() -> Vec<String> {
    let Ok(raw) = memory_path().and_then(|p| Ok(std::fs::read_to_string(p)?)) else { return Vec::new() };
    let all: Vec<String> =
        raw.lines().map(|l| l.trim().trim_start_matches('-').trim().to_string()).filter(|l| !l.is_empty()).collect();
    let from = all.len().saturating_sub(MEMORY_LINES);
    all[from..].to_vec()
}

pub fn memory_section() -> String {
    let known = memory();
    if known.is_empty() {
        return "What you remember about them: nothing yet. This is the first time you two have talked.".into();
    }
    format!(
        "What you remember about them, from this call and earlier ones. Use it the way a friend who knows them would, answer from it right away when they ask, and never read the list out:\n- {}",
        known.join("\n- ")
    )
}

fn remember(args: &Map<String, Value>) -> Result<String> {
    let fact = text(args, "fact").ok_or_else(|| anyhow!("nothing to remember"))?.replace('\n', " ");
    if memory().iter().any(|m| m.eq_ignore_ascii_case(&fact)) {
        return Ok(format!("already remembered: {fact}"));
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(memory_path()?)
        .context("could not open the memory file")?;
    writeln!(file, "- {fact}").context("could not save it")?;
    Ok(format!("remembered: {fact}"))
}

fn forget(args: &Map<String, Value>) -> Result<String> {
    let what = text(args, "what").ok_or_else(|| anyhow!("nothing to forget"))?.to_lowercase();
    let words: Vec<&str> = what.split_whitespace().filter(|w| w.len() > 2).collect();
    let path = memory_path()?;
    let raw = std::fs::read_to_string(&path).unwrap_or_default();
    let hit = |l: &str| !words.is_empty() && words.iter().all(|w| l.to_lowercase().contains(w));
    let gone: Vec<&str> = raw.lines().filter(|l| hit(l)).collect();
    if gone.is_empty() {
        bail!("nothing in memory matches {what}");
    }
    let kept: String = raw.lines().filter(|l| !hit(l)).map(|l| format!("{l}\n")).collect();
    std::fs::write(&path, kept).context("could not update memory")?;
    Ok(format!("forgot: {}", gone.iter().map(|l| l.trim_start_matches('-').trim()).collect::<Vec<_>>().join("; ")))
}

async fn command(t: &Tool, argv: &[String], args: &Map<String, Value>) -> Result<String> {
    let mut filled = Vec::with_capacity(argv.len());
    for part in argv {
        let mut part = part.clone();
        for name in t.parameters["properties"].as_object().into_iter().flat_map(Map::keys) {
            let slot = format!("{{{name}}}");
            if part.contains(&slot) {
                let value = text(args, name).ok_or_else(|| anyhow!("they did not say the {name}"))?;
                part = part.replace(&slot, &value);
            }
        }
        filled.push(part);
    }
    let (bin, rest) = filled.split_first().ok_or_else(|| anyhow!("it has no command"))?;
    let mut cmd = tokio::process::Command::new(bin);
    cmd.args(rest).stdin(std::process::Stdio::null()).kill_on_drop(true);
    allow(&format!("Run {}?", t.name), &shown(cmd.as_std())).await?;
    let out = tokio::time::timeout(RUN_TIMEOUT, cmd.output()).await.map_err(|_| anyhow!("it took too long"))?;
    let out = out.with_context(|| format!("could not run {bin}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("{}", err.lines().find(|l| !l.trim().is_empty()).unwrap_or("it did not work").trim());
    }
    let said = String::from_utf8_lossy(&out.stdout).split_whitespace().collect::<Vec<_>>().join(" ");
    Ok(if said.is_empty() { format!("{} done", t.name) } else { format!("{}: {said}", t.name) })
}

fn custom() -> Result<Vec<Tool>> {
    let path = home_dir()?.join(".config/milo/tools.json");
    let Ok(raw) = std::fs::read_to_string(&path) else { return Ok(Vec::new()) };
    let parsed: Value = serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
    let mut out = Vec::new();
    for entry in parsed.as_array().ok_or_else(|| anyhow!("{} must be a list", path.display()))? {
        let field = |k: &str| entry[k].as_str().map(str::trim).filter(|s| !s.is_empty());
        let (Some(name), Some(description)) = (field("name"), field("description")) else {
            bail!("every tool needs a name and a description");
        };
        if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            bail!("tool name {name:?} may only use letters, digits and underscores");
        }
        let argv: Vec<String> = entry["run"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        if argv.is_empty() {
            bail!("tool {name:?} needs a run list, like [\"shortcuts\", \"run\", \"Lights {{state}}\"]");
        }
        let parameters = match &entry["parameters"] {
            Value::Null => json!({ "type": "object", "properties": {} }),
            schema if schema["type"] == "object" => schema.clone(),
            _ => bail!("the parameters of tool {name:?} must be a JSON Schema with \"type\": \"object\""),
        };
        let mut can = description.trim_end_matches('.').chars();
        let can = can.next().map(|f| f.to_lowercase().chain(can).collect()).unwrap_or_default();
        out.push(Tool {
            name: name.into(),
            description: description.into(),
            can,
            parameters,
            kind: Kind::Command(argv),
        });
    }
    Ok(out)
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| anyhow!("no home folder"))
}

fn text(args: &Map<String, Value>, key: &str) -> Option<String> {
    let s = match args.get(key)? {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => return None,
    };
    (!s.is_empty() && !matches!(s.to_lowercase().as_str(), "null" | "none" | "unknown")).then_some(s)
}

fn number(args: &Map<String, Value>, key: &str) -> Option<f64> {
    match args.get(key)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.split_whitespace().next()?.parse().ok(),
        _ => None,
    }
    .filter(|n: &f64| n.is_finite() && *n > 0.0)
}

fn span(secs: u64) -> String {
    let unit = |n: u64, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    let (h, m, s) = (secs / 3600, secs % 3600 / 60, secs % 60);
    let mut parts = Vec::new();
    if h > 0 {
        parts.push(unit(h, "hour"));
    }
    if m > 0 {
        parts.push(unit(m, "minute"));
    }
    if s > 0 || parts.is_empty() {
        parts.push(unit(s, "second"));
    }
    parts.join(" ")
}

fn sky(code: Option<i64>) -> &'static str {
    match code {
        Some(0) => "clear",
        Some(1) => "mostly clear",
        Some(2) => "partly cloudy",
        Some(3) => "overcast",
        Some(45 | 48) => "foggy",
        Some(51 | 53 | 55 | 56 | 57) => "drizzling",
        Some(61 | 80) => "light rain",
        Some(63 | 81) => "rain",
        Some(65 | 82) => "heavy rain",
        Some(66 | 67) => "freezing rain",
        Some(71 | 85) => "light snow",
        Some(73) => "snow",
        Some(75 | 86) => "heavy snow",
        Some(77) => "snow flurries",
        Some(95) => "thunderstorms",
        Some(96 | 99) => "thunderstorms with hail",
        _ => "unsettled",
    }
}

fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn clip(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() <= RESULT_CHARS {
        return s.to_string();
    }
    let cut: String = s.chars().take(RESULT_CHARS).collect();
    match cut.rfind(". ") {
        Some(at) if at > RESULT_CHARS / 2 => cut[..=at].to_string(),
        _ => format!("{cut}…"),
    }
}

pub struct Look {
    pub lines: Vec<String>,
    pub time: String,
    pub promised: bool,
}

fn note(done: &mut VecDeque<(Instant, String, String)>, call: String, result: String) {
    done.push_back((Instant::now(), call, result));
    if done.len() > DONE_KEPT {
        done.pop_front();
    }
}

pub fn spawn(
    llm: Llm,
    tools: Arc<Tools>,
    talking: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,
    events: UnboundedSender<Event>,
) -> UnboundedSender<Look> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(work(llm, tools, talking, busy, events, rx));
    tx
}

async fn work(
    llm: Llm,
    tools: Arc<Tools>,
    talking: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,
    events: UnboundedSender<Event>,
    mut rx: UnboundedReceiver<Look>,
) {
    let schema = tools.schema();
    let system = Message::system(DOER_PROMPT);
    let mut done: VecDeque<(Instant, String, String)> = VecDeque::new();
    if let Err(e) = llm.warm_tools(&system, &schema).await {
        log::debug!("tool prompt warm-up failed: {e:#}");
    }
    let nothing = NOTHING.replace("{abilities}", &tools.abilities());
    while let Some(mut look) = rx.recv().await {
        while let Ok(newer) = rx.try_recv() {
            look = newer;
        }
        let Some(latest) = look.lines.pop() else { continue };
        let mut ask = format!("Local time: {}", look.time);
        if !look.lines.is_empty() {
            ask.push_str(&format!("\n\nEarlier in the call:\n{}", look.lines.join("\n")));
        }
        let known = memory();
        if !known.is_empty() {
            ask.push_str(&format!("\n\nKnown about the person:\n- {}", known.join("\n- ")));
        }
        if !done.is_empty() {
            ask.push_str("\n\nAlready done:");
            for (at, call, result) in &done {
                let short: String = result.chars().take(DONE_CHARS).collect();
                ask.push_str(&format!("\n- {} ago: {call} -> {short}", span(at.elapsed().as_secs())));
            }
        }
        ask.push_str(&format!("\n\nThe person's latest message:\n{latest}"));
        let mut messages = vec![system.clone(), Message::user(ask)];
        let mut tried: Vec<String> = Vec::new();
        let mut last: Option<String> = None;
        let mut report: Option<String> = None;
        let mut hushed = true;
        for step in 0..MAX_STEPS {
            let waited = Instant::now();
            while talking.load(Relaxed) && waited.elapsed() < YIELD_MAX {
                tokio::time::sleep(YIELD_POLL).await;
            }
            let reply = match llm.act(&messages, &schema).await {
                Ok(reply) => reply,
                Err(e) => {
                    log::warn!("tool step failed: {e:#}");
                    report =
                        last.clone().or_else(|| format!("{e:#}").contains("Tool call").then(|| CONFUSED.to_string()));
                    break;
                }
            };
            if reply.calls.is_empty() {
                let said = reply.text.trim();
                let said = said.strip_prefix("thought").unwrap_or(said).trim().trim_matches('"').trim();
                report = match step {
                    0 => look.promised.then(|| nothing.clone()),
                    _ if hushed && !look.promised => None,
                    _ if said.split_whitespace().count() < 3 || said.eq_ignore_ascii_case("none") => last.clone(),
                    _ => Some(clip(said)),
                };
                break;
            }
            let calls: Vec<Call> = reply.calls.into_iter().take(MAX_CALLS).collect();
            busy.store(true, Relaxed);
            messages.push(Message::called(&calls));
            let mut direct = step == 0 && calls.len() == 1;
            let mut asked = false;
            for call in &calls {
                let shown = format!("{} {}", call.name, Value::Object(call.args.clone()));
                if tools.gated(&call.name) {
                    let _ = events.send(Event::Doing(shown.clone()));
                    note(&mut done, shown, ASKED.to_string());
                    tools.clone().later(call.clone());
                    asked = true;
                    continue;
                }
                let (text, ok) = if tried.contains(&shown) {
                    (
                        "you already tried exactly this and it gave the same result; try something different or finish"
                            .to_string(),
                        false,
                    )
                } else {
                    let _ = events.send(Event::Doing(shown.clone()));
                    let out = tools.run(call).await;
                    note(&mut done, shown.clone(), out.0.clone());
                    last = Some(out.0.clone());
                    out
                };
                tried.push(shown);
                direct &= ok && tools.direct(&call.name);
                hushed &= ok && tools.quiet(&call.name);
                messages.push(Message::tool(&call.id, text));
            }
            if asked {
                report = last.clone().or_else(|| (!look.promised).then(|| TURNED.to_string()));
                break;
            }
            if direct || step + 1 == MAX_STEPS {
                report = if hushed && !look.promised { None } else { last.clone() };
                break;
            }
        }
        if let Some(report) = report {
            let _ = events.send(Event::Aside(report));
        }
        busy.store(false, Relaxed);
    }
}
