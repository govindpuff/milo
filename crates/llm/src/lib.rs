pub mod chunker;
pub mod server;

use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use bytes::Bytes;
use futures_core::Stream;
use serde::Serialize;
use serde_json::{Map, Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub role: Role,
    pub content: String,
    pub image: Option<String>,
    tool_calls: Option<Value>,
    tool_call_id: Option<String>,
}

impl Serialize for Message {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("role", &self.role)?;
        match &self.image {
            Some(image) => map.serialize_entry(
                "content",
                &json!([
                    { "type": "text", "text": self.content },
                    { "type": "image_url", "image_url": { "url": format!("data:image/jpeg;base64,{image}") } },
                ]),
            )?,
            None => map.serialize_entry("content", &self.content)?,
        }
        if let Some(calls) = &self.tool_calls {
            map.serialize_entry("tool_calls", calls)?;
        }
        if let Some(id) = &self.tool_call_id {
            map.serialize_entry("tool_call_id", id)?;
        }
        map.end()
    }
}

impl Message {
    fn new(role: Role, content: impl Into<String>) -> Self {
        Self { role, content: content.into(), image: None, tool_calls: None, tool_call_id: None }
    }

    pub fn called(content: impl Into<String>, calls: &[Call]) -> Self {
        let list: Vec<Value> = calls
            .iter()
            .map(|c| {
                json!({
                    "id": c.id,
                    "type": "function",
                    "function": { "name": c.name, "arguments": Value::Object(c.args.clone()).to_string() },
                })
            })
            .collect();
        Self { tool_calls: Some(list.into()), ..Self::new(Role::Assistant, content) }
    }

    pub fn answers(&self) -> Option<&str> {
        self.tool_call_id.as_deref()
    }

    fn items(&self, out: &mut Vec<Value>) {
        if let Some(id) = &self.tool_call_id {
            out.push(json!({ "type": "function_call_output", "call_id": id, "output": self.content }));
            return;
        }
        if let Some(calls) = &self.tool_calls {
            if !self.content.trim().is_empty() {
                out.push(json!({ "role": self.role, "content": self.content }));
            }
            for call in calls.as_array().into_iter().flatten() {
                out.push(json!({
                    "type": "function_call",
                    "call_id": call["id"],
                    "name": call["function"]["name"],
                    "arguments": call["function"]["arguments"],
                }));
            }
            return;
        }
        let content = match &self.image {
            Some(image) => json!([
                { "type": "input_text", "text": self.content },
                { "type": "input_image", "image_url": format!("data:image/jpeg;base64,{image}") },
            ]),
            None => self.content.clone().into(),
        };
        out.push(json!({ "role": self.role, "content": content }));
    }

