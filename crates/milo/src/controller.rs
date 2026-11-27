use std::cell::Cell;
use std::collections::VecDeque;
use std::io::{IsTerminal, Write};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use audio::Playback;
use llm::{Call, Llm, Message, Role};
use serde_json::Value;
use speech::{Speech, SpeechEvent};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use turn::TurnEvent;

use crate::capture::AudioClock;
use crate::computer;
use crate::react::Bank;
use crate::reply::generate;
use crate::tools::{Tools, shorten};
use crate::tts::{Gate, Job};

const HARD_WAIT: Duration = Duration::from_secs(3);
const HOLD: Duration = Duration::from_millis(80);
const SPECULATE: Duration = Duration::from_millis(250);
const MAX_WAIT: Duration = Duration::from_millis(800);
const SETTLE: Duration = Duration::from_millis(350);
const PATIENCE: Duration = Duration::from_millis(1500);
const BARGE_WORDS: usize = 2;
const TOKEN_SLACK: Duration = Duration::from_millis(1500);
const LATE_WORDS: Duration = Duration::from_secs(1);
const REFINE_WAIT: Duration = Duration::from_millis(450);
const TRAIL: Duration = Duration::from_millis(100);
const TAIL: Duration = Duration::from_millis(400);
const ECHO_WORDS: usize = 3;
const POLL: Duration = Duration::from_millis(5);
const IDLE: Duration = Duration::from_secs(3600);
const HISTORY_MAX: usize = 80;
const HISTORY_KEEP: usize = 30;
const GHOST_CHARS: usize = 72;
const SHORT_WORDS: usize = 3;
const SHORT_PATIENCE: Duration = Duration::from_millis(600);
const RESULT: &str = "(result)";
const WAITING: &str = "not done yet, waiting for them to approve on their screen:";
const WORKING: &str =
    "you were working on their computer when they said this, and that work is paused until you go on with it";
const MAX_ROUNDS: usize = 60;
const HELD: Duration = Duration::from_secs(8);
const HELD_POLL: Duration = Duration::from_millis(20);
const GRACE: Duration = Duration::from_millis(1200);
const UNDONE: &str = "they spoke before this was finished, so it was stopped. It may or may not have happened: check what is there now before you do anything else";
const HALTED: &str = "the person stopped it before it finished";
const STOPPED: &str = "they pressed Stop on their screen, so the work on their computer was stopped";
const BACKCHANNELS: &[&str] = &[
    "yeah", "yep", "yup", "uh-huh", "uh", "huh", "mhm", "mm-hmm", "mmhmm", "right", "okay", "ok", "sure", "wow",
    "nice", "cool", "oh", "ah", "ha", "haha", "true", "exactly", "gotcha", "hmm", "hm", "mm", "mhmm", "alright", "yes",
];
const FILLERS: &[&str] = &["um", "umm", "uh", "uhh", "uhm", "erm"];
const WORDLESS: &str = "(hmm)";
const WORDLESS_MS: std::ops::RangeInclusive<u32> = 190..=1500;
const ACK_EXTRA: Duration = Duration::from_millis(500);
const ACK_GAIN: f32 = 0.7;
const OVERLAP_AFTER: Duration = Duration::from_millis(3500);
const OVERLAP_GAP: Duration = Duration::from_secs(6);
const OVERLAP_GAIN: f32 = 0.45;
const OPENER_QUIET: Duration = Duration::from_millis(1200);
const EARLY: Duration = Duration::from_millis(1500);
const YIELD_EARLY: Duration = Duration::from_millis(250);
const YIELD_LATE: Duration = Duration::from_millis(700);
const REACT_AFTER: Duration = Duration::from_millis(280);
const REACT_GAP: Duration = Duration::from_millis(2500);
const REACT_MEMORY: Duration = Duration::from_secs(6);
const REACT_WORDS: usize = 3;
const SURE: f32 = 0.9;
const FAST_SETTLE: Duration = Duration::from_millis(220);
const DANGLE_PATIENCE: Duration = Duration::from_millis(3000);
const DANGLE_HARD: Duration = Duration::from_secs(6);
const DANGLERS: &[&str] = &[
    "um", "umm", "uh", "uhh", "uhm", "erm", "er", "i", "i'm", "i've", "i'd", "i'll", "we", "we're", "they", "they're",
    "he", "she", "he's", "she's", "it's", "that's", "there's", "like", "and", "but", "so", "or", "the", "a", "an",
    "to", "of", "was", "were", "because", "cause", "with", "my", "your", "our", "their", "for", "at", "if", "when",
    "while", "which", "gonna", "wanna", "kinda", "from", "into", "than", "as", "by",
];
const LEADIN_WAIT: Duration = Duration::from_millis(1600);
const SEARCHING: &[&str] = &[
    "what's it called",
    "what is it called",
    "what was it",
    "what's the word",
    "what's that called",
    "the name of it",
];
const ACK_WORDS: usize = 5;

pub enum Event {
    Speech(SpeechEvent, Instant),
    Voice(TurnEvent),
    LlmFirst { turn: u64, at: Instant },
    Chunk { turn: u64, text: String },
    Mood { turn: u64, text: String },
    Opener { turn: u64, key: usize, text: String },
    LlmDone { turn: u64, text: String, calls: Vec<Call>, error: Option<String> },
    TtsFirst { turn: u64, at: Instant },
    TtsEnd { turn: u64 },
    Did { turn: u64, id: String, text: String },
    Acted { turn: u64 },
    Halt,
    Quit,
    Aside(String),
    Trimmed { cut: usize, system: String },
    Fatal(String),
}

pub struct Settings {
    pub bank: Arc<Bank>,
    pub backchannel: bool,
    pub yielding: bool,
    pub sounds: &'static [&'static str],
    pub tools: Option<Arc<Tools>>,
    pub schema: Option<Arc<Value>>,
    pub listening: Arc<AtomicBool>,
    pub talking: Arc<AtomicBool>,
    pub busy: Arc<AtomicBool>,
    pub system: Option<String>,
}

