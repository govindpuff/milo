use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;
use std::time::Instant;

use futures_util::StreamExt;
use llm::chunker::Chunker;
use llm::{Llm, Message};
use tokio::sync::mpsc::UnboundedSender;

use crate::controller::Event;
use crate::react::{Bank, Probe};
use crate::tts::Job;

pub const FULL_WORDS: usize = 5;
const MOOD_CHARS: usize = 80;
const DULL: &[&str] = &[
    "flat",
    "resigned",
    "sigh",
    "monotone",
    "bored",
    "unimpressed",
    "patient",
    "clarifying",
    "deadpan",
    "dry",
    "sarcastic",
    "annoyed",
    "stern",
    "cold",
    "blunt",
    "teasing",
    "frustrated",
    "defensive",
    "firm",
    "irritated",
    "exasperated",
    "impatient",
    "hmm",
];
const EASY: &str = "warm, easy";

#[allow(clippy::too_many_arguments)]
pub async fn generate(
    llm: Llm,
    messages: Vec<Message>,
    turn: u64,
    stop: Arc<AtomicBool>,
    jobs: Sender<Job>,
    events: UnboundedSender<Event>,
    sounds: &'static [&'static str],
    trim_question: bool,
    bank: Arc<Bank>,
    said: Option<usize>,
    skip: Option<usize>,
    brief: bool,
    cap: usize,
) {
    let mut body = String::new();
    let mut capped = false;
    let mut mood: Option<String> = None;
    let mut lead: Option<String> = Some(String::new());
    let mut chunker = Chunker::new();
    let mut raw = String::new();
    let mut error = None;
    let mut spoken: Vec<String> = Vec::new();
    let mut rest: Vec<String> = Vec::new();
    let mut probe: Option<String> = (!bank.is_empty()).then(String::new);
    let mut first = false;
    let mut after: Option<usize> = None;
    let open = |held: String, end: bool, spoken: &mut Vec<String>| -> Result<(String, Option<usize>), String> {
        match bank.probe(&held, end) {
            Probe::Wait => Err(held),
            Probe::Miss => Ok((held, None)),
            Probe::Hit(key, _) if Some(key) == skip && Some(key) != said => Ok((held, None)),
            Probe::Hit(key, len) => {
                if Some(key) != said {
                    let text = held[..len].trim().to_string();
                    spoken.push(text.clone());
                    let _ = events.send(Event::Opener { turn, key, text });
                }
                Ok((held[len..].to_string(), Some(key)))
            }
        }
    };
    let say = |text: String, mood: &Option<String>, lead: Option<usize>, spoken: &mut Vec<String>| {
        let text = keep_sounds(&text, sounds);
        if !text.chars().any(char::is_alphanumeric) {
            return;
        }
        spoken.push(text.clone());
        let _ = jobs.send(Job::Say { turn, text: text.clone(), instruct: mood.clone(), lead, stop: stop.clone() });
        let _ = events.send(Event::Chunk { turn, text });
    };
    match llm.stream(&messages).await {
        Ok(mut stream) => {
            while let Some(item) = stream.next().await {
                match item {
                    Ok(delta) => {
                        if raw.is_empty() {
                            let _ = events.send(Event::LlmFirst { turn, at: Instant::now() });
                        }
                        raw.push_str(&delta);
                        let text = match lead.take() {
                            Some(mut held) => {
                                held.push_str(&delta);
                                match split_mood(&held) {
                                    Lead::Wait => {
                                        lead = Some(held);
                                        continue;
                                    }
                                    Lead::Mood(m, rest) => {
                                        if !m.is_empty() {
                                            let m = tone(&m);
                                            let _ = events.send(Event::Mood { turn, text: m.clone() });
                                            mood = Some(m);
                                        }
                                        rest
                                    }
                                    Lead::None => held,
                                }
                            }
                            None => delta,
                        };
                        let text = match probe.take() {
                            Some(mut held) => {
                                held.push_str(&text);
                                match open(held, false, &mut spoken) {
                                    Ok((text, key)) => {
                                        after = key;
                                        text
                                    }
                                    Err(held) => {
                                        probe = Some(held);
                                        continue;
                                    }
                                }
                            }
                            None => text,
                        };
                        for chunk in chunker.push(&text) {
                            body.push_str(&chunk);
                            body.push(' ');
                            if !first {
                                first = true;
                                say(chunk, &mood, after.take(), &mut spoken);
                            } else {
                                rest.push(chunk);
                            }
                        }
                        if sentences(&body).iter().filter(|s| whole(s)).count() >= cap {
                            capped = true;
                            break;
                        }
                    }
                    Err(e) => {
                        error = Some(format!("{e:#}"));
                        break;
                    }
                }
            }
        }
        Err(e) => error = Some(format!("{e:#}")),
    }
    let mut tail = lead.take().unwrap_or_default();
    if let Some(mut held) = probe.take() {
        held.push_str(&tail);
        tail = match open(held, true, &mut spoken) {
            Ok((text, key)) => {
                after = key;
                text
            }
            Err(held) => held,
        };
    }
    if !capped {
        if !tail.is_empty() {
            rest.extend(chunker.push(&tail));
        }
        rest.extend(chunker.finish());
    }
    if !rest.is_empty() {
        let joined = rest.join(" ");
        let kept = limit(spoken.last().map_or("", String::as_str), &joined, cap).to_string();
        rest = if kept.is_empty() { Vec::new() } else { vec![kept] };
    }
    if trim_question && let Some(last) = rest.pop() {
        let kept = drop_final_question(&last);
        if !kept.is_empty() {
            rest.push(kept.to_string());
        } else if rest.is_empty() && spoken.is_empty() {
            rest.push(last);
        }
    }
    if brief && !spoken.is_empty() {
        rest.clear();
    }
    if !rest.is_empty() {
        say(rest.join(" "), &mood, after.take(), &mut spoken);
    }
    let text = match (&mood, spoken.is_empty()) {
        (Some(m), false) => format!("[{m}] {}", spoken.join(" ")),
        (Some(m), true) => format!("[{m}]"),
        (None, _) => spoken.join(" "),
    };
    let _ = jobs.send(Job::End { turn });
    let _ = events.send(Event::LlmDone { turn, text, error });
}

