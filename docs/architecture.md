# milo architecture

This document explains how milo works, for readers who are about to read the
code. For installation and use, see the [README](../README.md).

## Overview

milo is one Rust binary. It runs a pipeline from the microphone to the speakers
and starts up to two child processes: the Rapid-MLX model server, when the
model runs on your Mac, and a Python voice server for the Breeze voice. The following diagram shows how audio and
text move through the pipeline:

```text
microphone                                                         speakers
    |                                                                  ^
    v                                                                  |
audio input callback                                 audio output callback
    |                                                     |            ^
    | mic, resampled to 16 kHz mono        echo reference |            |
    v                                                     v            |
capture thread <------------------------------------------+            |
    |                            |                                     |
    | mic + echo reference       | mic, or mic with echo removed       |
    v                            v                                     |
speech: echo removal,        turn: voice activity,                     |
streaming recognition        Smart Turn, second pass                   |
    |                            |                                     |
    +-------------+--------------+                                     |
                  | events                                             |
                  v                                                    |
             controller <------------> tools                           |
                  |                                                    |
                  v                                                    |
             reply task: model stream, chunker, tool calls             |
                  |                                                    |
                  v                                                    |
             voice thread: voice server or Phonon                      |
                  |                                                    |
                  v                                                    |
             gate: holds audio until the turn commits -----------------+
```

## Crates

The workspace has the following crates:

| Crate | What it does |
|---|---|
| `crates/milo` | The `milo` binary. `main.rs` has the command-line options, the system prompt, start-up, and shutdown. `controller.rs` runs the conversation. `reply.rs` streams a reply from the model and shapes it for speech. `tts.rs` runs the voice thread and the gate. `react.rs` records and picks reaction clips. `capture.rs` runs the capture thread. `tools.rs` has the tools and memory. `computer.rs` has the tools that act in your apps. `skills.rs` keeps the notes that milo learns about each app and website. `config.rs` reads the providers, the models, and the API keys. |
| `crates/audio` | Microphone and speaker I/O through cpal, lock-free ring buffers, resampling, echo reference alignment, and playback with flush and onset timing. |
| `crates/speech` | Rust bindings to `swift/MiloSpeech`, a Swift static library that wraps FluidAudio: LocalVQE echo removal, Parakeet EOU streaming recognition, and Parakeet TDT 0.6B v2 for the second pass. Its build script runs `swift build` and links the result. |
| `crates/turn` | Voice activity detection (Silero VAD) and end-of-turn detection (Smart Turn v3.2) through ONNX Runtime, and the second-pass thread. It downloads both models on first use. |
| `crates/llm` | The client for OpenAI-compatible servers, which streams text and tool calls over chat completions or the OpenAI Responses API, the sentence chunker, and the Rapid-MLX launcher. |
| `crates/voice` | Starts the Python voice server for Breeze, and streams audio from it. The server script, `sidecar/voice_server.py`, is embedded in the binary. |
| `crates/notch` | Rust bindings to `swift/MiloNotch`, the panel in the notch. |
| `crates/control` | Rust bindings to `swift/MiloControl`, a Swift static library that reads the front window through the macOS Accessibility API, and clicks, types, presses keys, and chooses menu items. |
| `crates/browser` | The client for the Chrome DevTools Protocol. It connects to a running Chromium browser, lists and closes tabs, reads a page through `page.js`, and sends clicks, text, and keys. |
| `crates/phonon` | The Phonon voice, which runs in-process through the `ptts` crate. milo uses it with `--tts phonon` and when the voice server can't start. |

The `setup.sh` script installs the tools, creates the voice server's Python
environment from `sidecar/requirements.txt`, and builds `crates/milo`.

## Threads, tasks, and processes

milo runs the following threads, tasks, and processes:

- **Audio callbacks.** cpal calls the input and output callbacks on Core Audio
  threads. The callbacks only touch ring buffers and atomics, with no
  allocation, locks, or logging.
- **Capture thread.** Reads 20 ms blocks of 16 kHz mono microphone audio, each
  paired with the time-aligned echo reference. It pushes each block to the
  speech recognizer, and records an audio clock that maps recognizer timestamps
  to wall-clock time. With `--no-aec`, it also pushes each block to the turn
  detector, which runs Silero VAD on this thread.
- **Speech task.** Inside MiloSpeech, one Swift task passes the pushed audio
  through LocalVQE and the streaming recognizer, and calls back into Rust with
  partial transcripts, end-of-utterance events, and the audio with the echo
  removed.
- **Detect thread.** With echo cancellation on, feeds the audio with the echo
  removed to the turn detector, so that voice activity, Smart Turn, and the
  second pass don't hear milo's own voice.
- **Turn thread.** Runs Smart Turn when a voice segment ends.
- **Refine thread.** Runs the second-pass transcription.
- **Voice thread.** Takes synthesis jobs from a channel one at a time, and
  passes the audio to the gate.
- **Tokio runtime.** Two worker threads run the controller task, one reply task
  for each round of a turn, one task that runs the tool calls of a round,
  timers, and cache warming requests.
- **Child processes.** `rapid-mlx serve` and `voice_server.py`, each in its own
  process group.

