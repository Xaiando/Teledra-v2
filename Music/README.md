# Music

Owns the typed Court Score schema, validation, synthesis, WAV rendering,
continuous playback, musical-craft analysis, immutable human feedback events,
and the persistent Organist Studio DAW surface.

At normal Teledra startup the last valid score is restored (or the bounded
default is used), rendered, acoustically verified, persisted as
`music/current_score.json` plus `music/current.wav`, and played as one seamless
Rust-owned audio loop. Organist Studio hot-reloads the same score and provides
transport, movement countdown, arrangement/chord timeline, craft score,
sequencer, per-track gain/pan, reversible mute, keep/reject feedback, immediate
native evolution, a model-composed New Movement, and three bounded change-rate
presets. Every edit returns to Teledra over typed JSONL and is rendered and
verified before it replaces playback; Python never creates a duplicate audio
source. Set `TELEDRA_DISABLE_MUSIC` for an intentionally silent startup.

Every accepted score has a monotonic `evolution` revision. Native evolution is
guaranteed independently of model compliance: normally every 18--36 seconds,
Rust develops exactly one named movement axis—pulse, motif, counterpoint,
texture, harmony, then modulation. The tonic and tempo remain anchors until the
modulation movement; harmony changes retain a diatonic cadence, mixer settings
are preserved, and texture movements re-orchestrate within functional voice
families. Ctrl+U and Evolve Now run that path immediately. New Movement asks
the Organist for an entirely new score, while the slower automatic composer
nudge remains available for larger creative departures.

Accepted scores and audio are independently fingerprinted. The current files
remain under `music/`; the last 64 score revisions are stored in
`music/history/`, and `music/evolution.jsonl` records score and PCM digests for
each accepted playback replacement.

## Engine

| Module | Owns |
| --- | --- |
| `src/lib.rs` | Schema, validation, mix bus, master chain, verification |
| `src/arrange.rs` | Sections, harmony, groove, velocity, variation |
| `src/synth.rs` | Patches, band-limited oscillators, envelopes, filter |
| `src/effects.rs` | Delay, reverb, chorus, sidechain, tone, limiter |

A step is one division of a **bar**, chosen by the pattern's own length: four
steps are quarter notes, sixteen are sixteenths, and the pattern loops for the
whole piece. Patterns longer than sixteen run at sixteenths across several bars.

The piece is divided into **intro, rise, main, break, and finale**. Each section
mutes different parts, sets the dynamics, and drives filter automation. Within
that arc, four-bar phrases state a melody, answer it, develop it, and reprise it;
phrase-ending drum fills can now create transition hits in genuinely empty
slots. The kick ducks everything else in tempo. Melodic parts retain their
identity while resolving onto tones of the current chord.

Rendering is deterministic: the same score always produces the same audio, which
is what makes keep/reject feedback evidence rather than opinion.

## Craft analysis

Every score receives a deterministic 0--100 report across eight objective
foundations: pulse, bass foundation, chordal middle, distinct melody, cadence,
breathing room, instrumental palette, and form. `Masterwork` means all eight
are present; it does not pretend to replace human taste. The report appears in
Studio and is stored with each evolution ledger entry so keeper verdicts can be
compared with concrete arrangement evidence.

Sparse legacy and model scores cross a bounded mastering boundary before live
playback. Existing authored parts are preserved; missing functional seats use
fixed native kick, bass, organ, flute, harp, hat, or piano patterns. With all
eight seats occupied, only a duplicate functional role may be replaced. The
live form is bounded to 8--16 bars and a wandering progression is replaced by a
diatonic tonic-to-cadence loop. If the result still cannot reach concert-ready
craft, it is rejected rather than presented as finished music.

Hear it without starting the court:

```powershell
cargo run --release -p teledra-music --example render_demo -- court.wav
```

## Score fields

Everything below `tracks` is optional; a score written without them still loads.

| Field | Meaning |
| --- | --- |
| `swing` | How far off-beats lean, `0..0.6` |
| `seed` | Fixes the arrangement's variation |
| `sidechain` | Kick ducking depth, `0..0.85` |
| `evolution` | Monotonic accepted-variation revision; absent V1/V2 scores load as revision zero |
| `tracks[].instrument` | Patch role; inferred from the track name when absent |
| `tracks[].gate` | Note length as a multiple of the patch's own, `0.1..4` |
| `tracks[].pan` | Stereo position, `-1..1` |

Roles are `sub_bass`, `bass`, `pad`, `keys`, `pluck`, `lead`, `bell`, `arp`,
`kick`, `snare`, `hat`, and `perc`. Percussion ignores the harmony; the rest
follow the progression.
