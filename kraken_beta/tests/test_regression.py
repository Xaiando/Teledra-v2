#!/usr/bin/env python3
"""Kraken regression suite — locks in every hardening won during the academy.

Run:  D:\\Teledra\\.venv\\Scripts\\python.exe kraken\\tests\\test_regression.py

Fast, offline, deterministic (no Ollama, no network). Every test corresponds to
a real fix and a real exploit that was closed. Green = the taskforce's earned
safety properties still hold; any future edit that breaks one is caught here
before it ships. This is the net that makes continued self-improvement safe.
"""

from __future__ import annotations

import importlib.util
import json
import os
import shutil
import sys
import tempfile

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.dirname(ROOT))

from kraken.kernel import recall, query_guard
from kraken.kernel.queue import Queue


def _load(modpath: str, name: str):
    spec = importlib.util.spec_from_file_location(name, modpath)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


PASS, FAIL = [], []


def check(name: str, cond: bool, detail: str = ""):
    (PASS if cond else FAIL).append(name)
    mark = "PASS" if cond else "FAIL"
    print(f"  [{mark}] {name}" + (f" — {detail}" if detail and not cond else ""))


# ---- security: verify_code ACE guard + anti-gaming --------------------------
def test_verify_code():
    vc = _load(os.path.join(ROOT, "harness", "verify_code.py"), "vc_reg")
    scratch = tempfile.mkdtemp(prefix="kraken-reg-")
    ctx = {"root": ROOT, "workspace": os.path.join(ROOT, "workspace"),
           "workdir": scratch, "log": lambda m: None}

    # 1. external test path is refused, never executed
    marker = os.path.join(scratch, "pwned.txt")
    evil = os.path.join(scratch, "evil_test.py")
    open(evil, "w").write(f"open(r'{marker}','w').write('x')\n")
    mod = os.path.join(ROOT, "jobs", "reg-ace", "m.py")
    os.makedirs(os.path.dirname(mod), exist_ok=True)
    open(mod, "w").write("def f():\n    return 1\n")
    v = vc.verify({"id": "reg-ace", "skill": "code_forge"},
                  {"ok": True, "output": os.path.relpath(mod, ROOT), "tests": [evil]}, ctx)
    check("verify_code refuses external test path", not v["passed"]
          and not os.path.exists(marker))

    # 2. anti-gaming: constant-return __eq__ flagged
    gamed = os.path.join(scratch, "gamed.py")
    open(gamed, "w").write("class B:\n    def __eq__(self, o):\n        return True\n")
    check("verify_code flags constant __eq__ gaming",
          vc._detect_test_gaming(gamed) is not None)

    # 3. legit __eq__ NOT flagged (no false positive)
    legit = os.path.join(scratch, "legit.py")
    open(legit, "w").write("class P:\n    def __init__(s,x): s.x=x\n"
                           "    def __eq__(s,o): return s.x==o.x\n")
    check("verify_code spares legit __eq__", vc._detect_test_gaming(legit) is None)


# ---- security: prod_digest path allowlist + deny manifest -------------------
def test_prod_digest_allowlist():
    pd = _load(os.path.join(ROOT, "skills", "prod_digest", "run.py"), "pd_reg")
    scratch = tempfile.mkdtemp(prefix="kraken-reg-")
    ctx = {"root": ROOT, "workspace": os.path.join(ROOT, "workspace"),
           "workdir": scratch, "log": lambda m: None,
           "llm": _StubLLM()}
    r = pd.execute({"id": "reg-deny", "skill": "prod_digest",
                    "input": r"C:\Windows\System32 max_files=2"}, ctx)
    out = r.get("output", "")
    body = ""
    if out:
        p = out if os.path.isabs(out) else os.path.join(ROOT, out)
        body = open(p, encoding="utf-8").read() if os.path.exists(p) else ""
    check("prod_digest denies OS path", "denied" in body.lower())
    check("prod_digest deny writes manifest (verifier-safe)",
          os.path.exists(os.path.join(scratch, "sources_manifest.json")))


