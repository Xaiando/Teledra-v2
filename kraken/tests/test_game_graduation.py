import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.dirname(ROOT))

from kraken.harness import browser_game_probe, game_checks, game_graduation


MINIMAL_GAME = """<!doctype html><html><head><title>T</title></head><body>
<button id="playBtn">Play</button><canvas id="game" width="64" height="64" tabindex="0"></canvas>
<script>
const canvas=document.getElementById('game'); const ctx=canvas.getContext('2d');
document.getElementById('playBtn').addEventListener('click',()=>requestAnimationFrame(gameLoop));
window.addEventListener('keydown',()=>{});
function gameLoop(){ctx.fillStyle='#f00';ctx.fillRect(0,0,10,10);requestAnimationFrame(gameLoop)}
window.__KRAKEN_BEAST__={version:2,profile:'snake',snapshot(){return {actor:{x:1,y:1},metrics:{direction:'r',length:3,food:{x:2,y:2}}}},action(){return true}};
</script></body></html>"""


TD_BASE_CELLS = [
    ["path", "path", "path", "core"],
    ["build", "build", "path", "blocked"],
    ["build", "blocked", "path", "blocked"],
    ["blocked", "blocked", "path", "blocked"],
]


def tower_defense_state(
    cells,
    *,
    towers,
    enemies,
    hint,
    currency,
    base_hp=100,
    wave=1,
    total_waves=3,
    completed_waves=0,
    spawned=0,
    kills=0,
    leaks=0,
    shots=0,
    hits=0,
    spent=0,
    earned=0,
):
    return {
        "state": "playing",
        "board": {
            "rows": len(cells),
            "cols": len(cells[0]),
            "cells": cells,
            "geometry": {
                "origin_x": 4,
                "origin_y": 4,
                "stride_x": 12,
                "stride_y": 12,
                "cell_w": 10,
                "cell_h": 10,
            },
        },
        "probe": {"place_tower": hint},
        "towers": towers,
        "enemies": enemies,
        "metrics": {
            "board_hash": browser_game_probe._fnv1a_cells(cells),
            "towers": len(towers),
            "enemies": len(enemies),
            "currency": currency,
            "base_hp": base_hp,
            "wave": wave,
            "total_waves": total_waves,
            "completed_waves": completed_waves,
            "spawned": spawned,
            "kills": kills,
            "leaks": leaks,
            "shots": shots,
            "hits": hits,
            "spent": spent,
            "earned": earned,
        },
    }


def valid_tower_defense_report():
    initial_hint = {"row": 1, "col": 0, "type": "pulse", "cost": 20}
    second_hint = {"row": 1, "col": 1, "type": "arc", "cost": 30}
    remaining_hint = {"row": 2, "col": 0, "type": "frost", "cost": 25}
    initial = tower_defense_state(
        json.loads(json.dumps(TD_BASE_CELLS)), towers=[], enemies=[], hint=initial_hint, currency=100
    )
    pointer_cells = json.loads(json.dumps(TD_BASE_CELLS))
    pointer_cells[1][0] = "tower:pulse"
    tower_one = {"id": "tower-1", "row": 1, "col": 0, "type": "pulse", "cost": 20}
    pointer = tower_defense_state(
        pointer_cells, towers=[tower_one], enemies=[], hint=second_hint, currency=80, spent=20
    )
    placed_cells = json.loads(json.dumps(pointer_cells))
    placed_cells[1][1] = "tower:arc"
    tower_two = {"id": "tower-2", "row": 1, "col": 1, "type": "arc", "cost": 30}
    installed = [tower_one, tower_two]
    placed = tower_defense_state(
        placed_cells, towers=installed, enemies=[], hint=remaining_hint, currency=50, spent=50
    )
    enemy_one = {"id": "enemy-1", "hp": 10, "max_hp": 10, "progress": 0.1}
    wave_start = tower_defense_state(
        placed_cells, towers=installed, enemies=[enemy_one], hint=remaining_hint,
        currency=50, spent=50, spawned=1,
    )
    combat = tower_defense_state(
        placed_cells, towers=installed, enemies=[], hint=remaining_hint,
        currency=60, spent=50, spawned=1, kills=1, shots=2, hits=1, earned=10,
    )
    progress = tower_defense_state(
        placed_cells, towers=installed, enemies=[], hint=remaining_hint,
        currency=60, spent=50, spawned=1, kills=1, shots=2, hits=1, earned=10,
        wave=2, completed_waves=1,
    )
    enemy_two = {"id": "enemy-2", "hp": 14, "max_hp": 14, "progress": 0.05}
    before_leak = tower_defense_state(
        placed_cells, towers=installed, enemies=[enemy_two], hint=remaining_hint,
        currency=60, spent=50, spawned=2, kills=1, shots=2, hits=1, earned=10,
        wave=2, completed_waves=1,
    )
    after_leak = tower_defense_state(
        placed_cells, towers=installed, enemies=[], hint=remaining_hint,
        currency=60, base_hp=90, spent=50, spawned=2, kills=1, leaks=1,
        shots=2, hits=1, earned=10, wave=2, completed_waves=1,
    )
    return {
        "clickedPlay": True,
        "overlayHiddenAfterStart": True,
        "rafCount": 30,
        "audioStarts": 1,
        "beastApi": True,
        "canvasStart": {"width": 64, "height": 64, "colors": 8, "hash": 1},
        "canvasAfterRight": {"width": 64, "height": 64, "colors": 9, "hash": 2},
        "canvasFinal": {"width": 64, "height": 64, "colors": 9, "hash": 3},
        "actionResults": {
            "place_tower": True,
            "start_wave": True,
            "start_wave_for_leak": True,
            "leak": True,
        },
        "telemetry": {
            "initial": initial,
            "afterPointer": pointer,
            "afterPlace": placed,
            "afterWaveStart": wave_start,
            "afterCombat": combat,
            "afterWaveProgress": progress,
            "beforeLeak": before_leak,
            "afterLeak": after_leak,
            "transitions": [progress],
        },
    }


