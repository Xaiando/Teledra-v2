"""code_forge - qwen2.5 + Ornith coupled gen -> verify -> repair for small modules.

qwen2.5:7b (stronger generalist) assists as primary for initial generation and game logic.
Ornith (code specialist) is coupled for targeted repair passes.
Both via Ollama. Override with "model" in input payload.
"""

from __future__ import annotations

import importlib.util
import json
import os
import re
import shutil
import time
from pathlib import Path

try:
    from kraken.kernel import game_prompts
except Exception:
    game_prompts = None  # fallback keeps old behavior if import fails during transition


MAX_REPAIR_ATTEMPTS = 2
DEFAULT_FILENAME = "forged_module.py"
MODEL_TIMEOUT_S = 75
RICH_ARTIFACT_TIMEOUT_S = 600  # increased for complex browser game polish on local models (qwen2.5 primary + Ornith specialist for long-context full-file gens)


def _parse_input(raw: str) -> dict:
    raw = raw.strip()
    if raw.startswith("{"):
        # strict=False: operators type multi-line tests in chat, which puts
        # raw newlines inside JSON strings — accept them.
        data = json.loads(raw, strict=False)
    else:
        data = {"task": raw}
    data.setdefault("task", "Write a small, self-contained Python module.")
    data.setdefault("filename", DEFAULT_FILENAME)
    return data


def _safe_filename(name: str) -> str:
    name = os.path.basename(name.strip() or DEFAULT_FILENAME)
    name = re.sub(r"[^A-Za-z0-9_.-]", "_", name)
    if not Path(name).suffix:
        name += ".py"
    return name


def _inside(parent: str, child: str) -> bool:
    try:
        Path(child).resolve().relative_to(Path(parent).resolve())
        return True
    except ValueError:
        return False


def _strip_response(text: str) -> str:
    text = text.strip()
    if "</think>" in text:
        text = text.rsplit("</think>", 1)[1].strip()
    fence = re.search(r"```[A-Za-z0-9_-]*\s*(.*?)```", text, re.DOTALL)
    if fence:
        return fence.group(1).strip()
    dangling_fence = re.match(r"^```[A-Za-z0-9_-]*\s*\n?(.*)", text, re.DOTALL)
    if dangling_fence:
        return dangling_fence.group(1).strip()
    return text


def _artifact_kind(filename: str) -> str:
    suffix = Path(filename).suffix.lower()
    if suffix in {".html", ".htm"}:
        return "one complete, self-contained HTML document with embedded CSS and JavaScript"
    if suffix in {".js", ".mjs"}:
        return "one complete, self-contained JavaScript module"
    if suffix in {".json"}:
        return "one valid JSON document"
    if suffix in {".rs"}:
        return "one complete Rust source file"
    return "one complete, self-contained Python module"


def _artifact_guidance(filename: str, task: str = "", profile: str | None = None) -> str:
    suffix = Path(filename).suffix.lower()
    if suffix in {".html", ".htm"}:
        if game_prompts:
            try:
                return game_prompts.get_guidance(filename, task, profile=profile)
            except Exception:
                pass
        # Fallback (should rarely trigger): keep a minimal safe baseline
        return (
            "HTML/browser-game: full <!DOCTYPE html> + canvas + embedded script/style. "
            "requestAnimationFrame gameLoop. addEventListener wiring. Web Audio SFX. "
            "No external assets. First seconds playable. No markdown. "
            "Platformer: gravity per frame, grounded jump + coyote/buffer, AABB/tile collision no fallthrough, "
            "collectibles, hazards, camera follow, exit advances. Return raw file only."
        )
    return ""


def _is_python_artifact(filename: str) -> bool:
    return Path(filename).suffix.lower() in {"", ".py"}

def _looks_like_complete_html_game(text: str) -> bool:
    """Strict guard against fragments, hollow files, or missing critical structure for browser games.
    Must have full document + substantial inline script with game loop and wiring.
    """
    if not text or len(text) < 4000:
        return False
    t = text.lower()
    if not ("<!doctype" in t or "<html" in t):
        return False
    if "<script" not in t:
        return False
    if "requestanimationframe" not in t and "setinterval" not in t:
        return False
    if "canvas" not in t and "getcontext" not in t:
        return False
    # Critical for no "no inline JavaScript found" and launchability
    if "addeventlistener" not in t:
        return False
    # Do not require a particular loop identifier. `frame`, `tick`, and
    # `animate` are equally valid when a scheduling primitive is present.
    # Canvas sizing must be explicit
    if "canvas.width" not in t and "width=" not in t:
        return False
    return True


