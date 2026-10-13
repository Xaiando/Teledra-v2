# Music

Music owns the Court Score, a theory-aware arranger, synthesis, verified audio,
and the persistent Organist Studio. It also restores V1's PyMusic composition
workflow. Rust owns playback for both engines; the editors never open an audio
device themselves.

## PyMusic and engine comparison

`/PyMusic` opens the Python editor and closes native Organist Studio. `/Synth`
does the reverse. `/Music` resumes the selected engine. Manual switching stops
the old sound before rendering or starting the selected piece. With automatic
comparison enabled, the next composition alternates between native
Court Score and Python. `/MusicCompare` or either studio's Alternate button
toggles this behavior; the selection is saved across launches. Completed
automatic replacements wait for the current playback pass to finish.

With comparison disabled, development stays in the selected engine. PyMusic
revisions start from the accepted source, key and tempo, with focused
`PYTHON_MUSIC_EDIT` transactions instead of rewriting the whole working piece.
The runtime preserves other layers, renders the full edited arrangement, rejects
identical generated audio, and makes the accepted revision the next baseline.
Small revisions and their actual outcomes accumulate in `music/learning.json`;
operator feedback remains separate from technical verification. Editor drafts
do not silently replace the accepted parent. Verified replacements reuse one
audio output stream with a 40 ms smooth overlap; pause and mute survive.

Executable Python composition uses an artifact-only, schema-constrained model
request. The restored V1 arrangement exposes typed controls for answering-phrase
timing, note length, phrase breaths and bass articulation. The model chooses
musical changes; Rust updates the working code and corresponding note-event
declarations together. Other freeform sources use numbered line replacements.
Rust preserves indentation and line endings, attaches the accepted source
identity and checks the exact transaction. New
background pieces use a bounded musical recipe: the model chooses actual theme
and answer pitches, phrase rhythms, dynamics, harmony, timbres and section form.
The local `render_song` helper constructs the complete PyMusic mix; editable
Python source remains the saved artifact. Court dialogue
and its prose refiner do not process these generations. Only actual recent
outcomes and operator guidance return as learning context; old request examples
and placeholder identities do not. A failed edit gets one factual repair within
the same 90-second composition budget.

The restored `teledra_synth.py` retains V1's NumPy note synthesis, timed additive
mixing, stereo placement, automation, delay, reverb, filtering and granular
helpers. Python can arrange simultaneous notes and layers freely. It also has
all 72 native instruments, including piano and theremin, through:

```python
from teledra_synth import instrument_note, mix_waves, play_sound

a = instrument_note("C4", 4.0, instrument="piano", sr=48000)
b = instrument_note("E4", 1.5, instrument="theremin", sr=48000)
piece = mix_waves(a, b, start_time=1.0, sr=48000)
play_sound(piece, sr=48000, loop=True)
```

`instrument_note` uses the same native instrument bank as Court Score; V1's
`synth_note` retains its classic waveform synthesis. The comparison therefore
covers both authoring/effects workflows, with a shared additional sound palette.
Numeric pitches are Hz and durations include release. Use one sample rate
throughout a composition. `play_sound` captures one complete array; the worker
exports a verified stereo 48 kHz WAV after the script finishes. It opens no live
audio stream. The script has NumPy, math and the synth helpers, with bounded
array sizes and a 120-second worker timeout. File, network and process APIs
are unavailable to authored code.

The Organist receives the Python API and all 72 instruments at dispatch; both
editors also expose the catalog. A fresh Python session starts with the unchanged
V1 `Fivefold Court Engine` source, bundled as `Music/v1_music_example.py`.
`Music/pymusic_example.py` additionally demonstrates the expanded instrument
bank in a complete layered study. Python artifacts retain source, audio and a report under
`music/python/render-*` in the application data directory. A manifest identifies
the last accepted artifact; failed renders preserve it. Editor drafts and
Keep/Reject feedback are separate from accepted audio and native score feedback.
Play restores the last accepted artifact even when an editor draft is incomplete.
Failed model compositions receive at most one correction request per accepted
Python piece, including the exact error and failed source. Manual Run errors
remain in the editor for the operator; no new piece is accepted on failure.

