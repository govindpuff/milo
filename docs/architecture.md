# milo architecture

This document explains how milo works, for readers who are about to read the
code. For installation and use, see the [README](../README.md).

## Overview

milo is one Rust binary, which is also the executable of `Milo.app`. Speech
recognition, turn detection, and the voice run on your Mac. The language
model is a hosted model.

```text
microphone -> capture -> speech recognition -+-> controller -> reply -> voice -> gate -> speakers
                      -> turn detection -----+       |
                                                   tools
```

## Crates

| Crate | What it does |
|---|---|
| `crates/milo` | The binary. `main.rs` has the options and the system prompt. `controller.rs` runs the conversation. `reply.rs` streams a reply and shapes it for speech. `tts.rs` runs the voice thread and the gate. `react.rs` has the reaction clips. `tools.rs` has the tools and memory. `computer.rs` has the tools that act in your apps. `skills.rs` keeps the notes about each app and website. `config.rs` reads the providers, the models, the API keys, and the settings. `app.rs` has what only the app needs: the log file, the first start, the answers to the settings window, and the restart. |
| `crates/audio` | Microphone and speaker I/O, resampling, and the echo reference. |
| `crates/speech` | Bindings to `swift/MiloSpeech`: echo removal and Parakeet speech recognition through FluidAudio. |
| `crates/turn` | Voice activity detection (Silero VAD) and end-of-turn detection (Smart Turn). |
| `crates/llm` | The model client, which streams text and tool calls over chat completions or the OpenAI Responses API, and the sentence chunker. |
| `crates/voice` | Starts the Python voice server for the Breeze voice, and streams audio from it. |
| `crates/notch` | Bindings to `swift/MiloNotch`: the panel in the notch, the menu bar item, the settings window, and the window of the first start. |
| `crates/control` | Bindings to `swift/MiloControl`, which reads and uses app windows through the macOS Accessibility API. |
| `crates/browser` | The client for the Chrome DevTools Protocol. |

## A turn

`Controller` in `controller.rs` owns all conversation state. Every other part
of milo sends it events over one channel.

A *turn* is one reply, with the tool calls that the model makes on the way to
it. There's at most one turn at a time.

1.  **Recognition.** The streaming recognizer sends partial transcripts while
    you talk. After a short silence, a second, more exact recognizer
    transcribes the same audio, and its result replaces the transcript if the
    words differ.
2.  **Speculative reply.** When you pause, the controller starts a turn before
    it knows whether you've finished. milo generates and synthesizes the
    reply, and the gate holds its audio. If you keep talking, the controller
    cancels the turn.
3.  **End of turn.** Smart Turn scores whether you've finished. The controller
    waits a short time after a finished verdict and longer after an
    unfinished one, and then *commits* the turn.
4.  **Commit.** The held audio plays, and your message goes into the history
    with the local time.
5.  **Reply.** `reply::generate` streams the reply. The model starts each
    reply with a delivery note in square brackets, such as `[amused, quick]`,
    which goes to the voice as a direction and isn't spoken. The chunker
    removes Markdown, sends the first clause to the voice at once, and sends
    the rest when the model finishes.
6.  **End.** The turn ends when the reply is done and playback is empty. The
    controller prints a timing line and the cost of the session.

## Interruptions

While a reply generates or plays, two or more new words from you stop it.
Acknowledgments such as "yeah" don't count. With `--no-aec`, your voice alone
stops it.

- If the reply hasn't played for more than 1.5 seconds, the controller drops
  it, removes your message from the history, and adds your words to the
  front of your next message.
- Later in the reply, the controller stops the voice. The history keeps the
  text that had started playing, followed by an ellipsis.

## Reactions

`react.rs` holds a bank of short reactions, such as "Mm-hmm." and "Oh no.",
recorded once in the current voice and saved in `~/.cache/milo/clips`. milo
plays a clip when a reply starts with that reaction, and plays a quiet
listening sound when you pause in the middle of a thought or talk for a
while. `--no-backchannel` turns off the listening sounds.

## Tools

One model talks and acts. Every request carries the tool definitions, and the
model answers with text, with tool calls, or with both.

A turn runs in *rounds*. Each round is one streamed request. If the round has
tool calls, the controller waits until the turn is committed, runs the calls
in order, adds the results to the history, and starts the next round. A
speculative turn never runs a tool.

When you start to speak, milo holds the next call. When your words interrupt
the turn, each call that hasn't returned gets a result that says that it was
stopped, and your message goes into the same history.

Some results arrive after their turn, such as a timer that finishes or an
action that waited for your approval. When nobody is speaking, the controller
commits a turn whose message is `(result)` followed by the text.

Memory is the file `~/.config/milo/memory.md`. Its last 60 lines go into the
system prompt.

## Computer use