#[derive(PartialEq)]
enum Line {
    Unopened,
    Open,
    Closed,
}

struct Heard {
    text: String,
    words: Vec<String>,
    at: Instant,
    speech_end_ms: Option<u64>,
    refined: bool,
}

struct Verdict {
    complete: Option<bool>,
    prob: Option<f32>,
    speech_end: Instant,
    at: Instant,
    refining: bool,
    seq: u64,
    voiced_ms: u32,
    acked: bool,
}

struct Turn {
    id: u64,
    text: String,
    prompt: String,
    words: Vec<String>,
    base: Vec<String>,
    mood: Option<String>,
    opener: Option<usize>,
    opener_played: bool,
    aside: bool,
    calls: Vec<Call>,
    owed: Vec<String>,
    acting: Option<JoinHandle<()>>,
    rounds: usize,
    hush: bool,
    epoch: u64,
    said: String,
    started: Instant,
    eou: Option<Instant>,
    reason: String,
    decided: Instant,
    speech_end: Option<Instant>,
    stop: Arc<AtomicBool>,
    llm: JoinHandle<()>,
    committed: bool,
    llm_done: bool,
    tts_done: bool,
    full: String,
    held_chunks: Vec<String>,
    line: Line,
    llm_first: Option<Instant>,
    tts_first: Option<Instant>,
    onset_before: Option<Instant>,
    onset: Option<Instant>,
    metrics_printed: bool,
}

pub struct Controller {
    llm: Llm,
    speech: Arc<Speech>,
    gate: Arc<Gate>,
    playback: Playback,
    clock: Arc<AudioClock>,
    jobs: Sender<Job>,
    events: UnboundedSender<Event>,
    settings: Settings,
    history: Vec<Message>,
    trimming: bool,
    turn: Option<Turn>,
    billed: llm::Spent,
    heard: Option<Heard>,
    voice_reset: turn::Reset,
    voice_seq: u64,
    verdict: Option<Verdict>,
    refined_seq: Option<u64>,
    silent_seq: Option<u64>,
    react_n: usize,
    last_react: Option<(Instant, Option<usize>)>,
    last_opener: Option<usize>,
    voice_start: Option<Instant>,
    carry: Option<String>,
    stream_words: Vec<String>,
    echo: Vec<String>,
    leaked: bool,
    faded: Option<Instant>,
    gaps: (u64, u64),
    asides: VecDeque<String>,
    next_id: u64,
    dim: bool,
    ghost: Cell<bool>,
}

