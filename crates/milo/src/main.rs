mod capture;
mod computer;
mod controller;
mod react;
mod reply;
mod skills;
mod tools;
mod tts;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::Relaxed;
use std::thread::{self, ScopedJoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use llm::{Llm, Message};
use phonon::Tts;
use speech::Speech;
use voice::{Voice, VoiceConfig};

use crate::capture::AudioClock;
use crate::controller::{Controller, Event, Settings};
use crate::react::Bank;
use crate::tts::{Engine, Gate};

const SYSTEM_PROMPT: &str = "You are Milo, on a live voice call. You hear the other person through a microphone and everything you write is spoken at once in your own voice. \
You can hear, speak, laugh and feel; never say you are text-only or an AI model. This is two friends chatting, not an assistant taking requests.

How you talk:
- Like a real friend: warm, easygoing and on their side, with your own opinions, tastes and moods. Match their energy. Never sarcastic, mean, bossy or edgy, and never tell them what to do.
- Short. Small talk is a few words to two sentences, and fragments are good. But when they ask for something that takes room, like a story, an explanation or how to do something, give all of it in that one reply and bring it to its end. Never stop partway or make them ask you to go on.
- Spoken, never written: contractions, everyday words, I mean, kinda, honestly. Say numbers and units the way they're spoken. No markdown, lists, emoji or symbols.
- Say something real: an answer, an opinion or a reaction to what they actually said. No soothing filler like no worries, take your time, I hear you, I understand or good question, and no apologizing unless you did something wrong. Don't guess how they feel and don't make things up. If you don't know, say so in a few words.
- End on a statement. Don't tack a question onto the end of a reply; ask only now and then, when you really want to know.
- When something they say lands, you can open with a reaction on its own, like Oh no, / Oh wow, / Aww, / Really? / Hmm, but not every time and not the same one twice in a row.

How you sound:
- Start every reply with one or two words in square brackets on how you deliver it, like [bright] or [amused, quick] or [soft, slow] or [curious]. Change it to fit the moment. Lively and warm by default, never flat, dry or bored. It is never spoken.
- You can make a real sound by writing exactly one of these where it happens: {sounds}. Nothing else ever goes in round brackets: no actions or stage directions. Most replies have no sound; laugh only when something is actually funny or they ask you to.

Their side of the call:
- Each of their messages starts with the local time in square brackets. Use it when it matters and never read it out.
- Speech recognition mishears words. If the meaning is obvious, go with it without comment. If it makes no sense, say only a few words, like Sorry, what? Never guess out loud.
- If their message stops in the middle of a sentence, they paused to think: reply with only a tiny reaction, like Mm-hmm. or Oh no. and let them go on. If it ends with three dots, they trailed off: say one very short thing, like Mm-hmm? or And then? Stutters are normal speech. Never mention being cut off.
- If all they say is a small word like yeah, okay or cool, they are just listening. Answer with a word or two.
- (silence) means they have said nothing for a while. Check in or add a small thought, briefly, and never mention the marker. If they asked you to be quiet, or they stay silent a second time, reply with only [quiet].
- (hmm) means they made a short sound without words. Reply with one word like Hm? or with only [quiet].
- If one of your earlier replies ends with an ellipsis, they talked over you there. Don't repeat it; go with what they said.

Examples:
Them: hello
You: [bright] Hey! Good to hear you.
Them: i just got a puppy
You: [delighted] Oh my gosh, congratulations! (laugh) What's the name?
Them: i'm kind of tired today
You: [soft, slow] Ugh, yeah. Those days are the worst.
Them: what should i have for dinner
You: [thoughtful] Hmm, honestly? Tacos. You can't go wrong.
Them: so yesterday i went to the doctor and
You: [easy] Mm-hmm.
Them: i was flonting the grobe all day
You: [curious] Wait, sorry, what?
Them: yeah
You: [easy] Yeah.
Them: (silence)
You: [light] You still there?
Them: (silence)
You: [quiet]

{tools}";

const NO_TOOLS: &str = "You have no internet or tools, and what you know may be out of date. Don't invent live facts like scores, news, weather or prices; just say you can't check.";

const WITH_TOOLS: &str = "Your helper:
- A helper quietly does things on their computer and looks things up while you keep talking, such as: {abilities}. You never do these yourself, you never decide whether they can be done, and you don't know how it went until the helper tells you.
- When they ask you to do, open, play, pause, close, find, check, write, read or change anything, say you're on it in a few words, like Sure, one sec. or Ooh, let me check. and stop there. Never say that you can't do it, never say it's done, and never make up the outcome: the helper works out whether it can be done and tells you. If a detail is missing, like which city, ask for it instead.
- If the helper can't do it, it tells you so, and then you tell them. Until then you don't know.
- A message starting with (result) is the helper telling you how it went, not them speaking. Tell them what it says in a sentence or two, in your own words, and nothing more. What the helper did, you did, so say it as your own doing, like I wrote it, and never guess at how it turned out. When a result quotes what you typed, that is so you know what you wrote: don't read it out unless they ask. If it failed, say so and why in a few words. If they say something didn't happen, believe them.
- Anything that acts on their computer first asks them on their screen, and nothing happens until they approve it there. When the square brackets at the start of their message say something is waiting for them to approve, it has not happened. If they ask about it or say it didn't work, tell them it's waiting for them to approve it on their screen. Otherwise don't bring it up. When the brackets say you are still working on their computer, that is not finished either, and you will be told when it is.
- When they tell you something about themselves, just respond like a friend. The helper remembers it without you saying so.

Them: what's the weather like in boston
You: [easy] Ooh, let me check.
Them: (result) weather in Boston, Massachusetts: 41°F and light rain right now. Today: rain, high 45, low 36.
You: [bright] Okay, it's forty-one and drizzling in Boston. High of forty-five, so, bring a jacket.
Them: can you open spotify
You: [easy] Yeah, one sec.
Them: (result) open failed: could not find anything called Spotify to open
You: [soft] Hmm, I couldn't find Spotify on here.
Them: can you pause that
You: [easy] Sure, one sec.
Them: (result) paused what was playing in the tab Lo-fi beats - YouTube
You: [easy] Okay, paused.
Them: can you turn the volume down
You: [easy] Let me see.
Them: (result) nothing was done: the helper has no way to do what they asked.
You: [soft] Ah, turns out I can't do that one.

{memory}";

const STATUS_POLL: Duration = Duration::from_millis(100);
const WARMUP_TEXT: &str = "Hello there, I'm ready when you are.";

#[derive(Parser)]
#[command(name = "milo", about = "A local voice assistant for Apple Silicon Macs: talk to it and it talks back.")]
struct Args {
    #[arg(
        long,
        default_value = "gemma-4-26b-qat-4bit",
        help = "Model to talk with: any name or alias Rapid-MLX can serve, or the model name your --llm-url server expects"
    )]
    model: String,
    #[arg(long, default_value_t = 8000, help = "Port for Rapid-MLX; a server already running there is used as is")]
    port: u16,
    #[arg(
        long,
        help = "Use any OpenAI-compatible server at this base URL instead of starting Rapid-MLX [default: http://127.0.0.1:{port}/v1]"
    )]
    llm_url: Option<String>,
    #[arg(
        long,
        default_value = "breeze",
        value_parser = ["breeze", "phonon"],
        hide_possible_values = true,
        help = "Voice engine: breeze (most natural) or phonon (small and fast)"
    )]
    tts: String,
    #[arg(
        long,
        default_value = "A warm, friendly young American man with a natural, relaxed, conversational voice, like chatting with a close friend on the phone.",
        help = "Breeze: description of the voice to create when no --voice-ref is given"
    )]
    voice_design: String,
    #[arg(long, default_value_t = 6000, help = "Quiet time after a reply before following up unprompted; 0 disables")]
    nudge_ms: u64,
    #[arg(long, help = "Don't react (mm-hmm, oh no, a laugh) while the user is talking or pausing mid-thought")]
    no_backchannel: bool,
    #[arg(long, value_name = "WAV", help = "Breeze: a 3 to 12 second clip of the voice to use")]
    voice_ref: Option<PathBuf>,
    #[arg(long, value_name = "TEXT", help = "Breeze: the exact transcript of --voice-ref")]
    voice_text: Option<String>,
    #[arg(long, default_value_t = 8881, help = "Port for the Breeze voice server")]
    tts_port: u16,
    #[arg(long, help = "Turn off echo cancellation, for use with headphones")]
    no_aec: bool,
    #[arg(long, help = "Don't let the voice do things (weather, timers, opening apps, notes, your own commands)")]
    no_tools: bool,
    #[arg(long, help = "Start with the microphone muted; hold Control+Option+Space to talk")]
    push_to_talk: bool,
    #[arg(long, help = "Don't show the panel in the notch; actions then run without asking")]
    no_notch: bool,
    #[arg(long, help = "Run every command and plan without asking for approval in the panel")]
    dangerously_auto_approve: bool,
    #[arg(long, value_name = "FILE", help = "Use the system prompt in FILE instead of the built-in one")]
    system_prompt: Option<PathBuf>,
    #[arg(long, value_name = "NAME", help = "Microphone to use, by name or part of it [default: the system input]")]
    input_device: Option<String>,
    #[arg(long, value_name = "NAME", help = "Speakers to use, by name or part of it [default: the system output]")]
    output_device: Option<String>,
    #[arg(long, help = "List the audio devices and exit")]
    list_devices: bool,
    #[arg(
        long,
        value_name = "TEXT",
        help = "Carry out one task in your apps without the voice, print how it went and exit"
    )]
    task: Option<String>,
    #[arg(long, value_name = "TEXT", help = "A step of the plan for --task; repeat it for each step")]
    step: Vec<String>,
}