The report counts every completed note-helper call and rejects a failed helper
even if authored code catches its exception. This proves helper completion;
arbitrary NumPy code can still omit a generated array from its final mix, so the
counter is not a claim to audit that code's musical intent. Native short-note
tests cover every instrument, and explicit Court Score note audits remain
available separately.

The song renderer additionally records every actual additive placement into five
independent parts, layer levels, section levels, and the full event ledger. Its
`composition_audit` is attached only when the rendered PCM matches the final
captured array. Mutating or replacing the array cannot retain that completeness
claim. Scalar exercises, missing breaths, incomplete form and silent parts fail
before publication. This measures construction, not musical taste.

## Library and background study

Complete published songs enter `music/python/library.json`, retaining their
source, WAV and report in immutable render directories. Source/PCM duplicates
share a catalog entry. The PyMusic Library panel loads a saved track while the
court continues playing; manual selection fades in once loaded. Automatic
replacements finish the current pass. Keep and Reject apply to the exact playing
library ID and remain separate from technical
verification. These entries are ready for future playlists; automatic playlist
shuffle is not implemented.

The Organist develops existing pieces and starts a new track every fourth Python
composition request, using an engine-specific persisted counter. A failed new
piece stays the pending direction; the previous complete audio keeps playing.
Generated fragments shorter than 30 seconds cannot replace it. At startup an old
short fragment is recovered to the bundled full V1 reference; its old artifact
and draft backup remain available.

Scheduled Wizard work alternates app experiments and music study, with the
counter and evidence in `tower/wizard_work.json`. The Wizard sees actual recipe,
render facts and outcomes; its advice reaches the next Organist request as an
unverified hypothesis tied to the inspected source. Neither technical success nor
a Wizard comment substitutes for the listener's Keep/Reject verdict.

## Composing and experimenting

The Organist receives a concise theory workbench plus an analysis of the actual
accepted score at each turn. The analysis identifies the scale, Roman-numeral
harmony, tonal or modal arrival, motif contour and intervals, repetition, rests,
and possible voice-leading moves. It describes the notation, not subjective
listening. Keep/Reject feedback is scoped to the piece and comments supplied.
When a score is a repeating scale exercise, the model instead receives a compact
repair brief with its key, tempo, useful harmony and playing limits. The rejected
note arrays are omitted so they do not become the next draft's example.

Composition nudges rotate through twelve specific studies: motif development,
harmonic rhythm, contrary motion, phrase lengths, call and response, pedal tones,
chamber writing, ambient writing, syncopation, a rondo, harmonic colour, and
contrast through register and articulation. These are optional directions;
reported register, duplicated-part and phrase-space problems take priority over
novelty. The Organist writes a complete plan and actual contrasting material.
One composition can wait for model capacity; native automatic
evolution holds while that composition is queued or running.

New model scores use `score.schema.json`. Each authored note is one object,
such as `{"degree":2,"velocity":0.7,"duration":0.5}`; a rest has `degree:null`.
Pitch and expression cannot become misaligned parallel lists. The generation
schema supports 4/8/16/32/64 note positions per pattern and requires a complete
composition plan. `Score::from_json` losslessly maps these events to the existing
renderer format; old persisted scores still load. A valid first draft receives
its own theory analysis and measured note, rest, pitch and expression counts
for one musical revision against the original brief. A failed review keeps the
valid draft. An invalid first draft instead gets one precise structural repair;
failed repair retains previous playback. If the first draft reaches the model's
output-token limit, the typed truncation signal uses the remaining call to
regenerate a complete compact score. That regeneration preserves the original
brief and explicitly requested longer patterns; otherwise it prefers concise
4/8/16-event cells. A truncated review retains its already valid draft. Other
initial model or connection errors return immediately without a blind retry.
All paths use at most two model calls, and unfinished JSON is never accepted.
The helper never pads or cuts a melody merely to make the data fit.
New composition acceptance requires every native score and track field; a
partial response cannot inherit legacy defaults and become an unintended piece.

## Written intent

```json
{
  "form": "ternary",
  "phrase_bars": 4,
  "chord_bars": 2,
  "steps_per_bar": 8,
  "development": 0.35,
  "texture": "chamber",
  "harmony": "diatonic"
}
```

