"""Headless smoke-test for NightDesk/Organist Python music candidates.

The previous validator only ran ``py_compile`` (a syntax check), so code that
imported undefined helpers, loaded missing ``.npy`` files, or built mis-shaped
NumPy arrays passed validation and only crashed at playback time. This harness
actually *executes* the candidate with ``teledra_synth.play_sound`` stubbed so
nothing plays and no GUI opens, then asserts the produced ``full_track`` is a
finite, non-empty, non-silent 1D wave. Run:

    python tools/music_smoketest.py <candidate.py>

Exit code 0 means the composition runs and yields a usable wave.
"""

from __future__ import annotations

import os
import runpy
import sys
import traceback

import numpy as np

ROOT = os.path.abspath(os.path.dirname(__file__))
PARENT = os.path.abspath(os.path.join(ROOT, ".."))
if PARENT not in sys.path:
    sys.path.insert(0, PARENT)

_captured: dict = {}


def _fake_play_sound(wave, sr=44100, loop=False, **_kwargs):
    """Capture the wave instead of playing it or opening the visualizer."""
    _captured["wave"] = np.asarray(wave, dtype=float).flatten()
    _captured["sr"] = int(sr)


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: python music_smoketest.py <candidate.py>", file=sys.stderr)
        return 2

    candidate = sys.argv[1]
    if not os.path.isfile(candidate):
        print(f"missing music candidate: {candidate}", file=sys.stderr)
        return 2

    try:
        import teledra_synth
    except Exception as exc:  # pragma: no cover - environment dependent
        print(f"teledra_synth import failed: {exc}", file=sys.stderr)
        return 3

    # Neutralize playback/GUI so validation is silent and never blocks.
    teledra_synth.play_sound = _fake_play_sound
    try:
        import sounddevice as sd

        sd.play = lambda *a, **k: None
        sd.wait = lambda *a, **k: None
        sd.stop = lambda *a, **k: None
    except Exception:
        pass

    try:
        with open(candidate, "r", encoding="utf-8", errors="replace") as handle:
            candidate_source = handle.read().lower()
    except OSError:
        candidate_source = ""

    try:
        runpy.run_path(candidate, run_name="__main__")
    except SystemExit:
        pass
    except Exception as exc:
        print(f"music runtime error: {type(exc).__name__}: {exc}", file=sys.stderr)
        traceback.print_exc()
        return 4

    wave = _captured.get("wave")
    if wave is None:
        print(
            "music code did not call play_sound(full_track, loop=True)",
            file=sys.stderr,
        )
        return 5
    if wave.ndim != 1:
        print(f"wave is not 1D after flatten (shape={wave.shape})", file=sys.stderr)
        return 6
    n = int(wave.size)
    sr = int(_captured.get("sr", 44100))
    duration = n / float(sr) if sr else 0.0
    ambient_markers = ("ambient", "ambience", "soundscape", "drone", "atmosphere")
    min_duration = 45.0 if any(marker in candidate_source for marker in ambient_markers) else 32.0
    if duration < min_duration:
        print(
            f"wave too short for an expanded court composition ({duration:.2f}s < {min_duration:.0f}s)",
            file=sys.stderr,
        )
        return 7
    if not np.all(np.isfinite(wave)):
        print("wave contains NaN or Inf samples", file=sys.stderr)
        return 8
    peak = float(np.max(np.abs(wave)))
    if peak <= 1e-6:
        print("wave is silent (all samples ~0)", file=sys.stderr)
        return 9

    print(f"music ok: {n} samples, {duration:.2f}s at {sr}Hz, peak={peak:.3f}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
