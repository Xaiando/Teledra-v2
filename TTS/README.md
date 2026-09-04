# TTS

Owns voice profiles, the bounded PCM protocol, one session-long warm LuxTTS
worker, synthesis queueing, Rodio playback, interruption, named output-device
routing, and child cleanup. It consumes final visible court utterances and
never calls or edits Qwen output.

V1 paid the model load and reference transcription for every utterance. V2
loads once, warms once, caches each reference prompt, and appends rendered
audio to one playback sink. Clean prompt durations prevent a reference's final
word from leaking into generated segments. Synthesis and playback run away
from the TUI.

Prompt duration follows each complete reference instead of using the old fixed
five-second crop or an eight-second ceiling. A local word-timestamp audit pins
the end of every Queen mode inside its encoded prompt: custom +0.410 s, clean
+0.299 s, sarcastic +0.266 s, and analytical +0.223 s of clean margin after the
last transcribed word. The clean reference's "magnificently" finishes at 5.52 s;
its prompt continues through the final word to 6.72 s, preventing the leaked
"gnificent" continuation.

| File | Owner |
| --- | --- |
| `src/pcm.rs` | Strict sample-rate, frame, duration, and finite-sample bounds |
| `src/voice.rs` | Voice names and gain |
| `src/worker.rs` | Warm child protocol, startup/synthesis timeouts, reaping |
| `src/runtime.rs` | Background queue, Rodio stream/sink, interruption, device selection |
| `worker/voice_worker.py` | LuxTTS model host and per-voice prompt cache |

The nine reference WAV files live in the root `assets/` folder. Install
`zipvoice.luxvoice.LuxTTS` in `TELEDRA_PYTHON`, or point `TELEDRA_LUXTTS` at a
source checkout. There is intentionally no fallback into the old repository.

Set `TELEDRA_AUDIO_DEVICE` to a full or partial output-device name for an OBS
monitor or mix. `TELEDRA_TTS_SPEED` accepts `0.55` through `1.10` and defaults
to the natural court pace of `0.75`; a lower value speaks more slowly. Set
`TELEDRA_DISABLE_TTS=1` for a silent session. Playback gains preserve the V1
court mix, including the Archivist's 2.45x lift, and remain peak-limited.

Shutdown stops queued audio immediately. A worker that ignores shutdown is
terminated by its exact recorded process ID, including its child tree, and
then reaped before Teledra returns.
