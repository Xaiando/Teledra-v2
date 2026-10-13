# YouTube responses and sustained podcast sessions — 2026-09-18

## Operator controls

- Paste a YouTube video URL into chat, optionally with a question, or use the
  dedicated input or `/YouTube <URL>`.
- `/Lock <topic>` starts a 60-minute podcast plan.
- `/Podcast 90m <topic>` selects a 60–240 minute plan.
- `/Podcast status` and `/Rundown` report the active clock and chapter.
- `/Mode normal` pauses the episode and clock; bare `/Podcast` resumes.
- `/Unlock` ends the episode and restores the earlier mode. No timer or model
  tag releases the operator's lock. Sessions are held for the current app run.

## Changes

Chat video links previously went to generic webpage research. They now use the
YouTube sidecar, including common shared/mobile/Shorts/live/embed forms. The
sidecar selects available captions when English is absent, escapes JSON for
Windows pipes, retrieves title/channel metadata, and samples long transcripts
across the recording within 6,000 characters. Missing captions yield clearly
marked metadata-only evidence; total failure produces an honest spoken reply.
Operator video results take priority over queued audience chatter and their
evidence remains available in bounded conversation context.

Operator locks can no longer be overwritten by model TOPIC, UNLOCK or STOP_BABBLE
tags. PodcastSession owns twelve timed angles, bounded chapter notes, active
time excluding pauses, and continued exploration beyond the planned duration.
Only completed speech (or visible text fallback) enters episode memory. Session
and chapter identifiers prevent delayed speech from crediting a new chapter or
replacement episode. Pausing, stopping or changing the episode clears queued
speech and cancels its pending inference. Night Desk maintenance stays quiet.

Podcast speech uses a focused conversational identity and shared episode
context. Artifact rules, old office work and automatic composition enrichment
are excluded from scheduled podcast speech. The model returns a speech-only
JSON envelope; the envelope is never spoken. Runtime validation rejects copied
openings/passages, action tags and responses outside 50–180 words, and permits
one correction inside the original generation deadline. It never clips an
overlong answer into an incomplete sentence. The speech queue still gates
prefetch. Explicit podcast mode has its own cadence even when ordinary idle
autonomy is disabled.

Queen/Orator and Court prompts now favor specific callbacks, playful
counterpoints, concessions and substantive continuation. Generic private review
and refinement preserve the original bounded conversation and runtime context;
previously editing discarded the shared exchange.

## Verification and limits

- Live Python ingest succeeded for English captions (`jNQXAC9IVRw`), Japanese
  captions (`ZRtdQ81jPUQ`), and explicit metadata-only fallback with an interpreter
  lacking the captions package. All 16 YouTube regressions passed.
- Eighteen podcast tests cover simulated 70-minute progression, bounded memory,
  pause/resume, delayed speech and session replacement, prompt diversity, and
  validation of copied or overlong speech. App/runtime regressions cover paste
  routing, priority, retained video evidence, operator lock protection and
  conversation interruptions.
- A live four-turn conversation using the configured `qwen2.5-coder:14b` passed
  through the final structured speech path with accepted lengths and no copied
  passages. This checks conversational mechanics, not factual accuracy.
- Brain, Court, Sidecars and Agents library tests passed (59 tests), as did
  formatting, Clippy with warnings denied, and the optimized application build.
- The final serial application run passed 250 tests, ignored 9 optional probes,
  and hit one intermittent music mock-server connection failure. That exact
  failing test passed immediately in isolation; all 18 podcast tests also passed
  in isolation. Existing music mock-network tests were timing-sensitive in the
  parallel run too. The cancellation fixture has an unbounded wait for a request;
  it was excluded from the final serial run after hanging. The same isolated
  cancellation fixture passed with diagnostic tracing enabled. No music
  implementation was changed.
- A full hour of real-time TTS/audio playback was **not** performed. The
  70-minute check advances the session clock in tests. Local inference, speech
  availability and model knowledge still determine actual delivery quality;
  an unavailable model cannot be replaced by a timer or canned filler.

Re-run application regressions with `TELEDRA_MODEL=qwen2.5:7b` scoped to the
test process because an existing mock test hardcodes that default:

```powershell
$env:TELEDRA_MODEL='qwen2.5:7b'
cargo test -p teledra --bin teledra -- --test-threads=1 --skip cancellation_drops_the_single_request_without_a_later_repair
```

For the live conversation probe, set the model back to the installed launcher
model and run:

```powershell
$env:TELEDRA_MODEL='qwen2.5-coder:14b'
cargo test -p teledra --bin teledra live_podcast_conversation -- --ignored --nocapture
```
