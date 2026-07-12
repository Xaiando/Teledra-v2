"""verify_code - mechanical checks for code_forge outputs."""

from __future__ import annotations

import json
import os
import py_compile
import subprocess
import sys
from pathlib import Path

try:
    from . import game_checks
except Exception:
    # The harness dispatcher loads this file as a STANDALONE module
    # (spec_from_file_location), so the relative import fails and the whole
    # rich game-check suite would silently vanish. Load it by absolute path.
    game_checks = None
    try:
        import importlib.util as _ilu
        _gc_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "game_checks.py")
        _spec = _ilu.spec_from_file_location("kraken_game_checks", _gc_path)
        game_checks = _ilu.module_from_spec(_spec)
        _spec.loader.exec_module(game_checks)
    except Exception:
        game_checks = None  # truly unavailable — inline fallbacks still apply


try:
    from . import browser_game_probe
except Exception:
    browser_game_probe = None
    try:
        import importlib.util as _ilu
        _probe_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "browser_game_probe.py")
        _probe_spec = _ilu.spec_from_file_location("kraken_browser_game_probe", _probe_path)
        browser_game_probe = _ilu.module_from_spec(_probe_spec)
        _probe_spec.loader.exec_module(browser_game_probe)
    except Exception:
        browser_game_probe = None


TIMEOUT_S = 45
TEST_RUNNER = "kraken_test_runner.py"


def _abs(root: str, path: str) -> str:
    return path if os.path.isabs(path) else os.path.join(root, path)


def _inside(root: str, path: str) -> bool:
    try:
        Path(path).resolve().relative_to(Path(root).resolve())
        return True
    except ValueError:
        return False


def _allowed_code_path(root: str, workspace: str, path: str) -> bool:
    return _inside(root, path) or _inside(workspace, path)


def _run(cmd: list[str], *, cwd: str, env: dict | None = None) -> tuple[bool, str]:
    try:
        proc = subprocess.run(
            cmd,
            cwd=cwd,
            env=env,
            text=True,
            capture_output=True,
            timeout=TIMEOUT_S,
        )
    except subprocess.TimeoutExpired as exc:
        return False, f"timeout running {' '.join(cmd)}: {exc}"
    out = (proc.stdout or "") + (proc.stderr or "")
    return proc.returncode == 0, out[-4000:]


def _write_test_runner(workdir: str) -> str:
    path = os.path.join(workdir, TEST_RUNNER)
    if os.path.exists(path):
        return path
    code = r'''
from __future__ import annotations

import os
import runpy
import sys
from pathlib import Path


def _roots() -> list[Path]:
    raw = os.environ.get("KRAKEN_WRITE_ROOTS", "")
    return [Path(item).resolve() for item in raw.split(os.pathsep) if item]


ALLOWED_ROOTS = _roots()


def _inside_allowed(path: object) -> bool:
    try:
        candidate = Path(path).resolve()
    except Exception:
        return True
    for root in ALLOWED_ROOTS:
        try:
            candidate.relative_to(root)
            return True
        except ValueError:
            pass
    return False


def _write_intent(mode: object = None, flags: object = None) -> bool:
    if isinstance(mode, str) and any(ch in mode for ch in "wax+"):
        return True
    if isinstance(flags, int):
        return bool(flags & (os.O_WRONLY | os.O_RDWR | os.O_APPEND | os.O_CREAT | os.O_TRUNC))
    return False


def _guard(event: str, args: tuple) -> None:
    if event == "open" and args:
        mode = args[1] if len(args) > 1 else None
        flags = args[2] if len(args) > 2 else None
        if _write_intent(mode, flags) and not _inside_allowed(args[0]):
            raise PermissionError(f"write outside kraken root/workspace blocked: {args[0]}")
    elif event in {"os.remove", "os.rmdir", "os.mkdir", "os.rename"} and args:
        for candidate in args[:2]:
            if not _inside_allowed(candidate):
                raise PermissionError(f"{event} outside kraken root/workspace blocked: {candidate}")
    elif event in {"subprocess.Popen", "socket.connect"}:
        raise PermissionError(f"{event} blocked during verify_code tests")


sys.addaudithook(_guard)
runpy.run_path(os.environ["KRAKEN_TEST_PATH"], run_name="__main__")
'''
    with open(path, "w", encoding="utf-8", newline="\n") as fh:
        fh.write(code.strip() + "\n")
    return path


