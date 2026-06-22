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
import sys

ROOT = os.path.abspath(os.path.dirname(__file__))
PARENT = os.path.abspath(os.path.join(ROOT, ".."))
if PARENT not in sys.path:
    sys.path.insert(0, PARENT)

def main() -> int:
    if len(sys.argv) != 2:
        print("usage: python music_smoketest.py <candidate.py>", file=sys.stderr)
        return 2

    candidate = sys.argv[1]
    if not os.path.isfile(candidate):
        print(f"missing music candidate: {candidate}", file=sys.stderr)
        return 2

    try:
        with open(candidate, "r", encoding="utf-8", errors="replace") as handle:
            candidate_source = handle.read().lower()
    except OSError:
        candidate_source = ""
    ambient_markers = ("ambient", "ambience", "soundscape", "drone", "atmosphere")
    min_duration = 45.0 if any(marker in candidate_source for marker in ambient_markers) else 32.0
    try:
        from music_verify import append_lesson, verify_file
    except Exception as exc:  # pragma: no cover - environment dependent
        print(f"music verifier import failed: {exc}", file=sys.stderr)
        return 3

    report = verify_file(candidate, min_duration=min_duration)
    append_lesson(
        report,
        candidate,
        os.path.join(PARENT, "knowledge", "music_lessons.jsonl"),
    )
    import json

    rendered = json.dumps(report, ensure_ascii=True, sort_keys=True)
    if report["ok"]:
        print(rendered)
        return 0
    print(rendered, file=sys.stderr)
    return 4


if __name__ == "__main__":
    raise SystemExit(main())