RHYTHM_NOTES = [
    ("n0", 0, 500),
    ("n1", 1, 800),
    ("n2", 2, 1100),
    ("n3", 3, 1400),
    ("n4", 0, 1700),
    ("n5", 1, 2000),
]


def rhythm_state(
    judgements,
    *,
    clock_ms,
    score,
    combo,
    max_combo,
    state="playing",
    complete=False,
):
    chart = [
        {"id": note_id, "lane": lane, "time_ms": time_ms, "judgement": judgement}
        for (note_id, lane, time_ms), judgement in zip(RHYTHM_NOTES, judgements)
    ]
    perfect = sum(judgement == "perfect" for judgement in judgements)
    good = sum(judgement == "good" for judgement in judgements)
    misses = sum(judgement == "miss" for judgement in judgements)
    hits = perfect + good
    judged = hits + misses
    pending = [note for note in chart if note["judgement"] is None]
    keys = ["d", "f", "j", "k"]
    next_note = None
    if pending:
        note = pending[0]
        next_note = {
            "id": note["id"],
            "lane": note["lane"],
            "key": keys[note["lane"]],
            "time_ms": note["time_ms"],
        }
    immutable = [[note["id"], note["lane"], note["time_ms"]] for note in chart]
    accuracy = 1 if judged == 0 else (perfect + good * 0.7) / judged
    return {
        "state": state,
        "complete": complete,
        "score": score,
        "track": {
            "id": "pulse-one",
            "duration_ms": 2400,
            "lanes": 4,
            "lane_keys": keys,
            "timing": {"perfect_ms": 60, "good_ms": 120},
        },
        "chart": chart,
        "probe": {"next_note": next_note},
        "metrics": {
            "clock_ms": clock_ms,
            "chart_hash": browser_game_probe._fnv1a_json(immutable),
            "combo": combo,
            "max_combo": max_combo,
            "hits": hits,
            "perfect": perfect,
            "good": good,
            "misses": misses,
            "judged_notes": judged,
            "remaining_notes": len(chart) - judged,
            "total_notes": len(chart),
            "accuracy": accuracy,
        },
    }


def valid_rhythm_report():
    pending = [None] * len(RHYTHM_NOTES)
    after_first = ["perfect", None, None, None, None, None]
    after_second = ["perfect", "good", None, None, None, None]
    after_miss = ["perfect", "good", "miss", None, None, None]
    finished = ["perfect", "good", "miss", "miss", "miss", "miss"]
    initial = rhythm_state(pending, clock_ms=100, score=0, combo=0, max_combo=0)
    before_ordinary = rhythm_state(pending, clock_ms=490, score=0, combo=0, max_combo=0)
    after_ordinary = rhythm_state(after_first, clock_ms=505, score=100, combo=1, max_combo=1)
    before_hit = rhythm_state(after_first, clock_ms=790, score=100, combo=1, max_combo=1)
    after_hit = rhythm_state(after_second, clock_ms=805, score=170, combo=2, max_combo=2)
    before_miss = rhythm_state(after_second, clock_ms=850, score=170, combo=2, max_combo=2)
    after_miss_state = rhythm_state(after_miss, clock_ms=860, score=170, combo=0, max_combo=2)
    before_finish = rhythm_state(after_miss, clock_ms=870, score=170, combo=0, max_combo=2)
    after_finish = rhythm_state(
        finished, clock_ms=2400, score=170, combo=0, max_combo=2,
        state="results", complete=True,
    )
    after_restart = rhythm_state(pending, clock_ms=20, score=0, combo=0, max_combo=0)
    return {
        "clickedPlay": True,
        "clickedRestart": True,
        "overlayHiddenAfterStart": True,
        "rafCount": 30,
        "audioStartsAfterPlay": 1,
        "audioStarts": 4,
        "beastApi": True,
        "canvasStart": {"width": 64, "height": 64, "colors": 8, "hash": 1},
        "canvasAfterRight": {"width": 64, "height": 64, "colors": 9, "hash": 2},
        "canvasFinal": {"width": 64, "height": 64, "colors": 8, "hash": 3},
        "ordinaryLaneInput": {"code": "KeyD", "key": "d", "note_id": "n0", "lane": 0},
        "actionResults": {"hit_next": True, "miss_next": True, "finish": True},
        "telemetry": {
            "initial": initial,
            "beforeOrdinary": before_ordinary,
            "afterOrdinary": after_ordinary,
            "beforeHit": before_hit,
            "afterHit": after_hit,
            "beforeMiss": before_miss,
            "afterMiss": after_miss_state,
            "beforeFinish": before_finish,
            "afterFinish": after_finish,
            "afterRestart": after_restart,
            "transitions": [after_finish],
        },
    }


def snake_state(
    body,
    *,
    direction,
    food,
    steps,
    foods_eaten,
    deaths,
    score,
    state="playing",
    complete=False,
    target_foods=3,
):
    return {
        "state": state,
        "complete": complete,
        "score": score,
        "board": {"rows": 15, "cols": 20, "body": [{"x": x, "y": y} for x, y in body]},
        "actor": {"x": body[0][0], "y": body[0][1]},
        "metrics": {
            "direction": {"x": direction[0], "y": direction[1]},
            "length": len(body),
            "food": {"x": food[0], "y": food[1]},
            "steps": steps,
            "foods_eaten": foods_eaten,
            "deaths": deaths,
            "target_foods": target_foods,
        },
    }