def _discover_tests(workdir: str, result: dict) -> list[str]:
    raw = result.get("tests") or result.get("test")
    if isinstance(raw, str):
        return [raw]
    if isinstance(raw, list):
        return [str(item) for item in raw]
    if os.path.isdir(workdir):
        return [
            os.path.join(workdir, name)
            for name in sorted(os.listdir(workdir))
            if name.startswith("test_") and name.endswith(".py")
        ]
    return []


def _detect_test_gaming(path: str) -> str | None:
    """Catch the classic verifier-defeating trick: a comparison dunder that
    returns a constant, so equality-based tests pass for ANY value (used to
    'satisfy' impossible/contradictory specs). Cheap AST scan, no execution.
    """
    import ast
    try:
        with open(path, "r", encoding="utf-8") as fh:
            tree = ast.parse(fh.read())
    except (OSError, SyntaxError):
        return None
    suspect = {"__eq__", "__ne__", "__bool__", "__hash__"}
    for node in ast.walk(tree):
        if not isinstance(node, ast.FunctionDef) or node.name not in suspect:
            continue
        body = [n for n in node.body if not isinstance(n, ast.Expr)  # drop docstring
                or not isinstance(getattr(n, "value", None), ast.Constant)]
        if len(body) == 1 and isinstance(body[0], ast.Return) \
                and isinstance(body[0].value, ast.Constant):
            return (f"suspected test-gaming: {node.name} hardcodes a constant "
                    f"return ({body[0].value.value!r}); tests pass for any value")
    return None


def _verify_python(root: str, path: str, result: dict, ctx: dict) -> list[str]:
    reasons: list[str] = []
    workspace = ctx.get("workspace") or os.path.join(root, "workspace")
    try:
        py_compile.compile(path, doraise=True)
    except py_compile.PyCompileError as exc:
        reasons.append(f"py_compile failed: {exc.msg}")
        return reasons

    gamed = _detect_test_gaming(path)
    if gamed:
        reasons.append(gamed)
        return reasons

    workdir = ctx.get("workdir") or os.path.dirname(path)
    env = dict(os.environ)
    module_dir = os.path.dirname(path)
    env["PYTHONPATH"] = os.pathsep.join(
        [module_dir, root, env.get("PYTHONPATH", "")]
    )
    env["KRAKEN_WRITE_ROOTS"] = os.pathsep.join(
        [str(Path(root).resolve()), str(Path(workspace).resolve())]
    )
    runner = _write_test_runner(workdir)
    for test in _discover_tests(workdir, result):
        test_path = _abs(root, test)
        if not _allowed_code_path(root, workspace, test_path):
            reasons.append(f"test outside kraken root/workspace: {test}")
            continue
        if not os.path.exists(test_path):
            reasons.append(f"declared test missing: {test}")
            continue
        env["KRAKEN_TEST_PATH"] = test_path
        ok, output = _run([sys.executable, runner], cwd=module_dir, env=env)
        if not ok:
            reasons.append(f"test failed: {os.path.basename(test_path)}\n{output}")
    return reasons


def _run_declared_tests(root: str, path: str, result: dict, ctx: dict) -> list[str]:
    reasons: list[str] = []
    workspace = ctx.get("workspace") or os.path.join(root, "workspace")
    workdir = ctx.get("workdir") or os.path.dirname(path)
    env = dict(os.environ)
    module_dir = os.path.dirname(path)
    env["PYTHONPATH"] = os.pathsep.join(
        [module_dir, root, env.get("PYTHONPATH", "")]
    )
    env["KRAKEN_WRITE_ROOTS"] = os.pathsep.join(
        [str(Path(root).resolve()), str(Path(workspace).resolve())]
    )
    runner = _write_test_runner(workdir)
    for test in _discover_tests(workdir, result):
        test_path = _abs(root, test)
        if not _allowed_code_path(root, workspace, test_path):
            reasons.append(f"test outside kraken root/workspace: {test}")
            continue
        if not os.path.exists(test_path):
            reasons.append(f"declared test missing: {test}")
            continue
        env["KRAKEN_TEST_PATH"] = test_path
        ok, output = _run([sys.executable, runner], cwd=module_dir, env=env)
        if not ok:
            reasons.append(f"test failed: {os.path.basename(test_path)}\n{output}")
    return reasons