The `computer` tool takes a task name and a list of steps, and the panel
shows them as one card. One approval covers the whole plan. The model then
carries out the plan with the tools `go`, `click`, `type`, `press`, `scroll`,
`read`, `menu`, `look`, `tab`, and `close_tab`, one call in each round.

The model works in one *place* at a time: a browser tab, or the window of an
app. Each request ends with a numbered list of the controls in the place, and
with a picture of it when the model is likely to need one. A click names a
control by its number, by its exact name, or by a description of what the
picture shows.

A task ends when the model replies without a tool call, when you click
**Stop**, or after 40 actions. milo then asks the model to rewrite the notes
about each place that the task worked in.

### Browser tabs

`crates/browser` connects to a running Chromium browser over remote
debugging. `page.js` runs in the tab and lists the controls in the viewport.
Clicks, text, and keys go through the DevTools Protocol, so the tab doesn't
have to be in front. milo opens websites in one window of its own, behind
your windows. If a tab already has the exact address open, milo works in
that tab.

### App windows

`swift/MiloControl` walks the Accessibility tree of the window and captures
the window with ScreenCaptureKit. A click tries the control's Accessibility
action first, and then a mouse click posted to the app's process. Typed text
and keys also go to the app's process, so they don't reach the app that
you're using.

Background input uses private functions from the SkyLight framework. Apple
can change them in a macOS update.

### Test in a virtual machine

`milo --task` runs one task without the voice or the panel and exits.
`scripts/sandbox.sh` runs it in a [Lume](https://github.com/trycua/cua)
virtual machine named `milo-sandbox`:

```sh
scripts/sandbox.sh up
scripts/sandbox.sh push
scripts/sandbox.sh task "Search YouTube for cats" "Open youtube.com" "Search for cats"
scripts/sandbox.sh pull /tmp/last-run
```

## Models

A model name is `PROVIDER/MODEL`. `config.rs` has the providers: `openai`,
`anthropic`, `google`, and `openrouter`. The default model is
`openai/gpt-6.2-sol`.

milo looks for the API key of a provider in `~/.config/milo/auth.json`, and
then in the provider's environment variable. If it finds none, it asks for
the key at start, checks it with a request to the provider, and
saves it in `auth.json`. The app asks in a window, and the command asks in
the terminal.

milo uses up to three models: the main model, `small_model` for the notes
about a place, and `vision_model` to find a thing in a picture. The last two
default to the main model.

## Voice

milo starts `sidecar/voice_server.py` with the Python in `~/.cache/milo/venv`
and streams audio from it. If that Python environment is missing or its
packages have changed, milo first creates it with `uv`. The app has its own
copy of `uv`. Requests of the same turn share a session, so a
reply sounds like one delivery instead of separate sentences. Without
`--voice-ref`, the server creates the voice once from `--voice-design` and
saves it in `~/.cache/milo/voices`.

## Panel

The panel is a SwiftUI view from `swift/MiloNotch`, linked into the binary.
milo and the panel exchange JSON messages: milo sends the status and the
cards, and the panel sends back the action that you clicked. The keyboard
shortcuts are Carbon hot keys, which work in any app.

## The app

`scripts/bundle.sh` builds `target/Milo.app` from the binary, `uv`, and the
files in `app`.

To release a version, run `scripts/release.sh VERSION`. The script sets the
version, has Apple notarize the app, makes `target/Milo.dmg`, and publishes
it on GitHub as the release `vVERSION`.

When the binary runs inside the app, it writes its output to
`~/Library/Logs/Milo/milo.log`. Before it loads anything, it asks the panel
whether a setup window is needed: at the first start, when the API key is
missing, or when a permission is missing. It waits until that window is
finished, so milo never listens without the permissions. At each start, the app asks GitHub
for the latest release, and the menu offers a newer one.

The settings window and `config.json` hold the same settings. The window
sends each change to `app.rs`, which saves it. A change to the model, the
voice, or the audio devices stops the conversation and starts the app again.

## Files

| Path | Contents |
|---|---|
| `~/.config/milo/config.json` | The models and the settings. |
| `~/.config/milo/auth.json` | The saved API keys. |
| `~/.config/milo/memory.md` | Memory, one fact per line. |
| `~/.config/milo/tools.json` | Custom tools. |
| `~/.config/milo/skills/` | Notes about each app and website. |
| `~/.cache/milo/` | The voice server's Python environment, designed voices, reaction clips, and the turn models. |
| `~/.cache/huggingface/hub/` | The voice models. |
| `~/Library/Application Support/FluidAudio/Models/` | The speech recognition and echo removal models. |
| `~/Library/Logs/Milo/milo.log` | The app's log. |
| `$TMPDIR/milo-voice-PORT.log` | The voice server's log. |
| `$TMPDIR/milo-computer/` | The picture, the list, and the action of each step of the last plan. |
