use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use llm::{Call, Message, Role};
use serde_json::{Map, Value, json};
use tokio::sync::mpsc::UnboundedSender;

use crate::computer;
use crate::controller::Event;

const HTTP_TIMEOUT: Duration = Duration::from_secs(8);
const RUN_TIMEOUT: Duration = Duration::from_secs(20);
const APPROVE_TIMEOUT: Duration = Duration::from_secs(45);
const STARTED: &str = "started on it in the background. You are told when it is finished or when it needs them. Say in a few words that you are on it, and go on with the conversation";
const ASKED: &str = "asked them to approve it on their screen. Nothing has happened yet: it runs if they approve, and you are told how it went";
const RESULT_CHARS: usize = 700;
const FILE_SHOWN: usize = 900;
static UNASKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static PLANS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static NEWER: tokio::sync::Notify = tokio::sync::Notify::const_new();
const SCRIPTED: Duration = Duration::from_secs(20);
const PEOPLE_LISTED: usize = 80;
const CHATS: &str = "500";
const IMSG: [&str; 3] = ["/opt/homebrew/bin/imsg", "/usr/local/bin/imsg", "imsg"];
const NO_CHATS: &str = "imsg could not read any conversations. Milo, or the terminal app that runs it, needs Full Disk Access, under Privacy & Security in System Settings";
const NO_NAMES: &str = "imsg could read the conversations but not the names of the people in them. Milo, or the terminal app that runs it, needs access to Contacts, or Full Disk Access, under Privacy & Security in System Settings";
const NO_IMSG: &str = "nothing was sent: the imsg command is not installed on this Mac. They can install it with: brew install steipete/tap/imsg";
const MEMORY_LINES: usize = 60;
const MAX_TIMER: f64 = 24.0 * 3600.0;
const OLD: usize = 300;
const SHOWN: usize = 300;

pub fn unasked(on: bool) {
    UNASKED.store(on, Relaxed);
}

fn asking() -> bool {
    notch::on() && !UNASKED.load(Relaxed)
}

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
    Task,
    Message,
    See,
    Write,
    Media,
    CloseTab,
    ShowTab,
    File,
    Command(Vec<String>),
}

