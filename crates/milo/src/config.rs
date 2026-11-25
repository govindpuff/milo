use std::io::{BufRead, IsTerminal, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use llm::{Api, Host, Llm, Price};
use serde_json::{Map, Value, json};

pub const LOCAL: &str = "local";
pub const DEFAULT: &str = "openai/gpt-6.2-sol";
pub const VOICE: &str = "A warm, friendly young American man with a natural, relaxed, conversational voice, like chatting with a close friend on the phone.";
const SOUNDS: &str = "listening_sounds";
const SETTINGS: &[&str] = &[
    "model",
    "voice_design",
    "voice_ref",
    "voice_text",
    "auto_approve",
    "push_to_talk",
    "headphones",
    SOUNDS,
    "input_device",
    "output_device",
];
const LIVE: &[&str] = &["auto_approve", "push_to_talk"];

struct Known {
    id: &'static str,
    url: &'static str,
    api: Api,
    env: &'static str,
    limit: Option<&'static str>,
    check: &'static str,
    refuses: &'static [&'static str],
    headers: &'static [(&'static str, &'static str)],
}

const fn known(id: &'static str, url: &'static str, env: &'static str) -> Known {
    Known { id, url, api: Api::Chat, env, limit: None, check: "models", refuses: REFUSED, headers: &[] }
}

const KNOWN: &[Known] = &[
    known(LOCAL, "", ""),
    Known {
        id: "openai",
        url: "https://api.openai.com/v1",
        api: Api::Responses,
        env: "OPENAI_API_KEY",
        limit: Some("max_completion_tokens"),
        check: "models",
        refuses: REFUSED,
        headers: &[],
    },
    Known {
        headers: &[("anthropic-version", "2023-06-01")],
        ..known("anthropic", "https://api.anthropic.com/v1", "ANTHROPIC_API_KEY")
    },
    Known {
        refuses: &["answered 400", "answered 401", "answered 403"],
        ..known("google", "https://generativelanguage.googleapis.com/v1beta/openai", "GEMINI_API_KEY")
    },
    Known { check: "key", ..known("openrouter", "https://openrouter.ai/api/v1", "OPENROUTER_API_KEY") },
];

struct Rate {
    model: &'static str,
    input: f64,
    cached: f64,
    output: f64,
    fast: f64,
}

const fn rate(model: &'static str, input: f64, cached: f64, output: f64, fast: f64) -> Rate {
    Rate { model, input, cached, output, fast }
}

const RATES: &[Rate] = &[
    rate("gpt-6.2-astra", 8.0, 0.4, 40.0, 2.0),
    rate("gpt-6.2-sol", 2.0, 0.1, 10.0, 2.0),
    rate("gpt-6-astra", 10.0, 1.0, 50.0, 2.0),
    rate("gpt-6.1-sol", 2.0, 0.1, 10.0, 2.0),
    rate("gpt-6.1-terra", 1.0, 0.05, 5.0, 2.0),
    rate("gpt-6-luna", 0.1, 0.01, 0.5, 2.0),
    rate("gpt-6-sol", 2.0, 0.2, 10.0, 2.0),
    rate("gpt-5.6-sol", 4.0, 0.4, 20.0, 2.0),
    rate("gpt-5.6-terra", 2.0, 0.2, 12.0, 2.0),
    rate("gpt-5.6-luna", 0.2, 0.02, 1.2, 2.0),
    rate("gpt-5.5", 5.0, 0.5, 30.0, 2.5),
    rate("gpt-5.5-pro", 30.0, 30.0, 180.0, 1.0),
    rate("gpt-5.4", 2.5, 0.25, 15.0, 2.0),
    rate("gpt-5.4-pro", 30.0, 30.0, 180.0, 1.0),
    rate("gpt-5.4-mini", 0.75, 0.075, 4.5, 2.0),
    rate("gpt-5.4-nano", 0.2, 0.02, 1.25, 1.0),
    rate("gpt-5.2", 1.75, 0.175, 14.0, 2.0),
    rate("gpt-5.2-pro", 21.0, 21.0, 168.0, 1.0),
    rate("gpt-5.1", 1.25, 0.125, 10.0, 2.0),
    rate("gpt-5", 1.25, 0.125, 10.0, 2.0),
    rate("gpt-5-mini", 0.25, 0.025, 2.0, 1.8),
    rate("gpt-5-nano", 0.05, 0.005, 0.4, 1.0),
    rate("gpt-5-pro", 15.0, 15.0, 120.0, 1.0),
    rate("gpt-4.1", 2.0, 0.5, 8.0, 1.75),
    rate("gpt-4.1-mini", 0.4, 0.1, 1.6, 1.75),
    rate("gpt-4o", 2.5, 1.25, 10.0, 1.7),
    rate("gpt-4o-mini", 0.15, 0.075, 0.6, 5.0 / 3.0),
    rate("o3-pro", 20.0, 20.0, 80.0, 1.0),
    rate("o3", 2.0, 0.5, 8.0, 1.75),
    rate("chat-latest", 5.0, 0.5, 30.0, 1.0),
    rate("gpt-5.3-codex", 1.75, 0.175, 14.0, 2.0),
    rate("gpt-6.2-astra-healthcare", 8.0, 0.4, 40.0, 2.0),
    rate("gpt-6.2-sol-healthcare", 2.0, 0.1, 10.0, 2.0),
    rate("gpt-6-astra-healthcare", 10.0, 1.0, 50.0, 2.0),
    rate("gpt-6.1-sol-healthcare", 2.0, 0.1, 10.0, 2.0),
];

