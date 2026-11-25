# milo

milo is a voice assistant for your Mac. You talk, and it talks back, like a
phone call. By default, every model runs on your Mac. You can also
[pick a hosted model](#change-the-model). It starts preparing an answer before you finish
speaking, you can talk over it to interrupt, and it makes small listening
sounds, such as "mm-hmm," while you talk. It can also check the weather, set
timers, open apps, use apps for you, and remember what you tell it.

[![A one-minute demo of milo and its panel in the notch](docs/demo.gif)](docs/demo.mp4)

To hear the voices, watch the [demo video with sound](docs/demo.mp4).

## Requirements

- A Mac with Apple silicon, macOS 14 or later, and 32 GB of memory or more
- Xcode 26, or the Xcode Command Line Tools with Swift 6.2 or later
- About 20 GB of free disk space for the models

## Get started

1.  Clone the repository and run the setup script:

    ```sh
    git clone https://github.com/govindpuff/milo
    cd milo
    ./setup.sh
    ```

    The script installs Rust, uv, and the Rapid-MLX model server if they're
    missing, and then builds and installs the `milo` command. You can run it
    again at any time.

2.  Start milo:

    ```sh
    milo
    ```

    The first start downloads about 18 GB of models, which can take several
    minutes. When macOS asks whether your terminal can use the microphone,
    allow access.

3.  When milo prints `listening`, start talking.

If you use headphones, start milo with `milo --no-aec`. This turns off echo
cancellation, and milo stops talking as soon as it hears your voice.

To quit, press Control+C.

## Things to try

- "What's the weather in Chicago?"
- "Set a timer for two minutes."
- "Open Safari."
- "Make a new note that says call the dentist."
- "Text Priya that I'm running late."
- "Pause that video."
- "Close the YouTube tab."
- "Write a hello world Python file on my desktop, and open it in VS Code."
- "Remember that my sister's name is Priya."
- Talk over milo to interrupt it.
- Pause in the middle of a sentence. milo waits for you to go on.

## Approve actions

milo shows a small panel in the notch at the top of your screen. On a Mac
without a notch, the panel sits at the top center of the screen. The bars on
the left move with your voice, and the bars on the right move with milo's. To
see what milo is doing, hold the pointer over the panel.

To mute your microphone, hold the pointer over the panel and click the
microphone button. To unmute, click the button again.

Before milo opens an app or a website, closes a browser tab, writes a file,
sends a text message, or
runs one of your own tools, the panel shows exactly what it is about to do. To let it run, click **Approve**. To stop it, click
**Decline**. If you don't answer within 45 seconds, milo doesn't run the
command.

You can keep talking while a command waits. If you ask for something else, the
panel shows the newest command first, and the older ones wait behind it. If you
correct a plan before you approve it, the new plan replaces the old one.

milo pauses, resumes, and skips a video or music without asking, and it
switches to a browser tab without asking.

The following keyboard shortcuts work in any app:

| Shortcut | Action |
|---|---|
| Control+Option+Y | Approves the command in the panel. |
| Control+Option+N | Declines the command in the panel. |
| Control+Option+M | Mutes or unmutes your microphone. |
| Control+Option+Space | Unmutes your microphone while you hold the keys, and mutes it when you release them. |

To use milo like a walkie-talkie, start it with `milo --push-to-talk`. Your
microphone starts muted, and milo hears you only while you hold
Control+Option+Space.

To keep the panel but skip the approval, start milo with
`milo --dangerously-auto-approve`. milo then opens apps, writes files, sends
messages, and carries out plans as soon as you ask. The panel still shows what
milo is doing, and **Stop** still stops a plan.

To turn off the panel, start milo with `milo --no-notch`. milo then runs
commands without asking, and can't use apps for you.

## Let milo use your apps

milo can click, type, press keys, and choose menu items in the apps on your
Mac. For example, you can ask it to write a note, send a message, or search a
website.

1.  In System Settings, go to **Privacy & Security**, and turn on access for
    your terminal app under both **Accessibility** and **Screen Recording**.
2.  In Chrome, go to `chrome://inspect/#remote-debugging`, and turn on
    **Allow remote debugging for this browser instance**. You do this once.
3.  Start milo again. Chrome comes to the front and asks whether to allow
    remote debugging. Click **Allow**. Chrome asks this each time that milo
    starts.
4.  Ask milo for something that takes a few steps in an app or on a website.
5.  The panel shows the plan as a list of steps. To let milo carry out the
    whole plan, click **Approve**, or press Control+Option+Y. milo doesn't ask
    again for each step.
6.  While milo works, the panel shows the last thing that it did. To stop it,
    click **Stop**, press Control+Option+N, or tell milo to stop.

You can talk while milo works. When you start to speak, milo holds its next
action, and then goes by what you said. For example, if milo is writing an
email and you correct a name, milo changes the name in the same draft. To stop
the work, say so.

When milo finishes, it tells you what it did. milo works on one plan at a time,
and stops a plan after 40 actions.

### What milo learns

After each plan, milo writes short notes about the app or the website that it
worked in: which actions got something done there, and which did nothing. The
next plan in that place starts with those notes, so milo takes fewer steps.
milo rewrites the notes of a place when a plan there had steps that failed or
did nothing.

The notes are in `~/.config/milo/skills`, one file for each app or website. To
change what milo does in a place, edit its file. To make milo learn a place
again, delete its file.

### Quick actions

For some requests, milo doesn't click through a window at all:

- **Video and music.** milo pauses or resumes what is playing in a browser tab
  through the browser connection. For music in an app, it presses the play
  key or a track key.
- **Tabs.** milo closes a tab or switches to it by a word from its title.
- **Files.** milo writes the text of a file itself, under your home folder,
  and can open the file in an app. The panel shows the path and the text
  first. milo doesn't run the file.
- **Text messages.** milo sends a text message with the
  [imsg](https://github.com/steipete/imsg) command, without using the
  Messages window. It finds the person by the name that they are saved under,
  among the people that you have conversations with. The panel shows the
  message and the number or address first.

  To set this up, install imsg with `brew install steipete/tap/imsg`. In
  System Settings, go to **Privacy & Security**, and turn on **Full Disk
  Access** for your terminal app. The first time that milo sends a message,
  macOS asks whether your terminal can control Messages. Click **Allow**.

### Websites

milo reads a web page and uses its buttons, links, and fields through Chrome's
remote debugging connection. This also works in Brave, Edge, Arc, Vivaldi, and
Chromium.

- milo opens websites in a browser window of its own, behind your other
  windows, and leaves your tabs as they are.
- If a tab already has the exact address open, milo works in that tab instead
  of opening the address again.
- milo can also work in a tab that you already have open. For example, you can
  ask it to pause the video that is playing, or to close a tab.
- milo remembers where it worked last. You can ask for one thing, and then
  for the next thing on the same page.
- When you ask to see something, milo brings its tab to the front when it
  finishes. Otherwise, your screen stays as it is.

If remote debugging is off, or you use another browser, milo reads the browser
window off the screen instead. This is slower and less exact.

### Apps

In an app, milo looks at a picture of the window, and reads the names of its
buttons, fields, and text. milo brings the app to the front while it works
there, and puts your app back when the plan finishes.

milo starts each plan in the place where it worked last, or goes to the app or
the tab that the plan names. milo can work in your terminal app too, including
the window that milo runs in.

If something asks milo to allow, confirm, sign in, pay, delete, or send
something that the plan doesn't cover, milo stops and tells you what it asks.
milo can't see a window that is minimized.

In a save or open dialog, milo can type the file name and choose a folder from
the **Where** menu. It can't go to a folder that the menu doesn't list.

You can also ask milo what is on your screen. It looks at the window or the
tab in front, without asking for approval.

To run one task without the voice, use `--task`. To give milo a plan to follow,
add `--step` for each step:

```sh
milo --task "Search YouTube for cats" --step "Open youtube.com" --step "Search for cats"
```

milo prints each action and the result, and then exits. This runs the plan
without asking for approval.

## Memory

milo saves lasting facts that you mention, such as your name or where you live,
in `~/.config/milo/memory.md`, one fact per line. To remove a fact, ask milo to
forget it, or edit the file.

## Add your own tools

To let milo run your own commands, list them in `~/.config/milo/tools.json`:

```json
[
  {
    "name": "lights",
    "description": "Turn the lights on or off.",
    "parameters": {
      "type": "object",
      "properties": {
        "state": { "type": "string", "enum": ["on", "off"] }
      },
      "required": ["state"]
    },
    "run": ["shortcuts", "run", "Lights {state}"]
  }
]
```

- `name`: the tool name, in letters, digits, and underscores.
- `description`: what the tool does. milo reads this text to decide when to use
  the tool.
- `parameters`: optional. A [JSON Schema](https://json-schema.org) object that
  describes the values that milo fills in.
- `run`: the command and its arguments. milo replaces each parameter name in
  braces, such as `{state}`, with its value.

milo asks for your approval in the notch panel, runs the command without a
shell, and stops it after 20 seconds. Whatever
the command prints is the result that milo tells you.

**Caution:** The language model fills in the parameter values. Only add
commands that are safe to run with any value.

## Change the voice

To get a different voice, describe it:

```sh
milo --voice-design "A calm British woman with a low, slow, thoughtful voice."
```

To copy a voice, give a WAV recording of 3 to 12 seconds and the exact words
that it contains.

```sh
milo --voice-ref RECORDING.wav --voice-text "The exact words in the recording."
```

## Change the model

milo uses one language model to talk and to act. You name the model as
`PROVIDER/MODEL`. The provider is the model server that milo runs on your Mac,
another server that you run, or a hosted service that you have an API key for.

The model must stream chat completions and support tool calls. If it doesn't
support tool calls, add `--no-tools`.

### Run a model on your Mac

A name without a provider is a model that Rapid-MLX serves on your Mac. For
example, the following model is smaller than the default and runs on a Mac
with 18 GB of memory:

```sh
milo --model qwen3.5-9b-4bit
```

To use a model from Ollama or LM Studio, start the server of that app, and then
name the model:

```sh
milo --model ollama/MODEL_NAME
milo --model lmstudio/MODEL_NAME
```

### Use a hosted model

1.  Save your API key for the provider:

    ```sh
    milo auth openai
    ```

    milo asks for the key and saves it in `~/.config/milo/auth.json`, which
    only your user account can read. To remove the key, run the command again
    and leave the key empty.
2.  List the models that the provider offers:

    ```sh
    milo models openai
    ```

3.  Start milo with one of them:

    ```sh
    milo --model openai/MODEL_NAME
    ```

Instead of saving the key, you can set it in the environment variable of the
provider.

| Provider | Environment variable |
|---|---|
| `openai` | `OPENAI_API_KEY` |
| `anthropic` | `ANTHROPIC_API_KEY` |
| `google` | `GEMINI_API_KEY` |
| `openrouter` | `OPENROUTER_API_KEY` |
| `groq` | `GROQ_API_KEY` |
| `cerebras` | `CEREBRAS_API_KEY` |
| `xai` | `XAI_API_KEY` |
| `mistral` | `MISTRAL_API_KEY` |
| `deepseek` | `DEEPSEEK_API_KEY` |
| `together` | `TOGETHER_API_KEY` |
| `fireworks` | `FIREWORKS_API_KEY` |

To see every provider, its address, and whether it has a key, run
`milo models`.

With a hosted model, milo sends what you say, the lists of what is on your
screen, and screenshots to that provider.

### Set the models in a file

To choose the models without an option, create `~/.config/milo/config.json`:

```json
{
  "model": "openai/MODEL_NAME",
  "small_model": "groq/MODEL_NAME",
  "vision_model": "google/MODEL_NAME",
  "providers": {
    "openai": {
      "models": {
        "MODEL_NAME": {
          "options": { "reasoning": { "effort": "low" }, "service_tier": "priority" }
        }
      }
    },
    "mine": {
      "url": "http://192.168.1.20:8080/v1",
      "key": "{env:MY_SERVER_KEY}"
    }
  }
}
```

- `model`: the model that milo talks and acts with.
- `small_model`: optional. The model that writes the
  [notes about your apps](#what-milo-learns). The default is `model`.
- `vision_model`: optional. The model that finds a thing in a picture of the
  screen when the thing has no number in the list. The default is `model`.
- `providers`: optional. Settings for a built-in provider, or a provider of
  your own, such as `mine` in the example. You then name its models as
  `mine/MODEL_NAME`.

A provider has the following settings, all optional for a built-in provider:

| Setting | Description |
|---|---|
| `url` | The base URL of an OpenAI-compatible server. Required for a provider of your own. |
| `api` | The API that the server speaks: `chat` for chat completions, or `responses` for the OpenAI Responses API. The default is `chat`. For `openai`, the default is `responses`. |
| `key` | The API key. To read it from an environment variable, write `{env:NAME}`. |
| `headers` | Extra HTTP headers to send, as an object of names and values. |
| `limit` | The name of the request field that limits the length of a reply. The default is `max_tokens`. |
| `options` | Fields to add to every request to this provider. A field set to `null` is left out of the request. |
| `models` | Settings for one model, by its name: `options` adds fields to the requests for that model, `price` sets what its tokens cost, and `"pictures": false` sends that model the lists of what is on screen without the screenshots. |

For example, the `options` in the example set the reasoning effort and the
service tier that milo asks OpenAI for.

### See what a session costs

After each reply, milo prints the cost of that turn, the cost of the session so
far, and the number of tokens:

```text
cost $0.0121 this turn, $0.1840 this session | 212.4k in (190.1k cached), 3.1k out this session
```

The numbers come from the token counts that the provider reports for each
request, and include the requests for notes and pictures.

milo knows the prices of the OpenAI models, for the standard, flex, priority,
and ultrafast service tiers, as of November 24, 2026. For a model on a
regional OpenAI address, such as `us.api.openai.com`, it adds the 10% that
OpenAI charges there. For any other model, milo prints the tokens alone. To
get a cost for that model, or to change a price, set `price` on the model, in
US dollars for each million tokens:

```json
{
  "providers": {
    "groq": {
      "models": {
        "MODEL_NAME": { "price": { "input": 0.5, "cached": 0.25, "output": 2 } }
      }
    }
  }
}
```

## Options

| Option | Description |
|---|---|
| `--model` | The language model, as `PROVIDER/MODEL`. The default is the model in `config.json`, or `local/gemma-4-26b-qat-4bit`. |
| `--small-model` | The model that writes the notes about your apps. The default is `--model`. |
| `--vision-model` | The model that finds things in pictures of the screen. The default is `--model`. |
| `--port` | The port for the model server on your Mac. The default is `8000`. |
| `--system-prompt` | A file with your own system prompt, which replaces the built-in one. |
| `--voice-design` | A description of the voice to create. |
| `--voice-ref`, `--voice-text` | A WAV recording of a voice to copy, and its exact transcript. |
| `--tts` | The voice engine: `breeze` (the default), or `phonon`, which is smaller and faster but less natural. |
| `--tts-port` | The port for the voice. The default is `8881`. |
| `--no-aec` | Turns off echo cancellation. Use it with headphones. |
| `--push-to-talk` | Starts with your microphone muted. To talk, hold Control+Option+Space. |
| `--no-notch` | Turns off the panel in the notch. milo then runs commands without asking. |
| `--dangerously-auto-approve` | Runs every command and plan without asking for approval. The panel stays on. |
| `--no-tools` | Turns off tools and memory. |
| `--no-backchannel` | Turns off the listening sounds while you talk. |
| `--input-device`, `--output-device` | The microphone or the speakers, by name. |
| `--list-devices` | Lists the audio devices and exits. |
| `--task`, `--step` | Carries out one task in your apps without the voice, and exits. To give a plan, repeat `--step` for each step. |

## Troubleshooting

- **milo doesn't hear you.** In System Settings, go to **Privacy & Security** >
  **Microphone**, turn on access for your terminal app, and start milo again.
  To choose a microphone, run `milo --list-devices`, and then pass a name to
  `--input-device`.
- **The voice sounds different or flat.** The main voice couldn't start, and
  milo is using its built-in fallback. The log line that mentions Phonon gives
  the reason. Run `./setup.sh` again.
- **The build fails with a Rust version error.** Run `rustup update`, and then
  run `./setup.sh` again.
- **milo says that Accessibility or Screen Recording is turned off.** In System
  Settings, go to **Privacy & Security**, turn on access for your terminal app
  under **Accessibility** and **Screen Recording**, and start milo again.
- **milo says that no browser allows remote debugging.** In Chrome, go to
  `chrome://inspect/#remote-debugging`, turn on **Allow remote debugging for
  this browser instance**, and start milo again.
- **milo does the wrong thing in an app.** The folder `$TMPDIR/milo-computer`
  has the picture and the list that milo saw at each step of the last plan,
  and the action that led to it.
- **Something else is wrong.** Run `RUST_LOG=debug milo` for more detail. The
  model server and the voice write their own logs to
  `$TMPDIR/milo-rapid-mlx-8000.log` and `$TMPDIR/milo-voice-8881.log`.

## Uninstall

```sh
cargo uninstall milo
rm -rf ~/.cache/milo ~/.config/milo
```

The models stay in `~/.cache/huggingface/hub` and
`~/Library/Application Support/FluidAudio/Models` until you delete them.

## How it works

```text
microphone -> speech recognition -> turn detection -> model -> voice -> speakers
```

For the details, see [milo architecture](docs/architecture.md).

## License

milo is licensed under the [MIT License](LICENSE). The models that milo
downloads have their own licenses.
