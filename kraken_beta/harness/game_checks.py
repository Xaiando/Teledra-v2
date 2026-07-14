"""harness/game_checks.py — canonical, reusable static playability and genre checks.

Extracted from verify_code.py growth and the standalone audit_playability.py.
Goal: keep verify_code.py from becoming a 1k+ line spaghetti monster while
giving every training loop (code_forge, supervisor jobs, audit scripts) the
same high-quality rules.

Grok maintains these rules. Agents (workers) benefit when they forge games.
These are pure functions — no side effects, easy to unit test.
"""

from __future__ import annotations

import re
from typing import Iterable, List


def normalize_low(text: str) -> str:
    return (text or "").lower().replace("_", "").replace("-", "")


def has_duplicate_ids(html: str) -> List[str]:
    ids = re.findall(r'id=["\']([^"\']+)["\']', html, re.I)
    dupes = [i for i in ids if ids.count(i) > 1]
    if dupes:
        return [f"duplicate HTML ids break getElementById: {sorted(set(dupes))[:5]}"]
    return []


def lacks_tabindex_on_canvas(html: str, scripts: Iterable[str]) -> List[str]:
    low = html.lower()
    if "<canvas" not in low or "tabindex" in low:
        return []
    # Only a problem if the key listener is ON THE CANVAS. Games that listen on
    # window/document (the common case) work without canvas focus, so tabindex
    # is irrelevant — don't flag them.
    joined = "\n".join(scripts).lower()
    if re.search(r"(window|document)\s*\.\s*addeventlistener\s*\(\s*['\"]key", joined) or \
       re.search(r"\b(window|document)\.onkey", joined):
        return []
    if re.search(r"canvas\w*\s*\.\s*addeventlistener\s*\(\s*['\"]key", joined):
        return ["canvas lacks tabindex — keyboard listener is on the canvas but it can't focus"]
    return []


def canvas_size_not_set(html: str, scripts: Iterable[str]) -> List[str]:
    joined = "\n".join(scripts)
    low = html.lower()
    if "<canvas" not in low:
        return []
    if "canvas.width" in joined or re.search(r"<canvas[^>]*\bwidth\s*=", html, re.I):
        return []
    return ["canvas width/height not set in markup or JS (CSS-only sizing breaks hit coords)"]


def inline_handler_closure_problem(html: str, scripts: List[str]) -> List[str]:
    """Catch onclick=foo() when foo lives only inside an IIFE closure."""
    calls = re.findall(r"\bon\w+\s*=\s*['\"]\s*([A-Za-z_$][\w$]*)\s*\(", html)
    if not calls:
        return []
    joined = "\n".join(scripts)
    iife_wrapped = bool(re.search(r"\(\s*function\s*\([^)]*\)\s*\{", joined))
    reasons: List[str] = []
    for name in sorted(set(calls)):
        exported = re.search(rf"\b(?:window|globalThis)\s*\.\s*{re.escape(name)}\s*=", joined)
        declared = re.search(
            rf"\b(?:function\s+{re.escape(name)}\s*\(|(?:const|let|var)\s+{re.escape(name)}\s*=)",
            joined,
        )
        if not declared and not exported:
            reasons.append(
                f"inline handler calls {name}(), but no such function is defined; "
                "use addEventListener with a real function"
            )
        elif iife_wrapped and declared and not exported:
            reasons.append(
                f"inline handler calls {name}(), but the function is closure-local; "
                "use addEventListener or assign it to window"
            )
    return reasons


def play_calls_showstart(html: str, scripts: List[str]) -> List[str]:
    joined = "\n".join(scripts).lower()
    low = html.lower()
    if re.search(r"playbtn[^;]{0,120}\.addEventListener\s*\(\s*['\"]click['\"][\s\S]{0,400}showstartscreen\s*\(", joined, re.I):
        return ["play button click calls showStartScreen(); must hide start overlay and show HUD instead"]
    return []


def initgame_sets_running_while_overlay(html: str, scripts: List[str]) -> List[str]:
    joined = "\n".join(scripts)
    low = normalize_low(html)
    if re.search(r"initgame\s*\(\s*\)\s*;\s*requestanimationframe", joined, re.I | re.S):
        if "startscreen" in low:
            if re.search(r"gamestate\s*=\s*['\"]running['\"]", joined, re.I):
                return ["initGame() on load sets running while start overlay still visible; hide overlay or start in menu state"]
    return []