# ---- security: research_local padding + escalation guard --------------------
def test_research_local_guards():
    rl = _load(os.path.join(ROOT, "skills", "research_local", "run.py"), "rl_reg")
    check("padding query rejected",
          query_guard.query_sanity("A" * 6000 + " teledra policy") is not None)
    check("normal query accepted", query_guard.query_sanity("What is the treasury policy?") is None)
    check("kingdom lexicon blocks web escalation",
          rl._should_escalate("What does the Teledra kingdom swarm require?", _StubLLM()) is False)


# ---- security: code_forge workspace confinement -----------------------------
def test_code_forge_confinement():
    cf = _load(os.path.join(ROOT, "skills", "code_forge", "run.py"), "cf_reg")
    ws = os.path.join(ROOT, "workspace")
    # escape attempt resolves OUTSIDE workspace -> _inside must be False
    escape = os.path.join(ws, r"..\..\..\Windows\Temp\x")
    check("code_forge rejects workspace escape", cf._inside(ws, escape) is False)
    check("code_forge allows in-workspace dir",
          cf._inside(ws, os.path.join(ws, "proj")) is True)

    html = """<!doctype html><html><body><canvas id='g' width='800' height='600' tabindex='0'></canvas><script>
(function(){const canvas=document.getElementById('g');const ctx=canvas.getContext('2d');
function draw(){ctx.fillRect(0,0,10,10)} function gameLoop(){draw();requestAnimationFrame(gameLoop)}
window.addEventListener('keydown',()=>{});gameLoop();})();
</script><!--""" + ("real-game-padding " * 150) + """--></body></html>"""
    normalized = cf._normalize_html_game(html, "shooter", 2)
    check("code_forge normalizer preserves declared Beast profile",
          'profile: "shooter"' in normalized and "breakout_pinball" not in normalized)
    check("code_forge normalizer never emits literal $1 capture text", "$1" not in normalized)
    check("code_forge fallback adapter is fail-closed",
          "KRAKEN_ADAPTER_UNIMPLEMENTED" in normalized)
    check("code_forge normalizer does not invent mismatched overlay references",
          "const overlay" not in normalized and r"\'overlay\'" not in normalized)
    check("code_forge normalizer never invents gameState",
          "let gameState = 'start'" not in normalized)
    combined_ctx = html.replace(
        "const canvas=document.getElementById('g');const ctx=canvas.getContext('2d');",
        "const canvas=document.getElementById('g'),ctx=canvas.getContext('2d');",
    )
    combined_ctx_normalized = cf._normalize_html_game(combined_ctx, "breakout_pinball", 2)
    check("code_forge normalizer preserves comma-declared ctx without redeclaration",
          "let ctx =" not in combined_ctx_normalized
          and combined_ctx_normalized.count("ctx=canvas.getContext('2d')") == 1)
    check("code_forge comma-declared ctx normalization is idempotent",
          cf._normalize_html_game(combined_ctx_normalized, "breakout_pinball", 2)
          == combined_ctx_normalized)
    delegated_draw = html.replace(
        "function draw(){ctx.fillRect(0,0,10,10)}",
        "function draw(){ctx.save();drawBackground();ctx.restore()} "
        "function drawBackground(){ctx.fillRect(0,0,10,10)}",
    )
    delegated_draw_normalized = cf._normalize_html_game(delegated_draw, "shooter", 2)
    check("code_forge normalizer recognizes delegated visible drawing",
          'ctx.fillStyle = ctx.fillStyle || "#0a0a1f"' not in delegated_draw_normalized)
    nested_delegated_draw = html.replace(
        "function draw(){ctx.fillRect(0,0,10,10)}",
        "function draw(){ctx.save();if(true){ctx.translate(1,1)}drawBackground();ctx.restore()} "
        "function drawBackground(){ctx.fillRect(0,0,10,10)}",
    )
    nested_delegated_draw_normalized = cf._normalize_html_game(
        nested_delegated_draw, "shooter", 2,
    )
    check("code_forge delegated drawing survives a nested prelude",
          'ctx.fillStyle = ctx.fillStyle || "#0a0a1f"'
          not in nested_delegated_draw_normalized)
    explicit_size_normalized = cf._normalize_html_game(html, "shooter", 2)
    check("code_forge normalizer preserves explicit canvas markup dimensions",
          "canvas.width = 800; canvas.height = 600;" not in explicit_size_normalized)
    frame_named = html.replace("gameLoop", "frame") + (" " * 2000)
    check("code_forge completeness accepts a non-gameLoop RAF name",
          cf._looks_like_complete_html_game(frame_named))
    late_state = html.replace(
        "(function(){",
        "(function(){" + ("/* late-state padding */" * 80) + "let gameState='start';",
    )
    late_state_normalized = cf._normalize_html_game(late_state, "endless_runner", 2)
    check("code_forge normalizer preserves one late gameState declaration",
          len(__import__('re').findall(r'\b(?:let|const|var)\s+gameState\b', late_state_normalized)) == 1)

    puzzle_html = html.replace(
        "function draw(){",
        "const metrics={moves:0,crates_on_goals:0}; function tryMove(dx,dy){return false;} tryMove(1,0); function draw(){",
    )
    puzzle_normalized = cf._normalize_html_game(puzzle_html, "puzzle_grid", 2)
    check("code_forge normalizer never fabricates puzzle success metrics",
          "metrics.moves = (metrics.moves || 0) + 1" not in puzzle_normalized)
    check("code_forge puzzle fallback adapter is fail-closed",
          "KRAKEN_ADAPTER_UNIMPLEMENTED" in puzzle_normalized)
    check("code_forge normalizer never invents puzzle grid declarations",
          "var gridW = 11" not in puzzle_normalized)
    check("code_forge puzzle normalization is idempotent",
          cf._normalize_html_game(puzzle_normalized, "puzzle_grid", 2) == puzzle_normalized)
    seed_dir = os.path.join(ROOT, "scratch")
    os.makedirs(seed_dir, exist_ok=True)
    seed_path = os.path.join(seed_dir, "regression_seed.html")
    with open(seed_path, "w", encoding="utf-8") as fh:
        fh.write(html)
    try:
        seed, seed_error = cf._read_seed_file(ROOT, "scratch/regression_seed.html")
        check("code_forge loads reviewed seed_file inside scratch",
              seed == html and seed_error is None)
        escaped_seed, escaped_error = cf._read_seed_file(ROOT, "../coordination/STATUS_BOARD.md")
        check("code_forge refuses seed_file outside scratch/jobs",
              escaped_seed is None and "must stay inside" in (escaped_error or ""))
    finally:
        try:
            os.remove(seed_path)
        except OSError:
            pass

    # verify_only must certify transactionally without invoking the model
    publish_root = tempfile.mkdtemp(prefix="reg-publish-", dir=ws)
    staging = tempfile.mkdtemp(prefix="reg-stage-", dir=os.path.join(ROOT, "jobs"))
    try:
        project = os.path.join(publish_root, "project")
        os.makedirs(project, exist_ok=True)
        artifact = os.path.join(project, "verified.py")
        with open(artifact, "w", encoding="utf-8") as fh:
            fh.write("def answer():\n    return 42\n")
        payload = json.dumps({
            "task": "certify existing artifact",
            "filename": "verified.py",
            "dir": "project",
            "verify_only": True,
        })
        ctx = {"root": ROOT, "workspace": publish_root, "workdir": staging,
               "log": lambda message: None, "llm": _NoGenerateLLM()}
        result = cf.execute({"id": "reg-verify-only", "skill": "code_forge", "input": payload}, ctx)
        check("code_forge verify_only passes without model generation", result.get("ok") is True)
        check("code_forge verify_only preserves published artifact",
              open(artifact, encoding="utf-8").read() == "def answer():\n    return 42\n")
    finally:
        shutil.rmtree(publish_root, ignore_errors=True)
        shutil.rmtree(staging, ignore_errors=True)


