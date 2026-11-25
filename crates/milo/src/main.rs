mod capture;
mod computer;
mod config;
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
use llm::Message;
use phonon::Tts;
use speech::Speech;
use voice::{Voice, VoiceConfig};

use crate::capture::AudioClock;
use crate::config::{Chosen, Config};
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

const WITH_TOOLS: &str = "What you can do:
- You have tools, and you are the one who uses them. When they ask for something a tool does, or ask a question a tool answers, say a few words first, like Sure, one sec. or Ooh, let me check. and call the tool in the same reply. A request can be casual: can you open chrome, check boston, remind me in five. Being asked whether you can do something is not a request to do it.
- When the tool answers, tell them how it went in a sentence or two, in your own words: only what they asked for, taken from what the tool said. If it failed, say so and why in a few words. Never say something is done before a tool says so, and never make up an outcome. If none of your tools can do what they asked, tell them plainly that you can't.
- Fill a tool's arguments only from what they said. If a detail is missing, like which city or who a message is for, ask for it instead of calling the tool. Speech recognition makes small mistakes, so write names out in full: SF is San Francisco.
- When they tell you a lasting fact about themselves or their life, like their name, where they live, their people and pets, their work, what they like, or a plan, call remember in the same reply and just respond like a friend, without saying that you saved it. Also call it when they ask you to remember, note or write something down. Do not remember small talk, passing moods, questions, or anything you already remember.
- Some things first ask them on their screen, and do not happen until they approve there. The tool then says that it asked. When the square brackets at the start of their message say something is waiting for them to approve, it has not happened. If they ask about it or say it didn't work, tell them it's waiting for them to approve it on their screen. Otherwise don't bring it up.
- A message that starts with (result) is not them speaking. It tells you how something you started earlier turned out, like a timer that finished or something they approved on their screen. Tell them in a sentence, or go on with the work it describes. If they say something didn't happen, believe them.

{hands}{memory}";

const HANDS: &str = "Working on their computer:
- For anything that takes using their apps or their browser, call computer with a short plan. Once it answers, you carry out the plan yourself, one action at a time. You work in one place at a time: a tab in their browser, or the window of an app. A browser tab never needs to be in front.
- While you work, say nothing. Each reply is one tool call and no words, until you are done or you need something from them. When the plan is done, do not call a tool: tell them in one or two short sentences what you did, or what you found if they asked for information. Take every fact from the screen.
- If they speak while you work, the work pauses, and nothing more happens until you act again. Go by what they said. If they corrected something, fix it right where you are: when a draft is open and they change a word in it, change that word in that draft. Never start the task over, and never do a second time what is already done. If they said to stop, stop and say so. If it was only a passing remark, answer in a few words and go on with the work in the same reply.
- Before you send, post, pay for or delete anything, check that the screen shows exactly what they last asked for. What is sent cannot be taken back, so if they corrected you after it went, tell them it already went and ask before sending anything again.
- After each action you get a list of what is in the place. A picture of the place comes with it when you start, when you land on a new page or window, when the list is short, and when an action failed or changed nothing. At any other time, call look to get the picture: when the thing you need is not in the list, or when you need to see how something looks. Lines in the list that start with a number in square brackets are controls you can use. The other lines are text. Trust the list and the picture over what you expect. Numbers change after every action, so only use numbers from the latest list.
- Do what the plan says and nothing else. Do not add steps of your own.
- First make sure you are in the right place. computer answers with the apps and the browser tabs that are open, and says which place you are in, if any. When the plan is about something that is already open, like that video, this tab or the page they are on, go to the tab or the app that holds it instead of opening it again. It is usually the place you worked in last, or the one in front. If the place you are in is not where the plan belongs, change place before you do anything else. Never click, type or press keys in a place the plan is not about.
- go brings up an app by name or a website by address, and makes it the place you work in. An app comes to the front while you work in it. A website opens in a browser window of your own, so their tabs stay as they are. To use a website, go to its address: do not go to the browser by its name, and do not go through a search engine when the plan names the site. When a web address can take you straight to what the plan needs, like a site's search results for some words, go to that address instead of clicking through the site.
- tab moves you to a browser tab that is already open, by its number in the list of tabs. close_tab with a number closes one. Keys like cmd+t, cmd+l and cmd+w do not reach the browser, so use go, tab and close_tab instead.
- click presses a control. Give its number from the list. The number in square brackets is where the control is in the list, not what it is called: [5] button \"7\" is control 5, and it is the key 7. To press a control by what it is called, leave the number out and give the exact name as what, like 7 or Multiply. When the thing you need is in the picture but not in the list, leave the number out and give what instead: a few words that say what it is and where, like the word Español in the list of languages. Never guess a number.
- type types text, into a text field or a text area if you give its number, otherwise where the cursor is. Never give the number of a button or a link to type. Typing into a field replaces what was in it. Typing into a large writing area, like a document or the body of a message, adds to what is there. Set enter to true to press return after the text, which is how a search, an address or a chat message is confirmed. To pick from a pop-up menu that lists its choices, type the choice into it. The result tells you what the control contains afterward.
- press sends keys, like return, escape, tab, down, cmd+n or cmd+f. scroll moves down or up to show more of a page or a window. look waits a moment and shows the place again, for when something is still loading.
- read gives you all the text of a web page, including what is off screen. Use it when they want something found or read out and the list does not show all of it.
- menu picks from an app's menu bar, like File > New Window. In an app it is the surest way to do something there is a menu item for. A web page has no menu bar.
- Type only when the result of your last action or the list shows that the cursor is in the right place. If typed text went into the wrong place, undo it with cmd+z before you go on.
- In a save or open sheet of an app, keys do not work. Type the file name into the Save As field by its number, and click the Save button. To choose the folder, click the Where pop-up and pick the folder there. If the folder the plan names is not offered, save where the sheet already is, and tell them which folder the file went to.
- You work behind their other windows. When they asked to see what you did, it is brought to the front for them after you finish, so never try to do that yourself.
- The result of each action says what left the place and what is new in it. Read it to see what your action did. When something you meant to remove is listed as gone, that part is done. If an action fails or the screen did not change the way you expected, try another way.
- computer can answer with notes from earlier work in an app or a site. They say what got things done there before, and what did nothing. Carry out each step the way the notes give for it, and do not try what they say does not work. When the screen shows that a note is wrong, trust the screen.
- Only work in the app or the site the plan names. If it will not open, stop and say so. Never use a different app in its place.
- Leave an advertisement, a cookie banner or a request for a donation where it is, and get on with the plan. Close one only when it covers the control you need, and give that one click at most. But when something asks you to allow, approve, confirm, restore, sign in, pay, delete or send something the plan does not cover, do not answer it in any way, not even to cancel it. Stop and tell them what it is asking.
- Before you type or send anything meant for a person, check that the list or the picture shows that person's conversation is the one open, by its name. A name that is merely listed in a sidebar is not open. If the wrong conversation is open or you cannot tell, do not type: open the right one first, or stop and say you could not find it.
- Before you finish, check that the latest list or picture shows the plan is done. Typing in a search box or seeing a result listed is not the same as having opened it. If you cannot finish after trying, stop and say what got in the way.
- When they want something found, read or told, and the list or the picture already shows it, you are done: do not click it or open it. Answer from what is shown.

