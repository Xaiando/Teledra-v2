"""kernel/game_prompts.py — canonical, maintainable game agent doctrine.

Single source of truth for what the code_forge + hub translator teach the Ornith
coding agents about browser games. This replaces scattered monolithic strings,
makes genre knowledge testable and easy to evolve, and gives the swarm a
"memory" of what makes a Captain Comic-style classic platformer (or any genre)
actually playable.

Grok (reviewer) maintains this file only. Workers/agents use it when forging.
Never write to workspace/games/* from here.
"""

from __future__ import annotations

import re
from typing import Optional

# Core shared requirements for every polished browser game artifact.
BASE_GAME_REQUIREMENTS = (
    "Build as a single self-contained index.html (<!DOCTYPE html> or <html>). "
    "Embedded <style> + <script> only. No external assets, no CDN, no src= http. "
    "Canvas + requestAnimationFrame(gameLoop(timestamp)) that updates state and redraws. "
    "Keyboard + pointer/touch input. Start screen with Play that hides overlay and starts "
    "the loop. Restart on win/lose. Score/progress + HUD. Win and lose states. "
    "Web Audio must include a synthesized looping music bed, at least five distinct action SFX, "
    "a visible mute toggle, and AudioContext resume from the Play gesture. "
    "Animation must communicate state: idle/run/jump/hurt player poses, animated enemies, "
    "collectible motion, particles, and at least two moving background/parallax layers. "
    "Wire controls with addEventListener ONLY — NEVER inline onclick= or onmousedown=. "
    "Use unique button IDs. Canvas MUST have tabindex='0' AND canvas.width = 800; canvas.height = 600; (lowercase 'canvas.', literal) in JS right after getElementById. Also width/height attrs on <canvas>. "
    "First seconds after Play must be playable: spawn at least one enemy/target immediately or "
    "guard wave-clear with a real spawned-count budget. No instant terminal state on start. "
    "Mentally playtest the full experience for fun factor, responsiveness, and no frustrating bugs before finalizing. "
    "Return raw file bytes only — never markdown fences or commentary."
)

# Genre-specific deep guidance. These are injected into the initial and repair prompts.
# Keep them terse but authoritative — the model must internalize the invariants.
GENRE_GUIDANCE: dict[str, str] = {
    "platformer": (
        "PLATFORMER (Captain Comic / classic 80s side-scroller style — AIM FOR AMAZING QUALITY): "
        "Side-view running/jumping. Gravity applied every frame (vy += GRAV; y += vy). "
        "Grounded jump ONLY (set vy = -JUMP only when on ground; allow brief coyote time ~120ms and jump buffer). "
        "AABB or tile collision with solid platforms — player must not fall through or get stuck in walls; "
        "resolve by pushing out to nearest edge. Variable jump height (shorter button press = shorter hop). "
        "Horizontal camera that follows player with lookahead (smooth scroll or clamped). "
        "MUST generate AT LEAST 5 distinct levels (use a levels[] array + loadLevel(n) that swaps platforms/enemies). Start at level 0, advance on exit/flag. "
        "Increasing difficulty across levels: more gaps, moving/crumbling platforms, faster enemies, hazards (spikes/lava/pits that cost life or reset). "
        "Collectibles (coins/gems) that increment a visible counter. "
        "Power-ups or ability gates (speed boots, higher jump, 'clone' item that duplicates a projectile or temporary decoy, "
        "or weapon upgrade). "
        "ENEMY DROPS & COLLECTION (critical — MUST WORK PERFECTLY): When an enemy is defeated there is a good chance (20-40%) it drops a powerup/item at its location. "
        "Dropped powerups must remain on screen for a reasonable time (8-12s) and MUST be collectible by normal player overlap/rect collision "
        "(do NOT gate behind boost/magnet/special mode only). Player contact with a dropped or placed powerup immediately collects it, "
        "plays satisfying sound, applies effect with visual feedback, and removes it. Powerups should not instantly fall off bottom or despawn before the player can reach them. "
        "Enemies that patrol or have simple predictable movement; collision with them costs life or brief stun. "
        "Exit/door/flag at end of level advances level or wins. 3-5 lives typical. "
        "No fall-through on any solid. Player sprite clearly grounded when standing. "
        "Precise feel: acceleration, friction, max fall speed, responsive controls. "
        "For amazing polish: smooth 60fps feel, satisfying powerup pickups with effects and particles, varied and fun level design, clear visual feedback for jumps/hits/collects/enemy deaths, responsive tight controls, progression that feels rewarding, no jank on edges or collisions, good sound design, score system, high score or completion sense."
    ),
    "shooter": (
        "VERTICAL/HORIZONTAL SHOOTER: player ship with momentum or direct control. "
        "Bullets travel in intended direction (upward vy negative for vertical shooters). "
        "Spawn budget: decrement when enemy actually appears; wave clears only after budget exhausted + zero alive. "
        "Screen wrap or bounded playfield. Power-ups with timer buffs (shield, rapid, speed) preferred over array post-check."
    ),
    "endless_runner": (
        "ENDLESS RUNNER: world auto-scrolls. Jump over ground hazards, duck aerial. "
        "Spawn obstacles ahead of camera. Speed ramps. No back-scrolling of world."
    ),
    "frogger": (
        "FROGGER CROSSING: discrete grid hops. Moving lanes (cars + river logs). "
        "Player must ride logs (inherit velocity). Drown off log. Goals at top."
    ),
    "snake": (
        "SNAKE: continuous grid movement in current dir. No 180 instant reverse. "
        "Food on empty only. Tail grows. Wall/self = end."
    ),
    "match3": (
        "MATCH-3: swap only adjacent if creates match (else revert). Gravity refill from top. "
        "Cascades + multiplier. Reshuffle when no moves. Move-limited levels."
    ),
    "tower_defense": (
        "TOWER DEFENSE: build grid + path waypoints. Credit economy. "
        "Wave spawns follow path. Leak/lives system. Clear only after spawn budget + zero enemies."
    ),
    "rhythm": (
        "RHYTHM: note chart with real timestamps. Lanes scroll to hit line. "
        "Perfect/good/miss windows. Web Audio metronome/sequence. Combo. Results rank."
    ),
    "pinball": (
        "PINBALL: circle ball physics + impulse on flipper/bumper collisions. "
        "Drain loses ball. Plunger launch. Table walls contain."
    ),
    "roguelike": (
        "TURN-BASED ROGUELIKE: grid, one tile per key. Bump attacks. "
        "Enemy turns after player. Fog of war. Stairs to next floor. Potions on pickup."
    ),
}