def valid_snake_report():
    initial_body = [(5, 0), (4, 0), (3, 0)]
    initial = snake_state(
        initial_body, direction=(1, 0), food=(10, 5), steps=0,
        foods_eaten=0, deaths=0, score=0,
    )
    after_down = snake_state(
        [(5, 1), (5, 0), (4, 0)], direction=(0, 1), food=(10, 5), steps=1,
        foods_eaten=0, deaths=0, score=0,
    )
    after_right = snake_state(
        [(6, 1), (5, 1), (5, 0)], direction=(1, 0), food=(10, 5), steps=2,
        foods_eaten=0, deaths=0, score=0,
    )
    feed_setup = snake_state(
        [(6, 1), (5, 1), (5, 0)], direction=(1, 0), food=(7, 1), steps=2,
        foods_eaten=0, deaths=0, score=0,
    )
    fed = snake_state(
        [(7, 1), (6, 1), (5, 1), (5, 0)], direction=(1, 0), food=(10, 5), steps=3,
        foods_eaten=1, deaths=0, score=10,
    )
    collision_setup = snake_state(
        [(19, 7), (18, 7), (17, 7), (16, 7)], direction=(1, 0), food=(1, 1), steps=3,
        foods_eaten=1, deaths=0, score=10,
    )
    collision = snake_state(
        [(19, 7), (18, 7), (17, 7), (16, 7)], direction=(1, 0), food=(1, 1), steps=3,
        foods_eaten=1, deaths=1, score=10, state="lost",
    )
    restart = snake_state(
        initial_body, direction=(1, 0), food=(10, 5), steps=0,
        foods_eaten=0, deaths=0, score=0,
    )
    setup_one = snake_state(
        initial_body, direction=(1, 0), food=(6, 0), steps=0,
        foods_eaten=0, deaths=0, score=0,
    )
    tick_one = snake_state(
        [(6, 0), (5, 0), (4, 0), (3, 0)], direction=(1, 0), food=(10, 5), steps=1,
        foods_eaten=1, deaths=0, score=10,
    )
    setup_two = snake_state(
        [(6, 0), (5, 0), (4, 0), (3, 0)], direction=(1, 0), food=(7, 0), steps=1,
        foods_eaten=1, deaths=0, score=10,
    )
    tick_two = snake_state(
        [(7, 0), (6, 0), (5, 0), (4, 0), (3, 0)], direction=(1, 0), food=(10, 5), steps=2,
        foods_eaten=2, deaths=0, score=20,
    )
    setup_three = snake_state(
        [(7, 0), (6, 0), (5, 0), (4, 0), (3, 0)], direction=(1, 0), food=(8, 0), steps=2,
        foods_eaten=2, deaths=0, score=20,
    )
    tick_three = snake_state(
        [(8, 0), (7, 0), (6, 0), (5, 0), (4, 0), (3, 0)],
        direction=(1, 0), food=(10, 5), steps=3, foods_eaten=3,
        deaths=0, score=30, state="won", complete=True,
    )
    return {
        "clickedPlay": True,
        "clickedRestart": True,
        "overlayHiddenAfterStart": True,
        "rafCount": 30,
        "audioStartsAfterPlay": 1,
        "audioStarts": 5,
        "beastApi": True,
        "canvasStart": {"width": 64, "height": 64, "colors": 8, "hash": 1},
        "canvasAfterRight": {"width": 64, "height": 64, "colors": 9, "hash": 2},
        "canvasFinal": {"width": 64, "height": 64, "colors": 9, "hash": 3},
        "actionResults": {
            "feed": True,
            "collide": True,
            "progression_feed_0": True,
            "progression_feed_1": True,
            "progression_feed_2": True,
        },
        "telemetry": {
            "initial": initial,
            "afterDown": after_down,
            "afterRight": after_right,
            "afterFeedSetup": feed_setup,
            "afterFeed": fed,
            "afterCollisionSetup": collision_setup,
            "afterDamage": collision,
            "afterRestart": restart,
            "progressionSetups": [setup_one, setup_two, setup_three],
            "transitions": [tick_one, tick_two, tick_three],
        },
    }


def valid_shooter_report():
    initial = {
        "state": "playing",
        "complete": False,
        "actor": {"x": 0, "y": 0},
        "metrics": {"projectiles": 0, "enemies": 0, "lives": 3, "wave": 0},
    }
    moved = {
        "state": "playing",
        "complete": False,
        "actor": {"x": 0, "y": 12},
        "metrics": {**initial["metrics"]},
    }
    fired = {
        "state": "playing",
        "complete": False,
        "actor": {**moved["actor"]},
        "metrics": {**initial["metrics"], "projectiles": 1},
    }
    damaged = {
        "state": "playing",
        "complete": False,
        "actor": {**moved["actor"]},
        "metrics": {**fired["metrics"], "lives": 2},
    }
    advanced = {
        "state": "playing",
        "complete": False,
        "actor": {**moved["actor"]},
        "metrics": {**damaged["metrics"], "wave": 1},
    }
    return {
        "clickedPlay": True,
        "overlayHiddenAfterStart": True,
        "rafCount": 20,
        "audioStarts": 1,
        "beastApi": True,
        "canvasStart": {"width": 64, "height": 64, "colors": 8, "hash": 1},
        "canvasAfterRight": {"width": 64, "height": 64, "colors": 9, "hash": 2},
        "canvasFinal": {"width": 64, "height": 64, "colors": 9, "hash": 3},
        "actionResults": {"damage": True, "advance": True},
        "telemetry": {
            "initial": initial,
            "afterRight": moved,
            "beforeFire": moved,
            "afterFire": fired,
            "afterDamage": damaged,
            "transitions": [advanced],
        },
    }