fn tone(mood: &str) -> String {
    let kept: Vec<&str> = mood
        .split(',')
        .map(str::trim)
        .filter(|w| !w.is_empty() && !DULL.iter().any(|d| w.to_lowercase().contains(d)))
        .map(|w| if w == "ful" { "thoughtful" } else { w })
        .take(2)
        .collect();
    if kept.is_empty() { EASY.to_string() } else { kept.join(", ") }
}

pub fn sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if matches!(c, '.' | '!' | '?') && chars.peek().is_none_or(|(_, n)| n.is_whitespace()) {
            out.push(&text[start..i + c.len_utf8()]);
            start = i + c.len_utf8();
        }
    }
    if !text[start..].trim().is_empty() {
        out.push(&text[start..]);
    }
    out
}

fn ended(text: &str) -> bool {
    text.trim_end().ends_with(['.', '!', '?'])
}

pub fn words(sentence: &str) -> usize {
    sentence.split_whitespace().filter(|w| !w.starts_with('(') && w.chars().any(char::is_alphanumeric)).count()
}

fn whole(sentence: &str) -> bool {
    ended(sentence) && words(sentence) >= FULL_WORDS
}

fn limit<'a>(said: &str, rest: &'a str, cap: usize) -> &'a str {
    let mut n = sentences(said).iter().filter(|s| whole(s)).count();
    let mut open = !said.trim().is_empty() && !ended(said);
    let mut end = 0;
    for s in sentences(rest) {
        if n >= cap {
            break;
        }
        if whole(s) || (open && ended(s)) {
            n += 1;
        }
        open = false;
        end = s.as_ptr() as usize - rest.as_ptr() as usize + s.len();
    }
    rest[..end].trim()
}

fn drop_final_question(text: &str) -> &str {
    let t = text.trim_end();
    if !t.ends_with('?') {
        return t;
    }
    let body = &t[..t.len() - 1];
    match body.rfind(['.', '!', '?']) {
        Some(i) => t[..=i].trim_end(),
        None => "",
    }
}

fn keep_sounds(text: &str, sounds: &[&str]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut tail = text;
    while let Some(open) = tail.find('(') {
        out.push_str(&tail[..open]);
        let after = &tail[open + 1..];
        let Some(close) = after.find(')') else {
            tail = "";
            break;
        };
        let tag = after[..close].trim().to_lowercase();
        if sounds.contains(&tag.as_str()) {
            out.push('(');
            out.push_str(&tag);
            out.push(')');
        }
        tail = &after[close + 1..];
    }
    out.push_str(tail);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

enum Lead {
    Wait,
    Mood(String, String),
    None,
}

fn split_mood(held: &str) -> Lead {
    let t = held.trim_start();
    if t.is_empty() {
        return Lead::Wait;
    }
    let Some(body) = t.strip_prefix('[') else { return Lead::None };
    match body.split_once(']') {
        Some((mood, rest)) if !mood.trim().is_empty() => Lead::Mood(mood.trim().to_string(), rest.to_string()),
        Some((_, rest)) => Lead::Mood(String::new(), rest.to_string()),
        None if body.chars().count() > MOOD_CHARS => Lead::None,
        None => Lead::Wait,
    }
}