def empty_wave_instant_complete(scripts: List[str]) -> List[str]:
    joined = "\n".join(scripts).lower()
    compact = re.sub(r"\s+", "", joined)
    if "enemiesinwave" in compact and re.search(r"if\(\s*enemiesinwave={2,3}0&&enemiesalive={2,3}0\s*\)", compact):
        return ["wave-clear logic treats an empty initial wave as complete; spawn initial enemies or require enemiesInWave >= totalEnemiesPerWave"]
    return []


def downward_bullets(scripts: List[str]) -> List[str]:
    """Detect vy: speed in shootBullet for vertical shooters."""
    shoot = ""
    for s in scripts:
        m = re.search(r"\bfunction\s+shootBullet\b[^}]*", s, re.I | re.S)
        if m:
            shoot = m.group(0).lower()
            break
    if shoot and "bullets.push" in shoot and "vy:" in shoot:
        if re.search(r"vy\s*:\s*speed\b", shoot):
            return ["player shootBullet appears to fire downward with vy: speed; vertical shooters should use negative vy for upward shots"]
    return []


def closest_missing_playbtn(scripts: List[str], html: str) -> List[str]:
    joined = "\n".join(scripts)
    if re.search(r"closest\s*\(\s*['\"]#play-btn['\"]\s*\)", joined):
        if not re.search(r'id=["\']play-btn["\']', html, re.I):
            if re.search(r"filltext\s*\(\s*['\"]play", joined, re.I):
                return ["start PLAY is canvas-drawn but click handler expects missing #play-btn element; add HTML button or canvas hit-test"]
    return []


def basic_game_structure(html: str, scripts: List[str]) -> List[str]:
    reasons: List[str] = []
    low = html.lower()
    if html.lstrip().startswith("```"):
        reasons.append("HTML artifact starts with a markdown code fence; return raw file contents only")
    if "<html" not in low and "<!doctype" not in low:
        reasons.append("not an HTML document (no <html>/<!doctype>)")
    if "<script" not in low:
        reasons.append("no <script> — a game needs JavaScript")
    if not scripts:
        reasons.append("no inline JavaScript found")
    else:
        # Detect hollow or near-empty script blocks (common in bad repairs)
        script_content = "\n".join(scripts)
        if len(script_content.strip()) < 300:
            reasons.append("inline JavaScript found but too small/empty (game code missing)")
    # external assets
    ext = re.findall(r"""(?:src|href)\s*=\s*['"]\s*((?:https?:)?//[^'"]+)""", html, re.IGNORECASE)
    cdn = re.findall(r"https?://[^\s'\"<>]+", html)
    if ext or cdn:
        sample = (ext + cdn)[0][:80]
        reasons.append(f"external asset/URL found (must be self-contained): {sample}")
    return reasons


def _function_body(src: str, name: str) -> str:
    """Brace-matched body of `function name(...) { ... }`, or ''."""
    m = re.search(rf"function\s+{re.escape(name)}\s*\([^)]*\)\s*\{{", src)
    if not m:
        return ""
    i = m.end() - 1
    depth = 0
    for j in range(i, len(src)):
        if src[j] == "{":
            depth += 1
        elif src[j] == "}":
            depth -= 1
            if depth == 0:
                return src[i + 1:j]
    return src[i + 1:]