# Extra focused checklist appended for any platformer or "captain comic" request.
CAPTAIN_COMIC_STYLE_CHECKLIST = (
    "CLASSIC PLATFORMER INVARIANTS (Captain Comic 1988 feel):\n"
    "- Scrolling level or multi-room progression with clear advance.\n"
    "- Collect 8-12+ items total across types (score + optional power).\n"
    "- At least one 'special' powerup (temporary flight, clone shot, speed, shield).\n"
    "- Hazards that are deadly but readable (spikes visible, pits obvious).\n"
    "- Enemy variety: at least 2-3 types with different behavior.\n"
    "- Lives + simple HUD (score, lives, current power).\n"
    "- Death resets to safe checkpoint or level start with brief invuln.\n"
    "- No instant-kill on first frame; no stuck states on geometry.\n"
    "- Responsive jump (coyote + input buffer) + controllable air steering.\n"
)

BEAST_GAME_CONTRACT = (
    "LEGACY BEAST v1 (platformer-only, for Captain Comic migration period only):\n"
    "- The headless browser will click Play, hold Right, press Space/Up, invoke real damage, "
    "and force real exit collisions. Any uncaught JS error is an automatic failure.\n"
    "- Use edge-triggered jump input (a justPressed flag), not a held key that retriggers every frame.\n"
    "- Only when location.search contains 'krakenTest', expose window.__KRAKEN_BEAST__ with exactly "
    "snapshot(), damage(), and completeLevel() so the game is runtime-tested for v1 compatibility.\n"
    "Prefer profile-aware v2 via game_profiles + __KRAKEN_BEAST__.profile for new work."
)

def detect_genre(task: str, filename: str = "") -> str:
    """Lightweight heuristic to pick the best guidance block for a task."""
    t = (task or "").lower() + " " + (filename or "").lower()
    if any(k in t for k in ("platform", "jump", "side scroll", "side-scroller", "captain comic", "captaincomic", "vault runner")):
        return "platformer"
    if any(k in t for k in ("shoot", "bullet", "asteroid", "space", "invader", "shooter")):
        return "shooter"
    if "runner" in t or "endless" in t or "dash" in t:
        return "endless_runner"
    if "frog" in t or "cross" in t or "lane leap" in t:
        return "frogger"
    if "snake" in t or "serpent" in t:
        return "snake"
    if "match" in t or "gem" in t or "cascade" in t or "puzzle swap" in t:
        return "match3"
    if "tower" in t or "defense" in t or "pulse defense" in t:
        return "tower_defense"
    if "rhythm" in t or "beat" in t or "note" in t:
        return "rhythm"
    if "pinball" in t:
        return "pinball"
    if "rogue" in t or "dungeon" in t or "turn-based" in t:
        return "roguelike"
    if "<canvas" in t or "game" in t:
        return "shooter"  # default action bias for unknown canvas games
    return ""