const REFUSED: &[&str] = &["answered 401", "answered 403"];
const ULTRAFAST: f64 = 6.0;
const PROMOTED: f64 = 3.0;
const PROMOTION_ENDS: (i16, i8, i8) = (2027, 3, 1);
const REGIONAL: f64 = 1.1;
const HALF: f64 = 0.5;

fn listed(url: &str, model: &str, tier: Option<&str>) -> Option<Price> {
    let host = url.split_once("://").map_or(url, |(_, rest)| rest).split('/').next().unwrap_or_default();
    if !host.ends_with("openai.com") {
        return None;
    }
    let rate = RATES.iter().find(|rate| rate.model == model)?;
    let today = jiff::Zoned::now().date();
    let promoted = today < jiff::civil::date(PROMOTION_ENDS.0, PROMOTION_ENDS.1, PROMOTION_ENDS.2);
    let tier = match tier {
        Some("priority" | "fast") => rate.fast,
        Some("flex") => HALF,
        Some("ultrafast") if model.starts_with("gpt-6.2") && promoted => PROMOTED,
        Some("ultrafast") => ULTRAFAST,
        _ => 1.0,
    };
    let recent = model.starts_with("gpt-6") || model.starts_with("gpt-5.6");
    let region = if host != "api.openai.com" && recent { REGIONAL } else { 1.0 };
    let by = tier * region;
    Some(Price { input: rate.input * by, cached: rate.cached * by, output: rate.output * by })
}

pub struct Chosen {
    pub name: String,
    pub llm: Llm,
    pub local: Option<String>,
    pub pictures: bool,
}

pub struct Config {
    raw: Value,
    keys: Map<String, Value>,
}

fn folder() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| anyhow!("no home folder"))?;
    Ok(home.join(".config/milo"))
}

fn object(path: &PathBuf) -> Result<Value> {
    let Ok(raw) = std::fs::read_to_string(path) else { return Ok(json!({})) };
    let parsed: Value = serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
    if !parsed.is_object() {
        bail!("{} must hold a JSON object", path.display());
    }
    Ok(parsed)
}

fn filled(text: &str) -> Result<String> {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("{env:") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 5..];
        let end = after.find('}').ok_or_else(|| anyhow!("{text:?} has an unclosed {{env:"))?;
        let name = &after[..end];
        out.push_str(std::env::var(name).with_context(|| format!("the variable {name} is not set"))?.trim());
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

impl Config {
    pub fn load() -> Result<Config> {
        let dir = folder()?;
        let raw = object(&dir.join("config.json"))?;
        let keys = object(&dir.join("auth.json"))?.as_object().cloned().unwrap_or_default();
        Ok(Config { raw, keys })
    }

    pub fn named(&self, key: &str) -> Option<String> {
        self.raw[key].as_str().map(str::trim).filter(|name| !name.is_empty()).map(str::to_string)
    }

    pub fn flag(&self, key: &str) -> bool {
        self.raw[key].as_bool().unwrap_or(key == SOUNDS)
    }

    pub fn settings(&self) -> Value {
        let mut all = Map::new();
        for key in SETTINGS {
            let value = match *key {
                "model" => self.named(key).unwrap_or_else(|| DEFAULT.to_string()).into(),
                "auto_approve" | "push_to_talk" | "headphones" | SOUNDS => self.flag(key).into(),
                _ => self.named(key).unwrap_or_default().into(),
            };
            all.insert(key.to_string(), value);
        }
        Value::Object(all)
    }

    pub fn hosted(&self) -> Vec<Value> {
        let hosted = KNOWN.iter().filter(|known| !known.env.is_empty());
        hosted.map(|known| json!({ "id": known.id, "keyed": self.keyed(known.id) })).collect()
    }

