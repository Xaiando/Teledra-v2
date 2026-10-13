# Speech noise and conversation continuity — 2026-09-17

The operator reported electrical TTS noise, repetition, and scattered discussion.

## Audio

LuxTTS now uses its smooth vocoder output instead of the optional predicted
high-frequency extension. Sentence edges receive a 5 ms raised-cosine fade.
The playback leveler uses a smooth gain envelope for peak protection instead
of saturating individual samples above a 0.62 knee. Active-speech leveling and
the 0.98 ceiling remain; limiter timing follows the actual PCM sample rate.

Six fresh same-seed CPU renders using cached model weights measured a decrease
in energy above 12 kHz of 15.1 dB (Queen), 12.0 dB (Organist), and 23.1 dB
(Artist). Each A/B pair has identical duration and less than 0.05 dB difference
in active output level. A high-crest tone fixture's harmonic distortion fell
from 3.87% to 0.31%. This verifies synthesis/processing changes, not the user's
hardware or OBS mix. No live playback was performed.

Evidence: `target/tts-noise-20260917/report.json` and the adjacent A/B WAVs.

## Conversation

- Public turns now develop a shared subject instead of following the old
  room/history/quarrel/craving subject rota. Office identity is a perspective,
  not a requirement to start another art, music, or maintenance project.
- A bounded anchor remains for 24 accepted audible replies and then advances
  to the latest thought. Operator input updates focus, retaining a short prior
  subject for follow-ups such as “Go on.” Quiet work, failed inference, and
  withheld duplicates do not advance that window.
- Explicit locked subjects no longer expire after 20 dispatched turns. They
  remain until an unlock or replacement control action.
- Shared context holds 12 contributions instead of six, with both the opening
  and ending retained for long replies. Current context follows older role
  history. A large artifact no longer excludes every older fitting exchange.
- Repetition guidance draws on public contributions, excluding private work.
  High-overlap sentence reuse is recognized even with a changed opening or
  reordered sentences; changed negation and numeric qualifications are exempt
  from approximate matching.
- Recall searches substantive content instead of scheduler instructions and
  quoted avoidance clauses. Genuine operator questions with similar openings
  remain searchable.
- Durable recording of queued speech waits for completion or visible text
  fallback. Interruption and normal exit save only the heard/displayed part;
  unheard queued replies cannot reappear as recollection.

## Verification and activation

Regression coverage includes long conversations interleaved with private work,
topic locks beyond 20 turns, changed/reordered repetitions, operator follow-ups,
bounded shared context, and actual MemoryStore recall after speech interruption.
Audio coverage includes harmonic distortion, peak bounds, sentence edges,
sample rates, and the actual smooth-generation and retry arguments.

Results:

- Application unit suite: 225 passed, eight optional checks ignored, one
  unrelated Python-music cancellation test excluded after it stalled both in
  the combined run and alone. The stalled test processes were stopped by
  their exact test executable path, without touching the court.
- Court: 43 passed; Memory: nine passed; TTS: 30 passed (one hardware check
  ignored); Agents: three passed; Python voice worker: seven passed.
- Workspace formatting, strict Clippy for the changed crates, scoped whitespace
  checks, and the optimized release build passed.
- The broader application integration suite is not fully green: two
  `live_python_evolution` mock HTTP repair/deadline tests failed to capture their
  expected local requests. These Python-music tests are outside the changed
  speech/conversation paths and were left unchanged. Logs are in
  `target/conversation-tests-20260917.log` and
  `target/speech-crates-tests-20260917.log`.
- Mock-model tests used a command-local `TELEDRA_MODEL=qwen2.5:7b` because they
  assert that default model; the operator's launcher configuration was unchanged.

The release verification build is `target/speech-update/release/teledra.exe`.
The existing court process was left running. Launching the usual shortcut or
`Teledra.bat` rebuilds and activates these changes on restart. Fresh listening
through the user's normal output route remains the subjective acceptance check.