def _extract_scripts(html: str) -> list[str]:
    """Inline <script> bodies only (skip external src= scripts)."""
    import re
    blocks = []
    for m in re.finditer(r"<script\b([^>]*)>(.*?)</script>", html,
                         re.DOTALL | re.IGNORECASE):
        attrs, body = m.group(1), m.group(2)
        if re.search(r"\bsrc\s*=", attrs, re.IGNORECASE):
            continue  # external script — flagged separately as an asset
        if body.strip():
            blocks.append(body)
    return blocks


def _node_check(script: str, workdir: str) -> str | None:
    """Syntax-check one inline script with `node --check`. Returns an error
    string, or None if it parses (or node is unavailable — degrade, don't lie)."""
    import shutil
    node = shutil.which("node")
    if not node:
        return None
    tmp = os.path.join(workdir, "_kraken_jscheck.js")
    try:
        with open(tmp, "w", encoding="utf-8", newline="\n") as fh:
            fh.write(script)
        ok, out = _run([node, "--check", tmp], cwd=workdir)
        return None if ok else out.strip()[-500:]
    except Exception:
        return None
    finally:
        try:
            os.remove(tmp)
        except OSError:
            pass


def _verify_inline_handlers(html: str, scripts: list[str]) -> list[str]:
    if game_checks:
        try:
            return game_checks.inline_handler_closure_problem(html, scripts)
        except Exception:
            pass
    # minimal fallback
    import re
    calls = re.findall(r"\bon\w+\s*=\s*['\"]\s*([A-Za-z_$][\w$]*)\s*\(", html)
    if not calls:
        return []
    joined = "\n".join(scripts)
    iife_wrapped = bool(re.search(r"\(\s*function\s*\([^)]*\)\s*\{", joined))
    reasons = []
    for name in sorted(set(calls)):
        exported = re.search(rf"\b(?:window|globalThis)\s*\.\s*{re.escape(name)}\s*=", joined)
        top_level = re.search(rf"\bfunction\s+{re.escape(name)}\s*\(", joined)
        if iife_wrapped and top_level and not exported:
            reasons.append(f"inline handler calls {name}(), but the function is closure-local; use addEventListener or assign it to window")
    return reasons


def _extract_js_function(script: str, name: str) -> str:
    import re
    match = re.search(rf"\bfunction\s+{re.escape(name)}\s*\([^)]*\)\s*\{{", script)
    if not match:
        return ""
    start = match.end()
    depth = 1
    i = start
    while i < len(script) and depth:
        if script[i] == "{":
            depth += 1
        elif script[i] == "}":
            depth -= 1
        i += 1
    return script[start:i - 1]


def _verify_gameplay_static(scripts: list[str]) -> list[str]:
    if game_checks:
        try:
            return game_checks.empty_wave_instant_complete(scripts) + game_checks.downward_bullets(scripts)
        except Exception:
            pass
    import re
    reasons: list[str] = []
    joined = "\n".join(scripts).lower()
    compact = re.sub(r"\s+", "", joined)
    if "enemiesinwave" in compact and re.search(r"if\(\s*enemiesinwave={2,3}0&&enemiesalive={2,3}0\s*\)", compact):
        reasons.append("wave-clear logic treats an empty initial wave as complete; spawn initial enemies or require enemiesInWave >= totalEnemiesPerWave")
    shoot_body = _extract_js_function("\n".join(scripts), "shootBullet").lower()
    if shoot_body and "bullets.push" in shoot_body and "vy:" in shoot_body:
        if re.search(r"vy\s*:\s*speed\b", shoot_body):
            reasons.append("player shootBullet appears to fire downward with vy: speed; vertical shooters should use negative vy for upward shots")
    return reasons


