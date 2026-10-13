# Teledra overhaul — 2026-09-05

The follow-up [operational acceptance](OPERATIONAL_ACCEPTANCE.md) records the
2026-09-06 transport, worker recovery, launcher, colour, and lifecycle checks.

The Organist, DAW, court coordination, and TTS were exercised through their real
parsers and runtimes. The release executable was rebuilt. Existing uncommitted
music work was retained and extended; `D:\Teledra` remained a read-only reference.

## Music and DAW

- Composition turns use a bounded JSON schema through Ollama, then the host adds
  the `COURT_SCORE` wrapper. Normal music conversation keeps the Organist's voice.
  The previous unconstrained live check produced a truncated score and invalid
  note degrees; the structured check produced a valid, masterable score.
- Longer forms and the six-part native evolution cycle from the preceding work
  are preserved. Mastering and tempo changes extend a piece toward two minutes
  within 32–64 bars; the 64-bar limit makes two minutes impossible above 128 BPM.
  Default evolution waits two passes, bounded to 120–600 seconds. Operator cadence
  supports 60–900 seconds consistently across the UI, protocol, and runtime.
- Sustained instruments use playable attack spacing and release before their next
  attack, reducing stacked organ/cello/pad voices. Phrase development retains
  octave information. Irregular pattern lengths preserve their rhythmic position.
- Chord-quality repair keeps valid roots and in-mode suspensions. Muted instruments
  remain authored seats when mastering; a mute no longer invites an audible
  replacement. Section decisions consider the remaining audible instruments.
- Craft reporting now includes articulation and register separation. Empty or
  muted parts cannot satisfy audible musical roles. Scores no longer receive a
  perfect report merely for containing the right instrument names.
- Mixer requests compose against the latest draft, with one active render and one
  replaceable pending score. Stop invalidates pending work. Mix edits retain the
  musical seed, movement index, and title. New Movement explicitly requests a new
  composition.
- Replacement audio is verified off the UI thread and prepared paused before
  current playback changes. Pause and global mute survive replacement; global
  mute leaves the transport running. A failed render or output-device setup keeps
  the previous player.
- WAV and score files are staged before replacement, with the score as recovery
  authority. A failed WAV write leaves the previous score intact. History failures
  are reported without rejecting an already committed score. History pruning uses
  modification time, so several mixes within one movement are retained correctly.
- The DAW shows a common sixteenth-note time grid for short and multi-bar patterns,
  preserves mute restoration across texture changes, displays the exact cadence,
  and advances time while globally muted. Track activity reflects written onsets;
  it is not an audio-level meter.

## Agents and execution

All eleven role profiles received concrete responsibilities and actionable
examples or evidence requirements. The Organist profile was shortened and given
a complete executable musical example. Court guidance distinguishes requested,
pending, failed, and completed work.

Every dispatched turn receives bounded recent court activity, recent execution
reports, and relevant durable recall. The Organist also receives the actual
accepted score and feedback. Role history retains complete recent exchanges
within a character budget. Qwen receives an explicit 16,384-token context and a
4,096-token output budget; truncated conversational replies get one retry.

Delegation preserves backstage delivery, stops after three hops, deduplicates
queued requests, and respects queue limits. A turn executes at most four actions.
Stale replies are rejected before actions, speech, or persistence. Repeated speech
is withheld before it reaches TTS. A malformed artifact or empty Night Desk result
gets at most one correction; a specific `NO_ACTION` explanation is accepted.
The private refiner now retains the original task and role and protects live-art
tags. Successful and failed Scribe writes enter subsequent execution context.

The installed `qwen2.5:7b` passed five live contract cases: a masterable score, a
valid animated Fractus scene, an exact Scribe append, an evidence-seeking research
action, and an Organist delegation. The harness parses outputs without executing
those generated actions. This is a sampled integration check, not a guarantee
that a language model will always choose the right action.

Ollama's schema interface follows its
[official structured-output documentation](https://docs.ollama.com/capabilities/structured-outputs).

## TTS cause and fix

The installed LuxTTS `encode_prompt` defaults to a five-second crop. The clean
Queen reference lasts 6.869 seconds; the crop cuts it during "magnificently".
The worker had still called that default despite older documentation claiming
the reference was already fixed. The worker also ignored the selected Queen
reference mode and configured speech speed.

The worker now encodes the complete reference recording with a 50 ms read margin,
honors all four Queen modes and the bounded speed setting, and caches the result.
Markdown presentation markers are removed from spoken text. Each failed utterance
terminates with exactly one protocol marker so the following request stays aligned.

Real LuxTTS output was transcribed with the installed Whisper model. Requested:

> The silver flute carries the melody. The quiet organ leaves room for every phrase.

| Reference setup | Actual transcription |
| --- | --- |
| Old five-second clean crop | The silver flute carries the melody, magnificent. The quiet organ leaves room for every phrase. |
| Fixed clean | Exactly the requested words |
| Fixed custom | Exactly the requested words |
| Fixed sarcastic | Exactly the requested words |
| Fixed analytical | Exactly the requested words |

Speech requests use a nonblocking four-item queue and at most three rendered
utterances. Interrupt stops the active sink and invalidates queued/in-flight
speech immediately. Worker restarts preserve explicit environment and working
directory. The old repository is no longer an implicit executable TTS dependency.

## Additional repairs

- `/Help` works, a question mark can be typed in chat, and Escape closes help.
- Old proposals expire after seven days, or 48 hours for Moltbook interactions.
  Expiration preserves their records, and legacy timestamp-bearing IDs are handled.
- Read-only executable environment and portrait checks do not close the court.
- The launcher targets the executable in this checkout and stops on a failed build
  instead of opening an older binary.

## Verification and artifacts

- Rust workspace: 237 passing tests, including 60 Teledra binary tests and the
  stale-reply, action-limit, and persistence-failure regressions. The one normally
  ignored live-model test was run separately and passed all five role cases.
- `cargo clippy --workspace --all-targets -- -D warnings` and formatting checks pass.
- Worker Python unit suite: four passing tests. DAW self-test and hidden real Tk
  workstation integration test pass.
- Six consecutive renders from the installed music lineage passed acoustic
  verification. Durations: 124.80–126.79 seconds; peak: 0.89; RMS: 0.111–0.121;
  periodicity: 0.776–0.844. Score and audio digests differ across all six movements.
  Sample-level correlation is not evidence of a different melody or musical beauty;
  pitch-preservation and timing regressions check the intended musical invariants.
- Real Rust `WarmWorker` → Python → PCM probe passed two utterances (2.304 and
  3.285 seconds). Its worker exited without leaving a voice-worker process.
- Release build and `teledra.exe --check-environment` pass. Python, model caches,
  voice references, audio devices, music, Fractus, sidecars, and OCR are available.

Logs, six-movement WAV/score proof, the four fixed voice WAVs, legacy comparison,
and transcription JSON are under ignored `target/astra-verification/`. Verification
did not start an unattended public court or execute generated outreach actions.

```powershell
cargo test --workspace
cargo test -p teledra --test agent_contracts live_offices -- --ignored --nocapture
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
.\.venv\Scripts\python.exe Music/daw.py --self-test
.\.venv\Scripts\python.exe -m unittest discover -s Music -p test_daw.py
.\.venv\Scripts\python.exe -m unittest discover -s TTS/worker -p test_voice_worker.py
.\.venv\Scripts\python.exe TTS/worker/verify_voice.py target/voice-check --compare-legacy --modes clean custom sarcastic analytical
cargo run -p teledra-tts --example worker_probe
cargo build --release -p teledra
.\target\release\teledra.exe --check-environment
```