Put this object in the score's `composition` field. Forms are `arc`, `binary`
(A-B), `ternary` (A-B-A), and `rondo` (A-B-A-C-A). Explicit form boundaries fall
on whole phrases when the piece is long enough, and returning sections restart
the written motif and harmony together. `phrase_bars` accepts 2/4/8;
`chord_bars` accepts 1/2/4. Optional `steps_per_bar` accepts 4/8/16, with 16 as
the compatibility default. Studio uses the renderer's actual bar timelines.

`development` ranges from 0 to 1. Zero preserves written melodic degrees,
rhythmic cells and the patch's filter setting; positive values permit formal
pitch and filter contrast and passing-tone-aware harmonic arrivals. Planned
scores always retain authored attacks and rests: instruments' spacing advice,
random thinning and section presets cannot silently remove written notes.
`texture` is `ensemble`, `chamber`, or
`ambient`; it influences articulation and brightness while preserving every
explicitly authored track.

With `harmony=diatonic`, incompatible chord qualities are repaired against the
selected scale, preserving seventh chords when supported. `chromatic` preserves
intentional outside-scale chord colours. An explicit plan preserves the chosen
length, ensemble, progression shape, and rests. It does not acquire a mandatory
kick, five instruments, or two minutes of padding. Legacy unplanned scores retain
the previous bounded ensemble-completion path.

## Harmony, melody, and expression

- Modes: major, natural minor (`minor`), dorian, mixolydian, lydian, phrygian,
  and harmonic_minor. Tonic is a pitch class 0..11; steps are zero-based scale
  degrees 0..20. Degree 0 already means the selected tonic, not always C.
- Qualities: major, minor, diminished, augmented, suspended2, suspended4,
  major_seventh, minor_seventh, dominant_seventh, and half_diminished.
- Organ, pad and keys play actual three- or four-note voicings. Inversions and
  nearby common tones reduce inner-voice travel; each chord's gain is balanced.
  Piano and harp remain useful for independently written melodic answers.
- Weak-beat passing and neighbouring notes survive. Structural arrivals can
  resolve to chord tones. Planned bass parts can walk or state inversions;
  legacy bass patterns retain their root/fifth interpretation.
- Developed diatonic parts resolve long held offbeat notes and release tones
  that conflict with the next chord, with a short fade at the exact boundary.
  Common tones can continue. Zero development and chromatic plans retain
  authored suspensions. Ultrasonic oscillator partials are suppressed instead
  of being clamped to an unrelated audible frequency.
- In persisted scores, optional `tracks[].velocities` (0..1) and `durations` (0.125..8 quarter-note
  beats) align one-for-one with `steps`, including numeric placeholders at rests.
  Zero velocity is silent. Explicit duration overrides the patch gate;
  monophonic voices release before their next attack, while polyphonic piano and
  chord voices retain overlapping authored holds. Optional `gate` is 0.1..4,
  `pan` -1..1, `gain` 0..1.

The transport is currently 4/4. A pattern divides each bar into the smaller of
its event count and `steps_per_bar`: four steps are quarters; eight are eighths
unless the plan selects grid 4. Longer patterns continue over several bars,
up to 1024 saved positions, enough to write all 64 supported bars explicitly.
The model's atomic-note schema remains bounded to 64 events per track. Native
compositions and saved scores can use the full timeline. For example, 64 events on grid 8 span eight complete bars;
32 events on grid 4 also span eight. Omitted grid settings retain the original
sixteenth-note timing for long patterns. Swing (0..0.6) leans
supported offbeats; sidechain (0..0.85) ducks accompaniment under kick attacks.
Scores remain bounded to 40..220 BPM, 1..64 bars, 0..16 chords and 1..8 tracks.
Duration changes held length only. In a sixteen-position pattern, duration 0.5
on the default grid does not move the next onset from 0.25 to 0.5 beats. Use a
coarser grid, fewer positions or null spacers for slower parts. Automatic composition checks the selected instrument's
minimum attack spacing before accepting notes that the arranger would drop.