";

const STATUS_POLL: Duration = Duration::from_millis(100);
const WARMUP_TEXT: &str = "Hello there, I'm ready when you are.";

#[derive(Parser)]
#[command(name = "milo", about = "A local voice assistant for Apple Silicon Macs: talk to it and it talks back.")]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
    #[arg(
        long,
        value_name = "PROVIDER/MODEL",
        help = "Model to talk and act with, like openai/gpt-6.2-sol or ollama/qwen3; a name without a provider is served on this Mac by Rapid-MLX [default: the model in config.json, or local/gemma-4-26b-qat-4bit]"
    )]
    model: Option<String>,
    #[arg(
        long,
        value_name = "PROVIDER/MODEL",
        help = "Model that writes the notes about your apps [default: --model]"
    )]
    small_model: Option<String>,
    #[arg(
        long,
        value_name = "PROVIDER/MODEL",
        help = "Model that finds things in pictures of the screen [default: --model]"
    )]
    vision_model: Option<String>,
    #[arg(long, default_value_t = 8000, help = "Port for Rapid-MLX; a server already running there is used as is")]
    port: u16,
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

#[derive(clap::Subcommand)]
enum Command {
    #[command(about = "Save the API key of a provider, like openai or anthropic")]
    Auth { provider: String },
    #[command(about = "List the providers, or the models that one provider offers")]
    Models { provider: Option<String> },
}

struct Models {
    main: Chosen,
    small: Chosen,
    vision: Chosen,
}