def get_guidance(filename: str, task: str = "", rich_artifact: bool = True, profile: str = None) -> str:
    """Return the composed instruction block for this artifact + task.
    profile: if provided (from trusted job payload or .kraken-game.json), enables v2 profile-specific contract.
    """
    if not rich_artifact:
        return ""
    g = []
    g.append(BASE_GAME_REQUIREMENTS)

    # v2 profile-aware guidance (preferred). Falls back for migration.
    try:
        from kraken.kernel import game_profiles
        if profile:
            prof = game_profiles.get_profile(profile)
            if prof:
                g.append(f"BEAST CONTRACT v2 — profile={profile} (session: {prof.terminal_semantics}): {prof.description}")
                g.append("Driver must support actions: " + ", ".join(prof.action_names))
                g.append("snapshot() must include: " + ", ".join(prof.required_snapshot_paths))
            g.append("Universal (all profiles): " + "; ".join(game_profiles.UNIVERSAL_REQUIREMENTS))
        else:
            g.append(BEAST_GAME_CONTRACT)
    except Exception:
        g.append(BEAST_GAME_CONTRACT)  # legacy fallback
    genre = detect_genre(task, filename)
    if genre and genre in GENRE_GUIDANCE:
        g.append(GENRE_GUIDANCE[genre])
    if profile == "platformer" or genre == "platformer" or "captain" in (task or "").lower() or "platform" in (task or "").lower():
        g.append(CAPTAIN_COMIC_STYLE_CHECKLIST)
    # Always remind about safe collection and no shorthand bugs
    g.append(
        "GENERAL GAME HYGIENE: powerups/buffs use timers not post-splice array checks; "
        "use reverse loops or new arrays for collection/splice; never use undefined shorthand "
        "(ex,ey not x,y); never redeclare const/let in same block; preserve full gameLoop, "
        "input, and canvas on every repair."
    )
    if "research" in (task or "").lower() or "classic" in (task or "").lower() or "1988" in (task or "").lower() or "captain comic" in (task or "").lower():
        g.append(
            "RESEARCH FALLBACK FOR CLASSIC/OBSCURE GAMES: If local or web sources returned little or irrelevant information "
            "(tangential dictionary results, no direct hits), rely on the well-known mechanics of the named classic game + "
            "solid 80s/90s platformer conventions. Build a fun, playable, faithful-feeling version anyway. Prioritize "
            "working core loop (move, jump, collect, avoid/fight, progress) over perfect historical accuracy when sources fail."
        )
    if "improve" in (task or "").lower() or "fix" in (task or "").lower() or "existing" in (task or "").lower() or "seed_code" in (task or "").lower():
        g.append(
            "IMPROVING AN EXISTING LARGE GAME FILE (very important): The task is to improve a complete existing game. "
            "Start from the provided Current code / seed (it may be the full previous file or a description). "
            "ALWAYS output the COMPLETE full raw HTML file (doctype to last tag). Do not echo the task text, do not output short instructions, "
            "do not truncate. Preserve all working systems (physics, levels, camera, input, sounds, HUD) and surgically add/fix only the requested features "
            "(e.g. enemy drops that spawn collectible powerups at death location, reliable normal collision collection for every powerup/collectible). "
            "If the starting 'code' is invalid or placeholder text, treat the Task description as the spec and emit a full correct game. "
            "Never return a hollow or 100-byte file."
        )

    # Launchability requirements — this has been a repeated failure mode
    g.append(
        "LAUNCHABILITY (non-negotiable for any browser game — YOU KEEP FAILING THESE EXACT RULES. DO NOT RE-INTRODUCE): The HTML must be immediately runnable when opened in a browser (file:// or local server). "
        "CRITICAL — STOP REGRESSING ON BUTTONS AND SCRIPT PRESENCE:\n"
        "- NEVER EVER use inline onclick=, onmousedown=, .onclick=, or ANY inline event handler attribute for game functions (resetGame, startGame, play, restart etc.). They break in IIFEs/closures.\n"
        "- ALWAYS wire with .addEventListener('click', function() { ... }) or arrow. Search the ENTIRE FILE for any on* = and replace.\n"
        "- UNIQUE IDs ONLY: Use 'playBtn' or 'startBtn' for the main Play button. For restart/next/try-again use a *different* ID every time (e.g. 'restartLevelBtn', 'nextLevelBtn', 'tryAgainBtn'). NEVER duplicate id='restartBtn'. NEVER use 'restartBtn' at all if it risks dup.\n"
        "- NEVER put <button id=\"restartBtn\"> inside innerHTML. Clear container or use createElement + fresh unique ID.\n"
        "- Canvas MUST have tabindex=\"0\" attribute on the <canvas> tag AND immediately after getElementById: canvas.width=800; canvas.height=600; (lowercase, literal assignments).\n"
        "- The COMPLETE output MUST contain <script> ... </script> with the FULL game code inside (gameLoop, all state, listeners, update/draw). If 'no inline JavaScript found' appears, your <script> tag or its content was missing/empty — NEVER output without a substantial script block.\n"
        "- Play button must hide overlay, set gameState='playing', focus canvas.\n"
        "- NEVER put ${foo} template literals or JS expressions directly into the HTML body or static strings in markup. Output must be pure static HTML; dynamics only via JS .textContent after load.\n"
        "On EVERY repair or polish task, FIRST search the entire current code (use full file view) for ALL button definitions, onclicks, canvas tags, and <script> presence. Fix buttons + canvas + ensure full script FIRST before any feature work. Prioritize this above all."
    )
    g.append(
        "STARTUP & RENDERING DISCIPLINE (PREVENT GREY SCREEN + ONLY HUD + BROKEN LIVES + SCORE REPLACING HEARTS): "
        "MENTALLY PLAYTEST THE FIRST 3 SECONDS AFTER CLICKING PLAY: "
        "1. Click must hide the start overlay completely. "
        "2. gameState = 'playing'. "
        "3. DOM refs (canvas, ctx, overlay, hudEl or scoreBox/livesBox, level) obtained ONCE at top of IIFE. "
        "4. draw() MUST: ALWAYS fill dark bg first, THEN if NOT (playing or won) return early. But WHEN playing: ALWAYS draw bright visible platforms (e.g. fillStyle='#4ecca3' or '#4ecca3' highlight), player sprite, live enemies, coins/drops immediately. World MUST appear, not grey canvas + only UI. "
        "5. gameLoop MUST ALWAYS continue the RAF chain: update(); draw(); requestAnimationFrame(gameLoop);  -- DO NOT put the requestAnimationFrame inside 'if (gameState === \"playing\")'. Gate only the update/draw logic or early return inside functions. "
        "6. startGame (and any restart/next handlers) MUST kick the loop after setting state: gameState='playing'; ... loadLevel(...); updateHUD(); requestAnimationFrame(gameLoop);  -- otherwise the animation chain dies and screen stays grey forever. "
        "7. HUD: use textContent on dedicated elements (scoreBox, livesBox). updateHUD() must do: scoreBox.textContent = 'SCORE: ' + score; livesBox.textContent = 'HP: ' + '❤'.repeat(lives);  -- the lives/HP display MUST contain repeated heart chars and MUST NEVER contain the word 'score'. Call updateHUD() AFTER every score += N, lives--, lives=3 etc (in collect, die, powerup, start, level advance). "
        "8. Dynamic only in JS after load. No ${} anywhere in HTML source or static innerHTML strings. If grey + 'score' word where hearts should be, the livesBox was assigned score text or updateHUD was skipped or loop dead. Fix by searching whole file for gameLoop, startGame, updateHUD, lives, score assignments."
    )
    g.append(
        "REPAIR DISCIPLINE FOR GAMES (YOU KEEP BREAKING THIS — BE ABSOLUTELY RIGID): On every repair attempt for HTML games, your output must be a COMPLETE, valid, self-contained HTML document from the very first line (<!DOCTYPE html> ... full </html>). "
        "If previous produced fragment, JS snippet, tiny file, missing <script>, or syntax error, COMPLETELY IGNORE the broken content and emit a FRESH full correct game using the provided skeleton as base. "
        "Never output anything that is not the full playable HTML. "
        "ALWAYS search the WHOLE file for button code, inline handlers, duplicate ids, canvas attrs, and <script> blocks — fix to ONLY addEventListener + unique IDs + tabindex + full script + literal canvas sizes BEFORE doing feature work or polish. "
        "NEVER recreate restartBtn via innerHTML or duplicate IDs. If the verifier complains about inline handlers, duplicate IDs, missing tabindex, canvas size, or 'no inline JavaScript found' — fix EXACTLY those FIRST using the exact patterns. Do not re-introduce the bugs even once."
    )
    g.append(
        "GREY SCREEN / HUD-ONLY FAILURE MODE (YOU PRODUCED THIS — ALSO 'HP HEARTS GONE REPLACED BY SCORE'): When operator reports grey screen apart from UI + hearts replaced by the word 'score': "
        "- gameLoop only does requestAnimationFrame inside if(playing) at bottom — chain never starts/restarts after startGame. "
        "- startGame sets playing + load + updateHUD but does NOT do requestAnimationFrame(gameLoop) to kick the loop. "
        "- updateHUD not called after score/lives changes so stale or wrong display. "
        "- livesBox or HP element textContent assigned score value or contains 'SCORE' text by copy-paste. "
        "- draw early-returns before painting bright platforms/player. "
        "FIX: 1) Make gameLoop ALWAYS schedule next RAF unconditionally at end (update(); draw(); requestAnimationFrame(gameLoop); ). 2) In EVERY start/restart handler: after gameState='playing' and load/updateHUD, add requestAnimationFrame(gameLoop); 3) Call updateHUD() right after score += , lives-- etc. 4) livesBox.textContent = 'HP: ' + '❤'.repeat(lives); scoreBox separate 'SCORE: ' + score. 5) Ensure draw paints visible bright platforms + player right after state change. Search ENTIRE file for these patterns before editing."
    )
    return "\n\n".join(g)

