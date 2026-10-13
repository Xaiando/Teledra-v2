# Speech flow — 2026-09-13

The operator reported the transcript racing ahead of speech, repeated sentence
openings, slow Teledra delivery, and quiet Artist/Organist voices.

## Changes

- Speech previously declared Started/Finished events but never emitted them.
  Model replies entered the public feed immediately. Each utterance now carries
  its turn ID and emits Started/Progress/Finished from consumed PCM. The public
  feed reveals words over the actual audio duration. Word positions within an
  utterance are estimated from text length, not forced alignment.
- Admission covers two utterances total across queue, synthesis, and playback.
  The court prepares one reply ahead; explicitly quiet work can still use the
  inference slot when speech is full. Silent Night Desk turns do not spend an
  audible turn. Failed speech falls back to readable text.
- An accepted operator message interrupts speech and removes never-heard
  captions from immediate conversation history and repetition memory before
  preparing the response. Partially heard history retains only its visible
  fragment. Empty input and commands no longer clear speech accidentally.
- Terminal worker failures release all waiting captions even after generation
  invalidation; stale completion events cannot reveal interrupted replies.
- The shared policy favors short connected conversation. Repetition guidance
  refers to each role's own contributions without ordering a subject change.
  Narrow output/history cleanup removes observed empty ceremonial greetings
  and simplifies known rhetorical openings, preserving substantive text,
  quotations, real permission requests, and action payloads.
- Default TTS speed is 0.90 instead of 0.75; explicit supported speed overrides
  remain respected. Extra sentence gaps are 100 ms instead of 160 ms.
- LuxTTS attenuates generated audio to quiet reference prompts. Fixed role
  gains left an uneven mix. Each utterance now uses measured active-speech
  leveling to RMS 0.23 with a 0.98 soft ceiling, preserving internal dynamics
  and avoiding amplification of near-silence.

## Verification

- `cargo test -p teledra -p teledra-tts -p teledra-court -p teledra-agents`:
  139 pass; four optional live/hardware tests ignored in this run.
- TTS silent audio-device fixture run separately: passes sequential timing,
  two-utterance admission, interruption, crash/corrupt-worker recovery, shutdown.
- Python worker unit suite: four pass.
- Targeted strict Clippy, workspace formatting, and diff whitespace checks pass.
- Separate release build succeeds at `target/speech-update/release/teledra.exe`.
- Actual Rust leveling measured on 12 reference WAVs and five saved generated
  Queen WAVs. Reference active-level spread falls from 9.99 dB to 0.54 dB.
  Organist gains 2.26 dB and Artist 4.19 dB versus prior fixed gains. All measured
  output peaks remain below 0.98. Evidence: `target/voice-balance-20260913/report.json`.

The existing court process was left running on its original executable.
The normal shortcut/Teledra.bat rebuilds and activates these source changes on
restart. Fresh live synthesized Artist/Organist audio and subjective listening
at the new pace were not tested against the live session; measurements used
references and previously rendered Queen audio without starting another model.
