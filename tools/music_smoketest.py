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
import hashlib
import time

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

    report = verify_file(candidate, min_duration=min_duration, composer_grade=True)
    append_lesson(
        report,
        candidate,
        os.path.join(PARENT, "knowledge", "music_lessons.jsonl"),
    )
    import json

    # Keep compact, factual feedback for the Organist's next research/repair
    # cycle.  Source hashes distinguish a real revision from a renamed file.
    try:
        with open(candidate, "rb") as source_handle:
            source_bytes = source_handle.read()
        compact_report = {
            "timestamp": int(time.time()),
            "source_sha256": hashlib.sha256(source_bytes).hexdigest(),
            "ok": bool(report.get("ok")),
            "quality_score": report.get("quality_score"),
            "composer_metrics": report.get("composer_metrics", {}),
            "advisories": report.get("composer_advisories", [])[:5],
            "issues": report.get("issues", [])[:8],
        }
        report_path = os.path.join(PARENT, "knowledge", "music_harness_reports.jsonl")
        with open(report_path, "a", encoding="utf-8", newline="\n") as handle:
            handle.write(json.dumps(compact_report, ensure_ascii=True, sort_keys=True) + "\n")
    except OSError:
        pass

    rendered = json.dumps(report, ensure_ascii=True, sort_keys=True)
    if report["ok"]:
        print(rendered)
        return 0
    print(rendered, file=sys.stderr)
    return 4


if __name__ == "__main__":
    raise SystemExit(main())