    pub fn tool(id: &str, content: impl Into<String>) -> Self {
        Self { tool_call_id: Some(id.to_string()), ..Self::new(Role::Tool, content) }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self::new(Role::System, content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::new(Role::User, content)
    }

    pub fn seen(content: impl Into<String>, image: Option<String>) -> Self {
        Self { image, ..Self::new(Role::User, content) }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(Role::Assistant, content)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Api {
    #[default]
    Chat,
    Responses,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Price {
    pub input: f64,
    pub cached: f64,
    pub output: f64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Spent {
    pub input: u64,
    pub cached: u64,
    pub output: u64,
    pub cost: f64,
    pub priced: bool,
}

static METER: Mutex<Spent> = Mutex::new(Spent { input: 0, cached: 0, output: 0, cost: 0.0, priced: false });

pub fn spent() -> Spent {
    *METER.lock().unwrap_or_else(|e| e.into_inner())
}

fn meter(used: (u64, u64, u64), price: Option<Price>) {
    let (input, cached, output) = used;
    let cached = cached.min(input);
    let mut total = METER.lock().unwrap_or_else(|e| e.into_inner());
    total.input += input;
    total.cached += cached;
    total.output += output;
    if let Some(price) = price {
        total.priced = true;
        total.cost +=
            ((input - cached) as f64 * price.input + cached as f64 * price.cached + output as f64 * price.output) / 1e6;
    }
}

#[derive(Clone, Default)]
pub struct Host {
    pub key: Option<String>,
    pub api: Api,
    pub rapid: bool,
    pub limit: Option<String>,
    pub price: Option<Price>,
    pub headers: Vec<(String, String)>,
    pub options: Map<String, Value>,
}

#[derive(Clone)]
pub struct Llm {
    inner: Arc<Inner>,
}

struct Inner {
    model: String,
    http: reqwest::Client,
    base: String,
    host: Host,
}

#[derive(Clone, Debug)]
pub struct Reply {
    pub calls: Vec<Call>,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct Call {
    pub id: String,
    pub name: String,
    pub args: Map<String, Value>,
}

#[derive(Clone, Debug)]
pub enum Piece {
    Text(String),
    Calls(Vec<Call>),
}

#[derive(Serialize)]
struct PrefillRequest<'a> {
    kind: &'static str,
    messages: &'a [Message],
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a Value>,
    enable_thinking: bool,
}

impl Llm {
    pub fn new(base_url: &str, model: &str, host: Host) -> Llm {
        let http = reqwest::Client::builder()
            .no_proxy()
            .tcp_nodelay(true)
            .pool_idle_timeout(Duration::from_secs(25))
            .connect_timeout(if host.rapid { NEAR } else { FAR })
            .build()
            .expect("reqwest client");
        let base = base_url.trim_end_matches('/').to_string();
        Llm { inner: Arc::new(Inner { model: model.to_string(), http, base, host }) }
    }

    pub fn model(&self) -> &str {
        &self.inner.model
    }

    fn rapid(&self) -> bool {
        self.inner.host.rapid
    }

    fn send(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let req = match &self.inner.host.key {
            Some(key) => req.bearer_auth(key),
            None => req,
        };
        self.inner.host.headers.iter().fold(req, |req, (name, value)| req.header(name, value))
    }

    fn post(&self, path: &str) -> reqwest::RequestBuilder {
        self.send(self.inner.http.post(format!("{}/{path}", self.inner.base)))
    }

    fn merged(&self, mut body: Value) -> Value {
        if let Some(fields) = body.as_object_mut() {
            for (key, value) in &self.inner.host.options {
                match value {
                    Value::Null => fields.remove(key),
                    value => fields.insert(key.clone(), value.clone()),
                };
            }
        }
        body
    }

    fn body(&self, messages: &[Message], tools: Option<&Value>, stream: bool, tokens: u32, heat: f32) -> Value {
        if self.inner.host.api == Api::Responses {
            let mut input = Vec::new();
            for message in messages {
                message.items(&mut input);
            }
            let mut body = json!({
                "model": self.inner.model,
                "input": input,
                "stream": stream,
                "store": false,
                "max_output_tokens": tokens + ROOM,
            });
            if let Some(tools) = tools {
                body["tools"] = flat(tools);
                body["tool_choice"] = "auto".into();
            }
            return self.merged(body);
        }
        let mut body = json!({
            "model": self.inner.model,
            "messages": messages,
            "stream": stream,
            "temperature": heat,
        });
        match (self.rapid(), &self.inner.host.limit) {
            (true, _) => {
                body["max_tokens"] = tokens.into();
                body["chat_template_kwargs"] = json!({ "enable_thinking": false });
            }
            (false, Some(limit)) => body[limit.as_str()] = (tokens + ROOM).into(),
            (false, None) => body["max_tokens"] = (tokens + ROOM).into(),
        }
        if stream && !self.rapid() {
            body["stream_options"] = json!({ "include_usage": true });
        }
        if let Some(tools) = tools {
            body["tools"] = tools.clone();
            body["tool_choice"] = "auto".into();
        }
        self.merged(body)
    }

    fn path(&self) -> &'static str {
        match self.inner.host.api {
            Api::Responses => "responses",
            Api::Chat => "chat/completions",
        }
    }

    pub async fn stream(
        &self,
        messages: &[Message],
        tools: Option<&Value>,
    ) -> Result<impl Stream<Item = Result<Piece>> + Send + Unpin + use<>> {
        self.open(messages, tools, REPLY_TOKENS, TEMPERATURE, false).await
    }

    async fn open(
        &self,
        messages: &[Message],
        tools: Option<&Value>,
        tokens: u32,
        heat: f32,
        quiet: bool,
    ) -> Result<ChatStream> {
        let mut send = self.post(self.path()).json(&self.body(messages, tools, true, tokens, heat));
        if quiet && self.rapid() {
            send = send.header("X-Rapid-Priority", "background");
        }
        let resp = send.send().await.with_context(|| format!("request to {}", self.inner.base))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!("{} answered {status}: {body}", self.inner.model);
        }
        Ok(ChatStream {
            body: Box::pin(resp.bytes_stream()),
            buf: Vec::new(),
            out: VecDeque::new(),
            calls: Vec::new(),
            ended: false,
            done: false,
            id: None,
            http: self.inner.http.clone(),
            base: self.rapid().then(|| self.inner.base.clone()),
            events: self.inner.host.api == Api::Responses,
            used: None,
            price: self.inner.host.price,
        })
    }

    async fn collect(&self, messages: &[Message], tools: Option<&Value>, tokens: u32) -> Result<Reply> {
        let gather = async {
            let mut stream = self.open(messages, tools, tokens, 0.0, true).await?;
            let mut reply = Reply { calls: Vec::new(), text: String::new() };
            while let Some(piece) = std::future::poll_fn(|cx| Pin::new(&mut stream).poll_next(cx)).await {
                match piece? {
                    Piece::Text(text) => reply.text.push_str(&text),
                    Piece::Calls(calls) => reply.calls.extend(calls),
                }
            }
            Ok::<Reply, anyhow::Error>(reply)
        };
        let mut reply = tokio::time::timeout(ACT_TIMEOUT, gather).await.map_err(|_| anyhow!("it took too long"))??;
        reply.text = reply.text.trim().to_string();
        if reply.calls.is_empty() {
            reply.calls = inline_calls(&reply.text);
        }
        Ok(reply)
    }

    pub async fn see(&self, question: &str, image: &str) -> Result<String> {
        let messages = [Message::seen(question, Some(image.to_string()))];
        Ok(self.collect(&messages, None, SEE_TOKENS).await?.text)
    }

    pub async fn write(&self, messages: &[Message]) -> Result<String> {
        Ok(self.collect(messages, None, WRITE_TOKENS).await?.text)
    }

    pub async fn act(&self, messages: &[Message], tools: &Value) -> Result<Reply> {
        self.collect(messages, Some(tools), REPLY_TOKENS).await
    }

    pub async fn models(&self) -> Result<Vec<String>> {
        let req = self.send(self.inner.http.get(format!("{}/models", self.inner.base))).timeout(LIST_TIMEOUT);
        let resp = req.send().await.with_context(|| format!("request to {}", self.inner.base))?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("{} answered {status}: {body}", self.inner.base);
        }
        let body: Value = serde_json::from_str(&body).context("list of models")?;
        let list = body["data"].as_array().or_else(|| body["models"].as_array());
        let mut names: Vec<String> = list
            .into_iter()
            .flatten()
            .filter_map(|model| model["id"].as_str().or_else(|| model["name"].as_str()))
            .map(str::to_string)
            .collect();
        names.sort();
        Ok(names)
    }

    pub async fn warm(&self, messages: &[Message], tools: Option<&Value>) -> Result<()> {
        if !self.rapid() {
            return Ok(());
        }
        if let [m] = messages
            && m.role == Role::System
        {
            match self.prefill_prefix(messages, tools).await {
                Ok(()) => return Ok(()),
                Err(e) => log::debug!("prefix prefill unavailable, warming via chat: {e:#}"),
            }
        }
        let owned;
        let messages = match messages.iter().rposition(|m| m.role == Role::User) {
            Some(last) => &messages[..=last],
            None => {
                owned = [messages, &[Message::user("Hi")]].concat();
                &owned
            }
        };
        let req = self.body(messages, tools, false, 1, TEMPERATURE);
        let resp = self.post("chat/completions").json(&req).send().await.context("warm request")?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("warm {status}: {body}");
        }
        Ok(())
    }

    async fn prefill_prefix(&self, messages: &[Message], tools: Option<&Value>) -> Result<()> {
        let req = PrefillRequest { kind: "prefix", messages, tools, enable_thinking: false };
        let resp = self.inner.http.post(format!("{}/cache/prefill", self.inner.base)).json(&req).send().await?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("prefill {status}: {body}");
        }
        log::debug!("prefill: {body}");
        Ok(())
    }
}