class TestGameGraduation(unittest.TestCase):
    def test_discovers_inventory_and_excludes_captain_comic(self):
        with tempfile.TemporaryDirectory() as tmp:
            games = Path(tmp)
            for name in ("alpha", "captain_comic_clone", "beta"):
                (games / name).mkdir()
            self.assertEqual(
                [path.name for path in game_graduation.discover_games(games, {"captain_comic_clone"})],
                ["alpha", "beta"],
            )

    def test_v2_declaration_is_not_silently_resolved_as_v1(self):
        with tempfile.TemporaryDirectory() as tmp:
            game = Path(tmp)
            declared = {"runtime": "browser", "profile": "snake", "session": "finite", "contract_version": 2}
            resolved = game_graduation.resolve_declaration(game, declared)
            self.assertEqual(resolved["contract_version"], 2)
            self.assertEqual(resolved["profile"], "snake")

            (game / ".kraken-game.json").write_text(
                json.dumps({"profile": "snake", "session": "finite", "contract_version": 1}),
                encoding="utf-8",
            )
            conflict = game_graduation.resolve_declaration(game, declared)
            self.assertIn("PROFILE_CONFLICT", conflict["error"])

    def test_runtime_identity_must_match_declaration(self):
        report = {
            "beastVersion": 1,
            "beastProfile": "platformer",
            "telemetry": {"initial": {}},
        }
        declaration = {"profile": "snake", "session": "finite", "contract_version": 2}
        reasons = game_graduation.assess_identity_and_shape(report, declaration)
        self.assertTrue(any("version mismatch" in reason for reason in reasons))
        self.assertTrue(any("profile mismatch" in reason for reason in reasons))

    def test_snake_driver_uses_bubbling_canvas_input_and_tick_polling(self):
        driver = browser_game_probe.DRIVER
        snake_start = driver.index("profileName === 'snake'")
        snake_end = driver.index("profileName === 'endless_runner'", snake_start)
        snake_driver = driver[snake_start:snake_end]
        key_helper = driver[driver.index("const key ="):snake_start]
        self.assertIn("document.querySelector('canvas') || document", key_helper)
        self.assertIn("bubbles: true", key_helper)
        self.assertIn("waitSnake", snake_driver)
        self.assertIn("afterDown", snake_driver)
        self.assertLess(snake_driver.index("ArrowDown"), snake_driver.index("ArrowRight"))
        self.assertIn("afterFeedSetup", snake_driver)
        self.assertIn("afterCollisionSetup", snake_driver)

    def test_snake_contract_accepts_real_shared_tick_evidence_with_zero_coordinate(self):
        report = valid_snake_report()
        self.assertEqual(
            browser_game_probe.assess_structured(
                report,
                expected_profile="snake",
                session="finite",
                contract_version=2,
            ),
            [],
        )
        identity_report = {
            "beastVersion": 2,
            "beastProfile": "snake",
            "telemetry": {"initial": report["telemetry"]["initial"]},
        }
        self.assertEqual(
            game_graduation.assess_identity_and_shape(
                identity_report,
                {"profile": "snake", "session": "finite", "contract_version": 2},
            ),
            [],
        )

    def test_snake_contract_rejects_direct_feed_and_collision_mutation(self):
        report = valid_snake_report()
        report["telemetry"]["afterFeedSetup"] = json.loads(
            json.dumps(report["telemetry"]["afterFeed"])
        )
        report["telemetry"]["afterCollisionSetup"] = json.loads(
            json.dumps(report["telemetry"]["afterDamage"])
        )
        reasons = browser_game_probe.assess_structured(
            report,
            expected_profile="snake",
            session="finite",
            contract_version=2,
        )
        self.assertTrue(any("before a real shared tick" in reason for reason in reasons))
        self.assertTrue(any("collide action became terminal" in reason for reason in reasons))

    def test_snake_contract_rejects_missing_body_and_unearned_finite_win(self):
        report = valid_snake_report()
        del report["telemetry"]["initial"]["board"]["body"]
        report["telemetry"]["transitions"] = report["telemetry"]["transitions"][:2]
        report["telemetry"]["progressionSetups"] = report["telemetry"]["progressionSetups"][:2]
        reasons = browser_game_probe.assess_structured(
            report,
            expected_profile="snake",
            session="finite",
            contract_version=2,
        )
        self.assertTrue(any("body is not a unique contiguous" in reason for reason in reasons))
        self.assertTrue(any("never reached the finite snake win" in reason for reason in reasons))

    def test_missing_snake_adapter_does_not_mask_audio_failure(self):
        report = valid_snake_report()
        report["beastApi"] = False
        report["audioStartsAfterPlay"] = 0
        reasons = browser_game_probe.assess_structured(
            report,
            expected_profile="snake",
            session="finite",
            contract_version=2,
        )
        self.assertTrue(any("beast game contract missing" in reason for reason in reasons))
        self.assertTrue(any("audio did not start from the Play gesture" in reason for reason in reasons))

    def test_static_gate_rejects_missing_inline_handler_definition(self):
        html = '<button onclick="toggleMute()">Mute</button><script>(()=>{})();</script>'
        reasons = game_checks.inline_handler_closure_problem(html, ["(()=>{})();"])
        self.assertTrue(any("no such function is defined" in reason for reason in reasons))

    def test_breakout_profile_has_shape_and_driver_coverage(self):
        report = {
            "beastVersion": 2,
            "beastProfile": "breakout_pinball",
            "telemetry": {
                "initial": {
                    "metrics": {
                        "ball": {"x": 1, "y": 2, "vx": 3, "vy": 4},
                        "targets": 20,
                        "balls": 3,
                        "level": 1,
                    }
                }
            },
        }
        declaration = {
            "profile": "breakout_pinball",
            "session": "finite",
            "contract_version": 2,
        }
        self.assertEqual(
            game_graduation.assess_identity_and_shape(report, declaration),
            [],
        )

    def test_shooter_profile_has_compatible_shape_and_real_driver_coverage(self):
        report = valid_shooter_report()
        identity_report = {
            "beastVersion": 2,
            "beastProfile": "shooter",
            "telemetry": {"initial": report["telemetry"]["initial"]},
        }
        declaration = {"profile": "shooter", "session": "finite", "contract_version": 2}
        self.assertEqual(game_graduation.assess_identity_and_shape(identity_report, declaration), [])
        self.assertIn("shooter", game_graduation.PROFILE_DRIVER_COVERAGE)

        driver = browser_game_probe.DRIVER
        shooter_start = driver.index("profileName === 'shooter'")
        shooter_end = driver.index("profileName === 'breakout_pinball'", shooter_start)
        shooter_driver = driver[shooter_start:shooter_end]
        self.assertIn("['ArrowRight', 'KeyD']", shooter_driver)
        self.assertIn("key('keydown', 'Space')", shooter_driver)
        self.assertIn("report.actionResults.damage = result === true", shooter_driver)
        self.assertIn("report.actionResults.advance = result === true", shooter_driver)

    def test_shooter_driver_rejects_forged_noop_and_false_actions(self):
        report = valid_shooter_report()
        self.assertEqual(
            browser_game_probe.assess_structured(
                report, expected_profile="shooter", session="finite", contract_version=2
            ),
            [],
        )

        forged = json.loads(json.dumps(report))
        fixed = forged["telemetry"]["initial"]
        forged["telemetry"]["afterRight"] = fixed
        forged["telemetry"]["beforeFire"] = fixed
        forged["telemetry"]["afterFire"] = fixed
        forged["telemetry"]["afterDamage"] = fixed
        forged["telemetry"]["transitions"] = [fixed]
        reasons = browser_game_probe.assess_structured(
            forged, expected_profile="shooter", session="finite", contract_version=2
        )
        self.assertTrue(any("directional input" in reason for reason in reasons))
        self.assertTrue(any("Space input" in reason for reason in reasons))
        self.assertTrue(any("reduce real shooter lives" in reason for reason in reasons))
        self.assertTrue(any("increase shooter wave" in reason for reason in reasons))

        false_action = json.loads(json.dumps(report))
        false_action["actionResults"] = {"damage": False, "advance": False}
        reasons = browser_game_probe.assess_structured(
            false_action, expected_profile="shooter", session="finite", contract_version=2
        )
        self.assertTrue(any("damage action did not report" in reason for reason in reasons))
        self.assertTrue(any("advance action did not report" in reason for reason in reasons))

        terminal = json.loads(json.dumps(report))
        terminal["telemetry"]["transitions"] = [
            {
                **terminal["telemetry"]["afterDamage"],
                "state": "won",
                "complete": True,
            }
        ]
        self.assertEqual(
            browser_game_probe.assess_structured(
                terminal, expected_profile="shooter", session="finite", contract_version=2
            ),
            [],
        )

    def test_breakout_driver_launches_before_movement_probe(self):
        driver = browser_game_probe.DRIVER
        breakout_start = driver.index("profileName === 'breakout_pinball'")
        breakout_end = driver.index("profileName === 'puzzle_grid'", breakout_start)
        breakout_driver = driver[breakout_start:breakout_end]

        launch_down = breakout_driver.index("key('keydown', 'Space')")
        launch_up = breakout_driver.index("key('keyup', 'Space')")
        movement = breakout_driver.index("key('keydown', 'ArrowRight')")
        self.assertLess(launch_down, launch_up)
        self.assertLess(launch_up, movement)

    def test_probe_recognizes_hyphenated_start_overlay_id(self):
        selector_line = next(
            line for line in browser_game_probe.DRIVER.splitlines()
            if "const overlay = document.querySelector" in line
        )
        self.assertIn("#start-overlay", selector_line)

    def test_breakout_driver_requires_real_target_drain_and_level_changes(self):
        initial = {
            "state": "playing",
            "metrics": {
                "ball": {"x": 10, "y": 20, "vx": 2, "vy": -2},
                "targets": 12,
                "balls": 3,
                "level": 1,
            },
        }
        report = {
            "clickedPlay": True,
            "overlayHiddenAfterStart": True,
            "rafCount": 20,
            "audioStarts": 1,
            "beastApi": True,
            "canvasStart": {"width": 64, "height": 64, "colors": 8, "hash": 1},
            "canvasAfterRight": {"width": 64, "height": 64, "colors": 8, "hash": 2},
            "canvasFinal": {"width": 64, "height": 64, "colors": 8, "hash": 3},
            "telemetry": {
                "initial": initial,
                "afterRight": {
                    "state": "playing",
                    "metrics": {**initial["metrics"], "ball": {"x": 14, "y": 16, "vx": 2, "vy": -2}},
                },
                "afterHit": {
                    "state": "playing",
                    "metrics": {**initial["metrics"], "targets": 11},
                },
                "afterDamage": {
                    "state": "playing",
                    "metrics": {**initial["metrics"], "balls": 2},
                },
                "transitions": [
                    {"state": "playing", "metrics": {**initial["metrics"], "level": 2}}
                ],
            },
        }
        self.assertEqual(
            browser_game_probe.assess_structured(
                report,
                expected_profile="breakout_pinball",
                session="finite",
                contract_version=2,
            ),
            [],
        )

    def test_puzzle_driver_requires_move_push_and_level_changes(self):
        report = {
            "clickedPlay": True,
            "overlayHiddenAfterStart": True,
            "rafCount": 20,
            "audioStarts": 1,
            "beastApi": True,
            "canvasStart": {"width": 64, "height": 64, "colors": 8, "hash": 1},
            "canvasAfterRight": {"width": 64, "height": 64, "colors": 8, "hash": 2},
            "canvasFinal": {"width": 64, "height": 64, "colors": 8, "hash": 3},
            "telemetry": {
                "initial": {"actor": {"x": 1, "y": 1}, "metrics": {"moves": 0, "crates_on_goals": 0, "level": 1}},
                "afterRight": {"actor": {"x": 2, "y": 1}, "metrics": {"moves": 1, "crates_on_goals": 0, "level": 1}},
                "afterMove": {"actor": {"x": 2, "y": 2}, "metrics": {"moves": 2, "crates_on_goals": 0, "level": 1}},
                "afterPush": {"actor": {"x": 3, "y": 2}, "metrics": {"moves": 3, "crates_on_goals": 1, "level": 1}},
                "transitions": [{"actor": {"x": 1, "y": 1}, "metrics": {"moves": 0, "crates_on_goals": 0, "level": 2}}],
            },
        }
        self.assertEqual(
            browser_game_probe.assess_structured(
                report,
                expected_profile="puzzle_grid",
                session="finite",
                contract_version=2,
            ),
            [],
        )

    def test_match3_driver_requires_real_board_move_and_cascade_changes(self):
        def state(cells, *, score, moves, level_moves, cascades, level, completed):
            return {
                "state": "playing",
                "score": score,
                "board": {
                    "rows": 3,
                    "cols": 3,
                    "cells": cells,
                    "stable": True,
                    "geometry": {
                        "origin_x": 8,
                        "origin_y": 8,
                        "stride_x": 16,
                        "stride_y": 16,
                        "cell_w": 14,
                        "cell_h": 14,
                    },
                },
                "probe": {
                    "legal_swap": {
                        "from": {"row": 0, "col": 1},
                        "to": {"row": 1, "col": 1},
                    }
                },
                "metrics": {
                    "board_hash": browser_game_probe._fnv1a_cells(cells),
                    "moves": moves,
                    "level_moves": level_moves,
                    "cascades": cascades,
                    "level": level,
                    "completed_levels": completed,
                    "total_levels": 5,
                    "level_score": 0 if level_moves == 0 else 30,
                    "target_score": 100,
                },
            }

        initial = state(
            [["A", "B", "A"], ["C", "A", "D"], ["E", "F", "G"]],
            score=0, moves=0, level_moves=0, cascades=0, level=1, completed=0,
        )
        pointer = state(
            [["H", "I", "H"], ["J", "H", "K"], ["L", "M", "N"]],
            score=110, moves=1, level_moves=0, cascades=1, level=2, completed=1,
        )
        swapped = state(
            [["O", "P", "O"], ["Q", "O", "R"], ["S", "T", "U"]],
            score=140, moves=2, level_moves=1, cascades=2, level=2, completed=1,
        )
        report = {
            "clickedPlay": True,
            "overlayHiddenAfterStart": True,
            "rafCount": 20,
            "audioStarts": 1,
            "beastApi": True,
            "canvasStart": {"width": 64, "height": 64, "colors": 8, "hash": 1},
            "canvasAfterRight": {"width": 64, "height": 64, "colors": 9, "hash": 2},
            "canvasFinal": {"width": 64, "height": 64, "colors": 9, "hash": 3},
            "telemetry": {
                "initial": initial,
                "afterPointer": pointer,
                "afterSwap": swapped,
                "transitions": [pointer, swapped],
            },
        }
        self.assertEqual(
            browser_game_probe.assess_structured(
                report,
                expected_profile="match3",
                session="finite",
                contract_version=2,
            ),
            [],
        )

        unchanged = json.loads(json.dumps(report))
        unchanged["telemetry"]["afterSwap"] = unchanged["telemetry"]["afterPointer"]
        reasons = browser_game_probe.assess_structured(
            unchanged,
            expected_profile="match3",
            session="finite",
            contract_version=2,
        )
        self.assertTrue(any("canonical board" in reason for reason in reasons))
        self.assertTrue(any("total moves" in reason for reason in reasons))
        self.assertTrue(any("real cascade" in reason for reason in reasons))

        forged = json.loads(json.dumps(report))
        forged["telemetry"]["initial"]["metrics"]["board_hash"] = "deadbeef"
        forged_reasons = browser_game_probe.assess_structured(
            forged,
            expected_profile="match3",
            session="finite",
            contract_version=2,
        )
        self.assertTrue(any("not canonical" in reason for reason in forged_reasons))

        bad_hint = json.loads(json.dumps(report))
        bad_hint["telemetry"]["initial"]["probe"]["legal_swap"]["to"] = {"row": 2, "col": 2}
        hint_reasons = browser_game_probe.assess_structured(
            bad_hint,
            expected_profile="match3",
            session="finite",
            contract_version=2,
        )
        self.assertTrue(any("not adjacent" in reason for reason in hint_reasons))

    def test_tower_defense_profile_has_shape_and_driver_coverage(self):
        report = valid_tower_defense_report()
        identity_report = {
            "beastVersion": 2,
            "beastProfile": "tower_defense",
            "telemetry": {"initial": report["telemetry"]["initial"]},
        }
        declaration = {"profile": "tower_defense", "session": "finite", "contract_version": 2}
        self.assertEqual(game_graduation.assess_identity_and_shape(identity_report, declaration), [])
        driver = browser_game_probe.DRIVER
        tower_start = driver.index("profileName === 'tower_defense'")
        tower_end = driver.index("profileName === 'frogger'", tower_start)
        tower_driver = driver[tower_start:tower_end]
        self.assertIn("new MouseEvent('click'", tower_driver)
        for action in ("place_tower", "start_wave", "leak"):
            self.assertIn(f"invoke('{action}'", tower_driver)

    def test_tower_defense_driver_requires_real_economy_combat_leak_and_wave(self):
        report = valid_tower_defense_report()
        self.assertEqual(
            browser_game_probe.assess_structured(
                report,
                expected_profile="tower_defense",
                session="finite",
                contract_version=2,
            ),
            [],
        )

        forged_hash = json.loads(json.dumps(report))
        forged_hash["telemetry"]["initial"]["metrics"]["board_hash"] = "deadbeef"
        reasons = browser_game_probe.assess_structured(
            forged_hash, expected_profile="tower_defense", session="finite", contract_version=2
        )
        self.assertTrue(any("not canonical" in reason for reason in reasons))

        counter_only = json.loads(json.dumps(report))
        counter_only["telemetry"]["afterPointer"]["board"]["cells"] = json.loads(json.dumps(TD_BASE_CELLS))
        counter_only["telemetry"]["afterPointer"]["metrics"]["board_hash"] = browser_game_probe._fnv1a_cells(TD_BASE_CELLS)
        counter_only["telemetry"]["afterPointer"]["towers"] = []
        reasons = browser_game_probe.assess_structured(
            counter_only, expected_profile="tower_defense", session="finite", contract_version=2
        )
        self.assertTrue(any("stable-ID tower" in reason or "metrics.towers" in reason for reason in reasons))

        free_tower = json.loads(json.dumps(report))
        free_tower["telemetry"]["afterPointer"]["metrics"]["currency"] = 100
        reasons = browser_game_probe.assess_structured(
            free_tower, expected_profile="tower_defense", session="finite", contract_version=2
        )
        self.assertTrue(any("declared currency cost" in reason for reason in reasons))

        fake_enemy_count = json.loads(json.dumps(report))
        fake_enemy_count["telemetry"]["afterWaveStart"]["metrics"]["enemies"] = 2
        reasons = browser_game_probe.assess_structured(
            fake_enemy_count, expected_profile="tower_defense", session="finite", contract_version=2
        )
        self.assertTrue(any("live enemy roster" in reason for reason in reasons))

        no_real_combat = json.loads(json.dumps(report))
        no_real_combat["telemetry"]["afterCombat"] = json.loads(
            json.dumps(no_real_combat["telemetry"]["afterWaveStart"])
        )
        reasons = browser_game_probe.assess_structured(
            no_real_combat, expected_profile="tower_defense", session="finite", contract_version=2
        )
        self.assertTrue(any("tower combat" in reason or "real kill" in reason for reason in reasons))

        direct_hp_edit = json.loads(json.dumps(report))
        direct_hp_edit["telemetry"]["afterLeak"] = json.loads(
            json.dumps(direct_hp_edit["telemetry"]["beforeLeak"])
        )
        direct_hp_edit["telemetry"]["afterLeak"]["metrics"]["base_hp"] = 90
        direct_hp_edit["telemetry"]["afterLeak"]["metrics"]["leaks"] = 1
        reasons = browser_game_probe.assess_structured(
            direct_hp_edit, expected_profile="tower_defense", session="finite", contract_version=2
        )
        self.assertTrue(any("pre-existing stable-ID enemy" in reason for reason in reasons))

        direct_wave_edit = json.loads(json.dumps(report))
        direct_wave_edit["telemetry"]["afterWaveProgress"]["metrics"]["completed_waves"] = 0
        direct_wave_edit["telemetry"]["transitions"] = [direct_wave_edit["telemetry"]["afterWaveProgress"]]
        reasons = browser_game_probe.assess_structured(
            direct_wave_edit, expected_profile="tower_defense", session="finite", contract_version=2
        )
        self.assertTrue(any("completed_waves exactly once" in reason for reason in reasons))

        false_action = json.loads(json.dumps(report))
        false_action["actionResults"]["leak"] = False
        reasons = browser_game_probe.assess_structured(
            false_action, expected_profile="tower_defense", session="finite", contract_version=2
        )
        self.assertTrue(any("leak action did not report" in reason for reason in reasons))

    def test_rhythm_profile_has_shape_and_real_lowercase_driver_coverage(self):
        report = valid_rhythm_report()
        identity_report = {
            "beastVersion": 2,
            "beastProfile": "rhythm",
            "telemetry": {"initial": report["telemetry"]["initial"]},
        }
        declaration = {"profile": "rhythm", "session": "finite", "contract_version": 2}
        self.assertEqual(game_graduation.assess_identity_and_shape(identity_report, declaration), [])

        driver = browser_game_probe.DRIVER
        rhythm_start = driver.index("profileName === 'rhythm'")
        rhythm_end = driver.index("profileName === 'frogger'", rhythm_start)
        rhythm_driver = driver[rhythm_start:rhythm_end]
        self.assertIn(".toLowerCase()", rhythm_driver)
        self.assertIn("target.dispatchEvent(new KeyboardEvent('keydown'", rhythm_driver)
        self.assertIn("key: eventKey", rhythm_driver)
        for action in ("hit_next", "miss_next", "finish"):
            self.assertIn(f"invoke('{action}')", rhythm_driver)
        self.assertIn("findButton(/restart|retry|again/i)", rhythm_driver)
        self.assertIn("get currentTime()", browser_game_probe.BOOTSTRAP)

    def test_rhythm_driver_requires_canonical_note_causality_and_clean_restart(self):
        report = valid_rhythm_report()
        self.assertEqual(
            browser_game_probe.assess_structured(
                report,
                expected_profile="rhythm",
                session="finite",
                contract_version=2,
            ),
            [],
        )

        forged_hash = json.loads(json.dumps(report))
        forged_hash["telemetry"]["initial"]["metrics"]["chart_hash"] = "deadbeef"
        reasons = browser_game_probe.assess_structured(
            forged_hash, expected_profile="rhythm", session="finite", contract_version=2
        )
        self.assertTrue(any("not canonical" in reason for reason in reasons))

        forged_counter = json.loads(json.dumps(report))
        forged_counter["telemetry"]["initial"]["metrics"]["misses"] = 1
        reasons = browser_game_probe.assess_structured(
            forged_counter, expected_profile="rhythm", session="finite", contract_version=2
        )
        self.assertTrue(any("does not match chart judgements" in reason for reason in reasons))

        wrong_lane = json.loads(json.dumps(report))
        wrong_lane["ordinaryLaneInput"] = {"code": "KeyF", "key": "f", "note_id": "n0", "lane": 1}
        reasons = browser_game_probe.assess_structured(
            wrong_lane, expected_profile="rhythm", session="finite", contract_version=2
        )
        self.assertTrue(any("lowercase key for next_note" in reason for reason in reasons))

        unchanged_ordinary = json.loads(json.dumps(report))
        unchanged_ordinary["telemetry"]["afterOrdinary"] = json.loads(
            json.dumps(unchanged_ordinary["telemetry"]["beforeOrdinary"])
        )
        reasons = browser_game_probe.assess_structured(
            unchanged_ordinary, expected_profile="rhythm", session="finite", contract_version=2
        )
        self.assertTrue(any("exactly one real pending note" in reason for reason in reasons))

        mutated_chart = json.loads(json.dumps(report))
        mutated_chart["telemetry"]["afterHit"]["chart"][2]["time_ms"] = 1110
        immutable = [
            [note["id"], note["lane"], note["time_ms"]]
            for note in mutated_chart["telemetry"]["afterHit"]["chart"]
        ]
        mutated_chart["telemetry"]["afterHit"]["metrics"]["chart_hash"] = browser_game_probe._fnv1a_json(immutable)
        reasons = browser_game_probe.assess_structured(
            mutated_chart, expected_profile="rhythm", session="finite", contract_version=2
        )
        self.assertTrue(any("rewrote the immutable rhythm chart" in reason for reason in reasons))

        early_hit = json.loads(json.dumps(report))
        early_hit["telemetry"]["beforeHit"]["metrics"]["clock_ms"] = 600
        reasons = browser_game_probe.assess_structured(
            early_hit, expected_profile="rhythm", session="finite", contract_version=2
        )
        self.assertTrue(any("outside the declared timing window" in reason for reason in reasons))

        rewarded_miss = json.loads(json.dumps(report))
        rewarded_miss["telemetry"]["afterMiss"]["score"] += 50
        reasons = browser_game_probe.assess_structured(
            rewarded_miss, expected_profile="rhythm", session="finite", contract_version=2
        )
        self.assertTrue(any("score for a miss" in reason for reason in reasons))

        direct_finish = json.loads(json.dumps(report))
        direct_finish["telemetry"]["afterFinish"] = json.loads(
            json.dumps(direct_finish["telemetry"]["beforeFinish"])
        )
        direct_finish["telemetry"]["afterFinish"]["state"] = "results"
        direct_finish["telemetry"]["afterFinish"]["complete"] = True
        direct_finish["telemetry"]["transitions"] = [direct_finish["telemetry"]["afterFinish"]]
        reasons = browser_game_probe.assess_structured(
            direct_finish, expected_profile="rhythm", session="finite", contract_version=2
        )
        self.assertTrue(any("remaining real note" in reason or "full chart" in reason for reason in reasons))

        dirty_restart = json.loads(json.dumps(report))
        dirty_restart["telemetry"]["afterRestart"] = json.loads(
            json.dumps(dirty_restart["telemetry"]["afterFinish"])
        )
        reasons = browser_game_probe.assess_structured(
            dirty_restart, expected_profile="rhythm", session="finite", contract_version=2
        )
        self.assertTrue(any("clean playing track" in reason or "pending note" in reason for reason in reasons))

        action_only_audio = json.loads(json.dumps(report))
        action_only_audio["audioStartsAfterPlay"] = 0
        reasons = browser_game_probe.assess_structured(
            action_only_audio, expected_profile="rhythm", session="finite", contract_version=2
        )
        self.assertTrue(any("audio did not start from the Play" in reason for reason in reasons))

        false_action = json.loads(json.dumps(report))
        false_action["actionResults"]["hit_next"] = False
        reasons = browser_game_probe.assess_structured(
            false_action, expected_profile="rhythm", session="finite", contract_version=2
        )
        self.assertTrue(any("hit_next action did not report" in reason for reason in reasons))

    def test_frogger_driver_requires_hop_collision_goal_and_level(self):
        report = {
            "clickedPlay": True,
            "overlayHiddenAfterStart": True,
            "rafCount": 20,
            "audioStarts": 1,
            "beastApi": True,
            "canvasStart": {"width": 64, "height": 64, "colors": 8, "hash": 1},
            "canvasAfterRight": {"width": 64, "height": 64, "colors": 9, "hash": 2},
            "canvasFinal": {"width": 64, "height": 64, "colors": 9, "hash": 3},
            "telemetry": {
                "initial": {"actor": {"x": 4, "y": 8}, "metrics": {"lives": 3, "goals": 0, "level": 1}},
                "afterRight": {"actor": {"x": 4, "y": 7}, "metrics": {"lives": 3, "goals": 0, "level": 1}},
                "afterDamage": {"actor": {"x": 4, "y": 8}, "metrics": {"lives": 2, "goals": 0, "level": 1}},
                "transitions": [
                    {"actor": {"x": 4, "y": 8}, "metrics": {"lives": 2, "goals": 1, "level": 2}}
                ],
            },
        }
        self.assertEqual(
            browser_game_probe.assess_structured(
                report,
                expected_profile="frogger",
                session="finite",
                contract_version=2,
            ),
            [],
        )

    def test_manifest_uses_production_file_and_mocked_probe(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            games = root / "workspace" / "games"
            game = games / "serpent_grid"
            game.mkdir(parents=True)
            (game / "index.html").write_text(MINIMAL_GAME, encoding="utf-8")
            inventory = {
                "schema_version": 1,
                "excluded_games": ["captain_comic_clone"],
                "games": {
                    "serpent_grid": {
                        "runtime": "browser",
                        "entrypoint": "index.html",
                        "profile": "snake",
                        "session": "finite",
                        "contract_version": 2,
                    }
                },
            }
            (root / "game_inventory.json").write_text(json.dumps(inventory), encoding="utf-8")

            def passed_probe(path, declaration, scratch):
                self.assertEqual(path, game / "index.html")
                self.assertEqual(declaration["contract_version"], 2)
                return {"attempted": True, "passed": True, "reasons": []}

            manifest = game_graduation.build_manifest(root, browser_runner=passed_probe)
            row = manifest["games"][0]
            self.assertTrue(row["runtime_probe"]["attempted"])
            self.assertEqual(row["declaration"]["contract_version"], 2)
            self.assertEqual(len(row["sha256"]), 64)
            self.assertTrue(manifest["summary"]["inventory_complete"])

    def test_undeclared_directory_blocks_inventory_completion(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "workspace" / "games" / "mystery").mkdir(parents=True)
            (root / "game_inventory.json").write_text(
                json.dumps({"schema_version": 1, "excluded_games": [], "games": {}}),
                encoding="utf-8",
            )
            manifest = game_graduation.build_manifest(root, run_browser=False)
            self.assertFalse(manifest["summary"]["inventory_complete"])
            self.assertFalse(manifest["summary"]["all_browser_games_graduated"])


if __name__ == "__main__":
    unittest.main()
