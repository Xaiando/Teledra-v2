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

The worker explicitly encodes the full reference WAV duration, with a 50 ms
read margin. Calling LuxTTS without this argument silently crops at five seconds,
cutting the clean Queen during "magnificently". The 2026-09-05 real-model audit
reproduced an extra "magnificent" with the old crop; clean, custom, sarcastic
and analytical modes all transcribed as exactly the requested two sentences
after the fix. The selected Queen mode and speech speed reach the actual engine.

The runtime admits at most two utterances across synthesis, waiting, and
playback: the current voice and one prepared reply. The court schedules audible
turns against that limit while quiet work can continue. Each utterance carries
its turn ID. Started, progress, and finished events come from consumed PCM;
the public transcript starts with playback and reveals words along its duration.
Word positions are estimated within the audio, rather than aligned by a speech
recognizer. An unavailable worker falls back to readable text.

Interruption stops current playback immediately and invalidates queued or
in-flight speech and captions. Empty input and ordinary commands do not discard
the speaking turn. Worker failures emit exactly one end marker, keeping the
next request aligned. Explicit worker environment and working-directory settings
are preserved across restarts.

Broken writes and invalid or truncated PCM retire the worker before the next
request, including when pipe EOF races the Windows redirector's exit signal.
Empty framed output is reported as a synthesis failure. Hardware interruption,
restart, and cleanup checks are recorded in
[operational acceptance](../coordination/OPERATIONAL_ACCEPTANCE.md).

| File | Owner |
| --- | --- |
| `src/pcm.rs` | Strict sample-rate, frame, duration, and finite-sample bounds |
| `src/voice.rs` | Voice names, measured speech leveling, and peak protection |
| `src/worker.rs` | Warm child protocol, startup/synthesis timeouts, reaping |
| `src/runtime.rs` | Background queue, Rodio stream/sink, interruption, device selection |
| `worker/voice_worker.py` | LuxTTS model host and per-voice prompt cache |

Each of the eleven court members has a dedicated reference WAV in the root
`assets/` folder, and no two share a recording. Malthus, the Wizard and the
Treasurer use references baked offline from local Kokoro voices by
`worker/generate_character_references.py` (provenance in
`assets/additional_voice_references.json`; replaced recordings are kept in
`assets/replaced/`). LuxTTS carries a reference's pace into every line, so a
reference spoken too quickly makes that character rush.
Only Teledra uses the Queen voice and its selected mode. Unknown voices and
missing recordings fail the request with an operator-visible error and a valid
empty audio response; the next request stays aligned, and another character
never silently speaks in Teledra's voice. The startup check requires all eleven
reference voices. Install
`zipvoice.luxvoice.LuxTTS` in `TELEDRA_PYTHON`, or point `TELEDRA_LUXTTS` at a
source checkout. There is intentionally no fallback into the old repository.

Set `TELEDRA_AUDIO_DEVICE` to a full or partial output-device name for an OBS
monitor or mix. `TELEDRA_TTS_SPEED` accepts `0.55` through `1.10` and defaults
to `0.90`, 20% faster than the previous `0.75`; a lower value speaks more slowly.
The extra gap between synthesized sentences is 100 ms. Set
`TELEDRA_DISABLE_TTS=1` for a silent session.

Playback measures active speech in each rendered utterance and targets an RMS
of `0.23` (about -12.8 dBFS) before peak protection at `0.98`. LuxTTS scales
output to the reference recording, which made the Organist and Artist much
quieter than the Queen despite their old fixed boosts. Every office now uses
the same measured target. One leveling gain per utterance preserves the original
phrasing and dynamics; silent windows do not affect the measurement, near-silent
noise is left alone, and automatic amplification is capped at 32x. An explicit
`Voice::with_gain` value acts as an output trim after leveling.

Peak protection uses a smooth gain envelope with 5 ms look-ahead, 20 ms hold,
and 80 ms recovery. It leaves signals below the ceiling linear instead of
reshaping loud samples through a saturator, which added buzzy harmonics.
Limiter timing follows the actual PCM sample rate. Some emphatic lines may
finish slightly below the RMS target to preserve clean peaks.

The worker requests LuxTTS's smooth vocoder path. The installed engine resamples
its native 24 kHz output to 48 kHz without the default predicted high-frequency
extension; the wire rate and speech speed stay unchanged. Each sentence also
gets a 5 ms edge fade so transitions into the sentence gap cannot click. Invalid
channel shapes and nonfinite samples are rejected before reaching the pipe.

Shutdown stops queued audio immediately. A worker that ignores shutdown is
terminated by its exact recorded process ID, including its child tree, and
then reaped before Teledra returns.

Verification:

```powershell
cargo test -p teledra-tts
.\.venv\Scripts\python.exe -m unittest discover -s TTS/worker -p test_voice_worker.py
.\.venv\Scripts\python.exe TTS/worker/verify_voice.py target/voice-check --compare-legacy --modes clean custom sarcastic analytical
cargo run -p teledra-tts --example worker_probe
```

`level_probe` exercises the actual playback leveler on mono 48 kHz
little-endian float32 PCM without loading a model or opening an audio device:

```powershell
cargo run -p teledra-tts --example level_probe -- input.f32 leveled.f32 organist
```