const REPLY_TOKENS: u32 = 1200;
const TEMPERATURE: f32 = 0.7;
const SEE_TOKENS: u32 = 140;
const WRITE_TOKENS: u32 = 500;
const ACT_TIMEOUT: Duration = Duration::from_secs(60);
const LIST_TIMEOUT: Duration = Duration::from_secs(15);
const NEAR: Duration = Duration::from_secs(2);
const FAR: Duration = Duration::from_secs(10);
const ROOM: u32 = 4000;

fn flat(tools: &Value) -> Value {
    tools
        .as_array()
        .into_iter()
        .flatten()
        .map(|tool| {
            let f = &tool["function"];
            json!({
                "type": "function",
                "name": f["name"],
                "description": f["description"],
                "parameters": f["parameters"],
                "strict": false,
            })
        })
        .collect()
}

fn parse_call(name: &Value, args: &Value) -> Option<Call> {
    let name = name.as_str()?.trim();
    if name.is_empty() {
        return None;
    }
    let args = match args {
        Value::Object(m) => m.clone(),
        Value::String(s) => serde_json::from_str(s).unwrap_or_default(),
        _ => Default::default(),
    };
    Some(Call { id: String::new(), name: name.to_string(), args })
}

pub fn inline_calls(text: &str) -> Vec<Call> {
    let mut calls = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("<tool_call>") {
        let after = &rest[at + "<tool_call>".len()..];
        let end = after.find("</tool_call>").unwrap_or(after.len());
        if let Ok(v) = serde_json::from_str::<Value>(after[..end].trim())
            && let Some(mut c) = parse_call(&v["name"], &v["arguments"])
        {
            c.id = format!("call_{}", calls.len());
            calls.push(c);
        }
        rest = &after[end..];
    }
    calls
}