impl Controller {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        llm: Llm,
        speech: Arc<Speech>,
        gate: Arc<Gate>,
        playback: Playback,
        clock: Arc<AudioClock>,
        jobs: Sender<Job>,
        events: UnboundedSender<Event>,
        settings: Settings,
        history: Vec<Message>,
        voice_reset: turn::Reset,
    ) -> Controller {
        Controller {
            llm,
            speech,
            gate,
            playback,
            clock,
            jobs,
            events,
            settings,
            history,
            trimming: false,
            turn: None,
            billed: llm::spent(),
            heard: None,
            voice_reset,
            voice_seq: 0,
            verdict: None,
            refined_seq: None,
            silent_seq: None,
            react_n: 0,
            last_react: None,
            last_opener: None,
            voice_start: None,
            carry: None,
            stream_words: Vec::new(),
            echo: Vec::new(),
            leaked: false,
            faded: None,
            gaps: (0, 0),
            asides: VecDeque::new(),
            next_id: 0,
            dim: std::io::stdout().is_terminal(),
            ghost: Cell::new(false),
        }
    }

    pub async fn run(mut self, mut rx: UnboundedReceiver<Event>) -> Result<()> {
        let ctrl_c = tokio::signal::ctrl_c();
        tokio::pin!(ctrl_c);
        let result = loop {
            let wake = tokio::time::Instant::from_std(self.next_wake());
            tokio::select! {
                r = &mut ctrl_c => break r.map_err(Into::into),
                ev = rx.recv() => match ev {
                    Some(Event::Fatal(e)) => break Err(anyhow!(e)),
                    Some(Event::Quit) => break Ok(()),
                    Some(ev) => self.handle(ev),
                    None => break Ok(()),
                },
                _ = tokio::time::sleep_until(wake) => {}
            }
            self.tick();
        };
        self.shutdown();
        result
    }

    fn next_wake(&self) -> Instant {
        match &self.turn {
            Some(_) => Instant::now() + POLL,
            None => match &self.heard {
                Some(h) if !h.words.is_empty() => h.at + SPECULATE,
                _ if !self.asides.is_empty() || self.leaked => Instant::now() + POLL,
                _ => Instant::now() + IDLE,
            },
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        if let Some(reason) = self.ready(now) {
            self.commit(reason);
        }
        if self.turn.is_none()
            && let Some(h) = &self.heard
            && !h.words.is_empty()
            && (h.refined || now >= h.at + SPECULATE || self.finished() == Some(true))
        {
            let speech_end = h.speech_end_ms.and_then(|ms| self.clock.wall(ms));
            self.start(h.text.clone(), h.words.clone(), speech_end, None);
        }
        self.maybe_yield(now);
        self.maybe_react(now);
        self.maybe_aside();
        self.poll_onset();
        self.fade(now);
        self.check_end();
    }

    fn fade(&mut self, now: Instant) {
        if self.playback.queued_ms() > 0 {
            self.leaked |= self.turn.as_ref().is_some_and(|t| t.committed);
            self.faded = None;
            return;
        }
        if !self.leaked {
            return;
        }
        let at = *self.faded.get_or_insert(now);
        let silent = self.voice_start.is_none() && self.heard.as_ref().is_none_or(|h| h.words.is_empty());
        if silent && now >= at + TAIL {
            self.voice_reset.reset();
            self.leaked = false;
            self.faded = None;
        }
    }

    fn echoed(&self, words: &[String]) -> usize {
        if self.leaked { overlap(words, &self.echo) } else { 0 }
    }

    fn unechoed(&self, text: String) -> String {
        match self.echoed(&content_words(&text)) {
            0 => text,
            count => after(&text, count).to_string(),
        }
    }

    fn quiet(&self) -> bool {
        self.turn.is_none() && self.voice_start.is_none() && self.heard.as_ref().is_none_or(|h| h.words.is_empty())
    }

    fn maybe_aside(&mut self) {
        if self.asides.is_empty() || !self.quiet() || self.playback.queued_ms() > 0 {
            return;
        }
        let text: Vec<String> = self.asides.drain(..).map(|a| format!("{RESULT} {a}")).collect();
        if let Some(c) = self.carry.take() {
            remember(&mut self.history, Message::user(format!("[{}] {c}", local_time())));
        }
        self.start(text.join("\n"), Vec::new(), None, None);
        if let Some(t) = self.turn.as_mut() {
            t.aside = true;
        }
        self.commit("result".into());
    }

    fn proceed(&mut self) {
        let Some(tools) = self.settings.tools.clone() else { return };
        let Some(t) = self.turn.as_mut() else { return };
        if !t.committed || !t.llm_done || t.calls.is_empty() || t.acting.is_some() {
            return;
        }
        let calls = std::mem::take(&mut t.calls);
        let text = std::mem::take(&mut t.full);
        let words = unmarked(&text).to_string();
        t.rounds += 1;
        t.hush = words.chars().any(char::is_alphanumeric) && calls.iter().all(|call| tools.quiet(&call.name));
        t.owed = calls.iter().map(|call| call.id.clone()).collect();
        if !words.is_empty() {
            if !t.said.is_empty() {
                t.said.push(' ');
            }
            t.said.push_str(&words);
        }
        let id = t.id;
        shorten(&mut self.history);
        self.history.push(Message::called(text, &calls));
        for call in &calls {
            let mut args = call.args.clone();
            args.remove("why");
            self.dim_line(&format!("({} {})", call.name, Value::Object(args)));
        }
        self.settings.busy.store(true, Relaxed);
        let acting = tokio::spawn(act(tools, calls, id, self.events.clone(), self.settings.listening.clone()));
        if let Some(t) = self.turn.as_mut() {
            t.acting = Some(acting);
        }
    }

    fn on_acted(&mut self, id: u64) {
        let Some(t) = self.turn.as_mut().filter(|t| t.id == id) else { return };
        t.acting = None;
        self.settings.busy.store(false, Relaxed);
        if t.hush {
            let _ = self.jobs.send(Job::End { turn: id });
            return;
        }
        let tools = if t.rounds < MAX_ROUNDS { self.settings.schema.clone() } else { None };
        t.llm_done = false;
        t.epoch = computer::epoch();
        t.llm = tokio::spawn(generate(
            self.llm.clone(),
            computer::compose(&self.history),
            id,
            t.stop.clone(),
            self.jobs.clone(),
            self.events.clone(),
            self.settings.sounds,
            false,
            self.settings.bank.clone(),
            None,
            self.last_opener,
            false,
            tools,
        ));
    }

    fn bill(&mut self) {
        if let Some(line) = crate::tools::bill(self.billed) {
            self.dim_line(&line);
        }
        self.billed = llm::spent();
    }

    fn halt(&mut self) {
        if self.turn.as_ref().is_some_and(|t| t.committed) {
            self.interrupt();
        }
        if let Some(tools) = &self.settings.tools {
            tools.rest(HALTED, false);
        }
        self.asides.push_back(STOPPED.to_string());
    }

    fn verdict(&self) -> Option<&Verdict> {
        let v = self.verdict.as_ref()?;
        let h = self.heard.as_ref()?;
        let fresh = match h.speech_end_ms.and_then(|ms| self.clock.wall(ms)) {
            Some(token) => v.speech_end + TOKEN_SLACK >= token,
            None => v.at + HARD_WAIT / 2 >= h.at,
        };
        fresh.then_some(v)
    }

    fn finished(&self) -> Option<bool> {
        let v = self.verdict()?;
        let h = self.heard.as_ref()?;
        if h.text.trim_end().ends_with('?') {
            Some(true)
        } else if h.refined && dangling(&h.text) {
            Some(false)
        } else {
            v.complete
        }
    }

    fn maybe_yield(&mut self, now: Instant) {
        if !self.settings.yielding {
            return;
        }
        let Some(start) = self.voice_start else { return };
        let Some(t) = self.turn.as_ref().filter(|t| t.committed) else { return };
        if t.tts_done && self.playback.queued_ms() == 0 {
            return;
        }
        if start < t.decided {
            return;
        }
        let need = if early(t, start) { YIELD_EARLY } else { YIELD_LATE };
        if now >= start + need {
            self.barge();
        }
    }

    fn barge(&mut self) {
        self.poll_onset();
        let Some(t) = self.turn.as_ref() else { return };
        if t.aside || t.rounds > 0 || !early(t, Instant::now()) {
            self.interrupt();
            return;
        }
        let Some(mut t) = self.turn.take() else { return };
        t.stop.store(true, Relaxed);
        t.llm.abort();
        self.settings.talking.store(false, Relaxed);
        self.gate.interrupt(t.id);
        close_line(&mut t);
        if self.history.last().is_some_and(|m| m.role == Role::User && m.content == t.prompt) {
            self.history.pop();
        }
        self.carry = Some(t.text);
        self.dim_line("(you kept going)");
    }

    fn play_react(&mut self, key: usize, gain: f32, now: Instant, felt: Option<usize>) {
        let clip: Vec<f32> = self.settings.bank.clip(key, self.react_n).iter().map(|s| s * gain).collect();
        self.react_n += 1;
        self.last_react = Some((now, felt));
        self.playback.write(&clip);
    }

    fn maybe_react(&mut self, now: Instant) {
        let bank = &self.settings.bank;
        if !self.settings.backchannel
            || bank.is_empty()
            || self.playback.queued_ms() > 0
            || self.turn.as_ref().is_some_and(|t| t.committed)
        {
            return;
        }
        let since = self.last_react.map(|(at, _)| now.saturating_duration_since(at));
        if let Some(start) = self.voice_start {
            if now >= start + OVERLAP_AFTER
                && since.is_none_or(|d| d >= OVERLAP_GAP)
                && self.stream_words.len() >= ACK_WORDS
                && let Some(key) = bank.generic(self.react_n)
            {
                self.play_react(key, OVERLAP_GAIN, now, None);
            }
            return;
        }
        if since.is_some_and(|d| d < REACT_GAP) || self.finished() != Some(false) {
            return;
        }
        let Some(h) = self.heard.as_ref() else { return };
        let Some(v) = self.verdict().filter(|v| !v.acked) else { return };
        if h.words.len() < REACT_WORDS || now < v.speech_end + REACT_AFTER {
            return;
        }
        let dangle = h.refined && dangling(&h.text);
        let felt =
            self.turn.as_ref().and_then(|t| t.opener.filter(|k| !dangle && !t.opener_played && bank.listens(*k)));
        let Some(key) = felt.or_else(|| bank.generic(self.react_n)) else { return };
        if let Some(v) = self.verdict.as_mut() {
            v.acked = true;
        }
        if let (Some(_), Some(t)) = (felt, self.turn.as_mut()) {
            t.opener_played = true;
        }
        self.play_react(key, ACK_GAIN, now, felt);
    }

    fn maybe_wordless(&mut self) {
        let Some(v) = &self.verdict else { return };
        if self.silent_seq != Some(v.seq)
            || !WORDLESS_MS.contains(&v.voiced_ms)
            || self.turn.is_some()
            || self.playback.queued_ms() > 0
            || self.heard.as_ref().is_some_and(|h| !h.words.is_empty())
        {
            return;
        }
        self.heard = Some(Heard {
            text: WORDLESS.to_string(),
            words: vec![WORDLESS.to_string()],
            at: Instant::now(),
            speech_end_ms: None,
            refined: true,
        });
    }

    fn ready(&self, now: Instant) -> Option<String> {
        let t = self.turn.as_ref().filter(|t| !t.committed)?;
        let last = self.heard.as_ref().map_or(t.started, |h| h.at);
        let dangle = self.heard.as_ref().is_some_and(|h| h.refined && dangling(&h.text));
        if now >= last + if dangle { DANGLE_HARD } else { HARD_WAIT } {
            return Some("timeout".into());
        }
        if self.voice_start.is_some() {
            return None;
        }
        if self.leadin(t) && now < self.verdict().map_or(last, |v| v.speech_end) + LEADIN_WAIT {
            return None;
        }
        if self.verdict().is_some_and(|v| v.refining && now < v.at + REFINE_WAIT) {
            return None;
        }
        let label = |v: &Verdict, name: &str| match v.prob {
            Some(p) => format!("{name} {p:.2}"),
            None => name.to_string(),
        };
        let finished = self.finished();
        match self.verdict() {
            Some(v) if finished == Some(true) => {
                let ended = self.heard.as_ref().is_some_and(|h| h.text.trim_end().ends_with(['.', '!', '?']));
                let sure = ended && v.prob.is_some_and(|p| p >= SURE);
                let settle = if sure { FAST_SETTLE } else { SETTLE };
                (now >= (v.speech_end + settle).max(last + TRAIL)).then(|| label(v, "turn"))
            }
            Some(v) if finished == Some(false) => {
                let short = self
                    .heard
                    .as_ref()
                    .is_some_and(|h| h.words.len() <= SHORT_WORDS || trailing(&h.text) || h.refined && closed(&h.text));
                let wait = if dangle {
                    DANGLE_PATIENCE
                } else if short {
                    SHORT_PATIENCE
                } else {
                    PATIENCE
                };
                let wait = if v.acked { wait + ACK_EXTRA } else { wait };
                (now >= v.speech_end + wait).then(|| label(v, "patience"))
            }
            _ if dangle => (now >= last + DANGLE_PATIENCE).then(|| "trailed off".into()),
            _ => match t.eou {
                Some(eou) => (now >= eou + HOLD).then(|| "eou".into()),
                None => (now >= last + MAX_WAIT).then(|| "timeout".into()),
            },
        }
    }

    fn on_voice(&mut self, ev: TurnEvent) {
        match ev {
            TurnEvent::Start { seq } => {
                self.settings.listening.store(true, Relaxed);
                if self.settings.tools.as_ref().is_some_and(|tools| tools.hands()) {
                    tokio::spawn(computer::notice());
                }
                self.voice_seq = seq;
                self.voice_start = Some(Instant::now());
                self.verdict = None;
                if let Some(h) = self.heard.as_mut() {
                    h.refined = false;
                }
            }
            TurnEvent::End { seq, prob, complete, refining, voiced_ms, speech_end, at } if seq == self.voice_seq => {
                log::debug!("voice end: complete {complete:?} prob {prob:?} voiced {voiced_ms} ms");
                self.settings.listening.store(false, Relaxed);
                self.voice_start = None;
                let refining = refining && self.refined_seq != Some(seq);
                self.verdict = Some(Verdict { complete, prob, speech_end, at, refining, seq, voiced_ms, acked: false });
                self.maybe_wordless();
            }
            TurnEvent::End { .. } => self.settings.listening.store(false, Relaxed),
            TurnEvent::Refined { seq, text } => {
                self.refined_seq = Some(seq);
                if seq != self.voice_seq {
                    return;
                }
                if let Some(v) = &mut self.verdict {
                    v.refining = false;
                }
                match text.map(|t| self.unechoed(t)).filter(|t| !content_words(t).is_empty()) {
                    Some(text) => self.refine(text),
                    None => {
                        self.silent_seq = Some(seq);
                        self.maybe_wordless();
                    }
                }
            }
        }
    }

    fn refine(&mut self, text: String) {
        log::debug!("refined: {text}");
        if self.turn.as_ref().is_some_and(|t| t.committed) {
            return;
        }
        let words = content_words(&text);
        if words.is_empty() {
            return;
        }
        if let Some(h) = self.heard.as_mut()
            && h.words == words
        {
            h.refined = true;
            h.text = text;
            return;
        }
        self.cancel();
        self.draw_ghost(&text);
        let (at, speech_end_ms) = self.heard.as_ref().map_or((Instant::now(), None), |h| (h.at, h.speech_end_ms));
        self.heard = Some(Heard { text, words, at, speech_end_ms, refined: true });
    }

    fn current(&mut self, id: u64) -> Option<&mut Turn> {
        self.turn.as_mut().filter(|t| t.id == id)
    }

    fn handle(&mut self, ev: Event) {
        match ev {
            Event::Speech(SpeechEvent::Partial { text, speech_end_ms }, at) => self.on_partial(text, speech_end_ms, at),
            Event::Speech(SpeechEvent::Eou { text, speech_end_ms }, at) => self.on_eou(text, speech_end_ms, at),
            Event::Voice(ev) => self.on_voice(ev),
            Event::LlmFirst { turn, at } => {
                if let Some(t) = self.current(turn) {
                    t.llm_first = Some(at);
                }
            }
            Event::Chunk { turn, text } => self.on_chunk(turn, text),
            Event::Mood { turn, text } => {
                if let Some(t) = self.current(turn) {
                    t.mood = Some(text);
                }
            }
            Event::Opener { turn, key, text } => self.on_opener(turn, key, text),
            Event::LlmDone { turn, text, calls, error } => self.on_llm_done(turn, text, calls, error),
            Event::TtsFirst { turn, at } => {
                if let Some(t) = self.current(turn) {
                    t.tts_first.get_or_insert(at);
                }
            }
            Event::TtsEnd { turn } => {
                if let Some(t) = self.turn.as_mut().filter(|t| t.id == turn) {
                    t.tts_done = true;
                    self.settings.talking.store(false, Relaxed);
                }
            }
            Event::Did { turn, id, text } => {
                let Some(t) = self.turn.as_mut().filter(|t| t.id == turn) else { return };
                t.owed.retain(|owed| *owed != id);
                self.dim_line(&format!("-> {}", crate::tools::brief(&text)));
                self.history.push(Message::tool(&id, text));
            }
            Event::Acted { turn } => self.on_acted(turn),
            Event::Halt => self.halt(),
            Event::Aside(text) => self.asides.push_back(text),
            Event::Trimmed { cut, system } => {
                self.trimming = false;
                if 1 + cut < self.history.len() {
                    self.history.drain(1..1 + cut);
                    self.history[0].content = system;
                }
            }
            Event::Fatal(_) | Event::Quit => {}
        }
    }

    fn on_partial(&mut self, text: String, speech_end_ms: Option<u64>, at: Instant) {
        log::debug!("partial: {text}");
        self.check_end();
        let raw = content_words(&text);
        let text = self.unechoed(text);
        let words = content_words(&text);
        self.stream_words = raw.clone();
        match &self.turn {
            Some(t) if t.committed => {
                let fresh = fresh_words(&raw, &t.base);
                let fresh = &fresh[self.echoed(fresh)..];
                let few = fresh.len() < BARGE_WORDS;
                if fresh.is_empty() || is_backchannel(fresh) || (few && at < t.decided + LATE_WORDS) {
                    return;
                }
                if !few {
                    self.barge();
                }
            }
            _ if self.heard.as_ref().is_some_and(|h| h.refined) => return,
            Some(t) if t.words != words => {
                log::debug!("user kept talking, dropping turn {}", t.id);
                self.cancel();
            }
            _ => {}
        }
        if words.is_empty() && !raw.is_empty() {
            return;
        }
        if self.turn.as_ref().is_none_or(|t| !t.committed) {
            self.draw_ghost(&text);
        }
        self.heard = Some(Heard { text, words, at, speech_end_ms, refined: false });
    }

    fn on_eou(&mut self, text: String, speech_end_ms: Option<u64>, at: Instant) {
        log::debug!("eou: {text} (speech end {speech_end_ms:?} ms)");
        self.check_end();
        let raw = content_words(&text);
        let text = self.unechoed(text);
        let words = content_words(&text);
        let speech_end = speech_end_ms.and_then(|ms| self.clock.wall(ms));
        if words.is_empty() {
            let pending = self.turn.as_ref().is_some_and(|t| !t.committed);
            self.speech.rearm(pending);
            if self.turn.is_none() {
                self.heard = None;
                self.clear_ghost();
                flush();
            }
            return;
        }
        let refined = self.heard.as_ref().is_some_and(|h| h.refined);
        let echoed = self.turn.as_ref().filter(|t| t.committed).map_or(0, |t| self.echoed(fresh_words(&raw, &t.base)));
        match self.turn.as_mut() {
            Some(t) if t.committed => {
                let fresh = fresh_words(&raw, &t.base);
                let fresh = &fresh[echoed..];
                let few = fresh.len() < BARGE_WORDS;
                if fresh.is_empty() || is_backchannel(fresh) || (few && at < t.decided + LATE_WORDS) {
                    self.speech.rearm(false);
                    return;
                }
                self.speech.rearm(true);
                self.barge();
            }
            Some(t) if refined || t.words == words => {
                self.speech.rearm(true);
                t.eou.get_or_insert(at);
                t.speech_end = speech_end.or(t.speech_end);
                return;
            }
            Some(_) => {
                self.speech.rearm(true);
                self.cancel();
            }
            None => {
                self.speech.rearm(true);
                if refined {
                    return;
                }
            }
        }
        self.start(text, words, speech_end, Some(at));
    }

    fn start(&mut self, text: String, words: Vec<String>, speech_end: Option<Instant>, eou: Option<Instant>) {
        self.next_id += 1;
        let id = self.next_id;
        let stop = Arc::new(AtomicBool::new(false));
        self.gate.open(id);
        let brief = self.heard.as_ref().is_some_and(|h| h.refined && h.text == text) && dangling(&text);
        let text = match &self.carry {
            Some(c) => format!("{c} {text}"),
            _ => text,
        };
        self.settings.talking.store(true, Relaxed);
        let waiting = notch::waiting();
        let mut stamp = local_time();
        if !waiting.is_empty() {
            stamp.push_str(&format!("; {WAITING} {}", waiting.join(" ")));
        }
        if computer::running() {
            stamp.push_str(&format!("; {WORKING}"));
        }
        if let Some(seen) = computer::noticed().filter(|_| !words.is_empty()) {
            stamp.push_str(&format!("; {seen}"));
        }
        let prompt = format!("[{stamp}] {text}{}", if brief { " ..." } else { "" });
        let said = self.last_react.and_then(|(at, key)| key.filter(|_| at.elapsed() < REACT_MEMORY));
        let mut messages = self.history.clone();
        messages.push(Message::user(prompt.clone()));
        let llm = tokio::spawn(generate(
            self.llm.clone(),
            computer::compose(&messages),
            id,
            stop.clone(),
            self.jobs.clone(),
            self.events.clone(),
            self.settings.sounds,
            self.asked_lately(),
            self.settings.bank.clone(),
            said,
            self.last_opener,
            brief,
            self.settings.schema.clone(),
        ));
        let now = Instant::now();
        self.turn = Some(Turn {
            id,
            text,
            prompt,
            words,
            base: Vec::new(),
            mood: None,
            opener: None,
            opener_played: false,
            aside: false,
            calls: Vec::new(),
            owed: Vec::new(),
            acting: None,
            rounds: 0,
            hush: false,
            epoch: computer::epoch(),
            said: String::new(),
            started: now,
            eou,
            reason: String::new(),
            decided: now,
            speech_end,
            stop,
            llm,
            committed: false,
            llm_done: false,
            tts_done: false,
            full: String::new(),
            held_chunks: Vec::new(),
            line: Line::Unopened,
            llm_first: None,
            tts_first: None,
            onset_before: None,
            onset: None,
            metrics_printed: false,
        });
    }

    fn cancel(&mut self) {
        if let Some(t) = self.turn.take() {
            t.stop.store(true, Relaxed);
            t.llm.abort();
            self.gate.close(t.id);
            self.settings.talking.store(false, Relaxed);
        }
    }

    fn commit(&mut self, reason: String) {
        self.clear_ghost();
        let voice_end = self.verdict().map(|v| v.speech_end);
        self.heard = None;
        self.verdict = None;
        self.voice_reset.reset();
        self.echo.clear();
        self.leaked = false;
        self.faded = None;
        self.carry = None;
        let quiet = self.last_react.is_some_and(|(at, _)| at.elapsed() < OPENER_QUIET);
        let Some(t) = self.turn.as_mut() else { return };
        t.committed = true;
        if quiet {
            t.opener_played = true;
        }
        t.base = self.stream_words.clone();
        t.decided = Instant::now();
        t.speech_end = voice_end.or(t.speech_end);
        t.reason = reason;
        t.onset_before = self.playback.last_onset();
        if let Some(key) = t.opener.filter(|_| !t.opener_played) {
            t.opener_played = true;
            self.last_opener = Some(key);
            self.playback.write(&self.settings.bank.clip(key, self.react_n));
            self.react_n += 1;
        }
        self.gate.commit(t.id);
        self.speech.rearm(false);
        remember(&mut self.history, Message::user(t.prompt.clone()));
        if !t.aside {
            computer::told();
        }
        if t.aside {
            println!("{}", if self.dim { format!("\x1b[2m{}\x1b[0m", t.text) } else { t.text.clone() });
        } else {
            println!("you: {}", t.text);
        }
        if !t.held_chunks.is_empty() {
            self.echo.extend(t.held_chunks.iter().flat_map(|chunk| content_words(chunk)));
            print!("{}{}", milo_label(t.mood.as_deref(), self.dim), t.held_chunks.join(" "));
            t.held_chunks.clear();
            t.line = Line::Open;
        }
        if t.llm_done {
            close_line(t);
            if !t.calls.is_empty() {
                t.line = Line::Unopened;
            }
        }
        flush();
        self.proceed();
    }

    fn on_chunk(&mut self, id: u64, text: String) {
        let dim = self.dim;
        let Some(t) = self.current(id) else { return };
        if !t.committed {
            t.held_chunks.push(text);
            return;
        }
        match t.line {
            Line::Unopened => print!("{}{text}", milo_label(t.mood.as_deref(), dim)),
            Line::Open => print!(" {text}"),
            Line::Closed => return,
        }
        t.line = Line::Open;
        flush();
        self.echo.extend(content_words(&text));
    }

    fn on_opener(&mut self, id: u64, key: usize, text: String) {
        let Some(t) = self.turn.as_mut().filter(|t| t.id == id) else { return };
        t.opener = Some(key);
        if t.committed && !t.opener_played {
            t.opener_played = true;
            self.last_opener = Some(key);
            self.playback.write(&self.settings.bank.clip(key, self.react_n));
            self.react_n += 1;
        }
        self.on_chunk(id, text);
    }

    fn on_llm_done(&mut self, id: u64, text: String, calls: Vec<Call>, error: Option<String>) {
        if let Some(e) = &error {
            log::warn!("llm: {e}");
        }
        let Some(t) = self.current(id) else { return };
        t.llm_done = true;
        t.full = text;
        t.calls = calls;
        if t.committed {
            close_line(t);
            if !t.calls.is_empty() {
                t.line = Line::Unopened;
            }
            self.print_metrics();
            self.proceed();
        }
    }

    fn poll_onset(&mut self) {
        let Some(t) = self.turn.as_mut() else { return };
        if !t.committed || t.onset.is_some() {
            return;
        }
        let onset = self.playback.last_onset();
        if onset.is_some() && onset != t.onset_before {
            t.onset = onset;
            self.print_metrics();
        }
    }

    fn print_metrics(&mut self) {
        let Some(t) = self.turn.as_mut() else { return };
        let Some(onset) = t.onset else { return };
        if t.metrics_printed || t.line == Line::Open {
            return;
        }
        t.metrics_printed = true;
        let line = metrics(t, onset);
        self.dim_line(&line);
    }

    fn check_end(&mut self) {
        let Some(t) = &self.turn else { return };
        let acting = !t.calls.is_empty() || t.acting.is_some();
        if !(t.committed && t.llm_done && t.tts_done) || acting || self.playback.queued_ms() > 0 {
            return;
        }
        self.poll_onset();
        let Some(t) = self.turn.take() else { return };
        self.gate.close(t.id);
        let gaps = self.playback.gaps();
        if gaps.0 > self.gaps.0 {
            log::debug!("audio ran dry {}x, {}ms in total", gaps.0 - self.gaps.0, gaps.1 - self.gaps.1);
        }
        self.gaps = gaps;
        self.bill();
        if t.epoch == computer::epoch()
            && let Some(tools) = &self.settings.tools
        {
            tools.rest(&t.full, true);
        }
        if !t.full.trim().is_empty() && remember(&mut self.history, Message::assistant(t.full)) {
            self.trim();
        }
    }

    fn interrupt(&mut self) {
        self.poll_onset();
        let Some(mut t) = self.turn.take() else { return };
        t.stop.store(true, Relaxed);
        t.llm.abort();
        if let Some(acting) = t.acting.take() {
            acting.abort();
            tokio::spawn(computer::refresh());
        }
        for id in t.owed.drain(..) {
            self.history.push(Message::tool(&id, UNDONE));
        }
        self.settings.busy.store(false, Relaxed);
        self.settings.talking.store(false, Relaxed);
        let heard = self.gate.interrupt(t.id);
        let heard = match heard.strip_prefix(t.said.as_str()) {
            Some(rest) => rest.trim().to_string(),
            None => String::new(),
        };
        close_line(&mut t);
        if let (Some(onset), false) = (t.onset, t.metrics_printed) {
            self.dim_line(&metrics(&t, onset));
        }
        self.bill();
        if heard.is_empty() {
            self.dim_line("(interrupted)");
        } else {
            let heard = format!("{heard}…");
            self.dim_line(&format!("(interrupted) heard: {heard}"));
            if remember(&mut self.history, Message::assistant(heard)) {
                self.trim();
            }
        }
    }

    fn trim(&mut self) {
        if self.trimming {
            return;
        }
        let mut cut = self.history.len() - 1 - HISTORY_KEEP;
        while 1 + cut < self.history.len() && self.history[1 + cut].role != Role::User {
            cut += 1;
        }
        if 1 + cut >= self.history.len() {
            return;
        }
        let system = match &self.settings.system {
            Some(system) => system.replace("{memory}", &crate::tools::memory_section()),
            None => self.history[0].content.clone(),
        };
        let mut next = vec![Message::system(system.clone())];
        next.extend_from_slice(&self.history[1 + cut..]);
        self.trimming = true;
        let (llm, events, schema) = (self.llm.clone(), self.events.clone(), self.settings.schema.clone());
        tokio::spawn(async move {
            if let Err(e) = llm.warm(&next, schema.as_deref()).await {
                log::debug!("cache warm failed: {e:#}");
            }
            let _ = events.send(Event::Trimmed { cut, system });
        });
    }

    fn leadin(&self, t: &Turn) -> bool {
        !t.words.is_empty()
            && (is_backchannel(&t.words) || t.words.iter().all(|w| w == WORDLESS))
            && self.history.iter().any(|m| m.role == Role::Assistant)
            && !self.asked_last()
    }

    fn asked_lately(&self) -> bool {
        self.history
            .iter()
            .rev()
            .filter(|m| m.role == Role::Assistant)
            .take(3)
            .any(|m| m.content.trim_end().ends_with('?'))
    }

    fn asked_last(&self) -> bool {
        self.history
            .iter()
            .rev()
            .find(|m| m.role == Role::Assistant)
            .is_some_and(|m| m.content.trim_end().ends_with('?'))
    }

    fn shutdown(&mut self) {
        if let Some(mut t) = self.turn.take() {
            t.stop.store(true, Relaxed);
            t.llm.abort();
            if let Some(acting) = t.acting.take() {
                acting.abort();
            }
            self.gate.interrupt(t.id);
            close_line(&mut t);
        }
    }

    fn draw_ghost(&self, text: &str) {
        if !self.dim {
            return;
        }
        let count = text.chars().count();
        let tail: String = text.chars().skip(count.saturating_sub(GHOST_CHARS)).collect();
        let lead = if count > GHOST_CHARS { "…" } else { "" };
        print!("\r\x1b[2K\x1b[2m{lead}{tail}\x1b[0m");
        self.ghost.set(true);
        flush();
    }

    fn clear_ghost(&self) {
        if self.ghost.replace(false) {
            print!("\r\x1b[2K");
        }
    }

    fn dim_line(&self, text: &str) {
        self.clear_ghost();
        if self.dim {
            println!("\x1b[2m{text}\x1b[0m");
        } else {
            println!("{text}");
        }
    }
}

