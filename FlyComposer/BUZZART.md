# Buzzart

Buzzart is the Fly performer with a brain that learns. It keeps the real central-complex wiring (51 cell types, 1,309 directed links, 847,131 synapses) and plays through its own copy of the Fly note interpreter. What changes is how the circuit is driven and read, and, when asked, the real music it may lean on and the style it plays in.

## What training may change

The original Fly controller already treats these parts as engineered, so they are the only parts training touches:

| Part | Size | Original Fly |
|---|---:|---|
| Input projection (8 phrase features → cell types) | 51 × 8 | random per seed |
| Readout (cell types → 8 musical decision channels) | 8 × 51 | random per seed |
| Seed context (a 4-number mood per seed → cell types) | 51 × 4 | none |
| Per-cell-type excitability bias | 51 | none |
| Leak, recurrent gain, readout gain | 3 | fixed .28, .97, 2.2 |

That is 1,074 numbers in one brain shared by every seed. Connectome coupling, transmitter signs and normalization never change. A brain with its context set to zero, playing without a songbook or style, reproduces the original Fly performance note for note (see `tests/test_buzzart.py`).

## How it learns

Training is evolution strategies, a reinforcement-learning method that needs no gradients through the note interpreter:

1. Perturb the brain with mirrored Gaussian noise to make a population of candidate brains.
2. Every candidate performs the same training seeds (75 seconds each).
3. A designed critic scores each performance; candidates are ranked.
4. The brain moves toward the better-ranked perturbations (Adam), with a small pull back toward its starting point.
5. Held-out seeds show whether the improvement generalizes. The best held-out brain is kept.

With a songbook, fitness is the critic's score scaled by naturalness: `critic × ((1 − w) + w × naturalness)`, where each drill sets `w`.

## The critic

`flycomposer/buzzart/critic.py` scores seven areas, each from measurements with a target range rather than "more is better":

- **Melody**: stepwise motion, purposeful leaps, few large jumps or repeated notes, contour turns, range.
- **Breathing**: rests between foreground phrases, no long silences.
- **Rhythm**: onset-gap variety, no single dominant gap, a balance of on-beat and off-beat notes.
- **Motif**: interval patterns that recur without looping, distinct phrases, use of musical memory.
- **Harmony**: chord tones on strong beats, few clashing intervals, root changes and motion by fifths.
- **Texture**: note density, every part present, sensible polyphony.
- **Form**: density and dynamics arc, register movement, a recognizable return, ending on the tonic.

The reward is 75% the weighted mean and 25% the weakest area, so one trick cannot win. Variety across seeds counts for up to 40%: a composer that plays one piece for every seed fails. The critic is a transparent measurement, not listener taste; your Keep and Reject verdicts are the real test.

In console style a few ranges follow the chiptune songbook's own lead lines, measured on its 19 pieces: those leads are legato (median breath share 0.01), and one note length often dominates (median 65% of gaps), so breathing may reach zero and the dominant-gap ceiling is 70% instead of 45%. Density and form are measured on lead, answer and bass, because drums and arpeggios are dense by design. The off-beat range stays, since the corpus sits inside it (median 35%).

## Songbooks: real music to lean on

A songbook holds statistics and short motifs learned once from openly licensed music. No recording and no whole melody is stored, and nothing copyrighted is used.

| Songbook | Sources | Licence | What it holds |
|---|---|---|---|
| `data/foundation/songbook.json` | music21 corpus: O'Neill's Music of Ireland (1,341 tunes), Ryan's Mammoth Collection (1,059), 240 Bach chorale sopranos | public domain | 2,640 melodies, 264,186 notes, 900 motifs, chord-root motion from the chorales |
| `data/foundation/console.json` | 19 chiptune MIDI files from OpenGameArt | CC0 | 2,461 lead notes, 64 motifs, tempos 72–180 BPM, 743 bars of kick/snare/hat grids, bass pulse and arpeggio habits |
| `data/foundation/amiga.json` | 45 ProTracker modules from The Mod Archive | Public Domain on the archive | 16,922 lead notes, 122 motifs, tempos, 3,070 drum bars, octave-jumping bass, arpeggio-effect use |