    pub fn wants(&self, name: &str) -> Option<String> {
        let (id, _) = self.split(name.trim());
        let known = KNOWN.iter().find(|known| known.id == id)?;
        (!known.env.is_empty() && !self.keyed(id)).then(|| id.to_string())
    }

    fn provider(&self, id: &str) -> &Value {
        &self.raw["providers"][id]
    }

    fn has(&self, id: &str) -> bool {
        KNOWN.iter().any(|known| known.id == id) || self.provider(id).is_object()
    }

    pub fn providers(&self) -> Vec<String> {
        let mut all: Vec<String> = KNOWN.iter().map(|known| known.id.to_string()).collect();
        for id in self.raw["providers"].as_object().into_iter().flat_map(Map::keys) {
            if !all.contains(id) {
                all.push(id.clone());
            }
        }
        all
    }

    fn split<'a>(&self, name: &'a str) -> (&'a str, &'a str) {
        match name.split_once('/') {
            Some((id, model)) if self.has(id) && !model.is_empty() => (id, model),
            _ => (LOCAL, name),
        }
    }

    fn key(&self, id: &str) -> Result<Option<String>> {
        let env = KNOWN.iter().find(|known| known.id == id).map_or("", |known| known.env);
        let key = match self.provider(id)["key"].as_str() {
            Some(key) => Some(filled(key)?),
            None => self.keys.get(id).and_then(Value::as_str).map(str::to_string).or_else(|| std::env::var(env).ok()),
        };
        Ok(key.map(|key| key.trim().to_string()).filter(|key| !key.is_empty()))
    }

    pub fn ask(&mut self, name: &str, port: u16) -> Result<()> {
        let Some(id) = self.wants(name) else { return Ok(()) };
        if !std::io::stdin().is_terminal() {
            return Ok(());
        }
        loop {
            let key = typed(&format!("Paste your API key for {id}: "))?;
            if key.is_empty() {
                bail!("there is no API key for {id}: run `milo auth {id}`");
            }
            if enter(&id, &key, port)? {
                self.keys = Config::load()?.keys;
                eprintln!("saved the key for {id}");
                return Ok(());
            }
            eprintln!("{id} refused that key. Try again, or press Return to quit.");
        }
    }

    pub fn keyed(&self, id: &str) -> bool {
        self.key(id).is_ok_and(|key| key.is_some())
    }

    pub fn address(&self, id: &str, port: u16) -> String {
        let known = KNOWN.iter().find(|known| known.id == id);
        match (self.provider(id)["url"].as_str(), known) {
            (Some(url), _) => url.to_string(),
            (None, Some(known)) if known.id == LOCAL => format!("http://127.0.0.1:{port}/v1"),
            (None, Some(known)) => known.url.to_string(),
            (None, None) => String::new(),
        }
    }

    pub fn chosen(&self, name: &str, port: u16) -> Result<Chosen> {
        let (id, model) = self.split(name.trim());
        let known = KNOWN.iter().find(|known| known.id == id);
        let set = self.provider(id);
        let url = self.address(id, port);
        if url.is_empty() {
            bail!("the provider {id} in config.json needs a url");
        }
        let api = match set["api"].as_str() {
            Some("responses") => Api::Responses,
            Some("chat") => Api::Chat,
            Some(other) => bail!("the api of the provider {id} is {other:?}; it must be chat or responses"),
            None => known.map_or(Api::Chat, |known| known.api),
        };
        let key = self.key(id)?;
        let env = known.map_or("", |known| known.env);
        if key.is_none() && !env.is_empty() {
            bail!("there is no API key for {id}: run `milo auth {id}`, or set {env}");
        }
        let mut headers: Vec<(String, String)> = known
            .into_iter()
            .flat_map(|known| known.headers)
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        for (name, value) in set["headers"].as_object().into_iter().flatten() {
            headers.push((name.clone(), filled(value.as_str().unwrap_or_default())?));
        }
        let mut options = set["options"].as_object().cloned().unwrap_or_default();
        let own = &set["models"][model];
        options.extend(own["options"].as_object().cloned().unwrap_or_default());
        let limit = set["limit"].as_str().or(known.and_then(|known| known.limit)).map(str::to_string);
        let rapid = id == LOCAL;
        let number = |key: &str| own["price"][key].as_f64();
        let price = match (number("input"), number("output")) {
            (Some(input), Some(output)) => Some(Price { input, cached: number("cached").unwrap_or(input), output }),
            _ => listed(&url, model, options.get("service_tier").and_then(Value::as_str)),
        };
        let host = Host { key, api, rapid, limit, price, headers, options };
        Ok(Chosen {
            name: format!("{id}/{model}"),
            llm: Llm::new(&url, model, host),
            local: (rapid && set["url"].is_null()).then(|| model.to_string()),
            pictures: own["pictures"].as_bool().unwrap_or(true),
        })
    }
}

