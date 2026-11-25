# milo

milo is a voice assistant for your Mac. You talk, and it talks back, like a
phone call. You can talk over it to interrupt. It can also check the weather,
set timers, open apps, use apps and websites for you, send text messages, and
remember what you tell it.

[![A one-minute demo of milo and its panel in the notch](docs/demo.gif)](docs/demo.mp4)

To hear the voices, watch the [demo video with sound](docs/demo.mp4).

## Requirements

- A Mac with Apple silicon and macOS 14 or later
- Xcode 26, or the Xcode Command Line Tools with Swift 6.2 or later
- An API key for OpenAI, Anthropic, Google, or OpenRouter

## Get started

1.  Clone the repository and run the setup script:

    ```sh
    git clone https://github.com/govindpuff/milo
    cd milo
    ./setup.sh
    ```

2.  Start milo:

    ```sh
    milo
    ```

    The first start asks for your OpenAI API key, unless `OPENAI_API_KEY` is
    set, and saves it in `~/.config/milo/auth.json`. It then downloads the
    speech and voice models, which can take several minutes. When macOS asks
    whether your terminal can use the microphone, allow access.

3.  When milo prints `listening`, start talking.

If you use headphones, start milo with `milo --no-aec`. To quit, press
Control+C.

## Things to try

- "What's the weather in Chicago?"
- "Set a timer for two minutes."
- "Open Safari."
- "Make a new note that says call the dentist."
- "Text Priya that I'm running late."
- "Pause that video."
- "Write a hello world Python file on my desktop, and open it in VS Code."
- "Remember that my sister's name is Priya."

## The panel

milo shows a small panel in the notch at the top of your screen. To see what
milo is doing, hold the pointer over the panel.

Before milo opens an app or a website, closes a tab, writes a file, sends a
text message, or starts a plan in your apps, the panel shows what it is about
to do. Click **Approve** or **Decline**. If you don't answer within 45
seconds, milo doesn't act.

| Shortcut | Action |
|---|---|
| Control+Option+Y | Approves what the panel shows. |
| Control+Option+N | Declines it, or stops a plan. |
| Control+Option+M | Mutes or unmutes your microphone. |
| Control+Option+Space | Unmutes your microphone while you hold the keys. |

To skip the approval, start milo with `milo --dangerously-auto-approve`.

## Let milo use your apps

milo can click, type, and press keys in your apps and on websites.

1.  In System Settings, go to **Privacy & Security**, and turn on access for
    your terminal app under **Accessibility** and **Screen Recording**.
2.  In Chrome, go to `chrome://inspect/#remote-debugging`, and turn on
    **Allow remote debugging for this browser instance**.
3.  Start milo again. When Chrome asks whether to allow remote debugging,
    click **Allow**.
4.  Ask milo for something that takes a few steps, and approve the plan in
    the panel.

You can talk while milo works. To change the plan, say what you want
instead. To stop it, say so, or click **Stop**.

milo opens websites in a browser window of its own and leaves your tabs as
they are. This works in Chrome, Brave, Edge, Arc, Vivaldi, and Chromium.

To send text messages, milo needs [imsg](https://github.com/steipete/imsg),
and your terminal app needs **Full Disk Access**.

## Memory

milo saves lasting facts that you mention in `~/.config/milo/memory.md`, one
fact per line. To remove a fact, ask milo to forget it, or edit the file.

milo also keeps notes about each app and website that it has worked in, in
`~/.config/milo/skills`. To make milo learn a place again, delete its file.

## Change the model

The default model is `openai/gpt-6.2-sol`. milo supports the following
providers:

| Provider | Environment variable |
|---|---|
| `openai` | `OPENAI_API_KEY` |
| `anthropic` | `ANTHROPIC_API_KEY` |
| `google` | `GEMINI_API_KEY` |
| `openrouter` | `OPENROUTER_API_KEY` |

To use another model, name it when you start milo. milo asks for the API key
of the provider the first time, unless its environment variable is set:

```sh
milo --model anthropic/MODEL_NAME
```

To list the models of a provider, run `milo models PROVIDER`. To replace or
remove a saved key, run `milo auth PROVIDER`.

To keep the choice, create `~/.config/milo/config.json`:

```json
{
  "model": "anthropic/MODEL_NAME"
}
```

## Change the voice

To get a different voice, describe it:

```sh
milo --voice-design "A calm British woman with a low, slow, thoughtful voice."
```

To copy a voice, give a WAV recording of 3 to 12 seconds and the exact words
that it contains:

```sh
milo --voice-ref RECORDING.wav --voice-text "The exact words in the recording."
```

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

milo replaces each parameter name in braces, such as `{state}`, with its
value, asks for your approval in the panel, and runs the command without a
shell.

## Options

| Option | Description |
|---|---|
| `--model` | The model, as `PROVIDER/MODEL`. |
| `--voice-design` | A description of the voice to create. |
| `--voice-ref`, `--voice-text` | A WAV recording of a voice to copy, and its exact transcript. |
| `--no-aec` | Turns off echo cancellation. Use it with headphones. |
| `--push-to-talk` | Starts with your microphone muted. To talk, hold Control+Option+Space. |
| `--dangerously-auto-approve` | Runs every command and plan without asking for approval. |
| `--no-notch` | Turns off the panel. milo then can't use apps for you. |
| `--no-tools` | Turns off tools and memory. |
| `--input-device`, `--output-device` | The microphone or the speakers, by name. |
| `--task`, `--step` | Carries out one task in your apps without the voice, and exits. |

For every option, run `milo --help`.

## Troubleshooting

- **milo doesn't hear you.** In System Settings, go to **Privacy & Security** >
  **Microphone**, turn on access for your terminal app, and start milo again.
- **milo says that Accessibility or Screen Recording is turned off.** Turn on
  both for your terminal app, and start milo again.
- **milo says that no browser allows remote debugging.** Turn it on at
  `chrome://inspect/#remote-debugging`, and start milo again.
- **Something else is wrong.** Run `RUST_LOG=debug milo` for more detail.

## Uninstall

```sh
cargo uninstall milo
rm -rf ~/.cache/milo ~/.config/milo
```

## How it works

See [milo architecture](docs/architecture.md).

## License

milo is licensed under the [MIT License](LICENSE). The models that milo
downloads have their own licenses.
