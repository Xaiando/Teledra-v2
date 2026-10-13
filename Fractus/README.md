# Fractus

Fractus is Teledra's evolving geometric art studio. Animated fractals and
mandalas anchor the gallery; the Artist can research mathematical ideas,
invent compositions, and develop a study within one visual family at a time.
Gallery selections preserve the chosen subject, seed, palette and geometry
while removing unrelated background or foreground overlays.

## Geometry and motion

The Rust renderer generates every frame. Fractals use smooth escape-time
colouring, orbit detail, seeded boundary views, and periodic parameter motion.
Lotus and star mandalas, flower-of-life lattices, radial weaves, kaleidoscopes,
and phyllotactic structures have distinct geometric constructions. Roulette
curves develop moving pen offsets and layered luminous filaments. Seed changes
affect the construction, not just its colour.

Live scenes use lossless full-colour animated PNG, normally 120 frames at
24 fps, with a five-second seamless cycle. The Studio plays frames against
elapsed time, so resizing or a delayed UI tick cannot progressively slow the
artwork. The still command produces a still; presentation adds no fake drift.
Legacy GIF exports and gallery files remain supported.

```text
version 2
type=lotus_mandala
width=600
height=800
iterations=220
seed=731
palette=electric_cyan
symmetry=12
zoom=1.0
motion=0.65
```

`symmetry` accepts 6..24, `zoom` 0.5..8, and `motion` 0..1. Defaults are
12, 1, and 0.65; previous saved scenes load with these defaults. Each family
interprets geometry controls according to its construction. Use a new seed
for a new variation. `motion=0` freezes its internal geometry. Gallery payloads
use one `type=` and omit `layers=`. This also applies to queued work, saved
lineages and later variations, so old overlays cannot reappear after restart.
For an older layers-only payload, the first layer supplies the family and
palette instead of an unrelated default subject.

The low-level `Scene::parse` and rendering API still support explicitly
authored composites with up to eight validated layers and `normal`, `add`,
`screen`, or `multiply` blending. The gallery applies `single_family_dsl`
before using that renderer; composite support is not a gallery default.

The existing 36 family names and 20 palette names remain available. The
Artist profile lists their exact spellings. Unsupported equations or family
names are reported rather than silently rendered as a default image.

## Exploration and continuity

- Internal animation loops continuously; it does not wait for model replies.
- Routine development asks the Artist for one focused revision of the
  accepted composition, default every 150 seconds
  (`TELEDRA_FRACTUS_EVOLVE_SECS`, minimum 60). Restoring a scene preserves its
  geometry; it does not restart a repeating zoom/motion adjustment cycle.
- A separate Artist exploration cycle researches a geometric idea and asks for
  a complete new executable composition, default every 180 seconds
  (`TELEDRA_FRACTUS_EXPLORE_SECS`, minimum 120). Source excerpts return directly
  to the Artist as untrusted evidence. Unavailable research is reported, and
  an original experiment can proceed without fabricated citations.
- The current authored composition persists across restarts. Only one render
  runs at a time; newer requests supersede queued work, and stale results
  cannot replace the latest requested artwork.

The bounded `.art-learning.json` journal keeps the latest 32 actual outcomes:
canonical scene, parent identity, accepted render facts or an exact failure,
and repeat-attempt counts. Development and research progress survive restarts.
Failed candidates do not replace the successful baseline. Each Artist turn
receives recent evidence and the current baseline; render validity and motion
measurements are explicitly separate from operator preferences or aesthetic
approval. This improves composition through remembered experiments, without
allowing model output to rewrite the application's code. Optional damaged
learning state falls back gracefully and is preserved before replacement.

The court also applies the operator's request to reject overly simple art
before rendering an animation or publishing a still. An area-averaged
128×128 preview must have at least 8% visibly lit pixels (maximum channel
at least 32/255), 1.20 bits of tonal variation across 16 intensity bins, and
6% neighboring-pixel detail (maximum channel difference over 24/255).
Increasing output resolution does not raise this floor. Flat shapes and
smooth gradients fail; coherent detailed single-family compositions can
pass without overlays. These are measured minimums, not beauty scores or
proof of user approval. The manifest and art journal retain the measurements;
failures give the Artist concrete feedback and preserve accepted artwork.
General `Scene::render` remains available for deliberate primitive studies;
the court explicitly calls `Scene::require_visual_richness` for publication.

Rose curves now draw internal petal bands within the same family. `symmetry`
controls petals, `iterations` controls band detail, `seed` changes the geometry,
and `zoom` controls framing. A closed curve is drawn once to preserve its
palette rather than repeatedly accumulating white exposure. Motion has a
closed loop, and `motion=0` preserves a still frame.
The flower-of-life detail setting adds recessed circles inside its existing
lattice cells, giving that family visible depth without a second subject.

Research seeds include [Julia sets](https://mathworld.wolfram.com/JuliaSet.html),
[hypotrochoids](https://mathworld.wolfram.com/Hypotrochoid.html),
[rose curves](https://mathworld.wolfram.com/RoseCurve.html), and
[phyllotaxis](https://mathworld.wolfram.com/Phyllotaxis.html). They are starting
points for experiments rather than instructions to execute downloaded code.
The roulette renderer implements the rolling-circle equations documented by
MathWorld; its seed, pen offsets, and layered presentation are Fractus choices.

## Motion Gallery

Space pauses/resumes. Left/Right inspect frames; Home restarts the loop.
The speed control cycles 0.5×, 1×, and 1.5×. F11 or Present opens a clean
fullscreen canvas; Escape leaves presentation mode. Geometry fits the window
without distortion. Pause and speed survive hot reload. New studies update
the same window without repeatedly stealing focus.

Rust verifies each animation frame before publishing. Animation is bounded to
240 frames and 96 million decoded pixels; Studio rejects oversized files or
frames instead of silently truncating them. Only trusted PNG/GIF artifacts
can load, and the supervised process keeps its existing single-window lock
and cleanup contract.

## Verification

```powershell
cargo test -p teledra-fractus -p teledra
.\.venv\Scripts\python.exe -m unittest discover -s Fractus -p test_studio.py
cargo run --release -p teledra-fractus --example gallery_proof -- target/fractus-overhaul/gallery
cargo run --release -p teledra-fractus --example gallery_proof -- target/fractus-overhaul/gallery --animate --only lotus
cargo run --release -p teledra-fractus --example richness_proof -- target/fractus-richness
```

Gallery proofs are actual production renders with reproducible DSL and
manifests, including frame count, frame rate, colour-preserving output,
distinct-frame counts, motion delta, and loop-seam delta.