struct Partial {
    id: String,
    name: String,
    args: String,
}

struct ChatStream {
    body: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>,
    buf: Vec<u8>,
    out: VecDeque<Result<Piece>>,
    calls: Vec<Partial>,
    ended: bool,
    done: bool,
    id: Option<String>,
    http: reqwest::Client,
    base: Option<String>,
    events: bool,
    used: Option<(u64, u64, u64)>,
    price: Option<Price>,
}

impl ChatStream {
    fn feed(&mut self, bytes: &[u8]) {
        let mut buf = std::mem::take(&mut self.buf);
        buf.extend_from_slice(bytes);
        let mut start = 0;
        while let Some(pos) = buf[start..].iter().position(|&b| b == b'\n') {
            self.line(&buf[start..start + pos]);
            start += pos + 1;
        }
        buf.drain(..start);
        self.buf = buf;
    }

    fn fail(&mut self, what: &Value) {
        self.done = true;
        self.ended = true;
        self.calls.clear();
        self.out.push_back(Err(anyhow!("server error: {what}")));
    }

    fn close(&mut self) {
        self.done = true;
        let mut calls = Vec::new();
        for (i, held) in self.calls.drain(..).enumerate() {
            if let Some(mut call) = parse_call(&Value::String(held.name), &Value::String(held.args)) {
                call.id = if held.id.is_empty() { format!("call_{i}") } else { held.id };
                calls.push(call);
            }
        }
        if !calls.is_empty() {
            self.out.push_back(Ok(Piece::Calls(calls)));
        }
    }

    fn event(&mut self, event: &Value) {
        match event["type"].as_str() {
            Some("response.output_text.delta") => {
                if let Some(text) = event["delta"].as_str().filter(|text| !text.is_empty()) {
                    self.out.push_back(Ok(Piece::Text(text.to_string())));
                }
            }
            Some("response.output_item.done") if event["item"]["type"] == "function_call" => {
                let item = &event["item"];
                let field = |key: &str| item[key].as_str().unwrap_or_default().to_string();
                self.calls.push(Partial { id: field("call_id"), name: field("name"), args: field("arguments") });
            }
            Some("response.completed" | "response.incomplete") => {
                let usage = &event["response"]["usage"];
                if usage.is_object() {
                    let count = |value: &Value| value.as_u64().unwrap_or(0);
                    self.used = Some((
                        count(&usage["input_tokens"]),
                        count(&usage["input_tokens_details"]["cached_tokens"]),
                        count(&usage["output_tokens"]),
                    ));
                }
                self.close();
            }
            Some("error" | "response.failed") => self.fail(event),
            _ => {}
        }
    }