# ---- robustness: orphan reaper ----------------------------------------------
def test_reaper():
    tmp = tempfile.mkdtemp(prefix="kraken-reg-q-")
    q = Queue(tmp)
    job = q.add("x", "y")
    # write an ancient running record directly — q.update() would restamp
    # `updated` to now, which is itself the correct production behavior.
    job["status"] = "running"
    job["updated"] = "2000-01-01T00:00:00"
    with open(q.path, "a", encoding="utf-8") as fh:
        fh.write(json.dumps(job) + "\n")
    n = q.reap_stale(max_running_secs=900)
    after = [j for j in q.all() if j["id"] == job["id"]][0]
    check("reaper resets stale running -> queued", n == 1 and after["status"] == "queued")
    # a fresh running job is NOT reaped
    job2 = q.add("x", "z")
    job2["status"] = "running"
    q.update(job2)
    n2 = q.reap_stale(max_running_secs=900)
    check("reaper spares fresh running job", n2 == 0)


def test_supervisor_singleton():
    """Only one daemon/manual runner may own the model-backed worker."""
    from kraken.kernel import supervisor
    with tempfile.TemporaryDirectory() as td:
        first = supervisor._acquire_supervisor_lease(td)
        second = supervisor._acquire_supervisor_lease(td)
        check("supervisor lease admits first runner", first is not None)
        check("supervisor lease refuses competing runner", second is None)
        supervisor._release_supervisor_lease(first)
        third = supervisor._acquire_supervisor_lease(td)
        check("supervisor lease is reusable after release", third is not None)
        supervisor._release_supervisor_lease(third)


