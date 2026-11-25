use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};

use voice::Voice;

const VARIANTS: usize = 2;
const EDGE: f32 = 0.012;
const PAD_MS: usize = 25;
const FADE_MS: usize = 6;
const MAX_SECS: usize = 3;
const PUNCT: &[char] = &[',', '.', '!', '?', '…', ';', ':', '—'];

struct Spec {
    say: &'static str,
    how: &'static str,
    aliases: &'static [&'static str],
    listen: bool,
    generic: bool,
}

const fn spec(
    say: &'static str,
    how: &'static str,
    aliases: &'static [&'static str],
    listen: bool,
    generic: bool,
) -> Spec {
    Spec { say, how, aliases, listen, generic }
}

const SPECS: &[Spec] = &[
    spec("Mm-hmm.", "Soft, warm, just listening.", &["mm-hmm", "mmhmm", "mhm", "uh-huh", "mm"], true, true),
    spec("Yeah.", "Soft, warm, agreeing while listening.", &["yeah", "yep", "yes"], true, true),
    spec("Right.", "Soft, following along.", &["right", "i see", "gotcha"], true, true),
    spec("Oh no.", "Sympathetic, concerned, gentle.", &["oh no", "oh, no", "oh man", "ugh", "oof"], true, false),
    spec("Oh, wow.", "Genuinely impressed and surprised.", &["oh wow", "oh, wow", "wow", "whoa"], true, false),
    spec("Oh!", "Pleasantly surprised, bright.", &["oh"], true, false),
    spec("Aww.", "Touched, tender.", &["aww", "aw"], true, false),
    spec("Ooh.", "Intrigued, delighted.", &["ooh", "oh nice", "oh, nice", "nice"], true, false),
    spec(
        "Really?",
        "Surprised and curious.",
        &["really", "wait, really", "wait really", "no way", "wait, what", "wait"],
        true,
        false,
    ),
    spec("(laugh)", "Laughing warmly, amused.", &["(laugh)", "haha", "ha ha", "ha"], true, false),
    spec("Oh my gosh.", "Amazed and delighted.", &["oh my gosh", "oh my god", "oh my goodness"], true, false),
    spec("Hmm.", "Thinking it over, slow.", &["hmm", "hm"], false, false),
    spec("Um,", "Thinking, about to answer.", &["um", "uh"], false, false),
    spec("Well,", "Easygoing, starting a thought.", &["well"], false, false),
    spec("Okay.", "Easy, relaxed.", &["okay", "ok", "sure"], false, false),
    spec("Ah.", "Realizing, understanding.", &["ah", "ahh"], false, false),
    spec("Totally.", "Warm, agreeing.", &["totally", "exactly", "true"], true, false),
];

pub enum Probe {
    Wait,
    Hit(usize, usize),
    Miss,
}