fn early(t: &Turn, at: Instant) -> bool {
    t.onset.is_none_or(|o| at < o + EARLY)
}

fn trailing(text: &str) -> bool {
    let t = text.trim().to_lowercase();
    let t = t.trim_end_matches(|c: char| !c.is_alphanumeric());
    SEARCHING.iter().any(|p| t.ends_with(p))
}

fn closed(text: &str) -> bool {
    let t = text.trim_end();
    t.ends_with(['.', '!', '?']) && !t.ends_with("...")
}

fn dangling(text: &str) -> bool {
    let t = text.trim().to_lowercase();
    if t.ends_with(['.', '!', '?']) && !t.ends_with("...") {
        return false;
    }
    if trailing(&t) {
        return false;
    }
    let t = t.trim_end_matches(|c: char| !c.is_alphanumeric());
    t.rsplit(|c: char| !c.is_alphanumeric() && c != '\'' && c != '’')
        .next()
        .is_some_and(|w| DANGLERS.contains(&w.replace('’', "'").as_str()))
}

fn milo_label(mood: Option<&str>, dim: bool) -> String {
    match mood {
        Some(m) if dim => format!("milo \x1b[2m[{m}]\x1b[0m: "),
        Some(m) => format!("milo [{m}]: "),
        None => "milo: ".into(),
    }
}

