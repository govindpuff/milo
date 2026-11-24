use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use llm::{Llm, Message};

const NOTES: usize = 12;
const NOTE: usize = 260;
const RECORD: usize = 40;
const LINE: usize = 420;
const FEW: usize = 2;
const NAMED: usize = 2;
const NONE: &str = "none yet";

const WRITE: &str = "You keep notes on how to work in one app or one website on a person's Mac. Another assistant operates it one action at a time, and reads your notes before it starts, so that it gets there in fewer actions the next time. You get the notes so far, and a record of what it just did there: each action, and what the action led to.

Write the notes again, as a list.
- A note says how something is done in this place. It starts with what it is for, and then gives the shortest run of actions that got it done, with each control called what the record calls it. For example: To start a new message, click button \"compose\", and type the name of the person into text field \"To:\". Leave out the actions that failed or did nothing.
- A note can also say what does not work here and what to do instead, when the record shows an action that failed, did nothing, or put text in the wrong place.
- Only say that something works when the record shows what it led to.
- The notes are for the next task, which will be about other people, other words and other files. Write the name of the person, the text or the file, and never the ones in this record. Leave out the numbers of controls, because they change every time.
- Keep each earlier note that this record does not show to be wrong. Join notes that say the same thing.
- At most 12 notes. Each is one or two short plain sentences on a line that starts with a dash.
- Reply with only the list.";

pub struct Step {
    pub place: String,
    pub did: String,
    pub wasted: bool,
}

fn folder() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| anyhow!("no home folder"))?;
    Ok(home.join(".config/milo/skills"))
}

pub fn slug(place: &str) -> String {
    let kept: String =
        place.trim().to_lowercase().chars().map(|c| if c.is_alphanumeric() || c == '.' { c } else { '-' }).collect();
    kept.trim_matches(['-', '.']).to_string()
}

fn notes(place: &str) -> Vec<String> {
    let name = slug(place);
    if name.is_empty() {
        return Vec::new();
    }
    let Ok(raw) = folder().and_then(|dir| Ok(std::fs::read_to_string(dir.join(format!("{name}.md")))?)) else {
        return Vec::new();
    };
    listed(&raw)
}

fn listed(raw: &str) -> Vec<String> {
    raw.lines()
        .filter_map(|line| line.trim().strip_prefix('-').or_else(|| line.trim().strip_prefix('*')))
        .map(|line| line.trim().chars().take(NOTE).collect::<String>())
        .filter(|line| !line.is_empty())
        .take(NOTES)
        .collect()
}

pub fn brief(place: &str) -> Option<String> {
    let notes = notes(place);
    if notes.is_empty() {
        return None;
    }
    Some(format!("\n\nNotes from earlier work in {place}:\n- {}", notes.join("\n- ")))
}

fn within(text: &str, word: &str) -> bool {
    let edge = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric());
    text.match_indices(word)
        .any(|(at, _)| edge(text[..at].chars().next_back()) && edge(text[at + word.len()..].chars().next()))
}

pub fn named(text: &str) -> Vec<String> {
    let text = text.to_lowercase();
    let Ok(all) = folder().and_then(|dir| Ok(std::fs::read_dir(dir)?)) else { return Vec::new() };
    let mut out = Vec::new();
    for file in all.flatten() {
        let Ok(raw) = std::fs::read_to_string(file.path()) else { continue };
        let Some(place) = raw.lines().next().and_then(|line| line.strip_prefix("# ")).map(str::trim) else { continue };
        let lower = place.to_lowercase();
        let short = match lower.contains('.') && !lower.contains(' ') {
            true => lower.rsplit('.').nth(1).unwrap_or(&lower).to_string(),
            false => lower.clone(),
        };
        if !short.is_empty() && (within(&text, &lower) || within(&text, &short)) {
            out.push(place.to_string());
        }
    }
    out.sort();
    out.truncate(NAMED);
    out
}

async fn rewrite(llm: &Llm, place: &str, task: &str, record: &[&Step], ended: &str) -> Result<usize> {
    let had = notes(place);
    let before = if had.is_empty() { NONE.to_string() } else { format!("- {}", had.join("\n- ")) };
    let from = record.len().saturating_sub(RECORD);
    let lines: Vec<String> = record[from..]
        .iter()
        .enumerate()
        .map(|(i, step)| format!("{}. {}", i + 1, step.did.chars().take(LINE).collect::<String>()))
        .collect();
    let ask = format!(
        "The place: {place}\n\nThe notes so far:\n{before}\n\nWhat was asked for: {task}\n\nThe record:\n{}\n\nHow it ended: {ended}",
        lines.join("\n")
    );
    let reply = llm.write(&[Message::system(WRITE), Message::user(ask)]).await?;
    let now = listed(&reply);
    if now.is_empty() || now == had {
        return Ok(0);
    }
    let dir = folder()?;
    std::fs::create_dir_all(&dir).context("could not make the notes folder")?;
    std::fs::write(dir.join(format!("{}.md", slug(place))), format!("# {place}\n\n- {}\n", now.join("\n- ")))
        .context("could not save the notes")?;
    Ok(now.len())
}

pub async fn learn(llm: Llm, task: String, trail: Vec<Step>, ended: String) {
    let mut places: Vec<&str> = Vec::new();
    for step in &trail {
        if !step.place.is_empty() && !places.contains(&step.place.as_str()) {
            places.push(&step.place);
        }
    }
    for place in places {
        let record: Vec<&Step> = trail.iter().filter(|step| step.place == place).collect();
        let rough = record.iter().any(|step| step.wasted);
        if record.len() < FEW || (!rough && brief(place).is_some()) {
            continue;
        }
        match rewrite(&llm, place, &task, &record, &ended).await {
            Ok(0) => {}
            Ok(count) => log::info!("kept {count} notes about working in {place}"),
            Err(e) => log::warn!("the notes about {place} could not be written: {e:#}"),
        }
    }
}
