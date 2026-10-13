# Composed phrases after the scale regression — 2026-09-13

The operator rejected the previous work because the notes still amounted to a
repeated scale. Tuning and mastering repairs did not solve that composition
failure. This change addresses the actual written material and its playback.

## Immediate replacement

The eight-bar study below was superseded after the operator requested a V1
comparison, dependable note delivery, layers and motion. The final startup
replacement is **Light in Motion**, a fully written 32-bar, six-part arrangement.
See the final audit below. The smaller study remains an isolated historical
comparison and is no longer the startup recovery choice.

`Music/src/composer.rs` writes a complete eight-bar chamber study with an opening
motif, an answer, a contrasting middle and a closing return. It has three authored
motif contours, separate accompaniment and bass rhythms, mode-appropriate harmony
and real release space. It preserves the reference key and tempo. These are bounded
native compositions, not evidence that the language model learned to compose.

For the captured saved score, the result is **Returning Light**: C Dorian, 96 BPM,
eight bars, 20 seconds, Flute / Guitar / Double Bass. The reviewable artifact is
`target/phrase-composition/native-study/study.wav`; the same directory holds the
editable score, scheduled-note report and numerical audio checks. It has 66
scheduled notes, including all 34 written melody attacks. Peak is approximately
0.202 and RMS 0.0417. Signal and notation checks are not listening judgments.

Startup now recognizes the scalar failure, archives the original complete score
under `music/rejected/`, and prepares this native replacement through the ordinary
render, audio-verification and publication path. The accepted score and Studio do
not change while audio is incomplete. Evolve Now applies the same recovery if an
old scalar score is still current. Existing pass-boundary replacement, pause,
mix-edit and stop behavior remains covered by operational verification.

## Composition and timing changes

- Optional `composition.steps_per_bar` selects 4, 8 or 16 divisions per bar.
  Omission preserves legacy timing. This permits a 64-slot eighth-note line to
  contain eight actual written bars, rather than repeating a four-bar cell.
  The arranger, diagnostics, schema, prompts and Studio use the same grid.
- Automatic generation rejects long uniform scalar/fixed-interval walks that
  dominate the melodic material, including in-range solos. Brief runs, shaped
  stepwise phrases and arpeggios under an independent melody remain allowed.
- Rejected scale arrays are omitted from repair context. The brief retains useful
  key, tempo, harmony, playing limits and explicit operator requirements. The
  profile describes motif/answer/contrast/return and contains small event syntax
  examples, rather than a large score for the model to copy.
- All 72 instrument IDs, including piano and theremin, remain available. Normal
  score context includes the full catalog; compact repair context emphasizes
  current playing limits. Variety is optional and does not replace composition.
- Cadence diagnostics recognize an authored IV/V/VII-to-tonic ending as well as
  an arrival across the loop boundary. A terminal tonic by itself earns no credit.

## Live model evidence and limits

One bounded production request was run, using the normal maximum of two
inferences. The result, **Restraint in C Dorian**, failed the requested musical
study: it ignored the quarter-note grid and authored a four-bar cell repeated
twice, with uniform answering rhythm and no authored closing tonic destination.
Its audio was complete and technically valid, and the narrow scalar guard did
not flag it. This is not an aesthetic success or proof that all repetitive music
is prevented. It was neither played nor published. Evidence is retained in
`target/phrase-composition/model-study/outcome.txt`.

The native path supplies an immediate composed replacement for the identified
scale regression. Future model drafts still need operator judgment; no numerical
score is presented as a certificate of musical appeal.

## Validation

Targeted composer tests cover motif relationships, independent rhythms and
registers, actual scheduled pitches/onsets, all seven modes and twelve keys, and
all supported tempos for phrase release timing. Scalar-guard tests cover both
reported failure shapes and legitimate short gestures. Studio's twelve hidden
Tk tests passed, including eight-bar grid tracking.