fn fresh_words<'a>(words: &'a [String], base: &[String]) -> &'a [String] {
    if words.starts_with(base) { &words[base.len()..] } else { words }
}

fn overlap(heard: &[String], said: &[String]) -> usize {
    let mut back = vec![0usize; said.len() + 1];
    let mut row = back.clone();
    let mut best = 0;
    for (i, word) in heard.iter().enumerate() {
        let mut next = vec![i + 1; said.len() + 1];
        let mut ends = None;
        for (j, own) in said.iter().enumerate() {
            let same = word == own;
            next[j + 1] = (row[j] + usize::from(!same)).min(row[j + 1] + 1).min(next[j] + 1);
            if same && i > 0 && j > 0 && heard[i - 1] == said[j - 1] {
                ends = Some(ends.map_or(back[j - 1], |cost: usize| cost.min(back[j - 1])));
            }
        }
        if i + 1 >= ECHO_WORDS && ends.is_some_and(|cost| cost * 4 <= i + 1) {
            best = i + 1;
        }
        back = std::mem::replace(&mut row, next);
    }
    best
}

fn after(text: &str, count: usize) -> &str {
    let mut rest = text.trim_start();
    let mut left = count;
    while left > 0 && !rest.is_empty() {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        if !content_words(&rest[..end]).is_empty() {
            left -= 1;
        }
        rest = rest[end..].trim_start();
    }
    rest
}

