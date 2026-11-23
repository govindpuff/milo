# milo

milo is a voice assistant that runs on your Mac. You talk, and it talks back,
like a phone call: it starts preparing an answer before you finish speaking, you
can talk over it to interrupt, and it makes small listening sounds, such as
"mm-hmm," while you talk. It can also check the weather, look things up, set
timers, open apps, and remember what you tell it from one conversation to the
next.

Speech recognition, the language model, and the voice all run on your Mac.

## Requirements

To build and run milo, you need the following:

- A Mac with Apple silicon and macOS 14 or later.
- Enough memory for the language model. For the default model, Rapid-MLX lists
  a minimum of 18 GB. On a Mac with less memory, use a smaller model. For more
  information, see [Use another model or server](#use-another-model-or-server).
- Xcode 26, or the Xcode Command Line Tools with Swift 6.2 or later.
- Rust 1.95 or later.
- uv, a Python package manager.
- An internet connection for the setup and the first start.

The setup script installs the Command Line Tools, Rust, and uv if they're
missing. If you already have Rust, update it with `rustup update`. To check your
Swift version, run `swift --version`.

## Get started

1.  Clone the repository and go to its directory:

    ```sh
    git clone https://github.com/govindpuff/milo
    cd milo
    ```

2.  Install what milo needs and build it:

    ```sh
    ./setup.sh
    ```

    The script installs the Command Line Tools, Rust, uv, and the Rapid-MLX
    model server if they're missing. It then creates a Python environment for
    the voice in `~/.cache/milo/venv`, builds milo, and installs the `milo`
    command in `~/.cargo/bin`. It doesn't download any models. You can run the
    script again at any time; it skips what's already installed.

    If the script opens the Command Line Tools installer, finish the
    installation, and then run `./setup.sh` again.

3.  Start milo:

    ```sh
    milo
    ```

    If your shell doesn't find the `milo` command, open a new terminal window.

    The first start downloads about 9 GB of models and prepares them, which can
    take several minutes. milo then creates its voice and records a set of
    short reactions in that voice. Later starts skip these steps.

4.  When macOS asks whether your terminal app can use the microphone, allow
    access.

5.  When milo prints `listening`, start talking.

milo removes the echo of its own voice from the microphone signal, so you can
use speakers. If you use headphones, start milo with `--no-aec` instead:

```sh
milo --no-aec
```

With `--no-aec`, echo cancellation is off, and milo stops talking when it
detects your voice, without waiting for words.

## Talk to milo

Talk the way that you talk on a phone call. Try the following:

- Ask "What's the weather in Chicago?"
- Say "Set a timer for two minutes."
- Ask "Who was Ada Lovelace?"
- Say "Open Safari."
- Say "Remember that my sister's name is Priya." In a later conversation, ask
  "What's my sister's name?"
- Start telling a story, and pause in the middle of a sentence. milo makes a
  short sound, such as "mm-hmm," and waits for you to go on.

To interrupt milo, talk over it. Two or more words stop the reply, but short
words such as "yeah" or "okay" don't. If you don't say anything after a reply,
milo follows up after 6 seconds, and once more after a longer pause.

The terminal shows what milo heard and what it says. Each reply starts with a
delivery note in brackets, such as `[bright]`, which sets the tone of the voice
and isn't spoken.

To quit, press Control+C.

## Tools and memory

After milo answers you, a background agent that uses the same language model
checks what you said and runs a tool if you asked for something that a tool
does. milo doesn't wait for the tool; it tells you the result in a later reply.

milo has the following built-in tools:

| Tool | What it does |
|---|---|
| `weather` | Gets the current weather and the forecast for today and tomorrow from Open-Meteo, in Fahrenheit and miles per hour. |
| `lookup` | Gets the opening of the top English Wikipedia result for a topic. |
| `timer` | Starts a timer of 1 second to 24 hours. milo tells you when it finishes. |
| `open` | Opens an app by name, or a website. |
| `remember` | Saves a fact to memory. |
| `forget` | Removes matching facts from memory. |

To turn off tools and memory, start milo with `--no-tools`.

### Memory

milo stores facts about you in `~/.config/milo/memory.md`, one fact per line.
The agent saves lasting facts that you mention, such as your name or where you
live, and anything that you ask it to remember. The language model gets the last
60 lines of the file. To remove a fact, ask milo to forget it, or edit the file.

### Add your own tools

To add tools, list them in `~/.config/milo/tools.json`. milo reads this file
when it starts. The following example adds a `lights` tool that runs a shortcut
from the Shortcuts app:

```json
[
  {
    "name": "lights",
    "about": "Turn the lights on or off.",
    "can": "turn the lights on or off",
    "params": { "state": "on or off" },
    "run": ["shortcuts", "run", "Lights {state}"]
  }
]
```

When you ask milo to turn off the lights, the agent calls `lights` with `state`
set to `off`, and milo runs `shortcuts run "Lights off"`. For this example to
work, create shortcuts named `Lights on` and `Lights off` in the Shortcuts app.

Each tool has the following fields:

| Field | Required | Description |
|---|---|---|
| `name` | Yes | The tool name, in letters, digits, and underscores. A tool with the name of a built-in tool is skipped. |
| `about` | Yes | What the tool does. The agent reads this text to choose a tool. |
| `can` | No | A short phrase that milo uses to describe the tool in conversation. The default is the `about` text. |
| `params` | No | The parameters, as names mapped to descriptions. Every parameter is required. |
| `run` | Yes | The command and its arguments. In each argument, milo replaces a parameter name in braces, such as `{state}`, with the parameter's value. |

milo runs the command directly, without a shell and with no input, and stops it
after 20 seconds. What the command prints is the result that milo tells you. If
the command fails, the first line of its error output is the reason. If
`tools.json` has an error, milo ignores the whole file and logs a warning.

**Caution:** The language model fills in the parameter values. Only add commands
that are safe to run with any value.

## Choose a voice

milo has three voice engines, which you choose with `--tts`:

| Engine | Option | Description |
|---|---|---|
| Breeze | `--tts breeze` (default) | Creates a voice from a description, or copies a voice from a recording. Runs in a separate Python process, and downloads about 3 GB on first use. |
| Lure | `--tts lure` | Offers six preset voices, or copies a voice from a recording. Install it first with `./setup.sh --lure`. Downloads about 2.4 GB on first use. |
| Phonon | `--tts phonon` | A small voice that runs inside milo. It doesn't follow delivery notes or make sounds such as laughs. |

If Breeze or Lure can't start, milo uses Phonon and logs the reason.

To describe a different Breeze voice, use `--voice-design`:

```sh
milo --voice-design "A calm British man with a low, slow, thoughtful voice."
```

The first time that you use a description, milo creates the voice, saves it in
`~/.cache/milo/voices`, and records its reactions again. The default
description is of a warm, friendly young American woman.

To copy a voice with Breeze or Lure, give a WAV recording of 3 to 12 seconds
and the exact words that it contains:

```sh
milo --voice-ref RECORDING_PATH --voice-text "TRANSCRIPT"
```

Replace the following:

- `RECORDING_PATH`: the path to the WAV file
- `TRANSCRIPT`: the exact words spoken in the recording

Only copy the voice of someone who agrees to it.

To use a Lure preset voice, pass its name to `--voice`—for example:

```sh
milo --tts lure --voice imogen
```

The presets are `nora` (the default), `jess`, `wade`, `imogen`, `callum`, and
`tobi`.

For Phonon, choose a voice from the Phonon voice catalog, or a `.safetensors`
voice file, with `--phonon-voice`. The default is `Easton`.

## Use another model or server

By default, milo starts the Rapid-MLX server on port 8000 with the
`qwen3.5-9b-4bit` model, and stops the server when milo exits.

To use another model, pass its name to `--model`:

```sh
milo --model MODEL
```

Replace `MODEL` with any model name or alias that Rapid-MLX can serve—for
example, `qwen3.5-4b-4bit`, which needs less memory. Rapid-MLX downloads the
model the first time that you use it. Larger models take longer to answer.

If a server already answers on port 8000, milo uses it as is and doesn't start
one. To switch models, stop that server first.

### Use your own server

milo works with other servers that have an OpenAI-compatible API. If the server
runs on your Mac and answers at `http://127.0.0.1:PORT/v1`, give its port:

```sh
milo --port PORT --model MODEL_NAME
```

For a server at another address, give its base URL. With `--llm-url`, milo
never starts Rapid-MLX:

```sh
milo --llm-url BASE_URL --model MODEL_NAME
```

Replace the following:

- `PORT`: the port that the server listens on
- `MODEL_NAME`: the model name that the server expects
- `BASE_URL`: the base URL of the API, without `/chat/completions`—for
  example, `http://llm.example.com:8080/v1`

The server must meet the following requirements:

- It streams chat completions from `POST /chat/completions` as server-sent
  events.
- It doesn't require an API key. milo doesn't send one.
- It accepts the `chat_template_kwargs` field, which milo sends with every
  request to turn off thinking. If the server ignores the field, a reasoning
  model answers slowly and might speak its reasoning.
- For tools, it returns tool calls in the OpenAI format, or as `<tool_call>`
  JSON in the reply text. If it doesn't, start milo with `--no-tools`.

milo also uses three Rapid-MLX features that other servers don't need to
support: prompt cache warming, a lower priority for the agent's requests, and
canceling a reply when you interrupt it. Without them, milo still works.

## Common options

The following table lists the options that you're most likely to use. For all
options and their defaults, run `milo --help`.

| Option | Description |
|---|---|
| `--no-aec` | Turns off echo cancellation. Use it with headphones. |
| `--model` | Sets the model to talk with. The default is `qwen3.5-9b-4bit`. |
| `--llm-url` | Uses an OpenAI-compatible server at this base URL instead of starting Rapid-MLX. |
| `--tts` | Sets the voice engine: `breeze`, `lure`, or `phonon`. The default is `breeze`. |
| `--voice-design` | Describes the Breeze voice to create. |
| `--voice-ref`, `--voice-text` | Sets a WAV recording to copy the voice from, and its exact transcript. |
| `--no-tools` | Turns off tools and memory. |
| `--no-backchannel` | Turns off the listening sounds while you talk or pause. |
| `--nudge-ms` | Sets how long milo waits in silence after a reply before it follows up, in milliseconds. The default is `6000`. To turn off follow-ups, set it to `0`. |
| `--barge-words` | Sets how many words you need to say over a reply to interrupt it. The default is `2`. |
| `--patience-ms` | Sets how long milo waits when you pause in the middle of a sentence, in milliseconds. The default is `1500`. |
| `--system-prompt` | Replaces the built-in system prompt with the contents of a file. milo fills in `{sounds}`, `{tools}`, and `{memory}` in the file. |
| `--input-device`, `--output-device` | Sets the microphone or the speakers, by name or by a unique part of the name. |
| `--list-devices` | Lists the audio devices and exits. |

## How it works

```text
microphone -> speech recognition -> turn detection -> model -> voice -> speakers
```

milo is a Rust program. A streaming speech recognizer (Parakeet, through
FluidAudio) turns your voice into text as you speak, and a voice activity model
(Silero VAD) and a turn model (Smart Turn) estimate when you've finished. milo
starts a reply before your turn ends, sends the first phrase to the voice as
soon as the language model writes it, and holds the audio until the end of your
turn is confirmed. For more information, see
[milo architecture](docs/architecture.md).

## Troubleshooting

### Find the logs

milo prints its log in the terminal. The model server and the voice write their
own logs to `$TMPDIR/milo-rapid-mlx-8000.log` and `$TMPDIR/milo-voice-8881.log`;
the number in each name is the port. For more detail from milo, set the
`RUST_LOG` environment variable to `debug`:

```sh
RUST_LOG=debug milo
```

### milo doesn't respond when you talk

If milo prints `listening` but never shows what you said, try the following:

- In System Settings, go to **Privacy & Security** > **Microphone**, and turn
  on access for your terminal app. Then start milo again. Without access, macOS
  gives milo silence.
- To see your audio devices, run `milo --list-devices`. To choose a
  microphone, start milo with `--input-device` and the device name, or a unique
  part of it.

### A port is already in use

milo uses port 8000 for the model server and port 8881 for the voice.

- If another program uses port 8000, milo waits for it to answer as a model
  server and logs `server on port 8000 is not ready yet, waiting`. Start milo
  with `--port` and a free port.
- If another program uses port 8881, the voice can't start, and milo uses
  Phonon instead. Start milo with `--tts-port` and a free port.

### milo uses a different voice

If the log shows `the breeze voice is unavailable, using Phonon instead`, the
rest of that line gives the reason. If it says that the voice environment is
missing, run `./setup.sh` again. For Lure, run `./setup.sh --lure`.

### The first start stops before milo is ready

milo waits up to 20 minutes for the model server and up to 30 minutes for the
voice, including their downloads. On a slow connection, the first start can stop
with an error such as `rapid-mlx on port 8000 not ready after 1200s`. Start milo
again; files that finished downloading aren't downloaded again.

## Uninstall

1.  Remove the `milo` command:

    ```sh
    cargo uninstall milo
    ```

2.  Delete milo's cache and settings, including your memory and custom tools:

    ```sh
    rm -rf ~/.cache/milo ~/.config/milo
    ```

3.  Optional: Remove Rapid-MLX:

    ```sh
    uv tool uninstall rapid-mlx
    ```

4.  Optional: Delete the downloaded models. The speech recognition models are
    in `~/Library/Application Support/FluidAudio/Models`. The other models are
    in the Hugging Face cache, by default `~/.cache/huggingface/hub`, in the
    following directories:

    - `models--mlx-community--Qwen3.5-9B-4bit`
    - `models--mlx-community--Breeze-TTS-2-mlx-4bit`
    - `models--gradium--phonon-31m-en`
    - `models--gradium--phonon-voices`

## License

milo is licensed under the [MIT License](LICENSE). The models that milo
downloads have their own licenses.
