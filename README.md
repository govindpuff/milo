# milo

milo is a voice assistant that runs entirely on your Mac. You talk, and it talks
back, like a phone call. It starts preparing an answer before you finish
speaking, you can talk over it to interrupt, and it makes small listening
sounds, such as "mm-hmm," while you talk. It can also check the weather, look
things up, set timers, open apps, and remember what you tell it.

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
- "Who was Ada Lovelace?"
- "Open Safari."
- "Remember that my sister's name is Priya."
- Talk over milo to interrupt it.
- Pause in the middle of a sentence. milo waits for you to go on.

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

milo runs the command without a shell and stops it after 20 seconds. Whatever
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

To use another model, pass any model name that Rapid-MLX can serve. For
example, the following model is smaller and runs on a Mac with 18 GB of memory:

```sh
milo --model qwen3.5-9b-4bit
```

To use your own OpenAI-compatible server instead of Rapid-MLX, give its base
URL and the model name that it expects:

```sh
milo --llm-url http://127.0.0.1:1234/v1 --model MODEL_NAME
```

The server must stream chat completions and must not require an API key. If it
doesn't support tool calls, add `--no-tools`.

## Options

| Option | Description |
|---|---|
| `--model` | The language model. The default is `gemma-4-26b-qat-4bit`. |
| `--llm-url` | The base URL of your own OpenAI-compatible server. |
| `--port` | The port for the model server. The default is `8000`. |
| `--system-prompt` | A file with your own system prompt, which replaces the built-in one. |
| `--voice-design` | A description of the voice to create. |
| `--voice-ref`, `--voice-text` | A WAV recording of a voice to copy, and its exact transcript. |
| `--tts` | The voice engine: `breeze` (the default), or `phonon`, which is smaller and faster but less natural. |
| `--tts-port` | The port for the voice. The default is `8881`. |
| `--no-aec` | Turns off echo cancellation. Use it with headphones. |
| `--no-tools` | Turns off tools and memory. |
| `--no-backchannel` | Turns off the listening sounds while you talk. |
| `--nudge-ms` | How long milo waits in silence before it follows up, in milliseconds. The default is `6000`. To turn off follow-ups, set it to `0`. |
| `--input-device`, `--output-device` | The microphone or the speakers, by name. |
| `--list-devices` | Lists the audio devices and exits. |

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
