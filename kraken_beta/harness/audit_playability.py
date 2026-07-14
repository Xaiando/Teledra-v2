"""Quick static playability audit for worker-built HTML games."""
from __future__ import annotations

import sys
from pathlib import Path

try:
    from . import game_checks
except Exception:
    game_checks = None

ROOT = Path(__file__).resolve().parents[1]
GAMES = ROOT / "workspace" / "games"


def audit(html: str) -> list[str]:
    if game_checks:
        try:
            # Pull scripts roughly for the shared checker (best effort)
            import re
            scripts = [m.group(2) for m in re.finditer(r"<script\b([^>]*)>(.*?)</script>", html, re.DOTALL | re.IGNORECASE) if not re.search(r"\bsrc\s*=", m.group(1), re.I)]
            probs = game_checks.collect_all_static_issues(html, scripts)
            probs += game_checks.platformer_smells(scripts, html)
            return probs
        except Exception:
            pass
    # Fallback to previous inline logic if shared module unavailable
    import re
    low = html.lower()
    probs: list[str] = []
    if "closest('#play-btn')" in html or 'closest("#play-btn")' in html:
        if not re.search(r'id=["\']play-btn["\']', html, re.I):
            if re.search(r"filltext\s*\(\s*['\"]play", html, re.I):
                probs.append("canvas PLAY drawn but #play-btn element missing")
    if re.search(r"initgame\s*\(\s*\)\s*;\s*requestanimationframe", html, re.I | re.S):
        if "startscreen" in low.replace("_", "").replace("-", ""):
            if re.search(r"gamestate\s*=\s*['\"]running['\"]", html, re.I):
                probs.append("initGame() on load sets running while start overlay visible")
    m = re.search(r"<canvas[^>]*>", html, re.I)
    if m and "width=" not in m.group(0).lower():
        if not re.search(r"canvas\.width\s*=", html, re.I):
            probs.append("canvas may have zero default size (no width attr/assign)")
    if re.search(r"gamestate\s*=\s*['\"]start['\"]", html, re.I):
        has_play = bool(re.search(r'id=["\'](play|play-btn|playbtn|btn-start|btnstart)["\']', html, re.I))
        has_key = bool(re.search(r"(enter|space).{0,40}startgame|startgame.{0,40}(enter|space)", html, re.I | re.S))
        if not has_play and not has_key:
            probs.append("gameState start without HTML play button or Enter/Space start")
    dup_ids = re.findall(r'id=["\']([^"\']+)["\']', html, re.I)
    seen: set[str] = set()
    for i in dup_ids:
        if i in seen:
            probs.append(f"duplicate HTML id '{i}' (breaks getElementById)")
        seen.add(i)
    if "<canvas" in low and "tabindex" not in low:
        probs.append("canvas lacks tabindex (keyboard may not work until click)")
    return probs


def main() -> int:
    targets = sorted(GAMES.glob("*/index.html"))
    if len(sys.argv) > 1:
        targets = [Path(p) for p in sys.argv[1:]]
    any_issue = False
    for path in targets:
        html = path.read_text(encoding="utf-8", errors="ignore")
        probs = audit(html)
        if probs:
            any_issue = True
            print(f"{path.parent.name}:")
            for p in probs:
                print(f"  - {p}")
    if not any_issue:
        print("no issues flagged")
    return 1 if any_issue else 0


if __name__ == "__main__":
    raise SystemExit(main())