## The controller

`Controller` in `controller.rs` owns all conversation state. Every other part
of milo sends it an `Event` over one unbounded channel: partial transcripts and
end-of-utterance events from the recognizer, voice start and end and turn
verdicts from the detector, second-pass results, reply progress from the reply
task, voice progress from the voice thread, and tool results. The controller
handles each event and then runs `tick`, which checks its timers. While a turn
exists, it ticks every 5 ms. Otherwise, it sleeps until the next deadline.

A *turn* is one reply, with the tool calls that the model makes on the way to
it. A turn starts as *speculative*: milo generates and
synthesizes the reply, but the gate holds its audio. When the controller
determines that you've finished, it *commits* the turn: the held audio plays,
and your message goes into the history. There's at most one turn at a time.

The controller also draws what you're saying as a dim line in the terminal,
prints `you:` and `milo:` lines for each committed turn, and prints a timing
line after each reply.

## A turn, step by step

The following sections follow one turn from your first word to milo's last
sound.

### Streaming recognition

Parakeet EOU processes audio in 160 ms steps. Each partial transcript carries
everything that you've said since the recognizer was last cleared, and the
audio-clock time of the last recognized token. The recognizer signals the end
of an utterance after 160 ms of silence. On an end of utterance, the controller
rearms the recognizer and keeps the transcript, so further speech extends it.
On commit, the controller clears the transcript. Fillers such as "um" and "uh"
don't count as words.

### Voice activity and the turn verdict

The detector runs Silero VAD on 32 ms frames. Two voiced frames in a row start
a voice segment and send a `Start` event. After 200 ms of silence, the detector
sends the audio since the last commit, up to 30 seconds, to Smart Turn, which
scores the last 8 seconds for whether you've finished. A score above 0.5 is a
*finished* verdict; otherwise, the verdict is *unfinished*. The verdict arrives
in an `End` event, with the time when your voice stopped.

A verdict counts only if the last recognized token came no more than 1.5
seconds after the voice end that the verdict describes, so that an old verdict
can't end a newer utterance. A transcript that ends with a question mark counts
as finished, whatever the score.

### Second pass

After 128 ms of silence, the detector also sends the audio since the last
commit to Parakeet TDT 0.6B v2 on the refine thread. If the result has
different words from the streaming transcript, the result replaces the
transcript, and the controller restarts the speculative reply from it. Until
your voice resumes, the controller ignores later streaming partials.

If neither recognizer finds words in a voice segment with 190 to 1,500 ms of
voice, the controller treats it as a wordless sound, such as "hmm" or a short
laugh, and uses `(hmm)` as your message.

### Speculative reply

When no turn is running, the controller starts a speculative turn as soon as
one of the following happens:

- No new partial transcript arrives for 250 ms.
- The second pass returns.
- A finished verdict arrives.

A partial transcript with different words cancels the speculative turn, and an
end of utterance with different words replaces it. To cancel a turn, the
controller aborts its reply task, sets the turn's stop flag for the voice
thread, and closes the gate so that the turn's audio is dropped.

### End of turn

On every tick, the controller checks the following rules in order, and the
first rule that matches applies:

1. Three seconds after the last partial transcript, commit. If the second-pass
   transcript ends on a connector word, such as "and" or "the", the limit is
   six seconds.
1. While the VAD detects voice, wait.
1. If you said only a short acknowledgment, such as "yeah" or "okay", or made a
   wordless sound, and milo's last reply didn't end with a question, wait until
   1.6 seconds after your voice stopped, so that a following sentence can join
   it.
1. While a second pass is running, wait for up to 450 ms.
1. With a finished verdict, commit 350 ms after your voice stopped. If the
   transcript ends with punctuation and the score is 0.9 or more, commit after
   220 ms.
1. With an unfinished verdict, commit 1,500 ms after your voice stopped. The
   wait is 600 ms for three words or fewer, for a phrase such as "what's it
   called", or when the second-pass transcript ends with sentence punctuation;
   3 seconds when the transcript ends on a connector word; and 500 ms
   longer after a listening sound.
1. Without a verdict, commit 80 ms after the end of utterance, or 800 ms after
   the last partial transcript.

### Commit

When it commits a turn, the controller does the following:

- Clears the detector's audio history and the recognizer's transcript.
- Plays the reply's opening reaction clip, if the reply has one. For more
  information, see [Reactions](#reactions).
- Commits the gate, which writes the held audio to playback and lets later
  audio play as it arrives.
- Adds your message to the history, prefixed with the local date and time in
  brackets. The system prompt tells the model to use the time when it matters
  and never to read it out.
- Prints your message and the reply text so far.

### Reply text

`reply::generate` streams the reply from the model and shapes it for speech:

- **Delivery note.** The system prompt asks the model to start each reply with
  a note in square brackets, such as `[amused, quick]`. The reply task removes
  the note from the text and cleans it: it drops flat or negative words, such
  as "dry" or "bored", keeps at most two words, and uses `warm, easy` if
  nothing is left. The note goes to the voice as the delivery direction for
  every part of the reply. A reply of only `[quiet]` says nothing.
- **Opening reaction.** If the reply starts with a reaction from the reaction
  bank, such as "Oh no," that part is split off and played from its clip.
- **Chunking.** `llm::chunker` removes Markdown and emoji and splits the text.
  The first chunk ends at the first clause boundary after three words, at the
  end of a sentence, or after 12 words, and goes to the voice at once. The rest
  of a short reply goes to the voice as one request when the model finishes, so
  that Breeze delivers it in one piece. A long reply, such as a story, goes to
  the voice in pieces of about 40 words while the model is still writing. The chunker doesn't split numbers such
  as 3.5 or abbreviations such as "Dr."
- **Length.** The model decides how long a reply is. The system prompt asks
  for short replies in small talk and for a complete answer when you ask for
  something longer. The only limit in code is 1,200 tokens for each reply.
- **Questions.** If any of milo's last three replies ended with a question, a
  final question is dropped from this one.
- **Sounds.** Only the sounds that the voice engine supports, such as `(laugh)`
  and `(sigh)` for Breeze, stay in the text. Phonon removes them all.
- **Trailing off.** If your transcript ended in the middle of a sentence, only
  the first chunk is spoken.

The history records the delivery note and the text that went to the voice.

### Voice and playback

The reply task sends `Job::Say` messages to the voice thread. Each job has the
turn ID, the text, the delivery note, and the turn's stop flag. The voice thread
synthesizes one job at a time and passes the audio to the gate, which does the
following:

- Keeps audio only for the current turn, and drops audio from canceled turns.
- Holds the audio until commit, and then writes it to playback.
- Records the sample where each chunk's audio starts, so that after an
  interruption it can tell which text had started playing.

The Breeze voice server runs its own generation loop. It encodes the reference
voice into a key-value cache once, and copies that cache for each request. Each
request carries the turn ID as its `session`. A later request in the same
session continues the cache and the audio decoder state of the earlier one, so
the reply sounds like one delivery instead of separate sentences. A canceled
request ends the session, and the context is capped at 25 seconds of audio.

`crates/audio` resamples the voice's audio to the output device's rate. The
output callback writes everything that it plays, silence included, to an echo
reference ring, and the capture side aligns that reference with the microphone
so that it leads the echo slightly. A flush drops the queued audio with a 5 ms
fade. Playback also records when sound reaches the speaker after a silence,
which the controller uses for the timing line.

### End of a reply

A turn ends when the reply task is done, the voice thread has synthesized every
job, and playback is empty. The controller then adds the reply to the history.

### Timing line

After each reply, the controller prints a dim line with the following fields,
separated by `|`:

- What ended the turn (`turn` with the Smart Turn score, `patience`, `eou`,
  `timeout`, `trailed off`, or `result`), and how long after your
  voice stopped the commit came.
- `prepared`: when the reply started, relative to the commit. A negative value
  means that the reply started before the commit.
- `llm ttft`: the time from the start of the reply to the first token.
- `tts first`: the first synthesized audio, relative to the commit.
- `audio`: the first sound at the speaker, relative to the commit.
- `v2v`: the time from the end of your speech to the first sound.

## Interruptions and yielding

While a committed reply is generating or playing, your voice can stop it in two
ways:

- **Barge-in**, from recognized words. A partial transcript with two or more
  new words stops the reply, and so does an end of utterance with any new words
  more than one second after commit. Words that are only acknowledgments, such
  as "yeah" or "uh-huh", don't count.
- **Yielding**, from voice activity alone. Your voice must last 250 ms within
  1.5 seconds of the reply's first sound, or 700 ms later in the reply.
  Yielding is on only with `--no-aec`, for headphones, because on speakers the
  reply's own sound can trigger it.

Either way, the result is one of the following:

- **Undo.** If the reply hasn't played yet, or started playing less than 1.5
  seconds ago, the controller drops the reply, removes your message from the
  history, and keeps your words as a *carry*. The carry is prefixed to your
  next message, so the next reply answers everything that you said. Tool
  results are never undone.
- **Interrupt.** Later in the reply, the controller stops the voice, aborts the
  reply task, and flushes playback. Aborting the reply task drops the response
  stream, which asks the server to cancel the request. The history records the
  text whose audio had started playing, if any, followed by an ellipsis. The
  system prompt tells the model what the ellipsis means.

An interrupt also stops the tool calls of the turn. See
[Tools](#tools).

## Reactions

`react.rs` holds a bank of 17 short reactions, such as "Mm-hmm.", "Oh no.",
"Really?", and a laugh, each with a delivery direction. At start, milo loads
two takes of each reaction from `~/.cache/milo/clips`, and records any that are
missing in the current voice. The cache key includes the voice settings, so a
new voice records a new set. With Breeze, milo also registers the first take of
each reaction with the voice server, so that a reply can continue from it.

milo uses the bank in the following ways:

- **Reply openers.** If the reply starts with a reaction from the bank, milo
  plays the reaction from its clip instead of synthesizing it, at commit or as
  soon as the reaction arrives after commit. The first chunk asks Breeze to
  continue from the clip's words and audio. The same reaction doesn't open two
  replies in a row; the second time, the voice speaks it.
- **Listening sounds at a pause.** After an unfinished verdict, with at least
  three words heard and 280 ms of silence, milo plays a reaction at a lower
  volume without committing the turn. It plays the prepared reply's opener if
  that's a listening reaction, or a generic "mm-hmm", "yeah", or "right". At
  most one plays every 2.5 seconds, and the patience wait grows by 500 ms. A
  reaction played this way doesn't play again at commit, and a reply that
  starts within 6 seconds drops the same reaction from its text. No opener clip
  plays within 1.2 seconds of a listening sound.
- **Listening sounds while you talk.** After 3.5 seconds of continuous speech
  with at least five recognized words, a generic reaction plays quietly, at
  most once every 6 seconds.

`--no-backchannel` turns off the listening sounds.

## History and the prompt cache

The history starts with the system prompt and only grows, so that each request
starts with the previous one and the server can reuse its prompt cache. When the
history passes 80 messages after a reply, the controller prepares a shorter
history: the system prompt, rebuilt with the current memory, and about the last
30 messages, starting at one of your messages. A tool call and its result
each count as a message. The server has to read a
shortened history from the start, which takes a few seconds, so the controller
warms the server's cache with it in the background and keeps using the full
history until that finishes.

These steps apply to Rapid-MLX. Other providers keep their own caches, and
milo sends them no warming requests.

At start, milo warms the cache with the system prompt and the tool definitions
through Rapid-MLX's
`/v1/cache/prefill` endpoint. If the server doesn't have that endpoint, milo
sends a one-token chat request instead.

Rapid-MLX sizes its prompt cache from the memory that is free after the model
loads, which can be too small to hold one conversation with a large model. When
the cache is too small, every request reads the whole history again. milo sets
the cache to one fifth of the Mac's memory through two environment variables,
`RAPID_MLX_PREFIX_CACHE_MAX_BYTES` and `APC_MEMORY_MAX_GB`, because the server
applies both limits. If you set either variable yourself, milo keeps your
value. The server still removes cache entries when the Mac runs low on memory.
milo also sets `APC_MEMORY_RESERVE_GB` to 1, so that the server keeps 1 GB
free instead of a tenth of the GPU memory before it removes entries.

## Tools

One model talks and acts. Every request to the model carries the tool
definitions, and the model answers with text, with tool calls, or with both.
The system prompt tells the model to say a few words, such as "one sec", in
the same reply as its first tool call.

### Rounds

A turn runs in *rounds*. Each round is one streamed request:

1. The reply task streams the text to the voice thread as usual, and collects
   the tool calls that arrive on the same stream.
1. If the round has no tool calls, the turn ends like any other reply.
1. Otherwise, the controller waits until the turn is committed, adds the
   model's message and its calls to the history, and starts a task that runs
   the calls in order.
1. Each result goes into the history as a tool message. When the last call
   returns, the controller starts the next round with the same turn.

A speculative turn never runs a tool: the controller holds the calls until the
turn commits, and drops them if you keep talking.

A turn stops calling tools after 60 rounds. The next round then goes out
without the tool definitions, so the model can only speak.

If every call of a round is `remember`, and the model also said something in
that round, the turn ends there, so that milo doesn't speak twice.

### Your voice stops the work

The task that runs the calls waits before each call while voice activity is
on, and for 1.2 seconds after it ends, for up to 8 seconds in all. Your first
sound therefore holds the next action, before any word is recognized.

When your words interrupt the turn, the controller aborts the task. Each call
that hasn't returned gets a tool message that says that it was stopped and
might or might not have happened. Your message then goes into the same
history, so the model sees what it did, what was cut off, and what you said,
and decides what to do next. During a task, milo also captures the place
again, because the action that was cut off might have changed it.

### Results

The `weather` tool times out after 8 seconds. The `open` tool and
the commands from `tools.json` time out after 20 seconds. Tool results are cut
to 700 characters, except for the results of the tools that act in your apps.
When a new round starts, every earlier tool result that is longer than 300
characters is cut to that length, except for the answer of `computer` and the
newest page text from `read`.

Some results arrive after their turn: a timer that finishes, or an action that
waited for your approval in the panel. These go to the controller as an
`Aside` event. When nobody is speaking and nothing is playing, the controller
commits a turn whose message is `(result)` followed by the text, and the model
tells you about it, or goes on with the work that it describes.

### Memory

Memory is the plain-text file `~/.config/milo/memory.md`, with one `- fact` per
line. The last 60 lines go into the system prompt, where the prompt
has `{memory}`, at start and after each history cut. `remember` adds a line unless the same fact is already
there. `forget` removes every line that contains all the words of its argument
that are longer than two letters.

## Model server and voice server

milo starts the model server, the speech recognizer, and the voice at the same
time, on separate threads. It then loads the turn detector, opens the audio
devices, starts the capture thread, builds the reaction bank, starts the voice
thread, and warms the model's cache. When everything is ready, it prints
`listening`.

### Providers and models

`config.rs` turns a model name into a client. A name is `PROVIDER/MODEL`. When
the part before the first slash isn't a provider, the whole name is a model of
the `local` provider, which is Rapid-MLX. milo has a table of built-in
providers, each with a base URL, an API, and the environment variable for its
key. `~/.config/milo/config.json` can change a built-in provider or add one.

milo looks for the API key of a provider in the following order: the `key`
setting in `config.json`, `~/.config/milo/auth.json`, and the provider's
environment variable. `milo auth` writes `auth.json` with permissions that let
only your user account read it.

The client builds each request as JSON and then adds the `options` of the
provider and of the model, so an option can set or remove any field. For chat
completions, the client reads text from `delta.content` and tool calls from
`delta.tool_calls`. For the Responses API, it reads text from
`response.output_text.delta` events and tool calls from
`response.output_item.done` events. Requests to Rapid-MLX also turn off
thinking and can be canceled. Requests to other providers get 4,000 tokens of
room on top of the reply limit, for models that reason before they answer.

The client asks for the token counts of each streamed request, with
`stream_options` on chat completions, and reads them from the last event of
the stream. When the stream is dropped, it adds the counts to one total for
the process, and the cost if the model has a price. `config.rs` holds the
OpenAI prices and the multiplier of each service tier, and a `price` in
`config.json` replaces them. The controller prints the difference since the
last turn when a turn ends or is interrupted.

milo uses up to three models: the main model, a model that writes the notes
about a place, and a model that finds a thing in a picture. The last two
default to the main model.

### Rapid-MLX

When one of the models belongs to the `local` provider, milo sends
`GET /v1/models` to `127.0.0.1` on `--port` (8000) before it starts a server:

- If the answer has a `2xx` status code, milo uses that server as is.
- If something else answers, milo waits for it to become ready.
- If nothing answers, milo starts `rapid-mlx serve` with the model from
  `--model`.

milo starts the server in its own process group, listening only on
`127.0.0.1`, with telemetry and the version check off. It passes
`--hybrid-cache-entries 16`, so that the requests for notes and pictures don't
push the conversation out of the cache, and `--watchdog-ppid` with milo's process ID, so that the
server exits if milo stops unexpectedly. The server's output goes to
`$TMPDIR/milo-rapid-mlx-PORT.log`. milo waits up to 20 minutes for the server
to answer, which includes the first download of the model.

On exit, milo sends `SIGTERM` to the server's process group, waits up to 5
seconds, and then sends `SIGKILL`.

### Voice server

For Breeze, milo first sends `GET /health` to `127.0.0.1` on `--tts-port`
(8881). If a Breeze voice server answers, milo uses it, and any other answer is
an error. Otherwise, milo writes the embedded `voice_server.py` to
`~/.cache/milo`, starts it with the Python in `~/.cache/milo/venv` in its own
process group, and waits up to 30 minutes for it to answer. The server's output
goes to `$TMPDIR/milo-voice-PORT.log`.

The server loads the model, prepares the reference voice, speaks a warm-up
line, and then serves the following endpoints:

- `GET /health`: returns the engine name, `breeze`.
- `POST /speak`: takes `text`, `instruct` (the delivery note), `session`, and
  `lead`, and streams 16-bit little-endian PCM at 24 kHz. Closing the
  connection cancels the request.
- `POST /lead`: registers a reaction clip, sent as 16-bit PCM with `id` and
  `text` query parameters, that a later `/speak` request can continue from.

Without `--voice-ref`, Breeze creates the reference voice once from
`--voice-design`, by speaking a fixed line in that voice, and saves it as a WAV
file in `~/.cache/milo/voices`.

The server checks every second whether milo is still its parent process, and
exits if it isn't. On exit, milo sends `SIGTERM` to the server's process group,
waits up to 3 seconds, and then sends `SIGKILL`. If the voice server can't
start for any reason, milo logs a warning and loads Phonon instead.

### Speech recognizer

FluidAudio downloads the Parakeet and LocalVQE models on first use, and Core ML
compiles them. Starting the recognizer blocks until the models are loaded and
warmed up.

### Which tool

The tool descriptions tell the model to use the most direct tool: `media` for
what is playing, `close_tab` and `show_tab` for a browser tab, `file` to write
text into a file, `message` to send a text message, `open` for an app or a
site, and `computer` only for the rest. `media` pauses
every playing `video` and `audio` element in every tab over the browser
connection and marks them, so that `play` resumes the same ones. When nothing
in the browser was playing, it posts the system play key.

`message` runs the `imsg` command. `imsg chats --json` lists your
conversations, each with its number, the name from Contacts, the address, and
the service. milo leaves out group conversations, matches the name that the
model gave against the names, and prefers an iMessage conversation. If no
name matches, the tool returns the names, so that the model can call it
again with the right one. After you approve, `imsg send --chat-id` sends the
text to that conversation.

### Computer use

The `computer` tool takes a short task name and a list of steps. The model
writes both from what you said. The panel shows the steps as one card, and one
approval covers the whole plan. A new plan dismisses the approval card of the
plan before it. The tool exists only when the panel is on.

`computer` doesn't act. It starts a *task*: it shows the card with the last
action and a **Stop** button, connects to the browser, and answers with the
apps and the tabs that are open and the notes about the place. From then on,
the model carries out the plan itself with the tools `go`, `click`, `type`,
`press`, `scroll`, `read`, `menu`, `look`, `tab`, and `close_tab`, one call in
each round. Without a task, these tools do nothing and say so. A click names a
control by its number in the list, by its exact name, or by a description of
what the picture shows.

The picture and the list of the place aren't part of the history. milo keeps
only the newest ones, and adds them to the end of each request: to your
message when it is the last message, or as a message of their own after a tool
result. If a server refuses a request that has a picture, milo sends the lists
alone from then on.

The list goes with every request. The picture goes only when the model is
likely to need it, because a picture costs more tokens than the list: when a
task starts, when the model calls `look`, when an action lands on a new page
or window, when an action fails or leaves the list unchanged, and when the
list has fewer than five controls. In the list that milo sends, a control
whose name repeats the name of one of the four controls before it is written
as `same name as [N]`. Outside a
task, `see` always sends both.

A task ends when a turn ends, which is when the model replies without a tool
call. milo then removes the card, puts back the app that was in front, and
writes the notes. If the plan has the `show` flag, which the model sets when
you asked to see the result, milo brings the place to the front instead. When
your voice interrupts a turn, the task stays open, and your next message says
that the work is paused. The model can then go on in the same place without a
new plan. A task also ends when you click **Stop** on the card, and the model
is told that you stopped it. A task refuses further actions after 40.

The model works in one *place* at a time: a browser tab, or the window of an
app. Neither has to be in front. milo keeps the place and its own browser
window for as long as it runs, so a plan
can go on where the one before it ended. A plan starts in the place of the plan
before it if that place still exists. Otherwise it starts nowhere, and the
model has to go to a place with `go` or `tab` before any other tool works.

#### Notes about a place

`skills.rs` keeps notes about each app and website that milo has
worked in, in `~/.config/milo/skills`. A file is named after the app or the
host of the site, starts with that name as a heading, and holds up to 12
notes, one per line.

- **Recording.** milo records each action with its result, the place
  that it ended in, and whether the action failed, changed nothing, or
  repeated an earlier one.
- **Writing.** When a task ends, milo asks the small model to rewrite the
  notes of each place where the task took two actions or more. The model gets
  the notes so far, the task, the recorded actions, and how the task ended.
  milo skips a place that already has notes when no action there was wasted.
  The request runs after the result is spoken. `milo --task` waits for it
  before it exits.
- **Reading.** The answer of `computer` holds the notes of the place that the
  task starts in, and of up to two places that the task or the steps name.
  When the model arrives in another place, milo adds the notes of that place
  to the result of that action.

#### Browser tabs

`crates/browser` reads `DevToolsActivePort` in the profile folder of Chrome,
Brave, Edge, Arc, Vivaldi, or Chromium, and opens one WebSocket to the browser.
Chrome asks you to allow each new connection, so milo connects once at
start-up and keeps the connection. While Chrome asks, milo brings Chrome to the
front, and then puts the app that was in front back. If no browser allows
remote debugging, websites go through the app route that the next section
describes.

- **Reading.** `page.js` runs in the tab and walks the document, including
  open shadow roots and frames from the same site. It lists each control that
  is in the viewport and that a click at its center would reach, with its
  kind, name, value, and state, and the text between the controls. A video is
  listed with whether it plays and where it is. The walk takes about 20
  milliseconds. The picture comes from `Page.captureScreenshot`.
- **Covered windows.** Chrome treats a tab that another window covers as
  hidden, and such a page loads only part of its content. milo turns on
  `Emulation.setFocusEmulationEnabled` for each tab that it uses, which makes
  the page load and draw as if it were in front.
- **Acting.** A click scrolls the control into view and sends mouse events to
  its center through `Input.dispatchMouseEvent`. Text goes in through
  `Input.insertText` after milo focuses the control and selects what it held,
  and milo reads the control back. A pop-up menu takes its choice as typed
  text. If a click opens a new tab, the place moves to that tab.
- **Websites that milo opens.** `Target.createTarget` with `newWindow`
  and `background` makes one window that stays behind your windows. milo
  reuses that window for every address.
- **Settling.** milo polls the address, the load state, the number of
  elements, the scroll offset, and the length of the text every 300
  milliseconds until two polls match.

#### App windows

milo remembers the window by its ID, so the model stays in it even if you
use another window of the same app. `go` brings the app to the front,
because many apps turn off their menus and stop drawing while they are
covered. milo puts the app that was in front back when the task ends, unless
the plan has the `show` flag.

Before each request, milo estimates the size of the prompt. If the estimate is
over 6,500 tokens, milo sends the list without the picture, because the model
server limits a request that has a picture to 8,192 tokens. Results of earlier
actions are cut to 300 characters, except the text that `read` returned.

The loop works as follows:

1. `swift/MiloControl` walks the Accessibility tree of the window and collects
   the controls that can be used, with their names and frames. When the window
   holds a web page, the walk lists the page first, and then up to 24 controls
   of the app's own window, without the tabs that aren't selected. The walk
   skips anything outside the window's frame, skips a control that has the
   same name as the control that contains it, reads only the visible rows of
   tables and lists, and stops at 150 controls or 2.5 seconds.
2. It captures the window with ScreenCaptureKit, which returns the whole
   window even when other windows cover it, and scales the picture to about
   0.64 megapixels, which is what Gemma's image processor keeps of any image.
   Nothing is drawn on the picture.
3. The model gets the picture in a user message, with the numbered list,
   and calls one tool.
4. milo carries out the action, waits for the window to settle, and captures
   it again. Only the newest picture goes into the next request.

To wait for the window to settle, milo takes a small capture every 300
milliseconds until two in a row match and the page reports that it has loaded,
for up to 3 seconds, or 12 seconds after `go`.

The `click` tool takes the number of a control, or a description of something
in the picture. For a description, milo asks the model for the bounding box of
the thing, cuts out the third of the picture around that box, and asks again
on the closer view. It then asks the app which element is topmost at that
point, and clicks that element. If the app reports none, milo clicks the
point. When the model gives both a number and a description that don't
match, milo uses the description.

A click tries the following in order, and stops as soon as the window changes:

1. The control's Accessibility action. A row is selected, and a text field is
   focused.
2. A mouse click at the middle of the control, posted to the app's process.
3. A click through the system event stream. milo brings the app to the front,
   clicks, moves the pointer back to where it was, and brings back the app
   that was in front.

milo decides that the window changed when the list of controls differs, or
when a 24 by 24 grayscale copy of the picture differs by more than a small
amount. If an action changes nothing, the result says so, and tells the
model to try another way. If the same action leads to the same window a
second time, the result says so, and the fourth time the result tells the
model to stop and say what got in the way.

The `type` tool replaces what is in a text field and adds to what is in a text
area. Afterward it reads the control and puts what it contains in the result.
Typed text and key presses go to the app's process as key events, so they
don't reach the app that you're using.

The `go` tool handles a web address differently from an app name. If a tab
already has that exact address open, milo works in that tab. Otherwise, milo
uses a browser window of its own. The first time, it sends
Command+N to the default browser, which brings the browser to the front, and
then brings back the app that was in front. After that, it puts the address in
that window's address field and presses Return, which doesn't bring the
browser forward. For an app name, milo starts the app in the background, and
brings back the app that was in front if the new app takes its place. When
`go` fails, the model has no window to work in.

milo never works in a window that belongs to the system, such as a permission
prompt.

The helper keeps to one window of the app. It moves to a new window when an
action opens one and the app makes it the focused window, for example after
**File** > **New**. The list also holds the following:

- The items of a menu that is open on top of the window. A button that opens
  such a menu doesn't answer the press until the menu closes, so the helper
  waits for 1.2 seconds and then reads the menu.
- The field that has the cursor, when the window's own list doesn't have it,
  such as the name of a file that Finder is renaming.
- Files that Finder shows as icons.

Before the helper types into a field by its number, it checks that the cursor
went into that field. If the cursor stays in another field, nothing is typed,
and the model gets an error. Text for a text area goes in at its end.
Outside a text field, for example in Calculator, the helper sends one key at a
time.

Background input uses private functions from the SkyLight framework, which
milo loads at run time:

- `SLEventPostToPid` posts an event to one process. Chromium and Electron apps
  ignore mouse events that arrive through the public `CGEventPostToPid`
  function. If the function is missing, milo uses the public one.
- `SLPSPostEventRecordTo` tells an app that its window became the key window
  or stopped being it, without changing which window is in front. Before milo
  posts mouse or key events, it tells the app in front that its window is no
  longer the key window, tells the target app that its window is, and sends
  the two records that make that window the key window inside the app. An app
  ignores key combinations and turns off menu items without all of these.
  Afterward, milo reverses the first two.
- `_SLPSGetFrontProcess` gives the process that is in front.

The private `_AXUIElementGetWindow` function gives the window ID that
ScreenCaptureKit needs. Apple can change any of these functions in a macOS
update.

After each action, the result lists the lines that left the window's list and
the lines that are new in it, or says that most of the window changed and
gives the window's title.

Each of these tools takes a `why` argument first, in which the model writes
what the window shows and why the action is next. milo logs it.

The walk leaves out anything that is less than 3 points wide or high inside
the window, because Chrome reports an element that its container clips away
with a frame that small. milo makes its own browser window 1120 by 760
points, so that the picture is close to full size.

Save and open dialogs run in a separate process, so key events that milo posts
to the app don't reach them. When typed text doesn't appear in a field, milo
sets the field's value through Accessibility instead.

The `see` tool captures the window in front, and adds its picture and its list
to the next request, so that the model can answer from them. It runs without
approval. During a task, it shows the place of the task again.

milo saves the picture, the list, and the action of each step of the last
plan in `$TMPDIR/milo-computer`.

For Chromium and Electron apps, milo sets the `AXEnhancedUserInterface` and
`AXManualAccessibility` attributes on the app, which makes them build their
Accessibility tree, and waits up to 3 seconds for the page to appear in it.
macOS grants the Accessibility and Screen Recording permissions to the
terminal app that starts milo.

### Test computer use in a virtual machine

`milo --task` sends one message to the model without the voice or the panel,
runs the rounds of tool calls, prints each action, and exits. `scripts/sandbox.sh` runs it in a macOS virtual machine, so
that a test never touches your desktop. The virtual machine reaches the model
server on your Mac through an SSH tunnel.

To set up the virtual machine once, do the following:

1.  Install [Lume](https://github.com/trycua/cua), and pull an image:

    ```sh
    brew install lume
    lume pull macos-sequoia-vanilla:latest milo-sandbox
    lume set milo-sandbox --memory 8GB --display 2880x1800
    ```

2.  Start the virtual machine in recovery mode, open **Utilities** >
    **Terminal**, run `csrutil disable`, and restart it. This lets you grant
    permissions from a script.
3.  In the virtual machine, grant Accessibility and Screen Recording to
    `/Users/lume/milo` and to `com.apple.sshd-session` in
    `/Library/Application Support/com.apple.TCC/TCC.db`, and allow `sudo`
    without a password.
4.  Add your public key in `~/.lume/keys/sandbox.pub` to the virtual machine's
    `~/.ssh/authorized_keys`.
5.  Save the following as `~/gui.sh` in the virtual machine. It runs a command
    in the desktop session:

    ```sh
    #!/bin/sh
    exec sudo launchctl asuser 501 sudo -u lume env DYLD_LIBRARY_PATH=/usr/lib/swift HOME=/Users/lume RUST_LOG=info "$@"
    ```

To run a task, start the model server by starting milo once, and then run the
following:

```sh
scripts/sandbox.sh up
scripts/sandbox.sh push
scripts/sandbox.sh task "Search YouTube for cats" "Open youtube.com" "Search for cats"
scripts/sandbox.sh pull /tmp/last-run
```

### Notch panel

The panel is a SwiftUI view in a borderless, non-activating `NSPanel` above the
menu bar, built from `swift/MiloNotch` and linked into the binary. AppKit needs
the main thread, so milo runs the AppKit event loop there and runs everything
else on a second thread. With `--no-notch`, milo skips the event loop and runs
on the main thread.

The `notch` crate and the panel exchange JSON messages. milo sends a `status`
message when its state changes, a `card` message to show a title, a body, and a
list of actions, and a `dismiss` message to remove a card. The panel sends back
the card's ID and the ID of the action that you clicked. `notch::ask` shows any
card and waits for the answer, and `notch::approve` is a card with **Decline**
and **Approve** actions. The `open` tool and custom tools call it before they
run a command. A tool that asks for approval doesn't wait for the answer: its
result says that it asked, and a task waits for approval, runs the command,
and reports the result as an aside. A declined or unanswered card reports
nothing. The panel
shows the newest card first, and a second card for the same command is dropped
while the first one waits.

While a card waits, or a task is paused because you spoke, the time stamp at
the start of each of your messages says so.

The panel registers its keyboard shortcuts as Carbon hot keys, which work in
any app and don't need the Accessibility permission. When the mute state
changes, the panel tells the `notch` crate, and the capture thread replaces the
microphone audio with silence while the microphone is muted.

The panel's window has a fixed size and ignores the mouse. Thirty times a
second, the panel checks whether the pointer is over the visible shape, and
accepts clicks only while it is. On the same timer, it reads the loudest level
since the last check for each side and moves the bars. The capture thread
reports the level of the audio that milo is playing, and the level of your
voice after echo removal, or of the microphone when echo removal is off.

### Shutdown

When you press Control+C, the controller stops the current turn. milo then
closes the audio streams, waits for the capture thread, shuts down the Tokio
runtime, and waits for the voice thread, which stops the voice server. Last, it
releases the speech recognizer and stops Rapid-MLX.

## Files

milo reads and writes the following files:

| Path | Contents |
|---|---|
| `~/.config/milo/memory.md` | Memory, one fact per line. |
| `~/.config/milo/tools.json` | Custom tools. |
| `~/.config/milo/config.json` | The models and the providers. |
| `~/.config/milo/auth.json` | API keys that `milo auth` saved. |
| `~/.config/milo/skills/` | Notes about each app and website. |
| `~/.cache/milo/venv/` | The voice server's Python environment, created by `setup.sh`. |
| `~/.cache/milo/voice_server.py` | The voice server script, written from the binary at each start. |
| `~/.cache/milo/voices/` | Designed Breeze voices. |
| `~/.cache/milo/clips/` | Recorded reaction clips, as raw 32-bit float samples. |
| `~/.cache/milo/silero-vad.onnx` | The voice activity model. |
| `~/.cache/milo/smart-turn-v3.2-cpu.onnx` | The end-of-turn model. |
| `~/.cache/huggingface/hub/` | The language model, Breeze, and Phonon, in the default Hugging Face cache. |
| `~/Library/Application Support/FluidAudio/Models/` | The speech recognition and echo removal models. |
| `$TMPDIR/milo-rapid-mlx-PORT.log` | The model server's log. `PORT` is the server's port. |
| `$TMPDIR/milo-voice-PORT.log` | The voice server's log. `PORT` is the voice server's port. |