The synth has **72 playable instrument patches**, including acoustic-style piano
and theremin. `src/instruments.rs` is the shared catalog for exact score IDs,
families, roles, ranges, articulation, mix defaults and patch descriptions.
The Organist receives the complete grouped catalog in normal score context, with
current instruments marked and advice to choose complementary voices without
an instrument quota. Compact scalar-repair briefs retain the current voices'
playing limits; all 72 IDs remain available in the generation schema.
Native variations preserve an explicitly composed ensemble;
legacy scores without a composition plan retain their role-based texture changes.
These are distinct synthesized interpretations with individual envelopes and
spectra, rather than sampled acoustic recordings.

- Keyboards and free reeds: piano, electric_piano, honky_tonk_piano, keys, organ,
  harpsichord, clavinet, celesta, accordion, harmonica.
- Strings and basses: violin, viola, cello, double_bass, string_ensemble,
  pizzicato_strings, tremolo_strings, bass, sub_bass.
- Plucked strings: harp, guitar, steel_guitar, electric_guitar, jazz_guitar,
  muted_guitar, banjo, mandolin, ukulele, sitar, koto, oud.
- Winds: flute, reed, clarinet, oboe, piccolo, recorder, pan_flute, shakuhachi,
  bassoon, english_horn, soprano_sax, tenor_sax.
- Brass: horn, trumpet, trombone, tuba, flugelhorn, brass_ensemble.
- Mallets and bells: bell, vibraphone, marimba, xylophone, glockenspiel,
  tubular_bells, dulcimer.
- Electronic and vocal: pad, pluck, lead, arp, theremin, choir, synth_brass,
  warm_pad.
- Percussion: kick, snare, hat, perc, timpani, shaker, conga, cowbell.

The catalog lists sounding MIDI ranges (C4 = 60), while score notes remain
zero-based scale degrees. Choose the octave so the whole melody fits its voice.
If a single-note part exceeds the working register, the renderer chooses one
octave shift for the whole line. It preserves the melodic intervals instead of
wrapping individual notes into unexpected octave drops. The score analysis
reports remaining range problems for the composer to fix.
Theremin uses a clear single-note line with developing vibrato. Piano, mallets
and plucked strings can provide articulated answers; sustained voices need
space for their attacks and phrase releases. A larger palette does not increase
the eight-track limit or force a large ensemble.

Mastering limits peaks without boosting quiet scores. Track gain and dynamics
therefore control the actual output level, including a sparse solo. Audio checks
retain a silence floor and check section activity relative to that output level.
Unison oscillators retain a strong central fundamental, and triangle oscillators
correct their corners to reduce folded upper harmonics. Short authored notes
adapt an otherwise too-long envelope attack to their gate, so they can speak.
The master preserves the musical frequency range, with rumble and ultrasonic
filtering, instead of suppressing percussion through a fixed treble cut.
Render allocation and loop wrapping include complete note holds and releases,
even when a final note continues for more than one short loop cycle.

Studio's **SCORE CHECKS** describe notation diagnostics, not musical appeal.
Range overflow, identical pitched parts, and written durations that fill intended
rests are reported explicitly. A declared form with no contrasting material
does not count as developed form. Automatic composition requests repair the
observed failure pattern of an entire doubled, out-of-range melodic ensemble
before accepting it; ordinary score loading still permits intentional doubling.
Long uniform scale walks repeated as the dominant musical material are also
blocked, including in-range solos. Brief runs, shaped stepwise phrases, drones
and arpeggios supporting an independent melody remain possible. This diagnosis
does not certify that every other pattern is good music.

## Continuity and Studio