def is_platformer_task(task: str) -> bool:
    return bool(detect_genre(task) == "platformer" or "platform" in (task or "").lower() or "captain comic" in (task or "").lower())

def platformer_test_additions() -> str:
    """Extra deterministic assertions that can be appended to test_index.py for platformer missions."""
    return (
        "\n# Platformer-specific (injected by agent training)\n"
        "import re\n"
        "assert 'gravity' in html or 'vy' in html or 'vel' in html\n"
        "assert 'ground' in html or 'onfloor' in html or 'onground' in html or 'platform' in html\n"
        "assert 'jump' in html\n"
        "assert 'coyote' in html or 'buffer' in html or 'grounded' in html\n"
        "assert 'tabindex' in html\n"
        "assert not re.search(r'onclick=.*(reset|play|start|game)', html)\n"
        "ids = re.findall(r'id=[\"\\']([^\"\\']+)[\"\\']', html, re.I)\n"
        "assert ids.count('restartbtn') <= 1, 'duplicate restart button id'\n"
        "assert 'addeventlistener' in html\n"
        "assert 'canvas.width' in html or 'width=\"800\"' in html or \"width='800'\" in html\n"
        "if 'powerup' in html or 'drop' in html or 'collect' in html:\n"
        "    assert 'overlap' in html or 'rect' in html or 'collid' in html\n"
        "assert 'const canvas' in html or 'let canvas' in html or 'const ctx' in html\n"
        "assert 'liveenemies' in html or 'enemies =' in html\n"
        "assert 'requestanimationframe(gameloop)' in html\n"
        "assert 'updatehud' in html\n"
        "assert 'livesbox' in html or 'livesdisplay' in html or 'hp:' in html\n"
        "assert '__kraken_beast__' in html\n"
        "assert 'music' in html and ('mute' in html or 'soundtoggle' in html)\n"
        "assert 'anim' in html or 'sprite' in html\n"
    )