fn is_backchannel(words: &[String]) -> bool {
    words.len() <= 3 && words.iter().all(|w| BACKCHANNELS.contains(&w.as_str()))
}

fn unmarked(text: &str) -> &str {
    let text = text.trim();
    match text.strip_prefix('[').and_then(|rest| rest.split_once(']')) {
        Some((_, rest)) => rest.trim(),
        None => text,
    }
}

async fn act(
    tools: Arc<Tools>,
    calls: Vec<Call>,
    turn: u64,
    events: UnboundedSender<Event>,
    listening: Arc<AtomicBool>,
) {
    computer::round();
    for call in calls {
        let waited = Instant::now();
        let mut quiet = Instant::now();
        let mut heard = false;
        while waited.elapsed() < HELD && (listening.load(Relaxed) || (heard && quiet.elapsed() < GRACE)) {
            if listening.load(Relaxed) {
                heard = true;
                quiet = Instant::now();
            }
            tokio::time::sleep(HELD_POLL).await;
        }
        let text = tools.run(&call).await;
        let _ = events.send(Event::Did { turn, id: call.id, text });
    }
    let _ = events.send(Event::Acted { turn });
}

fn local_time() -> String {
    let now = jiff::Zoned::now();
    let zone = now.time_zone().iana_name().unwrap_or("local time").to_string();
    format!("{}, {zone}", now.strftime("%A, %B %-d, %Y, %-I:%M %p %Z"))
}