# ---- self-improvement: recall + multi-mission parse -------------------------
def test_recall_and_parse():
    hits = recall.code_lessons(ROOT, "Write add(a, b) returning the sum", k=3)
    check("recall surfaces relevant past lessons", len(hits) >= 1)
    txt = recall.format_code_lessons(hits)
    check("recall renders a briefing", "HARD-WON LESSONS" in txt if hits else True)


def test_game_prompts_and_checks():
    """Lock in the shared game training doctrine and verifier extraction.
    These make the swarm dramatically better at Captain Comic style clones
    and keep verify_code from turning into spaghetti.
    """
    try:
        from kraken.kernel import game_prompts
        g = game_prompts.get_guidance("index.html", "Make a Captain Comic style platformer with gravity and collectibles")
        check("game_prompts provides platformer/Captain Comic guidance", "PLATFORMER" in g and "Captain Comic" in g)
        check("game_prompts injects beast runtime contract", "__KRAKEN_BEAST__" in g and "runtime-tested" in g)
        check("game_prompts detects platformer task", game_prompts.is_platformer_task("side scrolling jump and run"))
        check("game_prompts clone scaffold available for platformers", game_prompts.get_seed_scaffold_for_clone("clone of classic platformer") is not None)

        from kraken.harness import game_checks
        issues = game_checks.collect_all_static_issues("<html><canvas></canvas>", [])  # broken: no script
        check("game_checks runs static playability suite", isinstance(issues, list))
        check("game_checks flags missing structure basics", any("no <script>" in i or "not an HTML" in i for i in issues))
        delegated_loop = """
const canvas=document.getElementById('game'); const ctx=canvas.getContext('2d');
let gameState='start'; function frame(){ctx.fillRect(0,0,2,2);requestAnimationFrame(frame)}
function startLoop(){requestAnimationFrame(frame)}
function startGame(){gameState='playing';startLoop()}
document.getElementById('playBtn').addEventListener('click',startGame);
"""
        delegated_issues = game_checks.launchability_smells(
            [delegated_loop],
            '<canvas id="game" width="800" height="600"></canvas><button id="playBtn">Play</button>',
        )
        check("game_checks accepts delegated RAF starter names",
              not any("requestAnimationFrame(gameLoop)" in issue for issue in delegated_issues))

        comma_overlay_script = """
const startOverlay=document.getElementById('start-overlay'),endOverlay=document.getElementById('end-overlay');
function startGame(){startOverlay.classList.add('hidden');endOverlay.classList.add('hidden')}
"""
        comma_overlay_issues = game_checks.launchability_smells(
            [comma_overlay_script],
            '<div id="start-overlay"></div><div id="end-overlay"></div>',
        )
        check("game_checks recognizes comma-declared overlay DOM refs",
              not any("overlay DOM ref used but never declared" in issue
                      for issue in comma_overlay_issues))

        from kraken.harness import browser_game_probe
        good_report = {
            "clickedPlay": True,
            "overlayHiddenAfterStart": True,
            "rafCount": 30,
            "audioStarts": 3,
            "errors": [],
            "consoleErrors": [],
            "beastApi": True,
            "canvasStart": {"width": 800, "height": 600, "colors": 12, "hash": 1},
            "canvasAfterRight": {"width": 800, "height": 600, "colors": 12, "hash": 2},
            "canvasAfterJump": {"width": 800, "height": 600, "colors": 12, "hash": 3},
            "telemetry": {
                "initial": {"state": "playing", "lives": 3, "player": {"x": 10, "y": 100, "vx": 0, "vy": 0}},
                "afterRight": {"state": "playing", "lives": 3, "player": {"x": 24, "y": 100, "vx": 2, "vy": 0}},
                "duringJump": {"state": "playing", "lives": 3, "player": {"x": 24, "y": 92, "vx": 0, "vy": -7}},
                "afterJump": {"state": "playing", "lives": 3, "player": {"x": 24, "y": 88, "vx": 0, "vy": -3}},
                "afterDamage": {"state": "playing", "lives": 2, "player": {"x": 10, "y": 100, "vx": 0, "vy": 0}},
                "transitions": [{"state": "playing", "level": 2}, {"state": "won", "level": 3, "complete": True}],
            },
        }
        check("browser probe accepts complete beast evidence", not browser_game_probe.assess(good_report, require_beast=True))
        bad_report = dict(good_report, errors=["ReferenceError: rangeMin is not defined"])
        check("browser probe rejects post-Play runtime errors", any("rangeMin" in reason for reason in browser_game_probe.assess(bad_report, require_beast=True)))
    except Exception as exc:
        check("game_prompts_and_checks importable", False, str(exc))


