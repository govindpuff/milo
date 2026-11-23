# milo architecture

This document explains how milo works, for readers who are about to read the
code. For installation and use, see the [README](../README.md).

## Overview

milo is one Rust binary. It runs a pipeline from the microphone to the speakers
and starts up to two child processes: the Rapid-MLX model server and, for the
Breeze voice, a Python voice server. The following diagram shows how audio and
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
             controller <------------- tool agent                      |
                  |                                                    |
                  v                                                    |
             reply task: model server stream, chunker                  |
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
| `crates/milo` | The `milo` binary. `main.rs` has the command-line options, the system prompt, start-up, and shutdown. `controller.rs` runs the conversation. `reply.rs` streams a reply from the model and shapes it for speech. `tts.rs` runs the voice thread and the gate. `react.rs` records and picks reaction clips. `capture.rs` runs the capture thread. `tools.rs` has the tool agent, the tools, and memory. |
| `crates/audio` | Microphone and speaker I/O through cpal, lock-free ring buffers, resampling, echo reference alignment, and playback with flush and onset timing. |
| `crates/speech` | Rust bindings to `swift/MiloSpeech`, a Swift static library that wraps FluidAudio: LocalVQE echo removal, Parakeet EOU streaming recognition, and Parakeet TDT 0.6B v2 for the second pass. Its build script runs `swift build` and links the result. |
| `crates/turn` | Voice activity detection (Silero VAD) and end-of-turn detection (Smart Turn v3.2) through ONNX Runtime, and the second-pass thread. It downloads both models on first use. |
| `crates/llm` | The client for the OpenAI-compatible model server, the sentence chunker, and the Rapid-MLX launcher. |
| `crates/voice` | Starts the Python voice server for Breeze, and streams audio from it. The server script, `sidecar/voice_server.py`, is embedded in the binary. |
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
  for each turn, the tool agent task, timers, and cache warming requests.
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

A *turn* is one reply. A turn starts as *speculative*: milo generates and
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
job, and playback is empty. The controller then adds the reply to the history
and starts the follow-up timer.

### Timing line

After each reply, the controller prints a dim line with the following fields,
separated by `|`:

- What ended the turn (`turn` with the Smart Turn score, `patience`, `eou`,
  `timeout`, `trailed off`, `nudge`, or `result`), and how long after your
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
  next message, so the next reply answers everything that you said. Follow-ups
  and tool results are never undone.
- **Interrupt.** Later in the reply, the controller stops the voice, aborts the
  reply task, and flushes playback. Aborting the reply task drops the response
  stream, which asks the server to cancel the request. The history records the
  text whose audio had started playing, if any, followed by an ellipsis. The
  system prompt tells the model what the ellipsis means.

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

## Follow-ups

After a reply ends, if milo detects no voice for `--nudge-ms` (6 seconds), the
controller commits a turn whose message is `(silence)`, and the model follows
up the way that a person on a call does. The second wait is three times as
long. After two follow-ups, milo stays quiet until you speak. Follow-ups wait
while the tool agent works.

## History and the prompt cache

The history starts with the system prompt and only grows, so that each request
starts with the previous one and the server can reuse its prompt cache. When the
history passes 40 messages after a reply, the controller prepares a shorter
history: the system prompt, rebuilt with the current memory, and about the last
20 messages, starting at one of your messages. The server has to read a
shortened history from the start, which takes a few seconds, so the controller
warms the server's cache with it in the background and keeps using the full
history until that finishes.

At start, milo warms the cache with the system prompt through Rapid-MLX's
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

milo uses the same model in two roles, so the voice never waits for a tool:

- The *talker* is the normal reply stream. With tools on, its system prompt
  lists what the tools can do, and tells the model to say that it's on it and
  never to make up an outcome.
- The *tool agent* in `tools.rs` calls tools when your message asks for
  something that a tool does.

### Tool agent

The controller sends a request to the agent after the reply of a committed turn
finishes generating. It skips follow-ups, tool results, and turns that were
only an acknowledgment or a wordless sound. The request has the last seven
lines of the conversation, ending with your message, and a flag that says
whether milo's reply promised something, such as "one sec" or "let me check".
If several requests are waiting, the agent takes only the newest.

The agent sends the model its own system prompt and a message with the local
time, the earlier lines, the memory file, its last four tool calls and their
results, and your latest message. It then runs up to four steps. In each step,
it does the following:

1. Waits, for up to 4 seconds, while the talker's reply is still being
   generated or synthesized.
1. Calls the model without streaming, at temperature 0, with the tool
   definitions and the `X-Rapid-Priority: background` header.
1. If the model answers with text, stops and uses the text as its report.
1. Otherwise, runs up to two of the requested tool calls and adds their results
   to its conversation. It doesn't run a call that repeats an earlier one.

The following rules shorten or skip the report:

- If the first step makes one successful call to `weather`, `timer`, `open`,
  `remember`, or `forget`, the tool's own result is the report.
- A result from `remember` alone isn't reported unless milo promised
  something.
- If the first step calls no tool and milo promised something, the report says
  that nothing was done.

The `weather` tool times out after 8 seconds. The `open` tool and
the commands from `tools.json` time out after 20 seconds, and so does each
model call. Tool results are cut to 700 characters.

### How results come back

The agent sends its report to the controller as an `Aside` event. When nobody
is speaking and nothing is playing, the controller commits a turn whose message
is `(result)` followed by the report, and the talker tells you about it in its
own words. A finished timer arrives the same way. While the agent works, the
controller holds follow-ups.

### Memory

Memory is the plain-text file `~/.config/milo/memory.md`, with one `- fact` per
line. The last 60 lines go into the talker's system prompt, where the prompt
has `{memory}`, at start and after each history cut. They also go into every
tool agent request. `remember` adds a line unless the same fact is already
there. `forget` removes every line that contains all the words of its argument
that are longer than two letters.

## Model server and voice server

milo starts the model server, the speech recognizer, and the voice at the same
time, on separate threads. It then loads the turn detector, opens the audio
devices, starts the capture thread, builds the reaction bank, starts the voice
thread, warms the model's cache, and starts the tool agent. When everything is
ready, it prints `listening`.

### Rapid-MLX

Unless you pass `--llm-url`, milo sends `GET /v1/models` to `127.0.0.1` on
`--port` (8000) before it starts a server:

- If the answer has a `2xx` status code, milo uses that server as is.
- If something else answers, milo waits for it to become ready.
- If nothing answers, milo starts `rapid-mlx serve` with the model from
  `--model`.

milo starts the server in its own process group, listening only on
`127.0.0.1`, with telemetry and the version check off. It passes
`--hybrid-cache-entries 16`, so that the tool agent's cache entries don't push
out the talker's, and `--watchdog-ppid` with milo's process ID, so that the
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