    fn delta(&mut self, call: &Value) {
        let text = |value: &Value| value.as_str().unwrap_or_default().to_string();
        let id = text(&call["id"]);
        let at = match call["index"].as_u64() {
            Some(index) => index as usize,
            None if !id.is_empty() && self.calls.last().is_none_or(|last| last.id != id) => self.calls.len(),
            None => self.calls.len().saturating_sub(1),
        };
        while self.calls.len() <= at {
            self.calls.push(Partial { id: String::new(), name: String::new(), args: String::new() });
        }
        let held = &mut self.calls[at];
        if !id.is_empty() {
            held.id = id;
        }
        let f = &call["function"];
        held.name.push_str(f["name"].as_str().unwrap_or_default());
        match &f["arguments"] {
            Value::String(part) => held.args.push_str(part),
            Value::Object(whole) => held.args = Value::Object(whole.clone()).to_string(),
            _ => {}
        }
    }

    fn line(&mut self, line: &[u8]) {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let Some(data) = line.strip_prefix(b"data:") else {
            return;
        };
        let data = data.strip_prefix(b" ").unwrap_or(data);
        if data == b"[DONE]" {
            self.close();
            return;
        }
        let Ok(chunk) = serde_json::from_slice::<Value>(data) else {
            log::warn!("unparseable SSE data: {}", String::from_utf8_lossy(data));
            return;
        };
        if self.events {
            self.event(&chunk);
            return;
        }
        if !chunk["error"].is_null() {
            self.fail(&chunk["error"]);
            return;
        }
        if self.id.is_none() {
            self.id = chunk["id"].as_str().map(str::to_string);
        }
        let usage = &chunk["usage"];
        if usage.is_object() {
            let count = |value: &Value| value.as_u64().unwrap_or(0);
            self.used = Some((
                count(&usage["prompt_tokens"]),
                count(&usage["prompt_tokens_details"]["cached_tokens"]),
                count(&usage["completion_tokens"]),
            ));
        }
        for choice in chunk["choices"].as_array().into_iter().flatten() {
            let delta = &choice["delta"];
            if let Some(text) = delta["content"].as_str().filter(|text| !text.is_empty()) {
                self.out.push_back(Ok(Piece::Text(text.to_string())));
            }
            for call in delta["tool_calls"].as_array().into_iter().flatten() {
                self.delta(call);
            }
            if !choice["finish_reason"].is_null() {
                self.close();
            }
        }
    }
}

impl Stream for ChatStream {
    type Item = Result<Piece>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = &mut *self;
        loop {
            if let Some(item) = this.out.pop_front() {
                return Poll::Ready(Some(item));
            }
            if this.ended {
                return Poll::Ready(None);
            }
            match this.body.as_mut().poll_next(cx) {
                Poll::Ready(Some(Ok(bytes))) => this.feed(&bytes),
                Poll::Ready(Some(Err(e))) => {
                    this.ended = true;
                    return Poll::Ready(Some(Err(anyhow::Error::new(e).context("chat stream"))));
                }
                Poll::Ready(None) => {
                    this.ended = true;
                    let rest = std::mem::take(&mut this.buf);
                    if !rest.is_empty() {
                        this.line(&rest);
                    }
                    this.close();
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl Drop for ChatStream {
    fn drop(&mut self) {
        if let Some(used) = self.used.take() {
            meter(used, self.price);
        }
        if self.done {
            return;
        }
        let Some(id) = self.id.take() else {
            return;
        };
        let Some(base) = self.base.take() else {
            return;
        };
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let req = self.http.post(format!("{base}/requests/{id}/cancel")).timeout(Duration::from_secs(2));
        rt.spawn(async move {
            let _ = req.send().await;
        });
    }
}