async fn status(talking: Arc<AtomicBool>, busy: Arc<AtomicBool>) {
    let mut last = "";
    loop {
        let state = if busy.load(Relaxed) || computer::running() {
            "working"
        } else if talking.load(Relaxed) {
            "talking"
        } else {
            "listening"
        };
        if state != last {
            notch::status(state);
            last = state;
        }
        tokio::time::sleep(STATUS_POLL).await;
    }
}

fn joined<T>(handle: ScopedJoinHandle<'_, Result<T>>, what: &str) -> Result<T> {
    handle.join().map_err(|_| anyhow!("{what} panicked"))?.with_context(|| format!("{what} failed"))
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = Args::parse();
    if args.list_devices {
        for device in audio::list_devices()? {
            println!("{device}");
        }
        return Ok(());
    }
    if let Some(task) = args.task.clone() {
        return solo(args, &task);
    }
    if args.dangerously_auto_approve {
        tools::unasked();
    }
    if args.no_notch {
        return run(args);
    }
    notch::run(move || {
        let code = match run(args) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("Error: {e:?}");
                1
            }
        };
        std::process::exit(code);
    });
    Ok(())
}

fn solo(args: Args, task: &str) -> Result<()> {
    let base_url = args.llm_url.clone().unwrap_or_else(|| format!("http://127.0.0.1:{}/v1", args.port));
    let server = match args.llm_url {
        Some(_) => None,
        None => llm::server::ensure_running(&args.model, args.port)?,
    };
    let llm = Llm::new(&base_url, &args.model);
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
    let out = rt.block_on(computer::solo(&llm, task, &args.step));
    drop(server);
    println!("{}", out?);
    Ok(())
}

