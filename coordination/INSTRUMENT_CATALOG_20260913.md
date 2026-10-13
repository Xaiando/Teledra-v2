# Expanded instrument catalog — 2026-09-13

The operator requested at least 70 instruments, explicit composer awareness,
piano if absent, and confirmed that “pherronome” meant theremin.

## Delivered

The catalog contains **72 distinct synthesized voices**: the existing 25 plus
47 new presets. Piano remains available; electric piano, honky-tonk piano,
harpsichord, clavinet and celesta extend the keyboard choices. The additions
also cover bowed and plucked strings, woodwinds, brass, mallets, electronic
textures and percussion. Theremin has delayed gentle vibrato and pitch glide.

`Music/src/instruments.rs` is the shared authority for exact IDs, labels,
working MIDI ranges, roles, articulation, mix levels, pan and playing advice.
The renderer, theory context, native texture evolution and Studio consume this
catalog. The static generation schema is checked against all entries.

Every Organist composition context includes all 72 exact IDs, grouped by family
with range and role guidance, and marks the current roster. Fresh studies are
encouraged to use complementary unused instruments. The shared Court prompt
also identifies the broader music capability. The eight-track composition
limit remains; instrument availability and simultaneous track count are separate.

Studio exposes a searchable **INSTRUMENTS · 72** library with descriptions and
working ranges. It receives metadata from Rust rather than maintaining a second
Python instrument list. Search does not change playback or generate music.

An initial model draft that hits the actual output limit can now use the unused
second inference for a complete compact regeneration. This preserves the
original brief and explicit longer patterns, never pads or truncates notes, and
keeps the two-call budget. Unrelated model/network failures are not retried;
a failed review retains an already valid draft. Deterministic regressions cover
these paths. Context and output resource limits were not increased.

## Validation

- All 72 instruments survive schema round-trip, score acceptance and verified
  audio rendering. Pairwise normalized synthesis comparisons detect duplicate
  recipes independently of labels, gain, pan and effects.
- All 65 pitched instruments pass pitch checks. Low, middle and high register
  probes for every voice produce finite, audible audio.
- Native texture evolution reaches all 65 pitched voices while retaining each
  part's role and avoiding duplicate timbres; percussion remains an authored
  rhythmic choice.
- Theremin connections use written note timing before gate taper. Connected slow
  notes glide; actual rests and harmony cuts break the connection. Full-render
  regressions distinguish those cases.
- Keyboard variants retain acoustic Piano's harmonic accompaniment treatment
  without turning their single authored notes into automatic chord voicings.
- The expanded Music suite passed, including 95 library tests and five palette
  integration tests. Eleven hidden Studio tests cover catalog receipt, all 72
  entries, searching and unchanged transport.

The local production composer generated a complete 20-second study using
theremin, piano and marimba. Live model attempts and their limitations are
recorded in `target/music-quality/palette-72-model-study/verification-notes.txt`.
These checks establish usable synthesis and executable composition, not
subjective musical appeal or sampled acoustic realism.

Export the current catalog directly from the code:

```powershell
cargo run --release -p teledra-music --example instrument_audit -- target/music-catalog/instruments.md
```

This produces `instruments.md` and `instruments.json` in the chosen directory.
Existing user music and saved scores were left in place. The rebuilt release
is used on the next normal launch through `Teledra.bat`.