def dead_loop_conditional_raf(scripts: List[str]) -> List[str]:
    """The grey-screen killer: a game loop that only reschedules
    requestAnimationFrame inside a state conditional (e.g.
    `if (gameState === 'playing') requestAnimationFrame(gameLoop);`). The loop
    dies on its first non-'playing' frame; if no start/restart handler re-kicks
    it, clicking Play loads a level onto a dead loop -> blank canvas forever.

    Only flags when the reschedule is gated AND nothing re-kicks the loop, so a
    game that legitimately gates-and-restarts is not penalised.
    """
    joined = "\n".join(scripts)
    low = joined.lower()
    state_words = r"(gamestate|state|running|isrunning|playing|started|active|paused|gameover|alive|inplay)"
    reasons: List[str] = []
    seen = set()
    # capture ORIGINAL-case loop names (function lookup is case-sensitive)
    for name in set(re.findall(r"requestAnimationFrame\s*\(\s*(\w+)\s*\)", joined, re.I)):
        low_name = name.lower()
        if low_name in seen:
            continue
        seen.add(low_name)
        body = _function_body(joined, name)
        if not body:
            continue
        body_low = body.lower()
        # is this the loop? it drives the frame (update/draw/tick/render) AND reschedules itself
        if not re.search(r"\b(update|draw|render|tick|step)\s*\(", body_low):
            continue
        self_raf = list(re.finditer(rf"requestanimationframe\s*\(\s*{low_name}\s*\)", body_low))
        if not self_raf:
            continue
        # is EVERY self-reschedule inside a state conditional?
        gated = 0
        for m in self_raf:
            preceding = body_low[max(0, m.start() - 140):m.start()]
            if re.search(rf"if\s*\([^)]*{state_words}[^)]*\)\s*\{{?[^{{}}]*$", preceding):
                gated += 1
        if gated and gated == len(self_raf):
            # A real "revive" is a requestAnimationFrame(loop) that sits in the
            # SAME block as a play-state assignment (e.g. inside startGame). The
            # initial top-level load kick fires once before play and does NOT
            # count. Forward-scan from each play-state assignment, bounded by the
            # enclosing block's closing brace.
            revive = False
            for a in re.finditer(rf"{state_words}\s*=\s*['\"]?(playing|running|true|active|inplay)", body_low + low):
                pass  # placeholder to keep name defined if no matches
            haystack = low
            for a in re.finditer(rf"{state_words}\s*=\s*['\"]?(playing|running|true|active|inplay)", haystack):
                depth = 0
                for j in range(a.end(), min(len(haystack), a.end() + 800)):
                    ch = haystack[j]
                    if ch == "{":
                        depth += 1
                    elif ch == "}":
                        if depth == 0:
                            break  # left the enclosing block
                        depth -= 1
                    if haystack.startswith(f"requestanimationframe({low_name}", j) or \
                       re.match(rf"requestanimationframe\s*\(\s*{low_name}\b", haystack[j:j + 60]):
                        revive = True
                        break
                if revive:
                    break
            if not revive:
                reasons.append(
                    f"game loop '{name}' only reschedules requestAnimationFrame inside a "
                    f"state conditional and no start/restart handler re-kicks it after "
                    f"setting the play state — the loop dies and the canvas stays grey. "
                    f"Make the reschedule unconditional (update(); draw(); "
                    f"requestAnimationFrame({name});) and gate update/draw instead."
                )
    return reasons


def collect_all_static_issues(html: str, scripts: List[str]) -> List[str]:
    """One call to run the full suite of cheap static playability + structure checks."""
    reasons: List[str] = []
    reasons.extend(basic_game_structure(html, scripts))
    reasons.extend(inline_handler_closure_problem(html, scripts))
    reasons.extend(play_calls_showstart(html, scripts))
    reasons.extend(initgame_sets_running_while_overlay(html, scripts))
    reasons.extend(closest_missing_playbtn(scripts, html))
    reasons.extend(empty_wave_instant_complete(scripts))
    reasons.extend(downward_bullets(scripts))
    reasons.extend(has_duplicate_ids(html))
    reasons.extend(lacks_tabindex_on_canvas(html, scripts))
    reasons.extend(canvas_size_not_set(html, scripts))
    reasons.extend(dead_loop_conditional_raf(scripts))
    return reasons