Each file has a manifest with its SHA-256, and `data/foundation/sources/` records every source page and file hash. Builders: `songbook_build.py`, `console_build.py` (both need music21, in `..\.venv`) and `amiga_build.py`.

Modules and chiptune MIDI files carry no reliable instrument names, so parts are found by behaviour. A drum is a sample played at one or two pitches, many times, in short notes; where it lands in the bar says kick, snare or hat. A lead has several pitches over a real range and is not one note retriggered. A part that leaps on most notes is an arpeggio, not a tune.

How Buzzart uses a songbook:

- **Vocabulary.** Intervals, note lengths and rests keep the songbook's proportions. Options are laid out from smallest to largest along their cumulative frequencies, and each drive's rank among the performance's recent drives picks the note. The circuit decides which notes are long, which leap and where rests fall; a whole piece still keeps real proportions.
- **Intervals keep their size.** A songbook octave leap is played as an octave, not folded into a repeated note; the register still folds lines that run off its edge.
- **Harmony.** Chord-root choices lean toward the songbook's transitions.
- **Memory.** Each performance opens with four songbook motifs to recall, continue and invert.
- **Naturalness.** How close a performance's intervals, onset gaps and root motion are to the songbook's own held-out average, from 0 to 1.

## Console style

`console_style.py` turns the songbook's idioms into arrangement: tempo from the corpus's tempo list, broken-chord arpeggios instead of held chords, a pulsing eighth-note bass, and a kick/snare/hat bar grid whose hit counts follow the source music. Voices are synthesized in `chip.py`; none are recordings:

| Palette | Lead | Answer | Arpeggio | Bass | Drums |
|---|---|---|---|---|---|
| `nes` | 25% pulse | 12.5% pulse | square | 4-bit triangle | noise |
| `genesis` | 2-operator FM | 25% pulse | FM bell | FM bass | noise |
| `amiga` | 8-bit saw loop | 8-bit square pluck | 8-bit organ loop | 8-bit square-saw loop | 8-bit drum samples |

The Amiga voices imitate the Paula chip: short 8-bit loops stepped through without interpolation, 64-step volume, drum samples made at tracker sample rates (8,363 Hz for the kick), and a moving average standing in for the A500's fixed low-pass.

## Calibrations, and the evidence for them

- **Vocabulary by rank.** The first rule weighed log-frequency against distance from the drive. Drives crowd toward −1 and 1, so two-beat notes filled 40–55% of the console lead against 4% in the songbook, and half-beat rests 28–67% against 9%. Ranked choices land within a few points of the songbook for untrained and drilled brains alike.
- **Openings by rank.** Key, mode, tempo and voices come from the first phrase's outputs, which saturate: untrained and drilled brains put 13–14 of 16 seeds in A minor. With a songbook or style, each output is ranked against the same brain's openings for 24 fixed reference seeds; 16 console seeds from the drilled brain then spread across D, F and A, minor, dorian and major, at 72–180 BPM.
- **Arpeggio parts out of the lead tables.** Four CC0 files had broken-chord parts picked as leads, which put 18% octave leaps into the interval table; excluding parts that leap on most notes brought that to 0.9%.

## Drills

A drill is a stage of training that puts extra weight on some critic areas:

| Drill | Emphasis | Naturalness weight |
|---|---|---:|
| `foundation`, `integration` | the full critic, evenly | run default (.35) |
| `melody` | melody ×3, motif ×1.5 | run default |
| `rhythm` | rhythm ×3, breathing ×1.5 | run default |
| `harmony` | harmony ×3 | run default |
| `form` | form ×3, motif ×1.5 | run default |
| `songbook` | melody ×1.5, motif ×1.5 | .6 |
| `arcade` | rhythm ×1.5, texture ×1.2, form ×1.2 | .5 |

