"""Truthful production-inventory graduation for Kraken games.

This report never publishes or edits a game.  It snapshots every directory in
``workspace/games``, resolves its declared contract, runs the browser gate on
that exact production file, and refuses graduation if the file changes during
the probe.  Captain Comic is excluded by the inventory declaration.
"""

from __future__ import annotations

import hashlib
import html as html_lib
import json
import os
import re
import subprocess
import tempfile
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable

from kraken.harness import browser_game_probe, game_checks, verify_code
from kraken.kernel import game_profiles


SCHEMA_VERSION = 1
PROFILE_DRIVER_COVERAGE = {
    "platformer",
    "endless_runner",
    "breakout_pinball",
    "frogger",
    "match3",
    "puzzle_grid",
    "rhythm",
    "snake",
    "shooter",
    "tower_defense",
    "roguelike",
}


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as fh:
        for block in iter(lambda: fh.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def discover_games(games_root: Path, excluded: set[str]) -> list[Path]:
    if not games_root.is_dir():
        return []
    return sorted(
        (path for path in games_root.iterdir() if path.is_dir() and path.name not in excluded),
        key=lambda path: path.name.casefold(),
    )


def resolve_declaration(game_dir: Path, declared: dict[str, Any]) -> dict[str, Any]:
    """Resolve central declaration plus an optional local manifest strictly.

    The central inventory is the production payload.  A local manifest may
    agree with it, but cannot silently change v2 into legacy v1 (or vice versa).
    """
    if declared.get("runtime") != "browser":
        return {"runtime": declared.get("runtime"), "error": None, "source": "inventory"}

    local = None
    local_path = game_dir / ".kraken-game.json"
    if local_path.exists():
        try:
            local = json.loads(local_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as exc:
            return {"error": f"invalid local declaration: {exc}", "source": "local_manifest"}

    resolved = game_profiles.resolve_trusted_profile(declared, local)
    resolved["runtime"] = "browser"
    if not resolved.get("error") and game_profiles.get_profile(str(resolved.get("profile") or "")) is None:
        resolved["error"] = f"unknown declared profile: {resolved.get('profile')}"
    return resolved


def _static_html_reasons(path: Path) -> list[str]:
    try:
        source = path.read_text(encoding="utf-8", errors="replace")
    except OSError as exc:
        return [f"cannot read html: {exc}"]
    scripts = verify_code._extract_scripts(source)
    reasons: list[str] = []
    reasons.extend(game_checks.basic_game_structure(source, scripts))
    reasons.extend(verify_code._verify_inline_handlers(source, scripts))
    reasons.extend(verify_code._verify_gameplay_static(scripts))
    reasons.extend(verify_code._verify_playability_static(source, scripts))
    reasons.extend(game_checks.launchability_smells(scripts, source))
    for index, script in enumerate(scripts):
        error = verify_code._node_check(script, str(path.parent))
        if error:
            reasons.append(f"JS syntax error in inline script {index + 1}: {error}")
            break
    return list(dict.fromkeys(reasons))


def _inject_identity_probe(source: str) -> str:
    driver = browser_game_probe.DRIVER.replace(
        "  report.beastApi = Boolean(api && typeof api.snapshot === 'function'",
        "  report.beastVersion = api && api.version != null ? api.version : null;\n"
        "  report.beastProfile = api && api.profile != null ? String(api.profile) : null;\n"
        "  report.beastApi = Boolean(api && typeof api.snapshot === 'function'",
        1,
    )
    head = re.search(r"<head\b[^>]*>", source, re.IGNORECASE)
    if head:
        source = source[:head.end()] + browser_game_probe.BOOTSTRAP + source[head.end():]
    else:
        source = browser_game_probe.BOOTSTRAP + source
    body_end = list(re.finditer(r"</body\s*>", source, re.IGNORECASE))
    if body_end:
        pos = body_end[-1].start()
        return source[:pos] + driver + source[pos:]
    return source + driver


def _extract_report(dumped_dom: str) -> dict[str, Any] | None:
    match = re.search(
        r'<pre\b[^>]*id=["\']kraken-runtime-report["\'][^>]*>(.*?)</pre>',
        dumped_dom,
        re.IGNORECASE | re.DOTALL,
    )
    if not match:
        return None
    try:
        return json.loads(html_lib.unescape(match.group(1)))
    except json.JSONDecodeError:
        return None


def _has_path(snapshot: Any, dotted: str) -> bool:
    value = snapshot
    for part in dotted.split("."):
        if not isinstance(value, dict) or part not in value:
            return False
        value = value[part]
    return value is not None


def assess_identity_and_shape(
    report: dict[str, Any], declaration: dict[str, Any]
) -> list[str]:
    reasons: list[str] = []
    expected_version = int(declaration["contract_version"])
    actual_version = report.get("beastVersion")
    actual_profile = str(report.get("beastProfile") or "").lower()
    expected_profile = str(declaration["profile"]).lower()

    if expected_version >= 2:
        if not isinstance(actual_version, (int, float)) or int(actual_version) != expected_version:
            reasons.append(
                f"beast contract version mismatch: declared v{expected_version}, runtime reported {actual_version!r}"
            )
        if actual_profile != expected_profile:
            reasons.append(
                f"beast profile mismatch: declared {expected_profile!r}, runtime reported {actual_profile or None!r}"
            )

    telemetry = report.get("telemetry") or {}
    initial = telemetry.get("initial")
    spec = game_profiles.get_profile(expected_profile)
    if expected_version >= 2 and spec:
        missing = [path for path in spec.required_snapshot_paths if not _has_path(initial, path)]
        if missing:
            reasons.append("beast snapshot missing declared profile fields: " + ", ".join(missing))
        if expected_profile not in PROFILE_DRIVER_COVERAGE:
            reasons.append(
                f"graduation driver does not yet exercise profile-specific actions for {expected_profile}"
            )
    return reasons


def run_browser_probe(path: Path, declaration: dict[str, Any], scratch_root: Path) -> dict[str, Any]:
    browser = browser_game_probe.find_browser()
    if not browser:
        return {"attempted": False, "passed": False, "reasons": ["headless browser unavailable"]}
    try:
        source = path.read_text(encoding="utf-8", errors="replace")
    except OSError as exc:
        return {"attempted": False, "passed": False, "reasons": [f"cannot read html: {exc}"]}

    scratch_root.mkdir(parents=True, exist_ok=True)
    try:
        with tempfile.TemporaryDirectory(prefix=".graduation-", dir=str(scratch_root)) as tmp:
            probe_path = Path(tmp) / "probe.html"
            probe_path.write_text(_inject_identity_probe(source), encoding="utf-8", newline="\n")
            profile_dir = Path(tmp) / "profile"
            query = (
                f"?krakenTest=1&profile={declaration['profile']}"
                f"&session={declaration['session']}&version={declaration['contract_version']}"
            )
            command = [
                browser,
                "--headless=new",
                "--disable-gpu",
                "--disable-extensions",
                "--disable-background-timer-throttling",
                "--disable-renderer-backgrounding",
                "--autoplay-policy=no-user-gesture-required",
                "--allow-file-access-from-files",
                "--no-first-run",
                "--no-default-browser-check",
                f"--user-data-dir={profile_dir}",
                f"--virtual-time-budget={browser_game_probe.VIRTUAL_TIME_MS}",
                "--run-all-compositor-stages-before-draw",
                "--dump-dom",
                probe_path.as_uri() + query,
            ]
            process = subprocess.run(
                command,
                cwd=tmp,
                capture_output=True,
                text=True,
                encoding="utf-8",
                errors="replace",
                timeout=browser_game_probe.PROCESS_TIMEOUT_S,
            )
            report = _extract_report(process.stdout or "")
            if report is None:
                detail = (process.stderr or process.stdout or "no browser output").strip()[-600:]
                return {
                    "attempted": True,
                    "passed": False,
                    "reasons": [f"headless browser did not produce a runtime report: {detail}"],
                }
            reasons = browser_game_probe.assess_structured(
                report,
                expected_profile=declaration["profile"],
                session=declaration["session"],
                contract_version=int(declaration["contract_version"]),
            )
            reasons.extend(assess_identity_and_shape(report, declaration))
            evidence = {
                "clicked_play": bool(report.get("clickedPlay")),
                "overlay_hidden_after_start": bool(report.get("overlayHiddenAfterStart")),
                "raf_count": int(report.get("rafCount") or 0),
                "audio_starts": int(report.get("audioStarts") or 0),
                "beast_api": bool(report.get("beastApi")),
                "beast_version": report.get("beastVersion"),
                "beast_profile": report.get("beastProfile"),
            }
            return {
                "attempted": True,
                "passed": not reasons,
                "reasons": list(dict.fromkeys(reasons)),
                "evidence": evidence,
            }
    except subprocess.TimeoutExpired:
        return {
            "attempted": True,
            "passed": False,
            "reasons": [f"headless browser probe timed out after {browser_game_probe.PROCESS_TIMEOUT_S}s"],
        }
    except OSError as exc:
        return {"attempted": True, "passed": False, "reasons": [f"headless browser failed: {exc}"]}


def build_manifest(
    root: Path,
    *,
    inventory_path: Path | None = None,
    games_root: Path | None = None,
    run_browser: bool = True,
    browser_runner: Callable[[Path, dict[str, Any], Path], dict[str, Any]] = run_browser_probe,
) -> dict[str, Any]:
    inventory_path = inventory_path or root / "game_inventory.json"
    games_root = games_root or root / "workspace" / "games"
    inventory = json.loads(inventory_path.read_text(encoding="utf-8"))
    excluded = {str(name) for name in inventory.get("excluded_games", [])}
    declarations = inventory.get("games") or {}
    discovered = discover_games(games_root, excluded)
    discovered_names = {path.name for path in discovered}
    rows: list[dict[str, Any]] = []

    for game_dir in discovered:
        declared = declarations.get(game_dir.name)
        row: dict[str, Any] = {"game": game_dir.name}
        if not isinstance(declared, dict):
            row.update({"accepted": False, "status": "undeclared", "reasons": ["inventory declaration missing"]})
            rows.append(row)
            continue

        row["runtime"] = declared.get("runtime")
        row["entrypoint"] = declared.get("entrypoint")
        resolved = resolve_declaration(game_dir, declared)
        row["declaration"] = resolved
        if resolved.get("error"):
            row.update({"accepted": False, "status": "declaration_error", "reasons": [resolved["error"]]})
            rows.append(row)
            continue

        entrypoint = game_dir / str(declared.get("entrypoint") or "")
        if not entrypoint.is_file():
            row.update({"accepted": False, "status": "missing_entrypoint", "reasons": [f"missing {entrypoint.name}"]})
            rows.append(row)
            continue
        before_hash = _sha256(entrypoint)
        row["sha256"] = before_hash
        row["bytes"] = entrypoint.stat().st_size

        if declared.get("runtime") != "browser":
            row.update({
                "accepted": False,
                "status": "outside_browser_campaign",
                "reasons": [f"runtime {declared.get('runtime')!r} has no browser graduation contract"],
            })
            rows.append(row)
            continue

        static_reasons = _static_html_reasons(entrypoint)
        runtime = (
            browser_runner(entrypoint, resolved, root / "output" / "graduation-tmp")
            if run_browser
            else {"attempted": False, "passed": False, "reasons": ["browser probe disabled"]}
        )
        after_hash = _sha256(entrypoint)
        changed = before_hash != after_hash
        reasons = list(static_reasons) + list(runtime.get("reasons") or [])
        if changed:
            reasons.append("production artifact changed while it was being probed; rerun on a stable snapshot")
        accepted = not static_reasons and bool(runtime.get("passed")) and not changed
        row.update({
            "accepted": accepted,
            "status": "graduated" if accepted else "rejected",
            "static": {"passed": not static_reasons, "reasons": static_reasons},
            "runtime_probe": runtime,
            "changed_during_probe": changed,
            "reasons": list(dict.fromkeys(reasons)),
        })
        rows.append(row)

    for name in sorted(set(declarations) - discovered_names - excluded):
        rows.append({
            "game": name,
            "accepted": False,
            "status": "declared_but_missing",
            "reasons": ["declared game directory is absent from production inventory"],
        })

    accepted_count = sum(1 for row in rows if row.get("accepted"))
    browser_count = sum(1 for row in rows if row.get("runtime") == "browser")
    inventory_complete = not any(
        row.get("status") in {"undeclared", "declared_but_missing"} for row in rows
    )
    return {
        "schema_version": SCHEMA_VERSION,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "games_root": str(games_root.resolve()),
        "inventory": str(inventory_path.resolve()),
        "excluded_games": sorted(excluded),
        "summary": {
            "discovered": len(discovered),
            "browser_games": browser_count,
            "graduated": accepted_count,
            "not_graduated": len(rows) - accepted_count,
            "inventory_complete": inventory_complete,
            "all_browser_games_graduated": inventory_complete and browser_count > 0 and all(
                row.get("accepted") for row in rows if row.get("runtime") == "browser"
            ),
        },
        "games": rows,
    }


def write_manifest(manifest: dict[str, Any], output_path: Path) -> None:
    output_path.parent.mkdir(parents=True, exist_ok=True)
    temp_path = output_path.with_suffix(output_path.suffix + ".tmp")
    temp_path.write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    os.replace(temp_path, output_path)


__all__ = [
    "PROFILE_DRIVER_COVERAGE",
    "assess_identity_and_shape",
    "build_manifest",
    "discover_games",
    "resolve_declaration",
    "run_browser_probe",
    "write_manifest",
]