Startup restores and verifies the last score, or uses the bounded default.
If the saved score is a repeating scale exercise, startup archives its complete
JSON under `music/rejected/` and composes a 32-bar layered movement before
publishing audio. This native composer develops one of three authored motif
contours through an opening, full statement, contrast and return. It preserves
key and tempo. Sustained harmony continues through the lead's breaths, bass and
offbeat guitar supply motion, and piano answers and a restrained pulse enter
later. The return sheds density. All 32 bars are written explicitly on grid 8;
they are not four copies of the same eight-bar cell. Chord voicings remain near
their authored register instead of drifting into the melody's range.
The ordinary render and audio-verification path
must finish before the accepted score, WAV and Studio change. Evolve Now uses
the same recovery if the current score still contains the scalar failure.
If the restored score has the known duplicated/out-of-range ensemble problem,
startup requests one composer repair immediately while retaining the saved work.
`TELEDRA_DISABLE_MUSIC` disables automatic music startup. Ctrl+U / Evolve Now
runs native variation; New Movement asks the Organist for fresh composition.
Automatic development normally starts 55% into the piece (30..120 seconds),
preparing its successor while accepted music plays. One cadence handles both
engines and retains an overdue request when the composer is busy. Explicit
`TELEDRA_MUSIC_VARIATION_SECS` or `TELEDRA_MUSIC_EVOLVE_SECS` overrides remain
available; `TELEDRA_MUSIC_EVOLVE_SECS=0` disables automatic composition requests.
Automatic native development also goes through the composer; explicit native
variation controls still provide the deterministic transformations below.
The Studio offers 2/5/10-minute presets, pause/play/stop, gain/pan, reversible
mute, Keep/Reject, and the movement countdown.

Native variations develop one axis: pulse, motif, counter-line, texture,
harmony, then modulation. Planned motif turns use sequence, inversion, and
retrograde on interior cells while retaining opening/ending anchors. Harmony
turns can extend one chord without rewriting the progression. Explicit zero
development preserves melodic cells. Pulse revisions change articulation rather
than reviving a muted mixer channel. One render runs at a time; the latest
pending revision supersedes older queued work. Pause survives replacement.
Fully rendered new movements wait for the current playback pass to finish;
Studio shows when the next movement is ready. The transport counts consumed
audio frames, so a pause cannot use up that remaining playback time. Mixer
revisions keep the current position instead of restarting the piece. Stop
cancels a waiting movement.

Accepted scores and audio have independent digests. Current files are under
`music/`; the last 64 score revisions are in `music/history/`; `evolution.jsonl`
records accepted changes. The same score deterministically renders the same PCM.

Studio shows theory observations, a proposed next experiment, the exact form and
chord timeline, sequencer, mixer, and a structural craft report. Chamber and
ambient plans are assessed without mandatory full-ensemble roles. Craft scores
are guidance, not aesthetic certification. The audio verifier checks bounded
finite audio, level, activity and loop continuity. Its autocorrelation statistic
is descriptive: clean polyphonic chords can score lower than a single tone.
The **INSTRUMENTS** browser searches the full runtime catalog by name, family,
role or description and shows each voice's range and playing advice.
The sequencer follows successive pages of long written arrangements. The
`note_trigger_audit` example reports every actual grid occurrence, distinguishing
scheduled triggers, rests, zero velocity, muted tracks and legacy omissions;
chord voice counts are separate from trigger counts.

## Verification and reference material

```powershell
cargo test --release -p teledra-music -p teledra
.\.venv\Scripts\python.exe -m unittest discover -s Music -p test_daw.py
cargo run --release -p teledra-music --example render_demo -- court.wav
cargo run --release -p teledra-music --example periodicity_probe
cargo run --release -p teledra-music --example palette_audition
cargo run --release -p teledra-music --example compose_study -- score.json target/native-study
cargo run --release -p teledra-music --example compose_study -- score.json target/layered-study --layered
cargo run --release -p teledra-music --example note_trigger_audit -- score.json
cargo test --release -p teledra --test organist_composition -- --ignored --nocapture
cargo test --release -p teledra --test organist_palette -- --ignored --nocapture
```

The optional model test produces an isolated theory-guided score and verified
WAV under `target/organist-overhaul/model-study`; it never starts playback.
If the production executable is running and locked on Windows, append
`--target-dir target/speech-update` to Cargo commands.

The original in-code theory guide draws on standard concepts explained in
[Open Music Theory: harmonic functions](https://open-musictheory.github.io/docs/harmony/harmonicFunctions/),
[Open Music Theory: cadences](https://openmusictheory.github.io/cadenceTypes), and
[Puget Sound: period form](https://musictheory.pugetsound.edu/mt21c/PeriodForm.html).
