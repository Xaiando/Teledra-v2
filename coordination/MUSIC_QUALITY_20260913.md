# Music quality and complete playback — 2026-09-13

The operator reported sour notes, incomplete launches, and insufficient instrument
variety. This update addresses the native synthesis, arrangement and transport
paths; Organist Studio remains a control/display process.

## Evidence and changes

- Read-only inspection of the live ledger found an 80-second movement replaced
  after 53.1 seconds. The current saved WAV contains its full 120 seconds. New
  verified movements now wait for the current pass to finish before publication.
  A consumed-audio-frame clock accounts for pauses. Stop cancels waiting work.
- Mixer edits target the displayed accepted ensemble and keep its playhead.
  A waiting future movement is preserved; a future composition whose render
  was interrupted by mixer edits is retained for the next free render slot.
- The saved flute part contains adjacent E/F eighth notes whose old release
  continued for roughly 155ms into the next note. Monophonic wind, brass, bowed,
  bass and lead voices now end smoothly by their next attack. Plucked and struck
  instruments retain their ringing polyphony.
- Sparse sustained voices could carry incompatible old chord tones across a
  harmony change. Developed diatonic arrangements now bound those voices at the
  change, preserving common tones. Long offbeat arrivals receive harmonic
  treatment; short passing notes, literal zero-development and chromatic
  suspensions remain available.
- Oscillator partials above the supported band were clamped to an unrelated
  frequency. They are now suppressed. Voice time comes from integer audio frames,
  keeping release boundaries exact.
- Seven distinct synthesized instruments bring the palette to 25: violin,
  clarinet, oboe, horn, trumpet, guitar and vibraphone. They participate in the
  schema, ranges, synthesis, mix, theory reports and native texture evolution.
  Composer context lists the current roster and unused colours, with directions
  to explore complementary voices in fresh pieces when the brief permits.
- New composition acceptance requires the complete native score and track
  fields. Legacy saved-score compatibility remains in the general loader.

## Verification and reproduction

The Music and Teledra release suites, strict Clippy and formatting checks passed.
Regressions cover harmonic and monophonic release boundaries, literal harmony,
25-voice schema compatibility, seven distinct verified renders, tuning at three
points of each new voice's range, and reachability through native variations.
The hidden Studio tests cover playback position and waiting status.

A live invocation of the production local Organist draft/review helper produced
**Clarinet and Guitar Serenade**, an eight-bar, 96-BPM study using clarinet,
guitar and vibraphone. Its complete 20-second render passed verification with
peak 0.890, RMS 0.267 and all eight analysis sections active. Artifacts are under
`target/music-quality/model-study`; no playback was started by this model test.

`cargo run --release -p teledra-music --example palette_audition` produces a
29.5-second separated comparison of all seven new voices and a timing report
under `target/music-quality`. Each voice plays the same three-note phrase.

The actual saved score was also rendered in isolation to
`target/music-quality/current-score-corrected.wav`: the complete 120 seconds,
peak 0.890, RMS 0.122 and a 1.70x dynamic range across measured sections.

The live model proof can be repeated with:

```powershell
cargo test --release -p teledra --test organist_palette live_organist_uses_new_instruments_in_a_complete_study -- --ignored --nocapture
```

Verification establishes pitch, signal, structure and transport behavior. It
does not establish subjective musical appeal. The voices remain synthesized
patches rather than sampled acoustic instruments. Existing user scores and
audio were inspected read-only; the new build takes effect through Teledra.bat.
