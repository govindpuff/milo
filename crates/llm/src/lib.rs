pub mod chunker;
pub mod server;

use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use bytes::Bytes;
use futures_core::Stream;
use serde::{Deserialize, Serialize};

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
    tool_calls: Option<serde_json::Value>,
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
                &serde_json::json!([
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

    pub fn called(calls: &[Call]) -> Self {
        let list: Vec<serde_json::Value> = calls
            .iter()
            .map(|c| {
                serde_json::json!({
                    "id": c.id,
                    "type": "function",
                    "function": { "name": c.name, "arguments": serde_json::Value::Object(c.args.clone()).to_string() },
                })
            })
            .collect();
        Self { tool_calls: Some(list.into()), ..Self::new(Role::Assistant, "") }
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

#[derive(Clone)]
pub struct Llm {
    inner: Arc<Inner>,
}

struct Inner {
    model: String,
    http: reqwest::Client,
    base: String,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [Message],
    stream: bool,
    max_tokens: u32,
    temperature: f32,
    chat_template_kwargs: TemplateKwargs,
}

#[derive(Serialize)]
struct TemplateKwargs {
    enable_thinking: bool,
}

#[derive(Serialize)]
struct ActRequest<'a> {
    model: &'a str,
    messages: &'a [Message],
    tools: &'a serde_json::Value,
    tool_choice: &'static str,
    stream: bool,
    max_tokens: u32,
    temperature: f32,
    chat_template_kwargs: TemplateKwargs,
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
    pub args: serde_json::Map<String, serde_json::Value>,
}

#[derive(Serialize)]
struct PrefillRequest<'a> {
    kind: &'static str,
    messages: &'a [Message],
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a serde_json::Value>,
    enable_thinking: bool,
}

impl Llm {
    pub fn new(base_url: &str, model: &str) -> Llm {
        let http = reqwest::Client::builder()
            .no_proxy()
            .tcp_nodelay(true)
            .pool_idle_timeout(Duration::from_secs(25))
            .connect_timeout(Duration::from_secs(2))
            .build()
            .expect("reqwest client");
        let base = base_url.trim_end_matches('/').to_string();
        Llm { inner: Arc::new(Inner { model: model.to_string(), http, base }) }
    }

    fn chat_request<'a>(&'a self, messages: &'a [Message], stream: bool, max_tokens: u32) -> ChatRequest<'a> {
        ChatRequest {
            model: &self.inner.model,
            messages,
            stream,
            max_tokens,
            temperature: TEMPERATURE,
            chat_template_kwargs: TemplateKwargs { enable_thinking: false },
        }
    }

    pub async fn stream(
        &self,
        messages: &[Message],
    ) -> Result<impl Stream<Item = Result<String>> + Send + Unpin + use<>> {
        let req = self.chat_request(messages, true, REPLY_TOKENS);
        let resp = self
            .inner
            .http
            .post(format!("{}/chat/completions", self.inner.base))
            .json(&req)
            .send()
            .await
            .context("chat completions request")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!("chat completions {status}: {body}");
        }
        Ok(ChatStream {
            body: Box::pin(resp.bytes_stream()),
            buf: Vec::new(),
            out: VecDeque::new(),
            ended: false,
            done: false,
            id: None,
            http: self.inner.http.clone(),
            base: self.inner.base.clone(),
        })
    }

    pub async fn see(&self, question: &str, image: &str) -> Result<String> {
        let messages = [Message::seen(question, Some(image.to_string()))];
        let mut req = self.chat_request(&messages, false, SEE_TOKENS);
        req.temperature = 0.0;
        let resp = self
            .inner
            .http
            .post(format!("{}/chat/completions", self.inner.base))
            .header("X-Rapid-Priority", "background")
            .timeout(ACT_TIMEOUT)
            .json(&req)
            .send()
            .await
            .context("picture request")?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("picture request {status}: {body}");
        }
        let body: serde_json::Value = serde_json::from_str(&body).context("picture response")?;
        Ok(body["choices"][0]["message"]["content"].as_str().unwrap_or("").trim().to_string())
    }

    pub async fn act(&self, messages: &[Message], tools: &serde_json::Value) -> Result<Reply> {
        let req = ActRequest {
            model: &self.inner.model,
            messages,
            tools,
            tool_choice: "auto",
            stream: false,
            max_tokens: ACT_TOKENS,
            temperature: 0.0,
            chat_template_kwargs: TemplateKwargs { enable_thinking: false },
        };
        let resp = self
            .inner
            .http
            .post(format!("{}/chat/completions", self.inner.base))
            .header("X-Rapid-Priority", "background")
            .timeout(ACT_TIMEOUT)
            .json(&req)
            .send()
            .await
            .context("tool request")?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("tool request {status}: {body}");
        }
        let body: serde_json::Value = serde_json::from_str(&body).context("tool response")?;
        let message = &body["choices"][0]["message"];
        let mut calls = Vec::new();
        for call in message["tool_calls"].as_array().into_iter().flatten() {
            let f = &call["function"];
            if let Some(mut c) = parse_call(&f["name"], &f["arguments"]) {
                if let Some(id) = call["id"].as_str() {
                    c.id = id.to_string();
                }
                calls.push(c);
            }
        }
        let text = message["content"].as_str().unwrap_or("").trim().to_string();
        if calls.is_empty() {
            calls = inline_calls(&text);
        }
        for (i, c) in calls.iter_mut().enumerate() {
            if c.id.is_empty() {
                c.id = format!("call_{i}");
            }
        }
        Ok(Reply { calls, text })
    }

    pub async fn warm(&self, messages: &[Message]) -> Result<()> {
        if let [m] = messages
            && m.role == Role::System
        {
            match self.prefill_prefix(messages, None).await {
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
        let req = self.chat_request(messages, false, 1);
        let resp = self
            .inner
            .http
            .post(format!("{}/chat/completions", self.inner.base))
            .json(&req)
            .send()
            .await
            .context("warm request")?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("warm {status}: {body}");
        }
        Ok(())
    }

    pub async fn warm_tools(&self, system: &Message, tools: &serde_json::Value) -> Result<()> {
        self.prefill_prefix(std::slice::from_ref(system), Some(tools)).await
    }

    async fn prefill_prefix(&self, messages: &[Message], tools: Option<&serde_json::Value>) -> Result<()> {
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
const ACT_TOKENS: u32 = 320;
const SEE_TOKENS: u32 = 60;
const ACT_TIMEOUT: Duration = Duration::from_secs(60);

fn parse_call(name: &serde_json::Value, args: &serde_json::Value) -> Option<Call> {
    let name = name.as_str()?.trim();
    if name.is_empty() {
        return None;
    }
    let args = match args {
        serde_json::Value::Object(m) => m.clone(),
        serde_json::Value::String(s) => serde_json::from_str(s).unwrap_or_default(),
        _ => Default::default(),
    };
    Some(Call { id: String::new(), name: name.to_string(), args })
}

fn inline_calls(text: &str) -> Vec<Call> {
    let mut calls = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("<tool_call>") {
        let after = &rest[at + "<tool_call>".len()..];
        let end = after.find("</tool_call>").unwrap_or(after.len());
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(after[..end].trim())
            && let Some(c) = parse_call(&v["name"], &v["arguments"])
        {
            calls.push(c);
        }
        rest = &after[end..];
    }
    calls
}

#[derive(Deserialize)]
struct Chunk {
    id: Option<String>,
    #[serde(default)]
    choices: Vec<Choice>,
    error: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct Choice {
    delta: Option<Delta>,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct Delta {
    content: Option<String>,
}

struct ChatStream {
    body: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>,
    buf: Vec<u8>,
    out: VecDeque<Result<String>>,
    ended: bool,
    done: bool,
    id: Option<String>,
    http: reqwest::Client,
    base: String,
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

    fn line(&mut self, line: &[u8]) {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let Some(data) = line.strip_prefix(b"data:") else {
            return;
        };
        let data = data.strip_prefix(b" ").unwrap_or(data);
        if data == b"[DONE]" {
            self.done = true;
            return;
        }
        let chunk: Chunk = match serde_json::from_slice(data) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("unparseable SSE data ({e}): {}", String::from_utf8_lossy(data));
                return;
            }
        };
        if let Some(err) = chunk.error {
            self.done = true;
            self.ended = true;
            self.out.push_back(Err(anyhow!("server error: {err}")));
            return;
        }
        if self.id.is_none() {
            self.id = chunk.id;
        }
        for choice in chunk.choices {
            if let Some(text) = choice.delta.and_then(|d| d.content)
                && !text.is_empty()
            {
                self.out.push_back(Ok(text));
            }
            if choice.finish_reason.is_some() {
                self.done = true;
            }
        }
    }
}

impl Stream for ChatStream {
    type Item = Result<String>;

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
                    this.done = true;
                    let rest = std::mem::take(&mut this.buf);
                    if !rest.is_empty() {
                        this.line(&rest);
                    }
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl Drop for ChatStream {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        let Some(id) = self.id.take() else {
            return;
        };
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let req = self.http.post(format!("{}/requests/{id}/cancel", self.base)).timeout(Duration::from_secs(2));
        rt.spawn(async move {
            let _ = req.send().await;
        });
    }
}