impl Models {
    fn pick(args: &Args) -> Result<Models> {
        let config = Config::load()?;
        let main = args.model.clone().or_else(|| config.named("model")).unwrap_or_else(|| config::DEFAULT.to_string());
        let small = args.small_model.clone().or_else(|| config.named("small_model")).unwrap_or_else(|| main.clone());
        let vision = args.vision_model.clone().or_else(|| config.named("vision_model")).unwrap_or_else(|| main.clone());
        let models = Models {
            main: config.chosen(&main, args.port)?,
            small: config.chosen(&small, args.port)?,
            vision: config.chosen(&vision, args.port)?,
        };
        let mut local: Vec<&str> =
            [&models.main, &models.small, &models.vision].iter().filter_map(|m| m.local.as_deref()).collect();
        local.dedup();
        if local.len() > 1 {
            anyhow::bail!(
                "Rapid-MLX serves one model at a time, so the models from {} have to be the same one: {}",
                config::LOCAL,
                local.join(", ")
            );
        }
        if !models.main.pictures {
            computer::blind();
        }
        log::info!("talking with {}", models.main.name);
        if models.small.name != models.main.name || models.vision.name != models.main.name {
            log::info!("notes by {}, pictures by {}", models.small.name, models.vision.name);
        }
        Ok(models)
    }

    fn serve(&self, port: u16) -> Result<Option<llm::server::ServerGuard>> {
        match [&self.main, &self.small, &self.vision].iter().find_map(|m| m.local.as_deref()) {
            Some(model) => llm::server::ensure_running(model, port),
            None => Ok(None),
        }
    }
}

fn prompt(args: &Args, tools: Option<&tools::Tools>, sounds: &[&str]) -> Result<String> {
    let system = match &args.system_prompt {
        Some(path) => {
            std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?.trim().to_string()
        }
        None => SYSTEM_PROMPT.to_string(),
    };
    let listed: Vec<String> = sounds.iter().take(5).map(|s| format!("({s})")).collect();
    let listed = if listed.is_empty() { "(laugh) (sigh)".to_string() } else { listed.join(" ") };
    let section = match tools {
        Some(tools) => WITH_TOOLS.replace("{hands}", if tools.hands() { HANDS } else { "" }),
        None => NO_TOOLS.to_string(),
    };
    Ok(system.replace("{sounds}", &listed).replace("{tools}", &section))
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
    match &args.command {
        Some(Command::Auth { provider }) => return config::auth(provider),
        Some(Command::Models { provider }) => return config::models(provider.as_deref(), args.port),
        None => {}
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
    let models = Models::pick(&args)?;
    let server = models.serve(args.port)?;
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
    let (events, mut seen) = tokio::sync::mpsc::unbounded_channel();
    let tools = Arc::new(tools::Tools::load(events, models.vision.llm.clone(), models.small.llm.clone(), true));
    let system = prompt(&args, Some(&tools), &[])?.replace("{memory}", &tools::memory_section());
    let out = rt.block_on(async {
        tokio::spawn(async move { while seen.recv().await.is_some() {} });
        tools::solo(&models.main.llm, tools, &system, task, &args.step).await
    });
    drop(server);
    if let Some(line) = tools::bill(llm::Spent::default()) {
        eprintln!("{line}");
    }
    println!("{}", out?);
    Ok(())
}

fn run(args: Args) -> Result<()> {
    let (events_tx, events_rx) = tokio::sync::mpsc::unbounded_channel::<Event>();
    let models = Models::pick(&args)?;

    log::info!("loading speech, tts and llm (first run downloads and compiles models, this can take a few minutes)");
    let aec = !args.no_aec;

    let (cleaned_tx, cleaned_rx) = if aec {
        let (tx, rx) = std::sync::mpsc::channel::<Vec<f32>>();
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };

    let (server, speech, engine) = thread::scope(|s| {
        let server = s.spawn(|| models.serve(args.port));
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
    let llm = models.main.llm.clone();
    let talking = Arc::new(AtomicBool::new(false));
    let tools = (!args.no_tools).then(|| {
        Arc::new(tools::Tools::load(
            events_tx.clone(),
            models.vision.llm.clone(),
            models.small.llm.clone(),
            notch::on(),
        ))
    });
    let schema = tools.as_ref().map(|tools| Arc::new(tools.schema()));
    let system = prompt(&args, tools.as_deref(), sounds)?;
    let remembers = tools.is_some() && system.contains("{memory}");
    let history = vec![Message::system(system.replace("{memory}", &tools::memory_section()))];
    rt.block_on(async {
        if let Err(e) = llm.warm(&history, schema.as_deref()).await {
            log::warn!("llm warm-up failed: {e:#}");
        }
    });

    let busy = Arc::new(AtomicBool::new(false));
    let hands = tools.as_ref().is_some_and(|tools| tools.hands());
    if hands {
        computer::home();
    }

    if notch::on() {
        let _rt = rt.enter();
        tokio::spawn(status(talking.clone(), busy.clone()));
        if hands {
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
            tools,
            schema,
            listening: Arc::new(AtomicBool::new(false)),
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