struct Tool {
    name: String,
    description: String,
    parameters: Value,
    kind: Kind,
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str], kind: Kind) -> Tool {
    Tool {
        name: name.into(),
        description: description.into(),
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
    minds: Arc<computer::Minds>,
}

impl Tools {
    pub fn load(events: UnboundedSender<Event>, minds: Arc<computer::Minds>, hands: bool) -> Tools {
        let mut list = vec![
            tool(
                "weather",
                "Get the current weather and the forecast for today and tomorrow in a city.",
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
                "Start a countdown timer or a timed reminder. You are told when it finishes.",
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
                "Open an app or a website in front of the person, for them to look at or use themselves.",
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
                "Save something to long-term memory: a lasting fact about the person, or anything they ask you to remember, note or write down.",
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
                json!({ "what": { "type": "string", "description": "A few words from the memory to remove." } }),
                &["what"],
                Kind::Forget,
            ),
        ];
        if hands {
            list.push(tool(
                "computer",
                "Start work on the person's computer, in their apps or in your own browser. It is carried out in the background, behind their windows, while you go on talking with them, and you are told how it went. Call it with a short plan whenever what they ask takes more than the other tools do, like sending an email, writing a note in an app, searching a site, changing a setting, or reading something off a page. The plan is all the work goes by, so it has to hold everything the work needs. If they have not said something the task needs, like what an email should say or who it is for, ask them instead of calling this.",
                json!({
                    "task": {
                        "type": "string",
                        "description": "A short name for what they asked, like Send Priya an email or Play some jazz.",
                    },
                    "steps": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "The plan, as one to six short steps in order, like Go to mail.google.com. Each step names the app, the site or the tab it is about, and the exact words to type, taken from what they said. Never make up words to type.",
                    },
                    "open": {
                        "type": "string",
                        "description": "The app or the web address the plan starts in, like Notes or mail.google.com, when it is not open yet. Leave it out when the plan is about something that is already open, or when you can't tell where to start.",
                    },
                    "show": {
                        "type": "boolean",
                        "description": "True only when they asked to see, watch or be shown it, like play a video or pull up a page. Then it is brought to the front of their screen when the work is done. Leave it out when they just want something done.",
                    },
                }),
                &["task", "steps"],
                Kind::Computer,
            ));
            list.push(tool(
                "task",
                "Manage work that you started with computer and that is still under way: pass on what the person said about it, stop it, or bring it to the front of their screen. With the action show and nothing under way, it brings up what was worked in last.",
                json!({
                    "action": {
                        "type": "string",
                        "enum": ["tell", "stop", "show", "take"],
                        "description": "tell passes on a change, a correction or an answer of theirs. stop ends the work. show brings it to the front for them to watch. take stops it and brings it to the front, for when they want to do the rest themselves.",
                    },
                    "which": { "type": "string", "description": "A word or two from the name of the work, when more than one is under way." },
                    "text": { "type": "string", "description": "For tell: what they said, in their words, like Make the subject Lunch on Friday, or I have signed in." },
                }),
                &["action"],
                Kind::Task,
            ));
            list.push(tool(
                "see",
                "Look at the window in front on the person's screen. It answers with a picture of the window and a list of what is in it. Use it when they ask what is on their screen, ask you to look at it, or say that something there is wrong.",
                json!({}),
                &[],
                Kind::See,
            ));
            list.push(tool(
                "write",
                "Type text where their cursor is, in the app in front of them. When they have text selected there, the new text takes its place. Use it when they ask you to write, rewrite, shorten, fix or fill in something right where they are, like tighten this paragraph or write a reply here. You write the whole text yourself, ready to use as it is.",
                json!({ "text": { "type": "string", "description": "The exact text to put there." } }),
                &["text"],
                Kind::Write,
            ));
            list.push(tool(
                "media",
                "Pause, stop, resume or skip whatever video or music is playing on the person's computer, in the browser or in an app.",
                json!({ "action": { "type": "string", "enum": ["pause", "play", "next", "previous"] } }),
                &["action"],
                Kind::Media,
            ));
            list.push(tool(
                "close_tab",
                "Close a tab that you opened in your own browser.",
                json!({
                    "which": { "type": "string", "description": "A word or two from the tab's title or address, like youtube. Leave it out when they say that tab, this tab or the tab, which means the one that was just worked in or that is showing." },
                }),
                &[],
                Kind::CloseTab,
            ));
            list.push(tool(
                "show_tab",
                "Bring a tab that you opened in your own browser to the front, for them to look at.",
                json!({ "which": { "type": "string", "description": "A word or two from the tab's title or address, like gmail." } }),
                &["which"],
                Kind::ShowTab,
            ));
            list.push(tool(
                "file",
                "Write a text file on the person's computer, like a note, a list or a short program, and open it in an app if they want. Use it whenever they ask for something to be written into a file: it writes the exact text, so nothing has to be typed in a window. You write the whole content yourself.",
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
                "Send a text message to someone through the Messages app, as an iMessage or an SMS. Use it whenever they want someone texted or messaged. It finds the person by name among the people they have conversations with. If they have not said who it is for or what it should say, ask them instead of calling this.",
                json!({
                    "to": { "type": "string", "description": "Who it is for, by the name they are saved under, like Priya Shah. When what you remember about them says what someone is saved as, use that name." },
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
        Tools { list, http, events, minds }
    }

    pub fn hands(&self) -> bool {
        self.list.iter().any(|t| matches!(t.kind, Kind::Computer))
    }

    pub fn schema(&self) -> Value {
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

    pub fn quiet(&self, name: &str) -> bool {
        self.list.iter().any(|t| t.name == name && matches!(t.kind, Kind::Remember))
    }

    fn gated(t: &Tool) -> bool {
        asking()
            && match t.kind {
                Kind::Open
                | Kind::Computer
                | Kind::File
                | Kind::Message
                | Kind::Write
                | Kind::CloseTab
                | Kind::Command(_) => true,
                _ => false,
            }
    }

    async fn attempt(&self, t: &Tool, call: &Call) -> Result<String> {
        match &t.kind {
            Kind::Weather => self.weather(&call.args).await,
            Kind::Timer => self.timer(&call.args),
            Kind::Open => open(&call.args).await,
            Kind::Remember => remember(&call.args),
            Kind::Forget => forget(&call.args),
            Kind::Computer => self.computer(&call.args).await,
            Kind::Task => {
                let field = |key: &str| text(&call.args, key).unwrap_or_default();
                computer::manage(&field("action"), &field("which"), &field("text")).await
            }
            Kind::See => computer::see().await,
            Kind::Write => {
                let words = call.args.get("text").and_then(Value::as_str).unwrap_or_default().to_string();
                if words.trim().is_empty() {
                    bail!("there is nothing to type; ask them what it should say");
                }
                let shown: String = words.chars().take(FILE_SHOWN).collect();
                computer::insert(&words, async |app| allow(&format!("Type this into {app}?"), &shown).await).await
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

    pub async fn run(self: &Arc<Self>, call: &Call) -> String {
        let Some(at) = self.list.iter().position(|t| t.name == call.name) else {
            return format!("there is no tool called {}", call.name);
        };
        let t = &self.list[at];
        if Self::gated(t) {
            self.clone().later(at, call.clone());
            return ASKED.to_string();
        }
        match self.attempt(t, call).await {
            Ok(text) => clip(&text),
            Err(e) => format!("{} failed: {}", call.name, clip(&format!("{e:#}"))),
        }
    }

    fn later(self: Arc<Self>, at: usize, call: Call) {
        tokio::spawn(async move {
            let t = &self.list[at];
            match self.attempt(t, &call).await {
                Ok(_) if matches!(t.kind, Kind::Computer) => {}
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
        let plan: Vec<String> = steps.iter().enumerate().map(|(i, s)| format!("{}. {s}", i + 1)).collect();
        let mine = PLANS.fetch_add(1, Relaxed) + 1;
        NEWER.notify_waiters();
        let newer = async {
            while PLANS.load(Relaxed) == mine {
                NEWER.notified().await;
            }
        };
        if asking() {
            match notch::approve_unless(&task, &plan.join("\n"), APPROVE_TIMEOUT, newer).await {
                Some(true) => {}
                _ => return Err(Skipped.into()),
            }
        }
        let plan = computer::Plan { task, steps, open: text(args, "open"), show };
        computer::start(self.minds.clone(), plan, self.events.clone());
        Ok(STARTED.to_string())
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
        if said.contains("authorization denied") {
            bail!("nothing was sent: {NO_CHATS}");
        }
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
    if !asking() {
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
        out.push(Tool { name: name.into(), description: description.into(), parameters, kind: Kind::Command(argv) });
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

pub fn shorten(history: &mut [Message]) {
    let read = history.iter().rposition(|m| m.role == Role::Tool && m.content.starts_with(computer::READ));
    for (at, old) in history.iter_mut().enumerate() {
        if old.role != Role::Tool || Some(at) == read {
            continue;
        }
        if old.content.chars().count() > OLD {
            old.content = old.content.chars().take(OLD).collect::<String>() + "…";
        }
    }
}

fn count(tokens: u64) -> String {
    match tokens {
        0..1000 => tokens.to_string(),
        _ => format!("{:.1}k", tokens as f64 / 1000.0),
    }
}

pub fn bill(before: llm::Spent) -> Option<String> {
    let now = llm::spent();
    if now.input + now.output == 0 {
        return None;
    }
    let tokens =
        format!("{} in ({} cached), {} out this session", count(now.input), count(now.cached), count(now.output));
    Some(match now.priced {
        true => format!("cost ${:.4} this turn, ${:.4} this session | {tokens}", now.cost - before.cost, now.cost),
        false => format!("tokens {tokens}"),
    })
}

pub fn brief(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default();
    match line.chars().count() > SHOWN {
        true => line.chars().take(SHOWN).collect::<String>() + "…",
        false => line.to_string(),
    }
}
