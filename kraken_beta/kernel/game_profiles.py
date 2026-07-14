"""kraken/kernel/game_profiles.py

Beast Contract v2 — profile-aware, truthful acceptance for browser games.

This replaces the monolithic platformer-only BEAST_GAME_CONTRACT with:
- Trusted resolution (payload + .kraken-game.json manifest)
- Universal requirements + per-profile capabilities, snapshot shape, actions
- v1 platformer backward compat for Captain Comic etc.
- Structured diagnostics and driver contracts

Never auto-detect as authoritative for publish. Detection is only fallback guidance.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Dict, List, Optional, Set


@dataclass(frozen=True)
class ProfileSpec:
    name: str
    real_driver_inputs: str
    action_names: List[str]
    required_snapshot_paths: List[str]
    terminal_semantics: str  # "finite" or "endless"
    description: str = ""


# Universal requirements (every profile must satisfy these in runtime evidence)
UNIVERSAL_REQUIREMENTS = [
    "visible Play/Start action discoverable and works",
    "start overlay no longer blocks play after action",
    "no uncaught runtime or console errors after start",
    "animation/state loop sustains configured frames",
    "drawable canvas exists and is not flat/blank after input",
    "canvas pixels or meaningful state hash changes after real input/ticks",
    "Web Audio source starts when audio capability required",
    "pause/restart or loss/restart works as appropriate for the session",
]

# Registry — data-driven, not a giant if-chain in the probe
PROFILES: Dict[str, ProfileSpec] = {
    "platformer": ProfileSpec(
        name="platformer",
        real_driver_inputs="hold Right; tap Space",
        action_names=["damage", "advance"],
        required_snapshot_paths=["actor.x", "actor.y", "actor.vy", "metrics.lives", "metrics.level"],
        terminal_semantics="finite",
        description="Side-view running/jumping with gravity, coyote, collect, enemies, exit advance to win.",
    ),
    "endless_runner": ProfileSpec(
        name="endless_runner",
        real_driver_inputs="wait; tap Space; hold Down",
        action_names=["damage", "fail"],
        required_snapshot_paths=["actor.y", "actor.vy", "metrics.distance", "metrics.posture", "metrics.lives"],
        terminal_semantics="endless",
        description="Auto-scroll or constant forward motion. Jump/duck. Genuine loss + clean restart.",
    ),
    "shooter": ProfileSpec(
        name="shooter",
        real_driver_inputs="directional input; primary fire",
        action_names=["damage", "advance"],
        required_snapshot_paths=["actor.x", "actor.y", "metrics.projectiles", "metrics.enemies", "metrics.lives", "metrics.wave"],
        terminal_semantics="finite",
        description="Fire changes world state; damage works; mission advances or endless loss/restart.",
    ),
    "snake": ProfileSpec(
        name="snake",
        real_driver_inputs="two legal direction changes",
        action_names=["feed", "collide"],
        required_snapshot_paths=[
            "state", "complete", "score",
            "board.rows", "board.cols", "board.body",
            "actor.x", "actor.y",
            "metrics.direction.x", "metrics.direction.y",
            "metrics.length", "metrics.food.x", "metrics.food.y",
            "metrics.steps", "metrics.foods_eaten", "metrics.deaths",
            "metrics.target_foods",
        ],
        terminal_semantics="finite",
        description=(
            "Canonical body moves and turns on a bounded grid. Probe actions only stage the next "
            "food/collision; an ordinary shared game tick must earn growth, loss, finite victory, and restart."
        ),
    ),
    "breakout_pinball": ProfileSpec(
        name="breakout_pinball",
        real_driver_inputs="launch; paddle/flipper keys",
        action_names=["hit_target", "drain", "advance"],
        required_snapshot_paths=["metrics.ball.x", "metrics.ball.y", "metrics.ball.vx", "metrics.ball.vy", "metrics.targets", "metrics.balls", "metrics.level"],
        terminal_semantics="finite",
        description="Ball and control physics. Targets change count. Drain loses ball. Finite levels advance.",
    ),
    "match3": ProfileSpec(
        name="match3",
        real_driver_inputs="ordinary pointer selection",
        action_names=["legal_swap"],
        required_snapshot_paths=[
            "board.rows", "board.cols", "board.cells", "board.stable",
            "board.geometry.origin_x", "board.geometry.origin_y",
            "board.geometry.stride_x", "board.geometry.stride_y",
            "board.geometry.cell_w", "board.geometry.cell_h",
            "probe.legal_swap.from.row", "probe.legal_swap.from.col",
            "probe.legal_swap.to.row", "probe.legal_swap.to.col",
            "metrics.board_hash", "metrics.moves", "metrics.level_moves",
            "metrics.cascades", "metrics.level", "metrics.completed_levels",
            "metrics.total_levels", "metrics.level_score", "metrics.target_score",
        ],
        terminal_semantics="finite",
        description="Pointer and action swaps change a canonical stable board; matches cascade and earned targets advance finite levels.",
    ),
    "tower_defense": ProfileSpec(
        name="tower_defense",
        real_driver_inputs="ordinary pointer selection",
        action_names=["place_tower", "start_wave", "leak"],
        required_snapshot_paths=[
            "board.rows", "board.cols", "board.cells",
            "board.geometry.origin_x", "board.geometry.origin_y",
            "board.geometry.stride_x", "board.geometry.stride_y",
            "board.geometry.cell_w", "board.geometry.cell_h",
            "probe.place_tower.row", "probe.place_tower.col",
            "probe.place_tower.type", "probe.place_tower.cost",
            "towers", "enemies",
            "metrics.board_hash", "metrics.towers", "metrics.enemies",
            "metrics.currency", "metrics.base_hp", "metrics.wave",
            "metrics.total_waves", "metrics.completed_waves",
            "metrics.spawned", "metrics.kills", "metrics.leaks",
            "metrics.shots", "metrics.hits", "metrics.spent", "metrics.earned",
        ],
        terminal_semantics="finite",
        description="Canonical pointer placement spends currency; stable-ID enemies take tower fire; real leaks hurt the base; earned finite waves advance.",
    ),
    "rhythm": ProfileSpec(
        name="rhythm",
        real_driver_inputs="start track; ordinary lane input",
        action_names=["hit_next", "miss_next", "finish"],
        required_snapshot_paths=[
            "state", "complete", "score",
            "track.id", "track.duration_ms", "track.lanes", "track.lane_keys",
            "track.timing.perfect_ms", "track.timing.good_ms",
            "chart",
            "probe.next_note.id", "probe.next_note.lane",
            "probe.next_note.key", "probe.next_note.time_ms",
            "metrics.clock_ms", "metrics.chart_hash",
            "metrics.combo", "metrics.max_combo",
            "metrics.hits", "metrics.perfect", "metrics.good", "metrics.misses",
            "metrics.judged_notes", "metrics.remaining_notes", "metrics.total_notes",
            "metrics.accuracy",
        ],
        terminal_semantics="finite",
        description="Canonical timestamped chart; ordinary lane input and adapter actions judge real notes; accounting conserves; track finishes and restarts cleanly.",
    ),
    "frogger": ProfileSpec(
        name="frogger",
        real_driver_inputs="directional hops",
        action_names=["collide", "reach_goal"],
        required_snapshot_paths=["actor.x", "actor.y", "metrics.lives", "metrics.goals", "metrics.level"],
        terminal_semantics="finite",
        description="Hops move actor; hazard damages; goal advances; finite win or endless semantics.",
    ),
    "roguelike": ProfileSpec(
        name="roguelike",
        real_driver_inputs="move and attack/interact keys",
        action_names=["damage", "advance"],
        required_snapshot_paths=["actor.x", "actor.y", "metrics.turn", "metrics.hp", "metrics.enemies", "metrics.floor"],
        terminal_semantics="finite",
        description="Turn advances; real combat/damage; floor/goal progress.",
    ),
    "puzzle_grid": ProfileSpec(
        name="puzzle_grid",
        real_driver_inputs="one legal grid move; one legal push",
        action_names=["move", "push", "advance"],
        required_snapshot_paths=["actor.x", "actor.y", "metrics.moves", "metrics.crates_on_goals", "metrics.level"],
        terminal_semantics="finite",
        description="Grid puzzle movement and legal pushes change the real board; solved boards advance finite levels.",
    ),
    "simulation": ProfileSpec(
        name="simulation",
        real_driver_inputs="directional movement; interact/action key",
        action_names=["move", "interact", "mine", "pan", "rest", "advance"],
        required_snapshot_paths=[
            "state",
            "actor.x", "actor.y",
            "metrics.hunger", "metrics.energy",
            "metrics.gold", "metrics.money",
        ],
        terminal_semantics="finite",
        description=(
            "Free-roam survival/exploration (Lost Dutchman Mine 1989 style). "
            "Actor moves on a world map. Survival stats (hunger, energy) decay over time. "
            "Gold/money earned via panning, mining, bounty, poker, fishing. "
            "Win by finding the legendary mine. Interact actions call real game logic."
        ),
    ),
}

# Universal capabilities always added for quality=beast
UNIVERSAL_CAPABILITIES = {"audio"}


def get_profile(name: str) -> Optional[ProfileSpec]:
    return PROFILES.get(name.lower()) if name else None


def list_profiles() -> List[str]:
    return sorted(PROFILES.keys())


def resolve_trusted_profile(
    payload: Dict[str, Any],
    manifest: Optional[Dict[str, Any]] = None,
) -> Dict[str, Any]:
    """
    Return dict with keys: profile, session, contract_version, source, error (or None)
    Strict precedence per handover §8.1.
    """
    result = {
        "profile": None,
        "session": None,
        "contract_version": None,
        "source": None,
        "error": None,
    }

    def _extract(d: Optional[Dict[str, Any]]) -> Optional[Dict[str, Any]]:
        if not d:
            return None
        p = d.get("profile") or d.get("genre")
        s = d.get("session")
        v = d.get("contract_version") or d.get("contractVersion")
        if p and s is not None and v is not None:
            return {"profile": str(p).lower(), "session": str(s).lower(), "contract_version": int(v)}
        return None

    payload_t = _extract(payload)
    manifest_t = _extract(manifest)

    if payload_t and manifest_t:
        if payload_t != manifest_t:
            result["error"] = "PROFILE_CONFLICT payload vs manifest"
            return result
        result.update(payload_t)
        result["source"] = "payload+manifest"
        return result

    if payload_t:
        result.update(payload_t)
        result["source"] = "payload"
        return result

    if manifest_t:
        result.update(manifest_t)
        result["source"] = "manifest"
        return result

    result["error"] = "PROFILE_MISSING trusted tuple required (profile, session, contract_version)"
    return result


def get_required_actions(profile_name: str) -> List[str]:
    spec = get_profile(profile_name)
    return spec.action_names if spec else []


def get_required_snapshot_paths(profile_name: str) -> List[str]:
    spec = get_profile(profile_name)
    return spec.required_snapshot_paths if spec else []


def is_universal_requirement(text: str) -> bool:
    t = text.lower()
    return any(req.lower() in t for req in UNIVERSAL_REQUIREMENTS)


# v1 platformer compat (for existing Captain Comic etc.)
V1_PLATFORMER_ACTIONS = ["damage", "completeLevel"]
V1_PLATFORMER_SNAPSHOT = ["state", "lives", "player", "level"]


def get_v1_platformer_compat() -> Dict[str, Any]:
    return {
        "actions": V1_PLATFORMER_ACTIONS,
        "snapshot_keys": V1_PLATFORMER_SNAPSHOT,
        "note": "Legacy Captain Comic / v1 beast adapter only. New jobs should use profile=platformer + contract_version=2.",
    }


__all__ = [
    "ProfileSpec",
    "PROFILES",
    "UNIVERSAL_REQUIREMENTS",
    "UNIVERSAL_CAPABILITIES",
    "get_profile",
    "list_profiles",
    "resolve_trusted_profile",
    "get_required_actions",
    "get_required_snapshot_paths",
    "get_v1_platformer_compat",
]