def get_seed_scaffold_for_clone(task: str) -> Optional[str]:
    """When the mission is explicitly a 'clone' or Captain Comic style, return a compact
    structural hint (not a full file — the agent still writes the real code).
    This helps the swarm 'clone' the feel without us touching any existing game files.
    """
    if not is_platformer_task(task) and "clone" not in (task or "").lower():
        return None
    return (
        "STRUCTURAL SEED HINT FOR CLASSIC PLATFORMER CLONE:\n"
        "Use a player = {x,y,vx,vy, w,h} with per-frame gravity, grounded flag updated by collision.\n"
        "Platforms as array of {x,y,w,h} or simple tile grid.\n"
        "AABB overlap test + resolve (separate X then Y pass recommended).\n"
        "Camera = {x} ; draw offset = -camera.x ; update camera to player.x + lookahead.\n"
        "Collectibles as list; on overlap increment score + remove or mark collected.\n"
        "Simple patrol enemies: x += dir*speed; flip dir at edges.\n"
        "Level data either hardcoded arrays for level 1+2 or a generator.\n"
        "Keep everything in one file; aim for crisp 60 fps feel even on modest hardware."
    )

__all__ = [
    "get_guidance", "detect_genre", "is_platformer_task",
    "platformer_test_additions", "get_seed_scaffold_for_clone",
    "GENRE_GUIDANCE", "BASE_GAME_REQUIREMENTS", "CAPTAIN_COMIC_STYLE_CHECKLIST",
    "BEAST_GAME_CONTRACT",
]