def _normalize_html_game(
    text: str,
    profile: str | None = None,
    contract_version: int | None = None,
    task: str | None = None,
) -> str:
    """Deterministic post-LLM structural normalizer.
    Fixes the top recurring failures seen in qwen2.5+Ornith repairs without burning another model call:
    - Hoist key DOM refs (canvas, ctx, overlay, playBtn) early in the IIFE
    - Guarantee unconditional requestAnimationFrame at end of gameLoop
    - Ensure draw does visible work (at least a background clear + something)
    - Promote raw level data to live .alive entities when the pattern exists
    Run this on every HTML candidate before the first write/verify.
    """
    if not text or len(text) < 2000:
        return text
    fixed = text

    # 1. Hoist the most important DOM refs to the top of the IIFE if they are missing or late.
    # CRITICAL: Never cause "Identifier 'canvas' has already been declared".
    # Detect from <canvas id=...> and only inject a declaration if NONE exists anywhere
    # (handles preloaded games that already do `let canvas, ctx;` or `const canvas = ...`).
    iife = re.search(r'\(function\s*\(\s*\)\s*\{', fixed)
    if iife:
        insert_pos = iife.end()
        head = fixed[insert_pos:insert_pos+1200]
        canvas_id_match = re.search(r'<canvas[^>]*id=["\']([^"\']+)["\']', fixed, re.I)
        canvas_id = canvas_id_match.group(1) if canvas_id_match else 'game-canvas'
        # Strong guard: any prior declaration of canvas (bare or assigned) means do not redeclare
        has_canvas_decl = bool(re.search(r'\b(let|const|var)\s+canvas\b', head))
        has_get_canvas = bool(re.search(r'canvas\s*=\s*document\.getElementById', head))
        if not has_canvas_decl and not has_get_canvas:
            fixed = (fixed[:insert_pos] +
                     f"\n  const canvas = document.getElementById('{canvas_id}');\n"
                     "  const ctx = canvas.getContext('2d');\n" +
                     fixed[insert_pos:])
        elif has_canvas_decl and not has_get_canvas:
            # Reuse existing canvas var: ensure it is assigned the element and sized
            # Find a safe insertion after the first canvas decl line
            decl_match = re.search(r'(\b(let|const|var)\s+canvas\b[^;\n]*[;\n])', head)
            if decl_match:
                # append assignment after the decl line in the original fixed
                assign = f"  canvas = document.getElementById('{canvas_id}'); if (typeof ctx === 'undefined' || !ctx) ctx = canvas.getContext('2d');\n"
                # insert after the matched decl in the full text (approximate at iife area)
                fixed = fixed[:insert_pos + decl_match.end()] + assign + fixed[insert_pos + decl_match.end():]

    # 2. Make sure gameLoop always reschedules RAF unconditionally
    if 'requestAnimationFrame(gameLoop)' not in fixed:
        fixed = re.sub(
            r'(function\s+gameLoop\s*\([^)]*\)\s*\{[\s\S]{0,400}?)(draw\(\);?\s*\n?\s*\})',
            r'\1draw();\n    requestAnimationFrame(gameLoop);\n  }',
            fixed, flags=re.I
        )

    # 3. Guarantee a visible draw heartbeat (prevents "blank/flat" and "pixels never change")
    draw_start = re.search(r'function draw\s*\([^)]*\)\s*\{', fixed)
    if draw_start:
        # Inspect a bounded prefix after the opening brace instead of stopping
        # at the first nested `}`. Screen-shake/clip guards commonly appear
        # before a delegated `drawBackground()` call.
        draw_prefix = fixed[draw_start.end():draw_start.end() + 600]
        delegates_visible_draw = bool(
            re.search(r'\bdraw[A-Za-z_$][\w$]*\s*\(', draw_prefix)
        )
        if (
            'fillRect' not in draw_prefix
            and 'clearRect' not in draw_prefix
            and not delegates_visible_draw
        ):
            fixed = re.sub(
                r'(function draw\s*\([^)]*\)\s*\{)',
                r'\1\n    ctx.fillStyle = ctx.fillStyle || "#0a0a1f";\n    ctx.fillRect(0, 0, canvas.width || 800, canvas.height || 600);',
                fixed, count=1
            )

    # 4. Promote entities to live objects if we see the classic raw iteration pattern
    if 'for (let e of level.enemies)' in fixed or 'level.enemies' in fixed:
        if 'currentEnemies' not in fixed and 'liveEnemies' not in fixed:
            fixed = re.sub(
                r'for \(let e of level\.enemies\)',
                'for (let e of (currentEnemies || (currentEnemies = level.enemies.map(d => createEnemy(d.type || d)))) )',
                fixed
            )

    # Ensure tabindex on canvas
    if '<canvas' in fixed and 'tabindex' not in fixed.lower():
        fixed = re.sub(r'(<canvas[^>]*)(>)', r'\1 tabindex="0"\2', fixed, count=1, flags=re.I)

    # Ensure canvas.width/height in JS if missing. Be tolerant of pre-existing canvas var.
    has_markup_canvas_size = bool(
        re.search(r'<canvas\b[^>]*\bwidth\s*=\s*["\']?\d+[^>]*\bheight\s*=\s*["\']?\d+', fixed, re.I)
        or re.search(r'<canvas\b[^>]*\bheight\s*=\s*["\']?\d+[^>]*\bwidth\s*=\s*["\']?\d+', fixed, re.I)
    )
    if (
        'canvas.width' not in fixed
        and 'canvas.height' not in fixed
        and not has_markup_canvas_size
    ):
        # Try to inject after a canvas get/decl
        fixed = re.sub(
            r'(canvas\s*=\s*document\.getElementById\([^;]+;\s*)',
            r'\1canvas.width = 800; canvas.height = 600;\n  ',
            fixed, count=1
        )
        if 'canvas.width' not in fixed:
            # Fallback: inject early after first canvas-related line we can find
            fixed = re.sub(
                r'(\bcanvas\b[^;\n]*[;\n]\s*)',
                r'\1canvas.width = 800; canvas.height = 600;\n  ',
                fixed, count=1
            )

    # Game state identifiers are semantic. Do not invent `gameState` when a
    # game legitimately uses `state`, `currentState`, or another scoped model.
    # Missing adapter bindings remain fail-closed and verifier-guided.

    # Grid dimensions and puzzle metrics are semantic game state.  Do not
    # invent, hoist, rename, or de-duplicate them with regex.  That previously
    # produced repeated `gridW` declarations when normalization ran more than
    # once and could turn blocked moves into fabricated success evidence.

    # Robust audio start for puzzle games: call the background music function in startGame if present.
    if profile == 'puzzle_grid':
        if 'startBackgroundMusic' in fixed and 'startBackgroundMusic()' not in fixed:
            # Insert after startGameLoop or after setting playing in the startGame function
            fixed = re.sub(
                r'(startGameLoop\(\)\s*;)',
                r'\1 startBackgroundMusic();',
                fixed, count=1
            )
        if 'startBgMusic' in fixed and 'startBgMusic()' not in fixed:
            fixed = re.sub(
                r'(startGameLoop\(\)\s*;)',
                r'\1 startBgMusic();',
                fixed, count=1
            )

    # Simulation / exploration (Duchman / Lost Dutchman style): ensure audio starts on play, proper gating, no platformer bleed.
    task_lower = (task or '').lower()
    if profile == 'simulation' or 'duchman' in task_lower or 'dutchman' in task_lower or ('mine' in task_lower and 'explor' in task_lower):
        if 'startMusic' in fixed or 'initAudio' in fixed:
            # Force call on play
            fixed = re.sub(
                r'(playBtn\.addEventListener\([^)]+\)\s*\{)',
                r'\1 if (typeof startMusic === "function") startMusic(); if (typeof initAudio === "function") initAudio(); ',
                fixed, count=1
            )
        # Gate updates in loop if not already
        if 'function gameLoop' in fixed and 'if (gameState !== "playing")' not in fixed:
            fixed = re.sub(
                r'(function gameLoop\([^)]*\)\s*\{)',
                r'\1 if (gameState !== "playing") { requestAnimationFrame(gameLoop); return; } ',
                fixed, count=1
            )
        # Clean beast of puzzle bleed for simulation
        if '__KRAKEN_BEAST__' in fixed:
            fixed = re.sub(r'crates_on_goals[^,}]*', 'crates_on_goals: 0', fixed)
            fixed = re.sub(r'"crates_on_goals"[^,}]*', '"crates_on_goals": 0', fixed)
        # For large Dutchman-style sim: ensure real-time decay, multiple activity handlers, inventory/map state, procedural hooks if present
        if 'hunger' in fixed.lower() or 'fatigue' in fixed.lower():
            # encourage decay timers or action costs
            pass
        # User sees no progress - force substantial implementation of full mechanics this iteration, and clean code for normalizer
        if 'dutchman' in task_lower or 'duchman' in task_lower:
            # Add comments or structure to encourage full systems
            pass
        # For 'nothing feels like the original' feedback: prioritize atmosphere, survival tension, free roam feel in code structure
        if 'original' in task_lower or 'soul' in task_lower or 'atmosphere' in task_lower:
            pass
        # Force 10-20MB with assets for Dutchman, and clean code for loop
        if 'dutchman' in task_lower or 'duchman' in task_lower or '20mb' in task_lower or 'mb' in task_lower:
            # Encourage lots of base64 assets + rich code, standard patterns
            pass

    # Ensure ctx is acquired if we have a canvas get but no ctx.
    # Use a declaring pattern that is safe inside 'use strict' IIFEs (no bare assignment to undeclared).
    # A declaration may start with another identifier, for example
    # `const canvas = ..., ctx = ...;`.  Treat the entire declaration statement
    # as authoritative so normalization never injects a duplicate `ctx`.
    has_ctx_declaration = bool(re.search(
        r'\b(?:let|const|var)\s+[^;\n]*\bctx\b',
        fixed,
    ))
    if re.search(r'canvas\s*=\s*document\.getElementById', fixed) and not has_ctx_declaration:
        fixed = re.sub(
            r'(canvas\s*=\s*document\.getElementById\([^;]+;\s*)',
            r'\1let ctx = (typeof ctx !== "undefined" && ctx) || canvas.getContext("2d");\n  ',
            fixed, count=1
        )

    # Do not invent or rename overlay DOM references. Games legitimately use
    # `startScreen`, `menuOverlay`, or other IDs; the old generic rewrite
    # produced null references and even emitted escaped quotes into JavaScript.
    # Let the verifier-guided repair bind the game's real element explicitly.

    # A missing v2 adapter is fail-closed for every profile, including
    # puzzle_grid.  The normalizer may describe the required contract, but it
    # must never mutate gameplay state merely to satisfy the probe.
    if (
        '__KRAKEN_BEAST__' not in fixed
        and '__kraken_beast__' not in fixed.lower()
        and profile
        and contract_version == 2
        and game_prompts is not None
    ):
        try:
            from kraken.kernel import game_profiles
            known_profile = game_profiles.get_profile(profile) is not None
        except Exception:
            known_profile = False
        if known_profile:
            profile_js = json.dumps(profile)
            beast = f"""
// KRAKEN_ADAPTER_UNIMPLEMENTED: replace this sentinel with real game-state wiring.
window.__KRAKEN_BEAST__ = {{
  version: 2,
  profile: {profile_js},
  snapshot() {{ throw new Error('KRAKEN_ADAPTER_UNIMPLEMENTED: snapshot for {profile}'); }},
  action(name) {{ throw new Error('KRAKEN_ADAPTER_UNIMPLEMENTED: action ' + name + ' for {profile}'); }}
}};
"""
            closures = list(re.finditer(r'\}\)\(\);?', fixed))
            insert_at = closures[-1].start() if closures else fixed.lower().rfind('</script>')
            if insert_at >= 0:
                fixed = fixed[:insert_at] + beast + '\n' + fixed[insert_at:]

    # Extra defensive for common runtime ref errors seen in journals (gameState, overlay, restart btns)
    # Ensure basic state if snapshot or actions reference it (sim/breakout common)
    if '__KRAKEN_BEAST__' in fixed and 'gameState' not in fixed:
        # Inject a minimal gameState if beast is present but state undefined in scope
        fixed = re.sub(
            r'(window\.__KRAKEN_BEAST__\s*=)',
            r'let gameState = gameState || "menu";\n  \1',
            fixed, count=1
        )
    if 'startOverlay' in fixed or 'startScreen' in fixed:
        # Ensure overlay var if referenced but not declared (common ReferenceError)
        if 'startOverlay' not in fixed.split('const canvas')[0] if 'const canvas' in fixed else True:
            fixed = re.sub(
                r'(const canvas = document\.getElementById)',
                r'const startOverlay = document.getElementById("overlay") || document.getElementById("startOverlay") || document.getElementById("startScreen");\n  \1',
                fixed, count=1
            )
    # Ensure playAgain or restart btn if referenced in error logs
    if 'playAgainBtn' in fixed or 'restartBtn' in fixed:
        if 'playAgainBtn' not in fixed and 'const playAgainBtn' not in fixed:
            fixed = re.sub(
                r'(</script>)',
                r'  const playAgainBtn = document.getElementById("playAgainBtn") || document.getElementById("restartBtn");\n\1',
                fixed, count=1
            )

    return fixed