# Platformer specific lightweight static hints (best-effort; full correctness needs runtime)
def platformer_smells(scripts: List[str], html: str) -> List[str]:
    joined = " ".join(scripts).lower()
    low = html.lower()
    out: List[str] = []
    if "platform" not in low and "gravity" not in joined and "vy" not in joined and "jump" not in joined:
        out.append("platformer mission but no obvious gravity/jump/platform terms in source")
    if "coyote" not in joined and "buffer" not in joined and "grounded" not in joined:
        out.append("platformer: consider coyote time + jump buffer for modern feel (recommended)")
    # Powerup / enemy drop collection (common breakage observed in swarm output — MUST BE PERFECT)
    if ("powerup" in joined or "collect" in joined or "pickup" in joined or "drop" in joined) and "enemy" in joined:
        if "drop" not in joined and "spawn" not in joined and "on death" not in joined and "ondeath" not in joined and "kill" not in joined:
            out.append("platformer mentions powerups + enemies but no obvious enemy drop / spawnPowerup on defeat logic — drops from enemies MUST exist")
        if ("rect" not in joined and "overlap" not in joined and "collide" not in joined) and ("player.x" not in joined or "player.y" not in joined):
            out.append("powerup collection logic may be missing or incomplete (player must collect drops by simple rect overlap / contact on normal play)")
    # More platformer quality
    if "platform" in low or "gravity" in joined or "jump" in joined:
        if "grounded" not in joined and "onground" not in joined and "onfloor" not in joined:
            out.append("platformer but no clear grounded/on ground check — may allow infinite jumping or bad collision")
        if "camera" not in joined and "scroll" not in joined and "offset" not in joined:
            out.append("platformer without obvious camera/scroll — may feel static or broken on wide levels")
        if "drop" not in joined and "dropped" not in joined and ("powerup" in joined or "collect" in joined):
            out.append("collectibles mentioned but no drop logic for enemies — powerups from kills must be implemented")
        # Grey screen root cause: static data iterated instead of live entities
        if ('level.enemies' in joined or re.search(r'for .* of level\.', joined)) and 'enemies = ' not in joined and 'liveenemies' not in joined:
            out.append("platformer iterates level.enemies raw data objects — must create live mutable enemy instances (with .alive) that update/draw actually use")
    # ctx bail in draw -> grey only UI
    if 'if (!ctx) return' in joined or 'if(!ctx) return' in joined:
        if 'const canvas = document.getelementbyid' not in joined or 'const ctx = canvas.getcontext' not in joined:
            out.append("draw has early 'if (!ctx) return' but no top-level canvas + ctx declaration — draw bails, grey screen, only HUD/UI visible")
    return out