pub fn enter(id: &str, key: &str, port: u16) -> Result<bool> {
    let mut config = Config::load()?;
    let known = KNOWN.iter().find(|known| known.id == id).ok_or_else(|| anyhow!("there is no provider called {id}"))?;
    config.keys.insert(id.to_string(), key.trim().into());
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    match rt.block_on(config.chosen(&format!("{id}/any"), port)?.llm.check(known.check)) {
        Err(e) if known.refuses.iter().any(|code| format!("{e:#}").contains(code)) => Ok(false),
        Err(e) => Err(e.context(format!("could not check the key for {id}"))),
        Ok(()) => save(&config.keys).map(|_| true),
    }
}

pub fn update(new: &Map<String, Value>) -> Result<bool> {
    let dir = folder()?;
    let path = dir.join("config.json");
    let mut raw = object(&path)?;
    let was = raw.clone();
    for key in SETTINGS {
        let plain = match new.get(*key) {
            Some(Value::String(text)) => text.trim().is_empty(),
            Some(Value::Bool(on)) => *on == (*key == SOUNDS),
            _ => continue,
        };
        match plain {
            true => raw.as_object_mut().and_then(|raw| raw.remove(*key)),
            false => raw.as_object_mut().and_then(|raw| raw.insert(key.to_string(), new[*key].clone())),
        };
    }
    std::fs::create_dir_all(&dir)?;
    std::fs::write(&path, format!("{}\n", serde_json::to_string_pretty(&raw)?))
        .with_context(|| format!("write {}", path.display()))?;
    Ok(SETTINGS.iter().any(|key| !LIVE.contains(key) && was[*key] != raw[*key]))
}

pub fn offered(id: &str, port: u16) -> Result<Vec<String>> {
    let chosen = Config::load()?.chosen(&format!("{id}/any"), port)?;
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    rt.block_on(chosen.llm.models())
}

fn typed(ask: &str) -> Result<String> {
    let stdin = std::io::stdin();
    let hidden = stdin.is_terminal();
    let mut was: libc::termios = unsafe { std::mem::zeroed() };
    if hidden {
        eprint!("{ask}");
        std::io::stderr().flush()?;
        unsafe {
            libc::tcgetattr(libc::STDIN_FILENO, &mut was);
            let mut quiet = was;
            quiet.c_lflag &= !libc::ECHO;
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &quiet);
        }
    }
    let mut line = String::new();
    let read = stdin.lock().read_line(&mut line);
    if hidden {
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &was) };
        eprintln!();
    }
    read?;
    Ok(line.trim().to_string())
}

fn save(keys: &Map<String, Value>) -> Result<PathBuf> {
    let dir = folder()?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("auth.json");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)
        .with_context(|| format!("write {}", path.display()))?;
    writeln!(file, "{}", serde_json::to_string_pretty(&Value::Object(keys.clone()))?)?;
    Ok(path)
}

pub fn auth(id: &str) -> Result<()> {
    let config = Config::load()?;
    if !config.has(id) {
        bail!("there is no provider called {id}; the providers are: {}", config.providers().join(", "));
    }
    let key = typed(&format!("API key for {id} (leave empty to remove it): "))?;
    let mut keys = config.keys;
    match key.is_empty() {
        true => keys.remove(id),
        false => keys.insert(id.to_string(), key.clone().into()),
    };
    let path = save(&keys)?;
    match key.is_empty() {
        true => println!("removed the key for {id} from {}", path.display()),
        false => println!("saved the key for {id} in {}", path.display()),
    }
    Ok(())
}

pub fn models(id: Option<&str>, port: u16) -> Result<()> {
    let config = Config::load()?;
    let Some(id) = id else {
        for id in config.providers() {
            let known = KNOWN.iter().find(|known| known.id == id);
            let needs = known.is_some_and(|known| !known.env.is_empty()) || !config.provider(&id)["key"].is_null();
            let key = match (needs, config.keyed(&id)) {
                (false, _) => "no key needed",
                (true, true) => "key set",
                (true, false) => "no key",
            };
            println!("{id:<12} {:<58} {key}", config.address(&id, port));
        }
        return Ok(());
    };
    if !config.has(id) {
        bail!("there is no provider called {id}; the providers are: {}", config.providers().join(", "));
    }
    for model in offered(id, port)? {
        println!("{id}/{model}");
    }
    Ok(())
}
