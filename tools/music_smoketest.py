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
import json
import subprocess
from pathlib import Path

ROOT = os.path.abspath(os.path.dirname(__file__))
PARENT = os.path.abspath(os.path.join(ROOT, ".."))

def main() -> int:
    if len(sys.argv) < 2:
        print("usage: python music_smoketest.py <candidate.py>", file=sys.stderr)
        return 22

    candidate = sys.argv[1]
    if not os.path.isfile(candidate):
        print(f"missing music candidate: {candidate}", file=sys.stderr)
        return 22

    try:
        with open(candidate, "r", encoding="utf-8", errors="replace") as handle:
            candidate_source = handle.read().lower()
    except OSError:
        candidate_source = ""
        
    ambient_markers = ("ambient", "ambience", "soundscape", "drone", "atmosphere")
    min_duration = 45.0 if any(marker in candidate_source for marker in ambient_markers) else 32.0

    run_id = os.environ.get("TELEDRA_RUN_ID", f"run-{int(time.time())}")
    run_dir = Path(PARENT) / ".teledra" / "runs" / run_id
    run_dir.mkdir(parents=True, exist_ok=True)

    # Launch the verifier in a subprocess to protect against early crashes
    try:
        result = subprocess.run(
            [sys.executable, os.path.join(PARENT, "music_verify.py"), candidate, "--min-duration", str(min_duration), "--composer-grade"],
            capture_output=True,
            text=True,
            cwd=PARENT
        )
    except Exception as exc:
        print(f"Failed to spawn verifier: {exc}", file=sys.stderr)
        return 21

    # Attempt to parse the standard JSON report from stdout
    report = None
    for line in reversed(result.stdout.splitlines()):
        try:
            report = json.loads(line)
            if "ok" in report:
                break
        except ValueError:
            pass
            
    # Classification Engine
    failure_class = None
    failure_stage = "VERIFIER_BOOTSTRAP"
    code = "unknown_error"
    message = "Unknown error"
    
    if result.returncode != 0 and report is None:
        # The verifier crashed before returning a report
        stderr_lower = result.stderr.lower()
        if "modulenotfounderror" in stderr_lower or "importerror" in stderr_lower:
            # Differentiate external vs internal missing module
            if "teledra_synth" in result.stderr or "composer_harness" in result.stderr or "music_verify" in result.stderr:
                failure_class = "RENDER_ENGINE_DEFECT"
            else:
                failure_class = "ENVIRONMENT_FAILURE"
        else:
            failure_class = "RENDER_ENGINE_DEFECT"
        code = "runtime_error"
        message = result.stderr.strip()[-500:] # Last 500 chars of trace
    elif report is not None and not report.get("ok"):
        failure_stage = "AUDIO_ANALYSIS"
        issues = report.get("issues", [])
        if issues:
            first = issues[0]
            code = first.get("code", "unknown_issue")
            message = first.get("message", "Verifier reported an issue.")
            
            if code in ("missing_score", "missing_sections", "thin_arrangement", "flat_form"):
                failure_class = "SCORE_SCHEMA_DEFECT"
                failure_stage = "COMPOSITION_ANALYSIS"
            elif code in ("dc_offset", "overcompressed_mix", "harsh_mix", "underpowered_mix", "clipping", "silent_mix", "loop_seam"):
                failure_class = "AUDIO_QUALITY_DEFECT"
            elif code == "runtime_error":
                failure_stage = "RENDER_EXECUTION"
                exc_type = first.get("exc_type", "")
                if exc_type in ("ImportError", "ModuleNotFoundError"):
                    failure_class = "ENVIRONMENT_FAILURE"
                else:
                    failure_class = "RENDER_ENGINE_DEFECT"
            elif code in ("invalid_audio_shape", "nonfinite_samples"):
                failure_class = "RENDER_ENGINE_DEFECT"
                failure_stage = "RENDER_EXECUTION"
            else:
                failure_class = "COMPOSITION_CONSTRAINT_DEFECT"
                failure_stage = "COMPOSITION_ANALYSIS"

    # Emit CompositionAnalysisReport if we have a valid report
    if report is not None:
        findings = []
        for adv in report.get("composer_advisories", []):
            findings.append({
                "code": adv.get("code", "WARNING"),
                "severity": "WARNING",
                "message": adv.get("message", ""),
                "measurement": {"metric": "heuristic", "actual": 0}
            })
        
        comp_report = {
            "schema_version": "1.0",
            "run_id": run_id,
            "score_hash": "sha256:unknown",
            "status": "REVIEW" if findings else "OK",
            "findings": findings
        }
        comp_path = run_dir / "composition_analysis_report.json"
        comp_path.write_text(json.dumps(comp_report, indent=2))
            
    if failure_class:
        # Create the Incident Report
        incident_id = f"INC-{int(time.time())}"
        incident = {
            "schema_version": "2.0",
            "run_id": run_id,
            "incident_id": incident_id,
            "score_hash": "sha256:unknown",
            "render_hash": "sha256:unknown",
            "failure_class": failure_class,
            "failure_stage": failure_stage,
            "severity": "BLOCKING",
            "failure_code": code,
            "observed": {"message": message},
            "reproduction": {
                "command_id": "music.verify.canonical_render",
                "arguments": {
                    "candidate": candidate
                },
                "expected_exit_codes": [0],
                "actual_exit_code": result.returncode
            }
        }
        
        rel_path = f"incidents/{incident_id}.json"
        incident_path = run_dir / rel_path
        incident_path.parent.mkdir(parents=True, exist_ok=True)
        incident_bytes = json.dumps(incident, indent=2).encode("utf-8")
        incident_path.write_bytes(incident_bytes)
        
        sha256 = hashlib.sha256(incident_bytes).hexdigest()
        
        envelope = {
            "event": "TELEDRA_INCIDENT_CREATED",
            "envelope_version": "1.0",
            "run_id": run_id,
            "incident_id": incident_id,
            "artifact_relpath": rel_path,
            "artifact_sha256": f"sha256:{sha256}",
            "artifact_bytes": len(incident_bytes)
        }
        
        print(json.dumps(envelope), file=sys.stdout)
        return 20
        
    if report and report.get("ok"):
        return 0
        
    return 22

if __name__ == "__main__":
    raise SystemExit(main())