pub struct Bank {
    clips: RwLock<Vec<Vec<Vec<f32>>>>,
    order: RwLock<Vec<(usize, &'static str)>>,
}

fn order(clips: &[Vec<Vec<f32>>]) -> Vec<(usize, &'static str)> {
    let mut order: Vec<(usize, &'static str)> = SPECS
        .iter()
        .enumerate()
        .filter(|(i, _)| !clips[*i].is_empty())
        .flat_map(|(i, s)| s.aliases.iter().map(move |a| (i, *a)))
        .collect();
    order.sort_by_key(|(_, a)| std::cmp::Reverse(a.len()));
    order
}

fn file(dir: &Option<PathBuf>, voice_id: &str, s: &Spec, variant: usize, rate: usize) -> Option<PathBuf> {
    dir.as_ref().map(|d| d.join(format!("{:016x}.f32", key(voice_id, s, variant, rate))))
}

impl Bank {
    pub fn build(engine: &Voice, voice_id: &str) -> Arc<Bank> {
        let rate = engine.sample_rate() as usize;
        let dir = cache_dir();
        let saved = |s: &Spec, v: usize| file(&dir, voice_id, s, v, rate).as_deref().and_then(load);
        let clips: Vec<Vec<Vec<f32>>> =
            SPECS.iter().map(|s| (0..VARIANTS).filter_map(|v| saved(s, v)).collect()).collect();
        let bank = Arc::new(Bank { order: RwLock::new(order(&clips)), clips: RwLock::new(clips) });
        let (filling, engine, voice_id) = (bank.clone(), engine.remote(), voice_id.to_string());
        std::thread::spawn(move || filling.fill(&engine, &voice_id, rate, &dir));
        bank
    }

    fn fill(&self, engine: &Voice, voice_id: &str, rate: usize, dir: &Option<PathBuf>) {
        let mut made = 0;
        for (i, s) in SPECS.iter().enumerate() {
            for v in 0..VARIANTS {
                let path = file(dir, voice_id, s, v, rate);
                if path.as_deref().and_then(load).is_some() {
                    continue;
                }
                if made == 0 {
                    log::info!("recording the voice's reactions in the background (once for each voice)");
                }
                made += 1;
                let mut pcm: Vec<f32> = Vec::new();
                let done =
                    engine.synthesize(s.say, Some(s.how), None, None, &AtomicBool::new(false), &mut |c: &[f32]| {
                        pcm.extend_from_slice(c);
                        Ok(())
                    });
                if let Err(e) = done {
                    log::warn!("could not record {:?}: {e:#}", s.say);
                    return;
                }
                let pcm = tidy(&pcm, rate);
                if pcm.is_empty() || pcm.len() > rate * MAX_SECS {
                    continue;
                }
                if let Some(p) = &path {
                    save(p, &pcm);
                }
                let mut clips = self.clips.write().unwrap();
                clips[i].push(pcm);
                *self.order.write().unwrap() = order(&clips);
            }
            let first = self.clips.read().unwrap()[i].first().cloned();
            if let Some(first) = first
                && let Err(e) = engine.lead(i, s.say, &first)
            {
                log::debug!("{e:#}");
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.order.read().unwrap().is_empty()
    }

    pub fn clip(&self, key: usize, n: usize) -> Vec<f32> {
        match self.clips.read().unwrap().get(key) {
            Some(v) if !v.is_empty() => v[n % v.len()].clone(),
            _ => Vec::new(),
        }
    }

    pub fn listens(&self, key: usize) -> bool {
        SPECS.get(key).is_some_and(|s| s.listen)
    }

    pub fn generic(&self, n: usize) -> Option<usize> {
        let clips = self.clips.read().unwrap();
        let keys: Vec<usize> =
            SPECS.iter().enumerate().filter(|(i, s)| s.generic && !clips[*i].is_empty()).map(|(i, _)| i).collect();
        (!keys.is_empty()).then(|| keys[n % keys.len()])
    }

    pub fn probe(&self, text: &str, end: bool) -> Probe {
        let order = self.order.read().unwrap().clone();
        if order.is_empty() {
            return Probe::Miss;
        }
        let lead = text.len() - text.trim_start().len();
        let body = &text[lead..];
        if body.is_empty() {
            return if end { Probe::Miss } else { Probe::Wait };
        }
        let lower = body.to_ascii_lowercase();
        let mut wait = false;
        for (key, alias) in &order {
            if let Some(after) = lower.strip_prefix(alias) {
                let sound = alias.ends_with(')');
                let mut tail = after.chars();
                match tail.next() {
                    None if end => return Probe::Hit(*key, text.len()),
                    None => wait = true,
                    Some(c) if PUNCT.contains(&c) => {
                        let more: usize = after.chars().take_while(|c| PUNCT.contains(c)).map(char::len_utf8).sum();
                        if (wait || more == after.len()) && !end {
                            wait = true;
                        } else {
                            return Probe::Hit(*key, lead + alias.len() + more);
                        }
                    }
                    Some(_) if sound && (!wait || end) => return Probe::Hit(*key, lead + alias.len()),
                    Some(_) => {}
                }
            } else if alias.starts_with(lower.as_str()) {
                wait = true;
            }
        }
        if wait && !end { Probe::Wait } else { Probe::Miss }
    }
}

fn cache_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("HOME")?).join(".cache/milo/clips");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn key(voice_id: &str, s: &Spec, variant: usize, rate: usize) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (voice_id, s.say, s.how, variant, rate).hash(&mut h);
    h.finish()
}

fn load(path: &Path) -> Option<Vec<f32>> {
    let bytes = std::fs::read(path).ok()?;
    let pcm: Vec<f32> = bytes.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
    (!pcm.is_empty()).then_some(pcm)
}

fn save(path: &Path, pcm: &[f32]) {
    let bytes: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
    if let Err(e) = std::fs::write(path, bytes) {
        log::debug!("could not cache {}: {e}", path.display());
    }
}

fn tidy(pcm: &[f32], rate: usize) -> Vec<f32> {
    let Some(first) = pcm.iter().position(|s| s.abs() > EDGE) else { return Vec::new() };
    let last = pcm.iter().rposition(|s| s.abs() > EDGE).unwrap_or(first);
    let pad = rate * PAD_MS / 1000;
    let mut out = pcm[first.saturating_sub(pad)..(last + pad).min(pcm.len())].to_vec();
    let fade = (rate * FADE_MS / 1000).min(out.len() / 2);
    let n = out.len();
    for i in 0..fade {
        let g = i as f32 / fade as f32;
        out[i] *= g;
        out[n - 1 - i] *= g;
    }
    out
}