def _get_game_skeleton() -> str:
    """Strong, complete, CORRECT minimal skeleton for browser platformer-style games.
    Every critical rule is demonstrated here so the model has a safe pattern:
    - Full valid HTML
    - <canvas> with tabindex + width/height attrs + JS literals
    - Play button with addEventListener ONLY, unique ID
    - No inline onclick anywhere
    - Full RAF gameLoop
    - gameState gating
    - Basic structure for levels, player, gravity, collect, enemy drop stub
    - HUD/overlay patterns
    NEVER regress to fragments or bad wiring when using this.
    """
    return """<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<title>Platformer</title>
<style>
* { margin:0; padding:0; box-sizing:border-box; }
body { background:#111; display:flex; justify-content:center; align-items:center; min-height:100vh; font-family:monospace; color:#fff; }
#gameCanvas { border:3px solid #e94560; background:#000; display:block; cursor:none; width:800px; height:600px; }
#overlay { position:fixed; top:0; left:0; width:100%; height:100%; display:flex; flex-direction:column; justify-content:center; align-items:center; background:rgba(0,0,0,0.85); z-index:10; }
#overlay.hidden { display:none; }
.btn { background:#e94560; color:#fff; border:none; padding:15px 40px; font-size:1.3em; cursor:pointer; margin:10px; font-family:monospace; }
#hud { position:fixed; top:10px; left:50%; transform:translateX(-50%); display:flex; gap:20px; color:#fff; z-index:5; pointer-events:none; }
</style>
</head>
<body>
<canvas id="gameCanvas" tabindex="0" width="800" height="600"></canvas>
<div id="overlay">
  <h1>GAME</h1>
  <p>Controls: Arrows/WASD + Space</p>
  <button class="btn" id="playBtn">PLAY GAME</button>
</div>
<div id="hud"><span id="score">Score: 0</span></div>
<script>
(function() {
'use strict';
const canvas = document.getElementById('gameCanvas');
canvas.width = 800;
canvas.height = 600;
const ctx = canvas.getContext('2d');

let gameState = 'menu'; // menu, playing, won, lost
let score = 0;
let currentLevel = 0;
let levels = [
  { platforms: [{x:0,y:480,w:800,h:20}], enemies: [{x:600,y:420,type:'basic'}] },
  // TODO: add 4+ more levels with increasing difficulty, moving platforms, hazards
];
let camera = { x: 0, y: 0 }; // simple follow camera

// INPUT - addEventListener only
const keys = {};
document.addEventListener('keydown', e => { keys[e.code] = true; if (e.code === 'Space') e.preventDefault(); });
document.addEventListener('keyup', e => { keys[e.code] = false; });

// DOM refs declared early (before any startGame / handler use) to avoid "ref before declaration" errors
// NOTE: canvas and ctx were already declared above from the <canvas> element; reuse, do not redeclare.
const overlay = document.getElementById('overlay');
const playBtn = document.getElementById('playBtn');

// Play button - CORRECT pattern, unique ID
playBtn.addEventListener('click', () => {
  overlay.classList.add('hidden');
  gameState = 'playing';
  canvas.focus();
  loadLevel(0);
});

// Basic player + platformer stubs (expand in real build)
let player = { x: 100, y: 400, vx: 0, vy: 0, w: 24, h: 36, onGround: false };
let platforms = [];
let enemies = [];
let drops = [];
let currentLevelData = null;

function rectOverlap(a,b){ return !(a.x+a.w < b.x || b.x+b.w < a.x || a.y+a.h < b.y || b.y+b.h < a.y); }

function loadLevel(n) {
  currentLevel = n;
  if (levels[n]) {
    currentLevelData = levels[n];
    platforms = [...currentLevelData.platforms];
    enemies = (currentLevelData.enemies || []).map(e => ({...e, alive:true}));
    // reset player position etc.
  }
}

function updateCamera() {
  // simple camera follow - keep player near center
  camera.x = Math.max(0, Math.min( player.x - 400, 800 ));
  // apply in draw: ctx.save(); ctx.translate(-camera.x, -camera.y);
}

function update() {
  // gravity + move stub
  player.vy += 0.6;
  player.x += player.vx;
  player.y += player.vy;
  player.onGround = false;
  for (let p of platforms) {
    if (rectOverlap(player, p)) { player.y = p.y - player.h; player.vy = 0; player.onGround = true; }
  }
  // simple enemy + drop stub
  for (let i=enemies.length-1; i>=0; i--) {
    let e = enemies[i];
    if (!e.alive) continue;
    e.x += (e.dir || 1) * 1;
    if (rectOverlap(player, e)) {
      // hit - enemy dies and drops
      e.alive = false;
      spawnDrop(e.x, e.y);
    }
  }
  // collect drops by contact
  for (let i=drops.length-1; i>=0; i--) {
    if (rectOverlap(player, drops[i])) {
      score += 10;
      drops.splice(i,1);
      // play sound stub
    }
  }
  updateCamera();
}

function draw() {
  ctx.fillStyle = '#0a0a1f';
  ctx.fillRect(0,0,canvas.width,canvas.height);
  ctx.save();
  ctx.translate(-camera.x, 0);
  ctx.fillStyle = '#4a4';
  for (let p of platforms) ctx.fillRect(p.x, p.y, p.w, p.h);
  ctx.fillStyle = '#e94560';
  ctx.fillRect(player.x, player.y, player.w, player.h);
  ctx.fillStyle = '#ff0';
  for (let d of drops) ctx.fillRect(d.x, d.y, 12, 12);
  // draw live enemies
  ctx.fillStyle = '#f44';
  for (let e of enemies) if (e.alive) ctx.fillRect(e.x, e.y, 20, 20);
  ctx.restore();
}

function gameLoop() {
  update();
  draw();
  // ALWAYS reschedule — never gate the rAF call itself on state (prevents grey after startGame)
  requestAnimationFrame(gameLoop);
}

// Start loop at load (will early gate inside draw/update until playing)
gameLoop();

// Example enemy drop on "defeat" (call this from real hit logic)
function spawnDrop(x, y) {
  drops.push({x:x, y:y, w:12, h:12, type:'powerup'});
}

// === CRITICAL ANTI-GREY + DOM ORDER + VISUAL FEEDBACK (MUST BE IN EVERY BUILD) ===
// 1. Declare ALL DOM refs at the VERY TOP of the IIFE (before any functions/handlers):
//    const canvas = ...; const ctx = ...; const overlay = ...; const playBtn = ...;
// 2. gameLoop MUST: update(); draw(); requestAnimationFrame(gameLoop);  // unconditional, always reschedule
// 3. startGame / Play handler MUST: hide overlay, set playing, loadLevel, kick RAF, focus canvas.
// 4. In draw (when playing): always do visible work (fill background + platforms + player + enemies + particles).
//    This ensures pixel changes and "at least 4 sampled colors" + animation on input.
// 5. Live entities: promote level data to live arrays with .alive; update and draw only alive ones.
// 6. HUD: separate lives (hearts) and score; update after every mutation.
// See full skeleton above for working example.
})();
</script>
</body>
</html>"""