def launchability_smells(scripts: List[str], html: str) -> List[str]:
    """Catch games that pass basic keyword checks but cannot actually be launched/started."""
    joined = "\n".join(scripts).lower()
    low = html.lower()
    out: List[str] = []

    # Has "press play" text but no actual interactive start mechanism
    if "press play" in low or "press start" in low or "play to start" in low:
        has_button = bool(re.search(r'<button|id=[\'"]?(play|start)', html, re.I))
        has_click_listener = "addeventlistener" in joined and ("click" in joined or "play" in joined or "start" in joined)
        has_key_start = bool(re.search(r'keydown|keyup|key.*(space|enter|play|start)', joined))
        if not (has_button or has_click_listener or has_key_start):
            out.append("menu says 'Press PLAY' or similar but there is no button element, click listener, or key handler that can start the game")

    # Game loop is called unconditionally at load time while still in menu
    if "requestanimationframe" in joined and "gameloop" in joined:
        if re.search(r'requestanimationframe\s*\(\s*game(loop|loop|update)', joined):
            if "gamestate" in joined and "menu" in joined:
                if not re.search(r'if\s*\(\s*gamestate\s*(?:===|!==)\s*[\'"]?playing', joined):
                    out.append("RAF gameLoop is started immediately but gameplay is not properly gated; menu state may block everything with no way to enter playing")

    # Common undefined var patterns on launch (jumpPressed etc referenced before any let/const in same scope)
    if "jumppressed" in joined and not re.search(r'(let|var|const)\s+jumppressed', joined):
        out.append("jumpPressed (or similar input flag) is used but may not be declared before gameLoop runs")

    # Button wiring hygiene (recurring failure in repairs)
    if re.search(r'onclick=[\'"].*?(reset|start|play|game)', low):
        out.append("inline onclick used for game control function — use addEventListener instead to avoid closure issues")

    ids = re.findall(r'id=["\']([^"\']+)["\']', html, re.I)
    if ids.count('playBtn') > 1 or ids.count('restartBtn') > 1 or ids.count('startBtn') > 1:
        out.append("duplicate button IDs (playBtn/restartBtn/startBtn) — use unique IDs for each button (e.g. nextLevelBtn for restart)")

    # tabindex only matters when the key listener is ON the canvas; window/
    # document listeners (the common case) work without it — defer to the
    # smarter lacks_tabindex_on_canvas() check to avoid false positives.
    out.extend(lacks_tabindex_on_canvas(html, [joined]))

    # Strong guard for hollow script on repairs
    joined_scripts = "\n".join(scripts)
    has_listener = "addeventlistener" in joined_scripts or "addeventlistener" in low
    if "<button" in low and not has_listener:
        out.append("buttons present but no addEventListener found in scripts — wire all controls with addEventListener only")

    if "<script" in low and len(joined_scripts.strip()) < 400:
        out.append("script tag present but game code is too small or empty")

    # NOTE: a "must use ❤ hearts" requirement was removed here — it is an
    # aesthetic preference, not a correctness bug. "LIVES: 3" as text is a
    # perfectly valid HUD (Breakout, arcade games). A verifier flags BUGS, not
    # a specific art style; forcing every game into Captain Comic's look wrongly
    # failed valid games and blocked completion.

    # Hearts replaced by score text (operator symptom: "HP hearts gone and replaced with the word score")
    if re.search(r'\b(?:livesbox|livesdisplay|livesel|liveshud)\s*\.\s*textcontent\s*=\s*[^;\n]*\bscore\b', joined, re.I):
        out.append("lives/HP display is being assigned score text or contains the word 'score' instead of hearts — fix livesBox.textContent = 'HP: ' + '❤'.repeat(lives) and keep scoreBox separate.")
    if ('❤' in html or '♥' in html) and re.search(r'HP:\s*[\'"]\s*\+?\s*score|textContent\s*=\s*[\'"].*HP:.*\$\{score', html, re.I):
        out.append("HP hearts slot contaminated with score expression or template — use dedicated lives var + repeat hearts.")
    # For Captain Comic / platformer v1: strongly prefer repeated heart symbols in lives display
    if re.search(r'captain|platformer', joined, re.I) and not re.search(r'❤.*repeat|\.repeat\(.*lives\)|hearts\s*\+=\s*[\'"]❤', joined, re.I):
        out.append("Captain Comic / platformer style: lives/HP should use '❤'.repeat(lives) pattern for visual hearts (user reported hearts replaced by 'score' text).")

    # No unparsed template literals in HTML source (causes grey + broken UI)
    if re.search(r'\$\{[^}]+\}', html) and '<script' in low:
        # Check if ${ appears outside <script> tags
        outside_scripts = re.sub(r'<script\b[^>]*>.*?</script>', '', html, flags=re.DOTALL | re.IGNORECASE)
        if re.search(r'\$\{[^}]+\}', outside_scripts):
            out.append("unparsed ${template} literals in HTML source (not inside <script>) — causes broken grey screen + only mangled UI visible. Use textContent in JS instead.")

    # === New: Grey screen / only HUD visible / broken start ===
    # Re-querying DOM inside hot paths often indicates fragile init
    if re.search(r'getElementById\(.canvas|querySelector.*canvas.*getContext', joined) and joined.count('getcontext') > 2:
        out.append("canvas ctx obtained repeatedly inside draw/update — cache once at init to avoid grey/blank frames on start")
    # Canvas size must be set in JS (matches test requirement)
    if '<canvas' in low and not (re.search(r'canvas\.width\s*=', joined) or re.search(r'<canvas[^>]+width=', html, re.I)):
        out.append("canvas.width/height not set in JS after getElementById (test requires 'canvas.width =' or <canvas width=) — causes sizing/coord bugs")
    # Catch duplicate canvas declarations (common cause of "Identifier 'canvas' has already been declared")
    # Match both assigned forms and bare multi-decls like "let canvas, ctx;"
    canvas_decls = re.findall(r'(?:const|let|var)\s+canvas\b', joined, re.I)
    if len(canvas_decls) > 1:
        out.append("Multiple canvas declarations found (const/let/var canvas ...) — declare exactly once early in IIFE using the exact ID from the <canvas> tag. When editing preloaded code, reuse the existing declaration; never add a second one.")

    # Using static level data directly as mutable entities (enemies lack .alive promotion)
    if 'level.enemies' in joined or 'for (let e of level' in joined:
        if 'enemies = []' not in joined and 'liveenemies' not in joined and 'runtimeenemies' not in joined:
            out.append("iterating level.enemies data directly without promoting to live objects with .alive — entities won't appear or update (grey screen + no content)")

    # Overlay / DOM ref used but never declared (ReferenceError on PLAY).
    # Match the ACTUAL identifier ending in 'overlay' — the old check matched
    # 'overlay.style' as a substring of 'startOverlay.style' and missed the
    # 'const startOverlay' declaration, firing on correct code.
    # Only BARE `overlay.style` (not a property access like cfg.overlay.style):
    # the (?<![.\w]) lookbehind excludes `.overlay` / `xoverlay`.
    _used_overlays = set(re.findall(r'(?<![.\w])(\w*overlay)\s*\.\s*(?:style|classlist)', joined))
    # declared as a variable OR as an object property (overlay: getElementById(...))
    _decl_overlays = set()
    for declaration in re.findall(r'\b(?:const|let|var)\s+([^;\n]+)', joined):
        _decl_overlays.update(re.findall(r'(?:^|,)\s*(\w*overlay)\b', declaration))
    _decl_overlays |= set(re.findall(r'(\w*overlay)\s*:', joined))          # object property
    _decl_overlays |= set(re.findall(r'\.\s*(\w*overlay)\s*=', joined))     # this.overlay = ...
    _undeclared = [u for u in _used_overlays if u not in _decl_overlays]
    if _undeclared:
        out.append(f"overlay DOM ref used but never declared ({_undeclared[:3]}) — ReferenceError on PLAY; declare via getElementById before use")

    # === Grey screen root: conditional RAF scheduling (prevents loop from running after startGame) ===
    if re.search(r'if\s*\(\s*gameState\s*===\s*[\'"]playing[\'"]\s*\)\s*\{\s*requestAnimationFrame', joined, re.I | re.S):
        out.append("gameLoop only schedules requestAnimationFrame inside if(playing) — after startGame the RAF chain is dead; canvas stays grey. ALWAYS schedule RAF unconditionally at end of gameLoop.")

    # Do not require start handlers to spell `requestAnimationFrame(gameLoop)`.
    # Valid games may delegate to a named starter or keep one frame chain alive.
    # The browser probe proves sustained RAF and visible pixel changes.

    # Conditional RAF is a top cause of "still grey screen apart from UI"
    if re.search(r'if\s*\(\s*gameState\s*===\s*[\'"]playing[\'"]\s*\)\s*\{\s*requestAnimationFrame', joined, re.I | re.S):
        out.append("gameLoop guards requestAnimationFrame behind if(playing) — the RAF chain dies after startGame. Make the rAF call unconditional at the end of gameLoop.")

    # HUD not refreshed after mutations (hearts become stale or show wrong value like initial score only)
    # NOTE: `joined` is lowercased above, so compare against a lowercase literal —
    # 'updateHUD' would never match and this fired on every game with a score.
    mutates_score = ('score += ' in joined or 'score+=' in joined or 'lives--' in joined
                     or 'lives -=' in joined or 'lives =' in joined)
    # HUD is "refreshed" if there's an updateHUD() OR inline DOM writes to a
    # score/lives/hud element (textContent/innerHTML). The old check only knew
    # about updateHUD and wrongly flagged games that update the HUD inline.
    # HUD counts as refreshed by ANY live mechanism: updateHUD(), inline DOM
    # writes (textContent/innerHTML), OR a canvas-drawn HUD (ctx.fillText that
    # renders score/lives every frame — common and perfectly valid).
    refreshes_hud = ('updatehud' in joined or
                     re.search(r"(score|lives|hud|hp)\w*\s*(\.\s*(textcontent|innerhtml)|\)\s*\.\s*(textcontent|innerhtml))", joined) or
                     re.search(r"getelementbyid\(\s*['\"][^'\"]*(score|lives|hud|hp)[^'\"]*['\"]\s*\)\s*\.\s*(textcontent|innerhtml)", joined) or
                     re.search(r"filltext\s*\([^)]*\b(score|lives|hp|wave)\b", joined) or
                     re.search(r"filltext\s*\(\s*[`'\"][^`'\"]*(score|lives|hp|wave)", joined))
    if mutates_score and not refreshes_hud:
        out.append("score or lives mutated but the HUD is never refreshed (no updateHUD() and no textContent/innerHTML write to a score/lives element) — HUD will show stale values.")

    return out
