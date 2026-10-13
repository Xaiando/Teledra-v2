# Organist overhaul — 2026-09-13

The operator requested a stronger composer that can use music theory to
experiment with original pieces.

## Delivered behavior

- Every Organist turn receives the actual accepted score, current notation
  analysis, a concise original theory guide, and accurately scoped operator
  feedback. Analysis covers scales, Roman-numeral harmony, cadence candidates,
  motif contour and intervals, repetition, rests, and voice-leading ideas.
- Twelve rotating composition studies invite concrete musical experiments.
  Fresh score context is attached at dispatch; native evolution waits while
  the Organist composes so the baseline cannot change underneath that work.
- New compositions explicitly choose form, phrase length, harmonic rhythm,
  development, texture and diatonic/chromatic policy. Short chamber and ambient
  pieces retain their authored length, instruments and rests. Legacy scores
  remain compatible with the previous ensemble-completion behavior.
- Valid model drafts receive one review using their own theory analysis and
  measured sounding-note/expression counts. Invalid drafts receive one
  structural repair instead. Both paths use at most two model calls; a failed
  review keeps the valid draft and a failed repair retains previous playback.
- Atomic authored note events keep degree, velocity and duration together.
  Conversion to the stored score format preserves every event exactly, without
  padding, truncation or pitch rewriting to satisfy the parser.
- Organ, pad and keys render real three- and four-note chord voicings with
  nearby inversions and common tones. Melodic passing notes survive; structural
  arrivals can resolve. Planned bass lines can use written passing notes and
  inversions. Per-note dynamics, duration and silence reach playback.
- Lydian, Phrygian and harmonic minor extend the mode palette. New qualities
  include augmented triads and major, minor, dominant and half-diminished
  sevenths. Diatonic repair derives chord compatibility from the actual scale.
- Binary, ternary and rondo forms schedule explicit phrase regions and returning
  material. Native motif experiments use sequence, inversion and retrograde
  while keeping audible anchors and rests. Muted notes remain silent and pulse
  evolution no longer revives a muted kick.
- Texture evolution preserves chordal versus melodic instrument roles, so
  re-orchestration cannot collapse a chord into a single reed line. A regression
  checks actual scheduled notes across 48 texture evolutions.
- Legacy harmony experiments rotate interior chords while keeping the opening
  tonic and closing harmonic destination. Final regression testing exposed an
  old rotation that displaced the tonic; that behavior was corrected without
  lowering the existing craft threshold.
- Studio displays theory, a suggested experiment, and exact renderer form and
  harmonic timelines. Its sequencer follows returning phrase positions and
  displays zero-velocity notes as silence.

## Verification

- The ordinary release test suite for Teledra, Music, Fractus, Agents, Court and
  TTS passed 259 tests. Final regressions cover re-orchestration and legacy tonal
  anchors; affected Teledra/Music suites passed again after those fixes. Seven optional
  live/hardware tests are excluded from the ordinary run; the Organist live
  test was exercised separately.
- All eight Studio Python tests passed, including hidden Tk behavior checks.
- Strict Clippy for Teledra and Music, workspace formatting and diff whitespace
  checks passed. The normal release executable was rebuilt and its read-only
  environment check passed.
- The final opt-in live test used the installed local model and the production
  draft/review helper. It produced **Harmonic Loop**, a 16-bar, 104-BPM study in
  A harmonic minor for cello, organ and flute. The score uses A-B-A form,
  four-bar phrases, two-bar harmonic rhythm, and V7-i harmony. The flute has a
  32-position cell with five distinct written degrees, rests and varied dynamics.
- The production renderer and audio verifier accepted the study: 36.923 seconds,
  peak 0.89, RMS 0.184, all eight measured sections active. The actual arrangement
  contains four cello pitches, seven organ pitches and five flute pitches;
  harmonic events render multiple simultaneous notes. No audio device was
  opened for this test.
- Reproducible artifacts are under `target/organist-overhaul/model-study`:
  `accepted-score.json`, `theory-analysis.txt`, `audio-verification.json`, and
  `organist-study.wav`. The earlier `four-bar-study.wav` is also a verified
  model-generated piece, not a hand-authored substitute.

## Findings and practical limits

Early live attempts exposed misaligned parallel expression arrays, missing plan
fields, and incomplete melodic cells. These drove the atomic-note schema and
native repair boundary. Later valid drafts missed some detailed musical
instructions, motivating factual self-review. The final live acceptance passed;
one passing experiment does not establish consistent adherence to every musical
brief. The model still sometimes chooses high written degrees despite guidance
to prefer a compact register. Native octave folding keeps them playable without
flattening all notes onto a range ceiling.

The transport is currently 4/4 with bounded repeating note cells. Theory and
craft reports describe notation and structure; they do not certify beauty or
claim subjective listening. The verifier's autocorrelation measure is also
descriptive: controlled clean polyphonic chords can have lower values than
single tones. Its regression now compares polyphony with deterministic white
noise rather than treating an arbitrary absolute threshold as musical quality.

The implementation and model-to-audio path were exercised in isolation; a long
session of the newly built court has not yet been observed. Existing live court
processes were not stopped by this work. Launch `Teledra.bat` normally to run the
updated `target/release/teledra.exe`; the build includes the preceding speech and
Fractus changes as well.