## Commands

Run from this folder. Training and evaluation use the Fly Composer environment; plots need matplotlib, which the Teledra development environment (`..\.venv`) provides.

```powershell
.\.venv\Scripts\python.exe buzzart.py train --run drills-1
.\.venv\Scripts\python.exe buzzart.py train --run arcade-1 --start brains\buzzart.npz --songbook data\foundation\console.json --style data\foundation\console.json --plan arcade:40,form:40,songbook:40,arcade:40,integration:30 --population 80 --seeds 8
.\.venv\Scripts\python.exe buzzart.py evaluate --brain training\buzzart\arcade-1\best.npz --songbook data\foundation\console.json --style data\foundation\console.json --first-seed 70000 --count 32
..\.venv\Scripts\python.exe buzzart.py observe --run arcade-1
.\.venv\Scripts\python.exe buzzart.py showcase --run arcade-1 --seeds 70003 --songbook data\foundation\amiga.json --style data\foundation\amiga.json --palette amiga
```

`train` resumes a run that already exists and keeps the songbook and style it started with; it refuses if either file has changed since. Runs are saved under `training\buzzart\<run>`: `history.jsonl` (every generation), `brains\gen-XXXX.npz` (validation checkpoints), `best.npz`, `latest.npz` and optimizer state. `showcase` renders the original Fly and Buzzart on the same seeds with the same instruments and matched loudness, then adds a session to the listening library. Close the Fly Composer server before running it; the server keeps the library in memory and would overwrite the new session.

## Results

### arcade-1 (2026-09-15)

Five drills, 190 generations, starting from the drills-1 brain: arcade 40, form 40, songbook 40, arcade 40, integration 30. Each generation, 80 candidate brains performed 8 training seeds with the console songbook and console style, on 10 worker processes; the run took 13 minutes. Held-out validation rose from 0.746 to a peak of 0.852 at generation 110, then held near 0.82 while the training-seed mean kept rising. The generation 110 brain is kept as `training\buzzart\arcade-1\best.npz` and copied to `brains\buzzart-arcade.npz`.

Scores on 32 test seeds (70000–70031) with the console songbook and style. The objective is the arcade drill's: critic fitness × (0.5 + 0.5 × naturalness). The original Fly plays its own arrangement and is judged by the standard critic.

| Author | Objective | Fitness | Naturalness | Melody | Breathing | Rhythm | Motif | Harmony | Texture | Form | Diversity |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Original Fly | 0.594 | 0.748 | 0.588 | 0.79 | 0.87 | 0.69 | 0.67 | 0.84 | 0.95 | 0.79 | 1.00 |
| Buzzart, untrained | 0.662 | 0.715 | 0.852 | 0.88 | 1.00 | 0.72 | 0.67 | 0.82 | 1.00 | 0.76 | 0.81 |
| Buzzart, drills-1 | 0.734 | 0.787 | 0.865 | 0.93 | 1.00 | 0.94 | 0.86 | 0.94 | 1.00 | 0.65 | 0.85 |
| Buzzart, arcade generation 110 | 0.819 | 0.847 | 0.934 | 0.94 | 1.00 | 0.95 | 0.93 | 0.94 | 0.97 | 0.73 | 0.93 |

The final brain (generation 190) scores the same within noise: objective 0.817, fitness 0.845.

Measured changes from drills-1 to arcade generation 110 (means over the test seeds):

- Sections now thin out and fill in: density variation across quarters rose from 0.08 to 0.21, with arpeggio notes per bar falling from 15.5 to 11.8 and drum hits from 7.4 to 5.3.
- Stepwise motion rose from 38% to 43% of intervals; leaps of five semitones or more fell from 26% to 21%.
- Naturalness against the chiptune songbook rose from 0.865 to 0.934.
- Keys and tempos stayed varied: 11 keys over 32 seeds, 72–180 BPM.

