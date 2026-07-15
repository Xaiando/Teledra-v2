# Fractus v2

Fractus is a deterministic, native tkinter studio for fractals, mandalas,
recursive geometry, harmonic curves, tessellations, optical art, and dynamic
fields. The live-code language is declarative and bounded: it cannot import
Python modules, evaluate expressions, access files, invoke a shell, or use the
network.

The current registry contains 35 families and 11 named palettes, plus bounded
custom gradients. It includes escape-time and Newton fractals, layered mandalas,
L-systems and IFS forms, spirograph/harmonograph/string curves, Truchet and hex
tessellations, cellular automata, reaction-diffusion, flow fields, and strange
attractors.

## Launch the studio

```powershell
& D:\Teledra\.venv\Scripts\python.exe D:\Teledra\Fractus\fractus_gui.py
```

Press `Ctrl+Enter` to compile the editor. The Quick tab retains the original
single-layer CLI contract, including `--type`, `--iterations`, `--palette`,
`--c-real`, `--c-imag`, and `--save`.

## Headless rendering and validation

```powershell
& D:\Teledra\.venv\Scripts\python.exe D:\Teledra\Fractus\fractus_gui.py `
  --headless --script D:\Teledra\Fractus\examples\lotus_circuit.fract `
  --output D:\Teledra\Fractus\output\lotus_circuit.png

& D:\Teledra\.venv\Scripts\python.exe D:\Teledra\Fractus\fractus_gui.py `
  --validate --script D:\Teledra\Fractus\examples\lotus_circuit.fract

& D:\Teledra\.venv\Scripts\python.exe D:\Teledra\Fractus\fractus_gui.py `
  --self-test
```

Every saved image has a neighboring JSON manifest containing the canonical
scene, deterministic recipe/render hashes, render duration, and structural
quality metrics.

## Live-code shape

```text
version 2
canvas 900 900
seed 424242
palette twilight

layer lotus_mandala symmetry=16 rings=8 twist=0.22 lace=0.8 contrast=1.4
layer guilloche density=30 samples=3200 ratio=5.2 offset=0.4 phase=0.2 warp=0.1 alpha=0.45 blend=screen
animate 0.twist from=-0.4 to=1.2 seconds=12 easing=sine loop=true
```

Animation: any continuous param can be animated with `animate <layer>.<param> from=... to=... seconds=...`

Particles upgrade (new family for 3D-ish green particles etc.):
```text
layer particles count=150 size=2.5 depth=3.0 rotation=1.2 phase=0 speed=2.0
animate 0.phase from=0 to=8 seconds=10 easing=sine loop=true
animate 0.rotation from=0 to=3.14 seconds=14
```
Use in studio: SAVE ANIM GIF button, or `render_animated_gif(scene)` from Python.
See examples/green_particles_3d.fract

Use `--capabilities` or the studio's Families tab for the full registry and
typed parameter bounds.

## Tests

```powershell
& D:\Teledra\.venv\Scripts\python.exe -m unittest discover `
  -s D:\Teledra\Fractus\tests -v
```