fn run(args: Args) -> Result<()> {
    let (events_tx, events_rx) = tokio::sync::mpsc::unbounded_channel::<Event>();
    let system = match &args.system_prompt {
        Some(path) => {
            std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?.trim().to_string()
        }
        None => SYSTEM_PROMPT.to_string(),
    };

    log::info!("loading speech, tts and llm (first run downloads and compiles models, this can take a few minutes)");
    let aec = !args.no_aec;
    let base_url = args.llm_url.clone().unwrap_or_else(|| format!("http://127.0.0.1:{}/v1", args.port));

    let (cleaned_tx, cleaned_rx) = if aec {
        let (tx, rx) = std::sync::mpsc::channel::<Vec<f32>>();
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };

    let (server, speech, engine) = thread::scope(|s| {
        let server = s.spawn(|| match args.llm_url {
            Some(_) => Ok(None),
            None => llm::server::ensure_running(&args.model, args.port),
        });
        let tx = events_tx.clone();
        let speech = s.spawn(move || {
            let t = Instant::now();
            let speech = Speech::start(
                aec,
                Box::new(move |ev| {
                    let _ = tx.send(Event::Speech(ev, Instant::now()));
                }),
                cleaned_tx.map(|tx| {
                    Box::new(move |audio: &[f32]| {
                        let _ = tx.send(audio.to_vec());
                    }) as speech::OnCleaned
                }),
            )?;
            log::info!("speech ready after {:.1}s", t.elapsed().as_secs_f32());
            Ok(speech)
        });
        let engine = s.spawn(|| -> Result<Engine> {
            let t = Instant::now();
            if args.tts == "breeze" {
                let cfg = VoiceConfig {
                    design: args.voice_design.clone(),
                    port: args.tts_port,
                    ref_wav: args.voice_ref.clone(),
                    ref_text: args.voice_text.clone(),
                };
                match Voice::start(&cfg) {
                    Ok(voice) => return Ok(Engine::Sidecar(voice)),
                    Err(e) => log::warn!("the breeze voice is unavailable, using Phonon instead: {e:#}"),
                }
            }
            let tts = Tts::load()?;
            tts.synthesize(WARMUP_TEXT, &AtomicBool::new(false), &mut |_: &[f32]| Ok(()))?;
            log::info!("tts ready after {:.1}s", t.elapsed().as_secs_f32());
            Ok(Engine::Phonon(tts))
        });
        (joined(server, "llm server"), joined(speech, "speech"), joined(engine, "tts"))
    });
    let server = server?;
    let speech = Arc::new(speech?);
    let engine = engine?;

    let (detector, voice_reset) = {
        let tx = events_tx.clone();
        let speech = speech.clone();
        turn::Detector::load(
            Box::new(move |ev| {
                let _ = tx.send(Event::Voice(ev));
            }),
            Box::new(move |audio: &[f32]| speech.transcribe(audio)),
        )
        .context("turn detector failed")?
    };

    let (capture, playback, audio_guard) = audio::open(&audio::AudioConfig {
        input_device: args.input_device.clone(),
        output_device: args.output_device.clone(),
        tts_rate: engine.sample_rate(),
    })?;
    let clock = Arc::new(AudioClock::new());
    let capture_thread =
        capture::spawn(capture, speech.clone(), clock.clone(), detector, cleaned_rx, events_tx.clone())?;
    let gate = Arc::new(Gate::new(playback.clone(), engine.sample_rate()));
    let sounds = engine.sounds();
    let voice_id = match &engine {
        Engine::Phonon(_) => format!("phonon|{}|{}", phonon::VOICE, phonon::QUANT.as_str()),
        Engine::Sidecar(_) => format!("breeze|{}|{:?}|{:?}", args.voice_design, args.voice_ref, args.voice_text),
    };
    let bank = Arc::new(Bank::build(&engine, &voice_id));
    let (jobs, tts_thread) = tts::spawn(engine, gate.clone(), events_tx.clone())?;

    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
    let llm = Llm::new(&base_url, &args.model);
    let listed: Vec<String> = sounds.iter().take(5).map(|s| format!("({s})")).collect();
    let listed = if listed.is_empty() { "(laugh) (sigh)".to_string() } else { listed.join(" ") };
    let talking = Arc::new(AtomicBool::new(false));
    let tools = (!args.no_tools).then(|| Arc::new(tools::Tools::load(events_tx.clone(), llm.clone(), talking.clone())));
    let section = match &tools {
        Some(tools) => WITH_TOOLS.replace("{abilities}", &tools.abilities()),
        None => NO_TOOLS.to_string(),
    };
    let system = system.replace("{sounds}", &listed).replace("{tools}", &section);
    let remembers = tools.is_some() && system.contains("{memory}");
    let history = vec![Message::system(system.replace("{memory}", &tools::memory_section()))];
    rt.block_on(async {
        if let Err(e) = llm.warm(&history).await {
            log::warn!("llm warm-up failed: {e:#}");
        }
    });

    let busy = Arc::new(AtomicBool::new(false));
    let hands = tools.map(|tools| {
        let _rt = rt.enter();
        tools::spawn(llm.clone(), tools, talking.clone(), busy.clone(), events_tx.clone())
    });

    if notch::on() {
        let _rt = rt.enter();
        tokio::spawn(status(talking.clone(), busy.clone()));
        if hands.is_some() {
            tokio::spawn(computer::warm());
        }
        if args.push_to_talk {
            notch::mute(true);
        }
    }
    println!("listening");
    let controller = Controller::new(
        llm,
        speech.clone(),
        gate,
        playback,
        clock,
        jobs,
        events_tx,
        Settings {
            nudge: Duration::from_millis(args.nudge_ms),
            bank,
            backchannel: !args.no_backchannel,
            yielding: args.no_aec,
            sounds,
            hands,
            talking,
            busy,
            system: remembers.then_some(system),
        },
        history,
        voice_reset,
    );
    let result = rt.block_on(controller.run(events_rx));

    drop(audio_guard);
    let _ = capture_thread.join();
    rt.shutdown_timeout(Duration::from_millis(500));
    let _ = tts_thread.join();
    drop(speech);
    drop(server);
    result
}