With the Amiga songbook and style, which neither brain trained with, the objective rose from 0.678 to 0.704 and fitness from 0.764 to 0.794, while naturalness (0.77) and melody (0.75) did not move.

Limits of this run:

- After generation 110 the population kept improving on training seeds but not on validation seeds, so the last 80 generations mostly fitted the training seeds.
- Form is still the weakest area (0.73). The answer voice shrank from 1.8 to 1.1 notes per bar, so the lead carries almost every tune.
- Amiga lead lines repeat notes far more than the critic's melody range allows, which caps melody with that songbook.
- These are critic and songbook measurements, not listener approval.

Evidence: `training\buzzart\arcade-1\train.log`, `history.jsonl`, `test-seeds-70000.json` and the plots in `observe\`.

Listen in Fly Composer (http://127.0.0.1:8789/). Each session plays one test seed as the original Fly, Buzzart drills-1 and Buzzart arcade generation 110, with one set of console voices and matched loudness:

| Session | Original Fly | drills-1 | Arcade gen 110 |
|---|---:|---:|---:|
| Buzzart arcade · NES voices · seed 70003 | 0.723 | 0.799 | 0.845 |
| Buzzart arcade · Genesis voices · seed 70011 | 0.737 | 0.820 | 0.895 |
| Buzzart arcade · Amiga voices · seed 70017 | 0.752 | 0.854 | 0.810 |

On the Amiga seed the arcade brain scored below drills-1; it played a much sparser piece (538 notes against 1,156).

### drills-1 (2026-09-14)

Six drills, 230 generations: 80 candidate brains per generation, each performing 8 training seeds, trained on 10 worker processes in about 12 minutes, without a songbook or style. The best held-out brain came from the final generation (validation fitness 0.890). It is saved as `training\buzzart\drills-1\best.npz` and copied to `brains\buzzart.npz`.

Scores on 32 test seeds (70000–70031) that neither training nor validation used:

| Author | Fitness | Melody | Breathing | Rhythm | Motif | Harmony | Texture | Form | Diversity |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Original Fly | 0.748 | 0.79 | 0.87 | 0.69 | 0.67 | 0.84 | 0.95 | 0.79 | 1.00 |
| Buzzart, untrained | 0.697 | 0.80 | 0.78 | 0.63 | 0.61 | 0.87 | 0.95 | 0.81 | 0.92 |
| Buzzart, generation 230 | 0.892 | 0.85 | 0.97 | 0.88 | 0.94 | 0.98 | 0.97 | 0.86 | 1.00 |

Measured changes from the original Fly to Buzzart (means over the test seeds):

- Foreground leaps of five semitones or more fell from 35% to 23% of intervals.
- Off-beat onsets fell from 81% to 66%.
- Recurring three-interval motifs rose from 17% to 39% of patterns.
- Chord tones on strong beats rose from 76% to 87%; root motion by fifths from 4% to 15%.
- The longest silence fell from 0.74 s to 0.40 s; velocity range across the piece rose from 0.037 to 0.053.

Limits of this run:

- Stepwise motion stayed near 19%. The Fly interpreter moves in scale degrees, and a two-degree move is a third, which the critic counts as neither a step nor a leap, so that target may be out of reach without changing the interpreter or the critic.
- Buzzart plays more notes (3.7 per second against 2.4) and uses a narrower register arc (4.6 semitones against 5.9).
- These are critic measurements. They show that training moved the music toward the critic's targets on unseen seeds, not that listeners prefer it.

Evidence: `training\buzzart\drills-1\history.jsonl`, `test-seeds-70000.json` and the plots in `observe\`. The drills-1 figures above were measured before songbooks, console style and the calibrations existed; performances without a songbook or style are unchanged by them.

Listen in Fly Composer (http://127.0.0.1:8789/): the sessions **Buzzart training check · seed 70003** and **Buzzart training check · seed 70017** play the original Fly, untrained Buzzart and generation 230 on test seeds, with the same instruments and matched loudness.
