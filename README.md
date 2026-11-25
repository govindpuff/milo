# milo

> **Note:** milo is experimental software, and it isn't ready for production
> use. It can click, type, and send messages on your Mac, and it can make
> mistakes. milo is provided as is, without warranty, and its authors aren't
> responsible for any loss or damage that comes from using it. For details,
> see the [license](LICENSE).

milo is a voice assistant for your Mac. You talk, and it talks back, like a
phone call. You can talk over it to interrupt. It can also check the weather,
set timers, open apps, use apps and websites for you, send text messages, and
remember what you tell it.

## Requirements

- A Mac with Apple silicon and macOS 14 or later
- An API key for OpenAI, Anthropic, Google, or OpenRouter

## Get started

1.  Download [Milo.dmg](https://github.com/govindpuff/milo/releases/latest/download/Milo.dmg),
    open it, and drag Milo to your Applications folder.
2.  Open Milo. A setup window asks for the following:

    - Your API key
    - Access to the microphone, Accessibility, and Screen Recording
    - The voice that you want

    Milo doesn't listen until you finish the setup.
3.  Wait while Milo downloads its voice and speech models, which can take
    several minutes.
4.  When the window says that Milo is ready, start talking.

Milo has no window of its own. It shows a panel in the notch and an **m** in
the menu bar. To quit, click the **m**, and then click **Quit Milo**.

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

To skip the approval, turn on **Approve everything automatically** in the
settings.

## Settings

To open the settings, click the **m** in the menu bar, and then click
**Settings**. You can change the following:

- The provider, its API key, and the model
- The voice
- The microphone and the speakers
- **Headphones**, which turns off echo cancellation
- **Push to talk**
- **Listening sounds**
- **Approve everything automatically**
- **Open at login**

When a change needs it, Milo restarts.

## Let milo use your apps

milo can click, type, and press keys in your apps and on websites.

1.  In the settings, under **Permissions**, allow **Accessibility** and
    **Screen Recording**.
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
and Milo needs **Full Disk Access** in System Settings, under **Privacy &
Security**.

## Memory

milo saves lasting facts that you mention in `~/.config/milo/memory.md`, one
fact per line. To remove a fact, ask milo to forget it, or edit the file.

milo also keeps notes about each app and website that it has worked in, in
`~/.config/milo/skills`. To make milo learn a place again, delete its file.

## Change the model

The default model is `openai/gpt-6.2-sol`. In the settings, choose a provider,
paste its API key, and choose a model. Milo supports OpenAI, Anthropic,
Google, and OpenRouter.

## Change the voice

In the settings, describe the voice that you want, such as "A calm British
woman with a low, slow, thoughtful voice."

To copy a voice, choose a WAV recording of 3 to 12 seconds, and type the
exact words that it contains.

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

## Troubleshooting

- **milo doesn't hear you.** In the settings, under **Permissions**, allow the
  microphone, and start Milo again.
- **milo says that Accessibility or Screen Recording is turned off.** Allow
  both in the settings, and start Milo again.
- **milo says that no browser allows remote debugging.** Turn it on at
  `chrome://inspect/#remote-debugging`, and start Milo again.
- **Something else is wrong.** See `~/Library/Logs/Milo/milo.log`.

## Uninstall

Move Milo from your Applications folder to the Trash, and then remove its
files:

```sh
rm -rf ~/.cache/milo ~/.config/milo ~/Library/Logs/Milo
```

## Build from source

You need Xcode 26, or the Xcode Command Line Tools with Swift 6.2 or later.

```sh
git clone https://github.com/govindpuff/milo
cd milo
./setup.sh
```

The script installs the `milo` command, which runs milo in your terminal with
the same settings. For its options, run `milo --help`. To build the app, run
`scripts/bundle.sh`. To build a notarized `Milo.dmg`, run
`scripts/release.sh`.

## How it works

See [milo architecture](docs/architecture.md).

## License

milo is licensed under the [MIT License](LICENSE). The models that milo
downloads have their own licenses.
