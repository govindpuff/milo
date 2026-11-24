use std::io::{BufRead, IsTerminal, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use llm::{Api, Host, Llm};
use serde_json::{Map, Value, json};

pub const LOCAL: &str = "local";
pub const DEFAULT: &str = "local/gemma-4-26b-qat-4bit";

struct Known {
    id: &'static str,
    url: &'static str,
    api: Api,
    env: &'static str,
    limit: Option<&'static str>,
}

const fn known(id: &'static str, url: &'static str, env: &'static str) -> Known {
    Known { id, url, api: Api::Chat, env, limit: None }
}

const KNOWN: &[Known] = &[
    known(LOCAL, "", ""),
    Known {
        id: "openai",
        url: "https://api.openai.com/v1",
        api: Api::Responses,
        env: "OPENAI_API_KEY",
        limit: Some("max_completion_tokens"),
    },
    known("anthropic", "https://api.anthropic.com/v1", "ANTHROPIC_API_KEY"),
    known("google", "https://generativelanguage.googleapis.com/v1beta/openai", "GEMINI_API_KEY"),
    known("openrouter", "https://openrouter.ai/api/v1", "OPENROUTER_API_KEY"),
    known("groq", "https://api.groq.com/openai/v1", "GROQ_API_KEY"),
    known("cerebras", "https://api.cerebras.ai/v1", "CEREBRAS_API_KEY"),
    known("xai", "https://api.x.ai/v1", "XAI_API_KEY"),
    known("mistral", "https://api.mistral.ai/v1", "MISTRAL_API_KEY"),
    known("deepseek", "https://api.deepseek.com/v1", "DEEPSEEK_API_KEY"),
    known("together", "https://api.together.xyz/v1", "TOGETHER_API_KEY"),
    known("fireworks", "https://api.fireworks.ai/inference/v1", "FIREWORKS_API_KEY"),
    known("ollama", "http://127.0.0.1:11434/v1", ""),
    known("lmstudio", "http://127.0.0.1:1234/v1", ""),
];

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
        let mut headers = Vec::new();
        for (name, value) in set["headers"].as_object().into_iter().flatten() {
            headers.push((name.clone(), filled(value.as_str().unwrap_or_default())?));
        }
        let mut options = set["options"].as_object().cloned().unwrap_or_default();
        let own = &set["models"][model];
        options.extend(own["options"].as_object().cloned().unwrap_or_default());
        let limit = set["limit"].as_str().or(known.and_then(|known| known.limit)).map(str::to_string);
        let rapid = id == LOCAL;
        let host = Host { key, api, rapid, limit, headers, options };
        Ok(Chosen {
            name: format!("{id}/{model}"),
            llm: Llm::new(&url, model, host),
            local: (rapid && set["url"].is_null()).then(|| model.to_string()),
            pictures: own["pictures"].as_bool().unwrap_or(true),
        })
    }
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

pub fn auth(id: &str) -> Result<()> {
    let config = Config::load()?;
    if !config.has(id) {
        bail!("there is no provider called {id}; the providers are: {}", config.providers().join(", "));
    }
    let key = typed(&format!("API key for {id} (leave empty to remove it): "))?;
    let dir = folder()?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("auth.json");
    let mut keys = config.keys;
    match key.is_empty() {
        true => keys.remove(id),
        false => keys.insert(id.to_string(), key.clone().into()),
    };
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)
        .with_context(|| format!("write {}", path.display()))?;
    writeln!(file, "{}", serde_json::to_string_pretty(&Value::Object(keys))?)?;
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
    let chosen = config.chosen(&format!("{id}/any"), port)?;
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    for model in rt.block_on(chosen.llm.models())? {
        println!("{id}/{model}");
    }
    Ok(())
}