def _verify_playability_static(html: str, scripts: list[str]) -> list[str]:
    if game_checks:
        try:
            return game_checks.collect_all_static_issues(html, scripts)
        except Exception:
            pass
    import re
    reasons: list[str] = []
    low = html.lower()
    joined = "\n".join(scripts)
    if re.search(r"closest\s*\(\s*['\"]#play-btn['\"]\s*\)", joined):
        if not re.search(r'id=["\']play-btn["\']', html, re.I):
            if re.search(r"filltext\s*\(\s*['\"]play", joined, re.I):
                reasons.append("start PLAY is canvas-drawn but click handler expects missing #play-btn element; add HTML button or canvas hit-test")
    if re.search(r"initgame\s*\(\s*\)\s*;\s*requestanimationframe", joined, re.I | re.S):
        if "startscreen" in low.replace("_", "").replace("-", ""):
            if re.search(r"gamestate\s*=\s*['\"]running['\"]", joined, re.I):
                reasons.append("initGame() on load sets running while start overlay still visible; hide overlay or start in menu state")
    if re.search(r"playbtn[^;]{0,120}\.addEventListener\s*\(\s*['\"]click['\"][\s\S]{0,400}showstartscreen\s*\(", joined, re.I):
        reasons.append("play button click calls showStartScreen(); must hide start overlay and show HUD instead")
    if "<canvas" in low:
        if "tabindex" not in low:
            reasons.append("canvas lacks tabindex — keyboard may not work until focused")
        if not re.search(r"canvas\.width\s*=", joined, re.I) and not re.search(r"<canvas[^>]*\bwidth\s*=", html, re.I):
            reasons.append("canvas width/height not set in markup or JS (CSS-only sizing breaks hit coords)")
    ids = re.findall(r'id=["\']([^"\']+)["\']', html, re.I)
    if len(ids) != len(set(ids)):
        dupes = sorted({i for i in ids if ids.count(i) > 1})
        reasons.append(f"duplicate HTML ids break getElementById: {dupes[:5]}")
    return reasons


def _beast_mode(job: dict | None) -> bool:
    if not job:
        return False
    raw = job.get("input", "")
    payload = raw if isinstance(raw, dict) else None
    if payload is None and isinstance(raw, str):
        try:
            payload = json.loads(raw)
        except json.JSONDecodeError:
            payload = None
    if isinstance(payload, dict):
        quality = str(payload.get("quality", "")).strip().lower()
        task = str(payload.get("task") or payload.get("prompt") or "").lower()
        return quality == "beast" or "beast mode" in task or "[beast]" in task
    lower = str(raw).lower()
    return "beast mode" in lower or "[beast]" in lower


def _resolve_profile(path: str, job: dict | None) -> dict | None:
    payload = {}
    if job:
        raw = job.get("input", "")
        if isinstance(raw, dict):
            payload = raw
        elif isinstance(raw, str):
            try:
                payload = json.loads(raw)
            except Exception:
                pass

    manifest = None
    manifest_path = Path(path).parent / ".kraken-game.json"
    if manifest_path.exists():
        try:
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        except Exception:
            pass

    try:
        from kraken.kernel import game_profiles
        res = game_profiles.resolve_trusted_profile(payload, manifest)
        if not res.get("error") and res.get("profile"):
            return res
    except Exception:
        pass
    return None


