# TTS

Owns voice profiles, synthesis, queueing, playback, and audio-device
selection. It consumes completed visible utterances; it does not edit court
text or call Qwen. **Status: core landed, playback not yet wired.**

## The problem this subsystem exists to solve

V1 started a fresh synthesiser for every line. Measured on the operator's
machine:

| | wall clock | audio produced | dead air |
| --- | --- | --- | --- |
| V1, process per line | 15.6 s | 3.5 s | **12.1 s** |
| warm worker, first line | 3.50 s | 3.47 s | 0.02 s |
| warm worker, thereafter | 1.5–1.7 s | 4.2–4.7 s | **none** |

Of V1's 15.6 s, **10.4 s was loading the model** — paid again on every single
line. On stream that is twelve seconds of nobody talking between sentences.

Warm, synthesis runs about 2.8× faster than playback. That is the margin the
design spends: the next line is rendered while the current one is still being
heard, so in steady state there is no gap at all.

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
- Reference recordings are not yet in this tree; `worker/voice_worker.py`
  expects them under `assets/`.