The muted operational test passed through the installed audio device and Studio
probe. It starts from a scalar source, verifies its archive and complete native
replacement, then checks persistence, full-pass publication, paused transport,
mix edits, pending renders and stop/play reuse.

## Final V1 comparison and independent critic audit

V1's saved 64-bar recording has eight roles and eight sections over 174.5 seconds.
V2's earlier miniature scheduled all 66 notes but deliberately ended every
two-bar phrase with the entire ensemble nearly silent. Its last phrase-second
was 20–26 dB quieter than the preceding music. That was an arrangement regression,
not a missing-note explanation for that specific fixture.

Broader audits found and fixed independent playback regressions:

- Instrument-spacing suppression scheduled only 16 of 64 authored violin notes.
  Planned scores now preserve every positive-velocity authored attack and rest;
  legacy unplanned generation retains its own optional fills and thinning.
- Explicit 1000 ms polyphonic piano notes were capped to 460 ms at 500 ms spacing.
  Their full holds now remain. Monophonic retrigger and intentional developed
  harmony limits remain separate.
- Slow envelopes could peak at only 6–11% on the shortest legal notes. Attack
  adapts to at most 30% of the gate when necessary, while long-note stages remain.
- The master attenuated 2–6 kHz by 12.5–21.1 dB. Neutral musical-passband filtering
  now retains percussion transients, with rumble and ultrasonic filtering intact.
- Repeated chord voicings drifted upward from the middle register into the lead.
  A bounded register target now permits nearby voice leading without that drift.
- Zero-development scores still inherited unrelated sectional filter changes.
  They now preserve patch filtering; the authored arrangement controls motion.
- Final long holds could exceed allocated render tails, and wrapping discarded
  continuations beyond one loop. Allocation covers actual gates/releases and
  every continuation cycle folds into the output.
- The alternate MUSIC_PATCH action bypassed complete-score/scalar checks. It now
  follows the same validation contract before rendering.

The final arrangement uses 256 written grid positions per part. Sustained Keys
support the Flute's breaths, Double Bass provides foundation, Guitar retains
interlocking offbeats, Piano answers enter later, and Shaker supplies a restrained
pulse. Chapter entrances, a thinner contrast, return and reduced closing density
are written into notes/rests/dynamics. Full native timelines up to 1024 positions
are supported without expanding the model's compact generation schema. Studio
follows later pages instead of displaying the first eight bars repeatedly.

The independent read-only critic checked the final **80-second** render:

- **385/385 authored attacks**, 449 chord-expanded voices, zero omitted triggers.
- Keys remain MIDI 58–70 instead of climbing to 84.
- Guitar retains its first three offbeats; Piano's descending answer order is
  preserved even at fast tempos.
- No strong or held theme conflicts against the actual harmony in this fixture;
  remaining Flute passing tones are short (0.38 beat) weak arrivals.
- Accompaniment continues through foreground breathing space: the last second
  of each short phrase retains RMS approximately 0.019–0.038.
- No remaining technical blocker in the reviewed fixture. The eight-chord
  progression still repeats; contrast relies on rhythm, entries and foreground
  roles. Listener preference is not established by these checks.

Final artifact: `target/phrase-composition/layered-study/study.wav`, with editable
score, score/audio checks, arrangement report and `trigger-audit.json` alongside.
V1/V2 comparison: `http://127.0.0.1:8767/`, served by the isolated local
`target/phrase-composition/player/serve.py`. The actual in-app browser loaded all
three recordings; muted playback of the final file advanced normally with no
media error. The page was left paused at the beginning, unmuted, volume 50%.
No listening claim is made.

Final validation: **329 release tests passed, 8 optional tests ignored**; strict
Clippy passed. The additional muted operational startup/render/persistence/
transport test passed with the new 32-bar recovery. All 13 hidden Tk tests pass.
The release executable was rebuilt and `teledra.exe --check` passed. Existing
saved user music is preserved until the normal startup render/verification path
publishes the replacement; V1 files were read only.