def _timeout_for_artifact(filename: str) -> int:
    suffix = Path(filename).suffix.lower()
    if suffix in {".html", ".htm", ".js", ".mjs"}:
        return RICH_ARTIFACT_TIMEOUT_S
    return MODEL_TIMEOUT_S


def _load_verify_code(root: str):
    path = os.path.join(root, "harness", "verify_code.py")
    spec = importlib.util.spec_from_file_location("kraken_verify_code_direct", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _write_tests(workdir: str, filename: str, tests: str | None) -> str | None:
    if not tests:
        return None
    stem = Path(filename).stem
    test_path = os.path.join(workdir, f"test_{stem}.py")
    with open(test_path, "w", encoding="utf-8") as fh:
        fh.write(tests.strip() + "\n")
    return test_path


def _read_seed_file(root: str, value: object) -> tuple[str | None, str | None]:
    """Read an operator-authored seed confined to Kraken scratch/jobs.

    This lets a difficult repair start from a deterministic reviewed candidate
    without embedding a large HTML document inside queue JSON.  It deliberately
    cannot read production, coordination, credentials, or paths outside Kraken.
    """
    raw = str(value or "").strip()
    if not raw:
        return None, None
    candidate = os.path.abspath(os.path.join(root, raw))
    allowed_roots = [os.path.join(root, "scratch"), os.path.join(root, "jobs")]
    if not any(_inside(allowed, candidate) for allowed in allowed_roots):
        return None, "seed_file must stay inside kraken/scratch or kraken/jobs"
    if Path(candidate).suffix.lower() not in {".html", ".htm", ".js", ".mjs", ".py"}:
        return None, "seed_file extension is not an allowed code artifact"
    try:
        return Path(candidate).read_text(encoding="utf-8"), None
    except OSError as exc:
        return None, f"seed_file could not be read: {exc}"


def _verify(root: str, job: dict, ctx: dict, output_rel: str,
            test_rel: str | None) -> dict:
    result = {"ok": True, "output": output_rel}
    if test_rel:
        result["tests"] = [test_rel]
    return _load_verify_code(root).verify(job, result, ctx)


def _append_lesson(ctx: dict, job: dict, task: str, attempts: int,
                   reasons: list[str], final_ok: bool, output_path: str) -> None:
    lessons_dir = os.path.join(ctx["root"], "lessons")
    os.makedirs(lessons_dir, exist_ok=True)

    # Always write to the main lessons for recall.
    path = os.path.join(lessons_dir, "code_forge_lessons.jsonl")
    # For rich/HTML artifacts also write a parallel game lessons log so recall can surface them.
    is_html = str(output_path).lower().endswith((".html", ".htm"))
    game_path = os.path.join(lessons_dir, "code_forge_game_lessons.jsonl") if is_html else None

    final_code = ""
    try:
        with open(output_path, "r", encoding="utf-8") as fh:
            final_code = fh.read()[-8000:]
    except OSError:
        pass
    entry = {
        "ts": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "job_id": job.get("id"),
        "task": task,
        "attempts": attempts,
        "reasons": reasons,
        "final_ok": final_ok,
        "final_code": final_code,
    }
    with open(path, "a", encoding="utf-8") as fh:
        fh.write(json.dumps(entry, ensure_ascii=False) + "\n")
    if game_path:
        with open(game_path, "a", encoding="utf-8") as fh:
            fh.write(json.dumps(entry, ensure_ascii=False) + "\n")


def _prompt(task: str, filename: str, code: str | None = None,
            reasons: list[str] | None = None, lessons: str = "",
            profile: str | None = None, contract_version: int | None = None) -> str:
    artifact = _artifact_kind(filename)
    guidance = _artifact_guidance(filename, task, profile=profile)
    contract = (
        f"Authoritative Beast contract: profile={profile}, contract_version={contract_version}. "
        "The runtime adapter must declare those exact values and expose real game-state behavior.\n"
        if profile and contract_version is not None else ""
    )
    if code is None:
        preface = (lessons + "\n") if lessons else ""
        return (
            preface +
            f"Write {artifact} for this task.\n"
            f"Filename: {filename}\n"
            f"Task: {task}\n\n" +
            contract + ((guidance + "\n\n") if guidance else "") +
            "Return only the complete file contents. No markdown. No commentary."
        )
    return (
        f"Repair this {artifact} so it passes the verifier and tests.\n"
        f"Filename: {filename}\n"
        f"Task: {task}\n" +
        contract + ((guidance + "\n") if guidance else "") +
        "CRITICAL FOR LARGE GAMES / IMPROVE EXISTING: You are working on a COMPLETE existing game file. "
        "The 'Current code' provided below (or the seed) is the full previous version. "
        "Your ENTIRE response MUST be the FULL valid complete file starting with <!DOCTYPE html> or <html> "
        "and ending with the last </html> or </script>. Output ONLY the raw code. "
        "NEVER return a short message, instructions, explanations, the task text, or a partial/truncated file. "
        "Preserve every working section (levels, movement, camera, sounds, gameLoop, input, styles) and only apply the minimal fixes needed. "
        "If the current 'code' looks like instructions or is broken, REBUILD the full correct game from the task description while keeping the intended mechanics. "
        "Return the full corrected file from first byte to last byte, not a diff, excerpt, or partial rewrite. "
        "Do not wrap it in markdown fences. Never return a hollow HTML shell. ALWAYS keep the complete <script> block with all game code, gameLoop, addEventListeners etc.\n"
        "LAUNCH CRITICAL (STOP MAKING THE SAME MISTAKES): The game must actually start when opened as a local file in a browser. There MUST be a real interactive Play/Start button created in the DOM using addEventListener (NEVER inline onclick=), with a unique id (e.g. 'playBtn' or 'startBtn' — for restart/next ALWAYS use a DIFFERENT unique ID like 'nextLevelBtn' or 'tryAgainBtn' — NEVER duplicate 'restartBtn'). "
        "Canvas MUST have tabindex='0' AND canvas.width = 800; canvas.height = 600; (lowercase 'canvas.', literals) in JS immediately after getting the canvas. Also width/height attrs on tag. Clear key handler for Space/Enter from menu. Start action must hide overlay and set gameState='playing'. "
        "Same strict rules for any restart buttons. On repairs/polish, fix button wiring (addEventListener only), unique IDs, and tabindex as HIGHEST priority — search the FULL current code for EVERY button creation and fix them all before touching game logic. NEVER use innerHTML containing id=\"restartBtn\". "
        "If the verifier lists inline handler / duplicate ID / missing tabindex / canvas size errors, you MUST address exactly those first. "
        "Correct pattern: const playBtn = document.getElementById('playBtn'); playBtn.addEventListener('click', () => { startOverlay.style.display='none'; gameState='playing'; canvas.focus(); }); canvas.setAttribute('tabindex','0'); canvas.width=800; canvas.height=600;\n"
        "Verifier failures:\n"
        + "\n".join(f"- {reason}" for reason in (reasons or []))
        + "\n\nCurrent code:\n"
        + code
        + "\n\nReturn only the complete corrected file contents. No markdown. No commentary."
    )


def _generate_code(ctx: dict, prompt: str, *, system: str,
                   timeout: int = MODEL_TIMEOUT_S,
                   temperature: float,
                   model: str | None = None) -> tuple[str | None, str | None]:
    """Generate using the chosen model. Supports coupling qwen2.5 (primary/assist) + Ornith (specialist)."""
    # Never reach here for verify_only paths.
    if ctx.get("verify_only"):
        return None, "verify_only path must not generate"
    chosen = model or getattr(ctx.get("llm"), "QWEN", None) or "qwen2.5:7b"
    try:
        return _strip_response(ctx["llm"].generate(
            prompt,
            model=chosen,
            system=system,
            timeout=timeout,
            retries=0,
            options={"temperature": temperature, "num_predict": 16000, "num_ctx": 32768},
        )), None
    except Exception as exc:
        return None, f"{chosen} generation failed: {exc}"


def execute(job: dict, ctx: dict) -> dict:
    try:
        data = _parse_input(job["input"])
    except (json.JSONDecodeError, ValueError) as exc:
        return {"ok": False, "notes": f"input JSON invalid: {exc}. "
                "Check quoting; multi-line strings are fine, unescaped quotes are not."}
    task = str(data.get("task") or data.get("prompt") or "").strip()
    filename = _safe_filename(str(data.get("filename", DEFAULT_FILENAME)))
    declared_profile = str(data.get("profile") or "").strip().lower() or None
    try:
        contract_version = int(data["contract_version"]) if data.get("contract_version") is not None else None
    except (TypeError, ValueError):
        contract_version = None
    beast_mode = str(data.get("quality", "")).strip().lower() == "beast"
    verify_only = bool(data.get("verify_only", False))
    workdir = ctx["workdir"]

    # Model selection is deliberately skipped for verify_only / certify paths.
    # verify_only must never touch the LLM (even for attribute access) per regression contract.
    chosen_model = None
    initial_model = None
    repair_model = None
    llm_mod = ctx["llm"]
    if not verify_only:
        llm_mod.ensure_models()  # guarantee qwen2.5:7b + Ornith (fix for possible pure-ollama regression)
        # === Model coupling (qwen2.5 assists as stronger primary; Ornith coupled as code specialist) ===
        # qwen2.5:7b is preferred for initial generation (stronger general reasoning, instruction following,
        # and complex game design per operator direction). Ornith assists on repair passes for its code precision.
        # Both served through Ollama. Specify "model" in the job payload to force one for the whole run.
        requested = str(data.get("model") or "").strip().lower()
        if requested in {"qwen2.5:7b", "qwen2.5", "qwen", "qwen2"}:
            chosen_model = llm_mod.QWEN
        elif requested in {"ornith", "ornith-1", "ornith-9b", "ornith-1.0-9b-gguf"}:
            chosen_model = llm_mod.ORNITH
        else:
            chosen_model = llm_mod.QWEN  # default: qwen2.5 assists instead of pure Ornith/Ollama path
        initial_model = chosen_model
        repair_model = llm_mod.ORNITH if chosen_model == llm_mod.QWEN else chosen_model
    publish_dir = None
    # Coherence overhauls may explicitly request a clean-room generation when
    # the production artifact itself is the poisoned base.  Existing behavior
    # remains the default for ordinary polish jobs.
    preload_existing = data.get("preload_existing", True) is not False
    # optional "dir": build inside the operator's workspace (free-rein zone)
    # instead of the throwaway job workdir. Confined to the workspace root.
    target_dir = str(data.get("dir", "")).strip()
    if target_dir:
        workspace = ctx.get("workspace") or os.path.join(ctx["root"], "workspace")
        # Normalize: callers sometimes pass "workspace/games/xxx" or "games/xxx".
        # Joining raw would create .../workspace/workspace/... double nesting (bug observed in logs).
        td = target_dir.replace("\\", "/").strip("/")
        if td.startswith("workspace/"):
            td = td[len("workspace/"):]
        candidate = os.path.abspath(os.path.join(workspace, td))
        if _inside(workspace, candidate):
            publish_dir = candidate
            ctx["log"](f"code_forge staging for workspace publish: {candidate}")
    # For workspace game dirs without explicit filename, default to index.html (common for browser games).
    # Placed after target_dir so the variable is defined.
    if target_dir and filename == DEFAULT_FILENAME:
        if any(g in target_dir.lower() for g in ['game', 'breakout', 'asteroid', 'gem', 'pinball', 'neon', 'pulse', 'lane', 'dash', 'dungeon', 'crate', 'serpent', 'vault', 'rhythm', 'space', 'make_']):
            filename = "index.html"
            ctx["log"]("code_forge defaulted filename to index.html for game target_dir")
    os.makedirs(workdir, exist_ok=True)

    output_path = os.path.join(workdir, filename)
    published_output_path = os.path.join(publish_dir, filename) if publish_dir else output_path
    existing_output_path = published_output_path if os.path.exists(published_output_path) else output_path
    try:
        output_rel = os.path.relpath(output_path, ctx["root"])
    except ValueError:  # workspace on a different drive
        output_rel = output_path
    tests = data.get("tests")
    test_path = _write_tests(workdir, filename, tests if isinstance(tests, str) else None)
    test_rel = None
    if test_path:
        try:
            test_rel = os.path.relpath(test_path, ctx["root"])
        except ValueError:
            test_rel = test_path

    code = data.get("seed_code")
    if not (isinstance(code, str) and code.strip()) and data.get("seed_file"):
        code, seed_error = _read_seed_file(ctx["root"], data.get("seed_file"))
        if seed_error:
            return {"ok": False, "output": output_rel, "notes": seed_error}
        if code:
            ctx["log"](f"code_forge starting from reviewed seed_file: {data.get('seed_file')}")
    if verify_only:
        # Robustness for verify_only on workspace/dir games: always prefer the real published game file
        # when a target_dir was given, even if the job-local forged_module.py does not exist.
        if target_dir and publish_dir:
            real_game = os.path.join(publish_dir, filename)
            if os.path.exists(real_game):
                existing_output_path = real_game
        if not Path(existing_output_path).exists():
            return {"ok": False, "output": output_rel,
                    "notes": f"verify_only source missing: {existing_output_path}"}
        code = Path(existing_output_path).read_text(encoding="utf-8", errors="ignore")
        ctx["log"](f"code_forge verify_only staged existing artifact: {existing_output_path}")
    if isinstance(code, str) and code.strip():
        code = code.strip()
        # Robustness for "improve existing in workspace dir": if the provided seed_code is suspiciously short
        # (e.g. a placeholder instruction instead of full content), and a reasonable file already exists
        # in the target workdir, prefer reading the existing file so we don't nuke a working version.
        if len(code) < 200 and target_dir and Path(existing_output_path).exists():
            try:
                existing = Path(existing_output_path).read_text(encoding="utf-8")
                if len(existing) > 1000 and ("<!DOCTYPE" in existing or "<html" in existing.lower() or "<script" in existing.lower()):
                    code = existing
                    ctx["log"]("code_forge using existing file on disk instead of short seed placeholder")
            except Exception:
                pass
        if not verify_only:
            ctx["log"]("code_forge starting from provided seed_code")
    else:
        # recall relevant past lessons and feed them forward — the flywheel:
        # forge -> fail -> repair -> log -> RECALL -> forge better.
        lessons_text = ""
        try:
            from kraken.kernel import recall
            hits = recall.code_lessons(ctx["root"], task, k=3)
            lessons_text = recall.format_code_lessons(hits)
            if hits:
                ctx["log"](f"recall: injected {len(hits)} past lesson(s)")
        except Exception as exc:
            ctx["log"](f"recall skipped: {exc}")
        # For polish/final/improve tasks on existing HTML games, preload the current on-disk
        # build into the *initial* prompt (using the "Repair/improve existing" path).
        # This makes large follow-up polishes much more reliable and faster than pure "Write from task",
        # reduces regressions, and lets the model surgically apply new requirements (drops, levels, etc.)
        # while preserving working systems. The previous pure-Ornith path on empty base was too slow/hard for big games.
        is_html_game = Path(filename).suffix.lower() in {".html", ".htm"}
        is_polish_task = any(k in task.lower() for k in ("polish", "final", "complete", "verify", "improve", "perfect", "rebuild", "finish"))
        initial_prompt_code = None
        if is_html_game and target_dir and preload_existing and Path(existing_output_path).exists():
            # Preload full current on-disk HTML for ANY target_dir game (polish or finish tasks).
            # Critical improvement: gives the model (qwen2.5 primary + Ornith coupled) full context + working base so it can surgically fix
            # (instead of regenerating everything and timing out or regressing). Matches introspect backlog.
            try:
                disk = Path(existing_output_path).read_text(encoding="utf-8", errors="ignore")
                looks_ok = _looks_like_complete_html_game(disk) or (len(disk) > 5000 and ("<canvas" in disk.lower() or "gameLoop" in disk or "requestAnimationFrame" in disk.lower()))
                if looks_ok:
                    _, marker, trailing = disk.lower().rpartition("</html>")
                    poisoned = bool(marker and trailing.strip() and len(trailing) > 200)
                    if not (beast_mode and poisoned):
                        initial_prompt_code = disk
                        ctx["log"]("code_forge preloading current on-disk game as base for targeted polish/repair")
            except Exception:
                pass
        elif is_html_game and target_dir and not preload_existing:
            ctx["log"]("code_forge clean rebuild requested; existing artifact not preloaded")

        # Coupled models: qwen2.5:7b (stronger general reasoning) as primary for initial;
        # Ornith as specialist for repairs. Default prefers qwen2.5 to assist the game work.
        ctx["log"](f"code_forge asking {initial_model} (primary) for initial module")
        prompt_for_initial = (
            _prompt(task, filename, code=initial_prompt_code, lessons=lessons_text,
                    profile=declared_profile, contract_version=contract_version)
            if initial_prompt_code else
            _prompt(task, filename, lessons=lessons_text,
                    profile=declared_profile, contract_version=contract_version)
        )
        code, error = _generate_code(
            ctx,
            prompt_for_initial,
            system="You are a silent coding engine. Output only code.",
            timeout=_timeout_for_artifact(filename),
            temperature=0.25,
            model=initial_model,
        )
        if error:
            _append_lesson(ctx, job, task, 0, [error], False, output_path)
            return {"ok": False, "output": output_rel, "notes": error}

        # Run normalizer immediately on the raw LLM output for HTML games.
        if Path(filename).suffix.lower() in {".html", ".htm"}:
            try:
                code = _normalize_html_game(code, declared_profile, contract_version, task=task or "")
            except Exception as e:
                ctx["log"](f"code_forge normalize failed (non-fatal, keeping raw): {e}")

        # A rich HTML request can consume most of the whole job budget. Do not
        # immediately issue another blind full-file generation. Preserve the
        # known base and spend the one repair call on concrete verifier output.
        is_html_game = Path(filename).suffix.lower() in {".html", ".htm"}
        if is_html_game and not _looks_like_complete_html_game(code or ""):
            ctx["log"]("code_forge initial response incomplete; preserving base for verifier-guided repair")
            code = initial_prompt_code if initial_prompt_code else _get_game_skeleton()

    last_verdict = {"passed": False, "reasons": ["not verified"]}
    failure_reasons: list[str] = []
    last_good_code = code if _is_python_artifact(filename) else None
    for attempt in range(MAX_REPAIR_ATTEMPTS + 1):
        candidate = code
        is_html_game = Path(filename).suffix.lower() in {".html", ".htm"}

        # Always run the deterministic normalizer on HTML games first.
        # This fixes the most common structural bugs (DOM order, RAF, visual draw, entity promotion)
        # without another expensive model call.
        if is_html_game:
            try:
                candidate = _normalize_html_game(candidate, declared_profile, contract_version, task=task or "")
                if candidate != code:
                    ctx["log"]("code_forge applied deterministic structural normalizer")
            except Exception as e:
                ctx["log"](f"code_forge normalize failed (non-fatal): {e}")
                candidate = code

        if is_html_game and not _looks_like_complete_html_game(candidate):
            # Write the best available candidate (or fallback skeleton) to disk so the verifier
            # always has a file to inspect and returns *targetable* failure reasons rather than
            # the terminal "no output file specified" block that prevents any repair.
            fallback = candidate if candidate and len(candidate) > 500 else _get_game_skeleton()
            # Prefer the on-disk version if it's more complete than what we generated.
            if preload_existing:
                try:
                    recovery_path = output_path if Path(output_path).exists() else existing_output_path
                    if Path(recovery_path).exists():
                        on_disk = Path(recovery_path).read_text(encoding="utf-8")
                        if _looks_like_complete_html_game(on_disk) or len(on_disk) > len(fallback) * 2:
                            fallback = on_disk
                            code = on_disk
                            ctx["log"]("code_forge preserved complete disk base; semantic fixes require verifier-guided repair")
                except Exception as exc:
                    ctx["log"](f"code_forge recovery read failed: {exc}")
            failure_reasons.append(
                "Generated output was not a complete self-contained HTML game (fragment, too small, or missing structure). "
                "Must return the FULL raw <!DOCTYPE html> ... </html> document."
            )
            ctx["log"]("code_forge wrote fallback to disk so verifier can produce targetable reasons")
            candidate = fallback

        with open(output_path, "w", encoding="utf-8", newline="\n") as fh:
            fh.write(candidate.rstrip() + "\n")

        last_verdict = _verify(ctx["root"], job, ctx, output_rel, test_rel)
        if last_verdict.get("passed"):
            if attempt:
                _append_lesson(ctx, job, task, attempt, failure_reasons,
                               True, output_path)
            final_output_rel = output_rel
            if publish_dir:
                os.makedirs(publish_dir, exist_ok=True)
                shutil.copy2(output_path, published_output_path)
                if test_path:
                    shutil.copy2(test_path, os.path.join(publish_dir, os.path.basename(test_path)))
                try:
                    final_output_rel = os.path.relpath(published_output_path, ctx["root"])
                except ValueError:
                    final_output_rel = published_output_path
                ctx["log"](f"code_forge published verified artifact: {published_output_path}")
            return {
                "ok": True,
                "output": final_output_rel,
                "tests": [test_rel] if test_rel else [],
                "notes": f"code_forge passed after {attempt} repair attempt(s)",
            }

        if attempt >= MAX_REPAIR_ATTEMPTS:
            break

        failure_reasons.extend(str(reason) for reason in last_verdict.get("reasons", []))
        ctx["log"](f"code_forge repair {attempt + 1}: {last_verdict.get('reasons')}")

        repair_code_for_prompt = code
        if is_html_game and Path(output_path).exists():
            try:
                disk = Path(output_path).read_text(encoding="utf-8", errors="ignore")
                if not _looks_like_complete_html_game(disk) or len(disk) < 4000:
                    repair_code_for_prompt = f"[CURRENT FILE ON DISK IS BROKEN/FRAGMENT/INCOMPLETE - SIZE {len(disk)} - MUST USE THIS SKELETON + FULL TASK TO GENERATE A COMPLETE VALID GAME. DO NOT OUTPUT FRAGMENT OR MISSING SCRIPT.]\n\n{_get_game_skeleton()}\n\n{task}"
                elif 'final' in task.lower() or 'complete' in task.lower() or 'polish' in task.lower() or 'rebuild' in task.lower():
                    repair_code_for_prompt = (
                        "[SURGICAL FULL-FILE REPAIR: The complete current game follows. Preserve every "
                        "working system and fix only the verifier failures. Do not replace it with the "
                        "minimal skeleton, truncate it, or discard levels/audio/animation.]\n\n" + disk
                    )
            except Exception:
                pass

        # If the failures are the recurring UI ones, BE BRUTAL — the model keeps ignoring this
        reasons_lower = [str(r).lower() for r in last_verdict.get("reasons", [])]
        ui_issues = [r for r in last_verdict.get("reasons", []) if "inline" in str(r).lower() or "tabindex" in str(r).lower() or "duplicate" in str(r).lower() or "id" in str(r).lower() or "canvas width" in str(r).lower() or "no inline javascript" in str(r).lower()]
        if ui_issues or any("no inline javascript" in r for r in reasons_lower) or any("canvas" in r and "width" in r for r in reasons_lower):
            specific = (
                "YOU HAVE FAILED THESE EXACT RULES REPEATEDLY ON CAPTAIN COMIC / PLATFORMER REPAIRS. THIS IS THE LAST CHANCE — DO NOT RE-INTRODUCE ANY OF THEM:\n"
                "1. NO inline onclick=, onmousedown=, .onclick= etc for ANY game function (reset, play, start, restart). Search WHOLE file and remove them.\n"
                "2. Buttons MUST use ONLY addEventListener('click', fn). Example: const btn = document.getElementById('playBtn'); btn.addEventListener('click', () => { ... });\n"
                "3. UNIQUE IDs: playBtn or startBtn for main play. For restart/level/next use DIFFERENT ids e.g. 'nextLevelBtn', 'tryAgainBtn', 'restartLevelBtn'. NEVER duplicate id='restartBtn'. NEVER put restartBtn in innerHTML.\n"
                "4. Canvas tag MUST have tabindex=\"0\". In JS immediately: canvas.width=800; canvas.height=600; (literals, lowercase canvas.)\n"
                "5. The output MUST be a FULL <!DOCTYPE html> ... </html> with ONE LARGE <script>...</script> block containing ALL game code (gameLoop, state, listeners etc). If verifier says 'no inline JavaScript found' your <script> was missing or empty — FIX BY INCLUDING THE FULL SCRIPT.\n"
                "6. FIX BUTTONS + CANVAS + SCRIPT PRESENCE FIRST by searching the ENTIRE current code, then add features. Never regress.\n"
                "Use the skeleton provided as the structural base. Output ONLY the complete raw HTML file."
            )
            repair_code_for_prompt = specific + "\n\n" + repair_code_for_prompt

        repaired, error = _generate_code(
            ctx,
            _prompt(task, filename, repair_code_for_prompt, last_verdict.get("reasons", []),
                    profile=declared_profile, contract_version=contract_version),
            system="You are a silent code repair engine. Output ONLY the complete raw corrected HTML/JS file with zero commentary. For browser games: ALWAYS full <!DOCTYPE to </html>, ONE complete <script> block with gameLoop + addEventListeners, canvas.width/height literals + tabindex, unique button IDs only (never duplicate restartBtn), no inline event handlers ever. Fix the listed verifier failures (especially no inline JS, canvas size, dups, inline handlers) as absolute highest priority by searching the whole file first. Use the skeleton if provided. Never output fragments or re-introduce bugs.",
            timeout=_timeout_for_artifact(filename),
            temperature=0.15,
            model=repair_model,
        )
        if error:
            failure_reasons.append(error)
            _append_lesson(ctx, job, task, attempt + 1, failure_reasons,
                           False, output_path)
            return {"ok": False, "output": output_rel,
                    "tests": [test_rel] if test_rel else [], "notes": error}
        code = repaired

    failure_reasons.extend(str(reason) for reason in last_verdict.get("reasons", []))
    _append_lesson(ctx, job, task, MAX_REPAIR_ATTEMPTS, failure_reasons,
                   False, output_path)
    return {
        "ok": False,
        "output": output_rel,
        "tests": [test_rel] if test_rel else [],
        "notes": "code_forge failed verification: " + "; ".join(last_verdict.get("reasons", [])),
    }
