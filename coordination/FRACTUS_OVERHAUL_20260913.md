# Fractus overhaul — 2026-09-13

The operator requested animated fractals, mandalas, and frequently invented
geometric art, with the Artist researching ideas and trying new compositions.

## Delivered behavior

- Rebuilt the main fractal and radial constructions with smooth fractal
  colouring, seeded boundary views, detailed mandalas, layered petals,
  ornament, lattices, and motion within the geometry. Roulette families use
  rolling-circle geometry with luminous moving filaments. Seeds affect shape.
- Live renders are full-colour lossless APNG: 120 frames at 24 fps, normally
  a five-second loop. Still requests remain still. Explicit zero motion stays
  frozen during native variation.
- The Studio uses elapsed-time playback, bounded decode/cache budgets, accurate
  pause and frame stepping, speed controls, and fullscreen presentation.
  Updates preserve playback preferences and do not repeatedly steal focus.
  Titles come from validated scene metadata instead of timestamp filenames.
- The Artist receives a geometric research excerpt and an invitation to make
  an executable original composition every 180 seconds by default, when model
  capacity permits. This is independent of native variation every 150 seconds.
  The model can combine the supported constructions and controls; it does not
  install arbitrary new renderer algorithms from retrieved text.
- Research returns directly to the Artist. Empty or blocked pages are rejected,
  missing evidence is acknowledged, and redundant requests for a source already
  supplied in the exploration prompt are suppressed.
- Authored compositions persist with atomic replacement. Native variation
  derives from that base instead of accumulating drift. One render runs at a
  time; pending work is replaceable, and obsolete completions cannot publish.
  Operator selections invalidate obsolete exploration work.

## Verification

- Final `cargo test -p teledra -p teledra-fractus -p teledra-agents`: 105 pass,
  four optional live/hardware tests ignored in the ordinary suite.
- The optional live research-to-render integration test was run separately
  against the installed model and a public MathWorld hypotrochoid source. It
  produced a valid original `orbital_lace` composition and a verified animation
  with 120 distinct frames. Artifacts are isolated under
  `target/fractus-overhaul/discovery`.
- Studio Python suite: 20 pass. A hidden Tk probe decoded the real 120-frame
  lotus animation, advanced in line with wall time, preserved its paused frame,
  reused its canvas image, and remained within the cache budget.
- Nine gallery studies rendered successfully. Every manifest reports 120
  distinct frames, 24 fps, and a five-second loop. Still frames and a contact
  sheet were visually inspected. Reproducible DSL, APNG files, and manifests
  are under `target/fractus-overhaul/gallery`.
- A video of three actual production renders is saved at
  `target/fractus-overhaul/fractus-preview.mp4`.
- Strict Clippy for Teledra and Fractus, workspace formatting, and diff
  whitespace checks pass. Final staging release build and its read-only
  `--check-environment` pass.

## Activation and practical limits

The existing court (PID 44300 during verification) was left running. The new
executable is built at `target/speech-update/release/teledra.exe`; the normal
Teledra shortcut rebuilds and activates the changes on restart. This build
also includes the speech-flow fixes documented in `SPEECH_FLOW_20260913.md`.

The research cycle and Studio were exercised in isolation. A sustained session
with the newly built court has not yet been observed. Exploration waits for
available inference capacity; 180 seconds is its default cadence, not a hard
deadline. Detailed fractal renders are slower than radial studies: the proof
gallery took roughly 1–36 seconds per five-second animation at 480×600.
The previous loop continues while a replacement is rendered.