def test_game_profiles_suite():
    from kraken.tests import test_game_profiles
    import unittest
    suite = unittest.defaultTestLoader.loadTestsFromModule(test_game_profiles)
    runner = unittest.TextTestRunner(stream=open(os.devnull, 'w'))
    result = runner.run(suite)
    check("game_profiles unit tests pass", result.wasSuccessful(), f"Failures: {len(result.failures) + len(result.errors)}")


def test_game_graduation_suite():
    import unittest
    from kraken.tests import test_game_graduation

    suite = unittest.defaultTestLoader.loadTestsFromModule(test_game_graduation)
    result = unittest.TextTestRunner(stream=open(os.devnull, "w")).run(suite)
    check("game_graduation unit tests pass", result.wasSuccessful(), f"Failures: {len(result.failures) + len(result.errors)}")


# ---- helpers ----------------------------------------------------------------
class _StubLLM:
    ORNITH = "ornith"
    def generate(self, *a, **k):
        return "internal"
    def generate_json(self, *a, **k):
        return []


class _NoGenerateLLM:
    QWEN = "qwen2.5:7b"
    ORNITH = "ornith"
    def generate(self, *args, **kwargs):
        raise AssertionError("verify_only must not invoke the model")
    def generate_json(self, *args, **kwargs):
        raise AssertionError("verify_only must not invoke the model")


def main():
    print("Kraken regression suite\n" + "=" * 40)
    for fn in (test_verify_code, test_prod_digest_allowlist, test_research_local_guards,
               test_code_forge_confinement, test_reaper, test_supervisor_singleton,
               test_recall_and_parse,
               test_game_prompts_and_checks,
               test_game_profiles_suite, test_game_graduation_suite):
        try:
            fn()
        except Exception as exc:
            check(fn.__name__ + " (crashed)", False, repr(exc))
    print("=" * 40)
    print(f"{len(PASS)} passed, {len(FAIL)} failed")
    if FAIL:
        print("FAILED:", ", ".join(FAIL))
        return 1
    print("All earned safety properties hold.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