def _verify_html(path: str, ctx: dict, job: dict | None = None) -> list[str]:
    """Verify a self-contained browser page — and, if it has a <canvas>, that
    it is a real interactive game/animation (loop + input), with valid JS and
    NO external assets. Static, Node syntax, and headless-browser behavior."""
    import re
    reasons: list[str] = []
    workdir = ctx.get("workdir") or os.path.dirname(path)
    try:
        with open(path, "r", encoding="utf-8", errors="ignore") as fh:
            html = fh.read()
    except OSError as exc:
        return [f"cannot read html: {exc}"]

    # Truncation check
    stripped = html.strip().lower()
    if not (stripped.endswith("</html>") or stripped.endswith("</script>")):
        reasons.append("STRUCTURE_TRUNCATED: HTML source is truncated (missing closing tags or ends mid-expression)")

    low = html.lower()
    scripts = _extract_scripts(html)  # define before any use (was a swallowed NameError)
    if game_checks:
        try:
            reasons.extend(game_checks.basic_game_structure(html, scripts))
            # external already covered inside basic, but keep any additional
        except Exception:
            pass
    if not reasons or not game_checks:  # fallback path or append if needed
        if html.lstrip().startswith("```"):
            reasons.append("HTML artifact starts with a markdown code fence; return raw file contents only")
        if "<html" not in low and "<!doctype" not in low:
            reasons.append("not an HTML document (no <html>/<!doctype>)")
        if "<script" not in low:
            reasons.append("no <script> — a game needs JavaScript")
        ext = re.findall(r"""(?:src|href)\s*=\s*['"]\s*((?:https?:)?//[^'"]+)""", html, re.IGNORECASE)
        cdn = re.findall(r"https?://[^\s'\"<>]+", html)
        if ext or cdn:
            sample = (ext + cdn)[0][:80]
            reasons.append(f"external asset/URL found (must be self-contained): {sample}")

    # inline JS must actually parse
    if not scripts:
        reasons.append("no inline JavaScript found")
    reasons.extend(_verify_inline_handlers(html, scripts))
    reasons.extend(_verify_gameplay_static(scripts))
    reasons.extend(_verify_playability_static(html, scripts))
    if game_checks:
        try:
            reasons.extend(game_checks.launchability_smells(scripts, html))
        except Exception:
            pass
    for i, script in enumerate(scripts):
        err = _node_check(script, workdir)
        if err:
            reasons.append(f"JS syntax error in inline script {i + 1}: {err}")
            break  # one is enough; it won't run

    # canvas => enforce the interactive-game properties
    if "<canvas" in low or "getcontext" in low:
        if "getcontext" not in low:
            reasons.append("canvas present but never acquires a 2d/webgl context")
        if "requestanimationframe" not in low and "setinterval" not in low:
            reasons.append("no animation loop (requestAnimationFrame/setInterval) — game is static")
        if not re.search(r"addeventlistener\s*\(\s*['\"](key|mouse|pointer|touch|click)",
                         low) and not re.search(r"\bon(key|mouse|click|pointer|touch)\w*\s*=", low):
            reasons.append("no input handling (keyboard/mouse/pointer) — not playable")
        if browser_game_probe:
            if not any("JS syntax error" in reason for reason in reasons):
                try:
                    prof_info = _resolve_profile(path, job)
                    if prof_info:
                        report = browser_game_probe.probe_structured(
                            path,
                            expected_profile=prof_info["profile"],
                            session=prof_info["session"],
                            contract_version=prof_info["contract_version"],
                            workdir=workdir,
                        )
                        if report is None:
                            reasons.append("headless browser did not produce a runtime report")
                        else:
                            reasons.extend(browser_game_probe.assess_structured(
                                report,
                                expected_profile=prof_info["profile"],
                                session=prof_info["session"],
                                contract_version=prof_info["contract_version"],
                            ))
                    else:
                        reasons.extend(browser_game_probe.probe(path, require_beast=_beast_mode(job)))
                except Exception as exc:
                    reasons.append(f"headless browser runtime probe crashed: {exc}")
        elif _beast_mode(job):
            reasons.append("beast mode requires the headless browser runtime probe, but it could not be loaded")
    return reasons


def _verify_cargo(path: str) -> list[str]:
    project = path if os.path.isdir(path) else os.path.dirname(path)
    while project and project != os.path.dirname(project):
        if os.path.exists(os.path.join(project, "Cargo.toml")):
            ok, output = _run(["cargo", "check"], cwd=project)
            return [] if ok else [f"cargo check failed:\n{output}"]
        project = os.path.dirname(project)
    return []


def verify(job: dict, result: dict, ctx: dict) -> dict:
    reasons: list[str] = []
    root = ctx["root"]
    if not result.get("ok"):
        reasons.append("code_forge reported ok=false")

    output = result.get("output")
    if not output:
        reasons.append("no output file specified")
        return {"passed": False, "reasons": reasons}

    path = _abs(root, output)
    workspace = ctx.get("workspace") or os.path.join(root, "workspace")
    if not _allowed_code_path(root, workspace, path):
        reasons.append(f"output outside kraken root/workspace: {output}")
        return {"passed": False, "reasons": reasons}
    if not os.path.exists(path):
        reasons.append(f"output file missing: {output}")
        return {"passed": False, "reasons": reasons}

    suffix = Path(path).suffix.lower()
    if suffix == ".py":
        reasons.extend(_verify_python(root, path, result, ctx))
    elif suffix == ".rs" or os.path.exists(os.path.join(path, "Cargo.toml")):
        reasons.extend(_verify_cargo(path))
    elif suffix in {".json"}:
        try:
            with open(path, "r", encoding="utf-8") as fh:
                json.load(fh)
        except Exception as exc:
            reasons.append(f"json parse failed: {exc}")
    elif suffix in {".html", ".htm"}:
        reasons.extend(_verify_html(path, ctx, job))
        reasons.extend(_run_declared_tests(root, path, result, ctx))
    else:
        if os.path.getsize(path) < 20:
            reasons.append("output suspiciously small")
        reasons.extend(_run_declared_tests(root, path, result, ctx))

    return {"passed": not reasons, "reasons": reasons}
