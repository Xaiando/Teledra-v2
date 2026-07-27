# TTS

Owns voice profiles, synthesis, queueing, playback, and audio-device
selection. It consumes completed visible utterances; it does not edit court
text or call Qwen. **Status: core landed, playback not yet wired.**

## The problem this subsystem exists to solve

V1 started a fresh synthesiser for every line. Measured on the operator's
machine:

| | wall clock | audio produced | dead air |
| --- | --- | --- | --- |
| V1, process per line | 15.6 s | 3.5 s | **12.1 s, every line** |
| V2 startup | 13.6 s | — | once per session |
| V2, any line after that | 0.24–0.48 s | 1.8–3.0 s | **0.00 s** |

Of V1's 15.6 s, **10.4 s was loading the model** — paid again on every single
line. On stream that is twelve seconds of nobody talking between sentences.

Warm, synthesis runs four to eight times faster than playback, so the audio is
always ready before it is wanted. Three things buy that:

1. **The model loads once**, not once per line.
2. **Each voice's reference encoding is cached.** Encoding transcribes the
   reference recording, which never changes; V1 redid it every utterance.
3. **The first synthesis happens before `READY`.** It costs several seconds
   more than the rest, so the worker spends them during startup while nobody
   is listening. Measured, this took the first spoken line from 6.83 s to
   0.46 s. `READY` therefore means "ready to speak quickly", not just
   "loaded".

A cold voice speaking for the first time mid-session still renders in about
half a second, so the court can bring anyone in without a pause.

## Shape

| File | Owns |
| --- | --- |
| `src/pcm.rs` | The audio wire format, and refusing malformed streams |
| `src/voice.rs` | Which voice each member speaks in, and its gain |
| `src/worker.rs` | The warm worker process and the `Synthesizer` trait |
| `src/lib.rs` | `SpeechQueue` — ordering and render-ahead |
| `worker/voice_worker.py` | The model host: loads once, serves many |

## Two hazards, both learned the hard way

**Drain the worker's output, always.** Audio streams to stdout as it is
rendered. A reader that stops consuming fills the operating system's pipe
buffer at around 64 KB and the worker blocks mid-utterance, indistinguishable
from a hang. This was hit while measuring the numbers above. Every read path
consumes a whole utterance or fails.

**A dead worker must never read as silence.** A crashed model produces a
truncated stream, and `pcm.rs` reports that as an error rather than an empty
utterance, so the court can say why it went quiet instead of just going quiet.

## Still to do

- Playback and audio-device selection (nothing here reaches a speaker yet).
- Wire `SpeechQueue` into the court so accepted utterances are spoken.
- An end-to-end check that measures real dead air and fails if it regresses.

## Dependencies

The speech library (`zipvoice.luxvoice.LuxTTS`) is third-party and is *not*
vendored here — V1 kept a copy in its repository, which is why the worker
still has a fallback search. Install it into the environment, or point
`TELEDRA_LUXTTS` at a source checkout.

Reference recordings live in `assets/` at the repository root, one per voice.