fn content_words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
        .filter(|w| !w.is_empty() && !FILLERS.contains(&w.as_str()))
        .collect()
}

fn remember(history: &mut Vec<Message>, message: Message) -> bool {
    let reply = message.role == Role::Assistant;
    history.push(message);
    reply && history.len() - 1 > HISTORY_MAX
}

fn close_line(t: &mut Turn) {
    if t.line == Line::Open {
        println!();
        flush();
    }
    t.line = Line::Closed;
}

fn flush() {
    let _ = std::io::stdout().flush();
}

fn metrics(t: &Turn, onset: Instant) -> String {
    let opt = |v: Option<i64>, sign: bool| match v {
        Some(ms) if sign => format!("{ms:+}ms"),
        Some(ms) => format!("{ms}ms"),
        None => "?".into(),
    };
    let end = t.reason.as_str();
    let cause = end.split(' ').next().unwrap_or(end);
    let wait = t.speech_end.map(|s| ms_between(s, t.decided));
    let ttft = t.llm_first.map(|f| ms_between(t.started, f));
    let tts = t.tts_first.map(|f| ms_between(t.decided, f));
    let audio = ms_between(t.decided, onset);
    let v2v = t.speech_end.map(|s| ms_between(s, onset));
    format!(
        "{end} {} | prepared {:+}ms | llm ttft {} | tts first {} | audio {audio:+}ms after {cause} | v2v {}",
        opt(wait, true),
        ms_between(t.decided, t.started),
        opt(ttft, false),
        opt(tts, true),
        opt(v2v, false)
    )
}

fn ms_between(from: Instant, to: Instant) -> i64 {
    if to >= from { (to - from).as_millis() as i64 } else { -((from - to).as_millis() as i64) }
}
