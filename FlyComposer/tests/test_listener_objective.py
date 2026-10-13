import copy
import math
import unittest

from flycomposer.buzzart.listener_objective import evaluate


def event(role, start, length, pitch, velocity=.6):
    return {"layer": role, "start_seconds": start * .5, "duration_seconds": length * .5,
            "midi": pitch, "velocity": velocity}


def flowing():
    events = []
    # Four different phrases with a sparse bridge and alternating articulation.
    for quarter in range(4):
        for step in range(8):
            position = quarter * 16 + step * 2
            role = "answer" if (step + quarter) % 3 == 0 else "lead"
            pitch = 60 + (step * (quarter + 1) + quarter) % 12
            events.append(event(role, position, 1.2 if step % 2 else 1.9, pitch, .5 + .025 * step))
            events.append(event("bass", position, 1.8, 36 + (step + quarter) % 7, .5))
            if quarter != 2 or step % 2 == 0:
                events.append(event("harmony", position, 1.7, 48 + (step * 2 + quarter) % 12, .4))
    return {"duration_seconds": 33., "bpm": 120, "events": events,
            "voices": {"lead": "fm:lead", "answer": "chip:pulse25", "harmony": "fm:bell", "bass": "fm:bass"}}


class ListenerObjectiveTests(unittest.TestCase):
    def test_foreground_stopping_cannot_hide_behind_backing(self):
        good = flowing()
        fallen = copy.deepcopy(good)
        fallen["events"] = [e for e in fallen["events"] if e["layer"] not in ("lead", "answer") or e["start_seconds"] < 8]
        a, b = evaluate(good), evaluate(fallen)
        self.assertGreater(a["reward"], b["reward"] + .20)
        self.assertEqual(b["facts"]["quarter_foreground_onsets"], [8, 0, 0, 0])

    def test_chord_notes_are_not_independent_roles(self):
        score = flowing()
        score["events"] = [e for e in score["events"] if e["layer"] == "harmony"]
        single = evaluate(score)
        chords = copy.deepcopy(score)
        chords["events"] += [{**e, "midi": e["midi"] + pitch} for pitch in (4, 7) for e in score["events"]]
        self.assertEqual(evaluate(chords)["facts"]["mean_distinct_pitched_roles"], single["facts"]["mean_distinct_pitched_roles"])
        self.assertEqual(evaluate(chords)["reward"], single["reward"])

    def test_drums_do_not_improve_pitched_activity(self):
        score = flowing()
        reference = evaluate(score)
        score["voices"]["texture"] = "chip:noise"
        score["events"] += [event("texture", n / 4, .2, 60) for n in range(256)]
        result = evaluate(score)
        self.assertEqual(reference["reward"], result["reward"])
        self.assertEqual(result["facts"]["ignored_percussion_silent_or_outside_notes"], 256)

    def test_drone_spam_and_transposed_loops_do_not_win(self):
        reference = evaluate(flowing())
        drone = flowing()
        drone["events"] = [event(role, 0, 64, pitch) for role, pitch in (("lead", 60), ("answer", 64), ("harmony", 67), ("bass", 36))]
        spam = flowing()
        spam["events"] += [event("lead", n / 12, .08, 60 + n % 7) for n in range(768)]
        loop = flowing()
        loop["events"] = [e for e in loop["events"] if e["layer"] not in ("lead", "answer")]
        loop["events"] += [event("lead", phrase * 8 + step, .9, 60 + phrase + step % 4, .5 + .03 * (step % 4))
                           for phrase in range(8) for step in range(8)]
        self.assertLess(evaluate(drone)["reward"], reference["reward"] * .5)
        self.assertLess(evaluate(spam)["reward"], reference["reward"] * .7)
        self.assertLess(evaluate(loop)["components"]["development"], reference["components"]["development"])
        self.assertEqual(evaluate(loop)["facts"]["dominant_two_bar_phrase_share"], 1.)

    def test_tempos_and_pitched_instruments_do_not_set_the_target(self):
        score = flowing()
        original = evaluate(score)
        slower = copy.deepcopy(score)
        slower["bpm"] = 72
        slower["duration_seconds"] = (score["duration_seconds"] - 1) * 120 / 72 + 1
        for e in slower["events"]:
            e["start_seconds"] *= 120 / 72
            e["duration_seconds"] *= 120 / 72
        slower["voices"] = {role: "piano" for role in slower["voices"]}
        self.assertAlmostEqual(evaluate(slower)["reward"], original["reward"], places=10)
        self.assertGreater(original["reward"], .8)  # broad targets tolerate detached phrasing and a sparse bridge

    def test_loudness_does_not_earn_reward_and_flat_dynamics_have_cost(self):
        score = flowing()
        original = evaluate(score)
        quiet = copy.deepcopy(score)
        for e in quiet["events"]:
            e["velocity"] *= .5
        self.assertAlmostEqual(evaluate(quiet)["reward"], original["reward"])
        flat = copy.deepcopy(score)
        for e in flat["events"]:
            e["velocity"] = .6
        self.assertLess(evaluate(flat)["components"]["restraint"], original["components"]["restraint"])
        for e in flat["events"]:
            e["velocity"] = 1.
        self.assertLess(evaluate(flat)["reward"], original["reward"] * .5)

    def test_declared_chord_led_long_phrases_need_no_busy_melody(self):
        score = {"duration_seconds": 33, "bpm": 120, "events": [],
                 "voices": {"harmony": "warm_pad", "bass": "cello"},
                 "buzzart": {"style": {"genome": {"harmony": "hold", "spans": [4, 6, 8],
                                                    "mix": {"harmony": .9, "bass": .5, "lead": .45}}}}}
        for index, root in enumerate((48, 53, 55, 50, 57, 55, 52, 48)):
            for interval in (0, 4, 7):
                score["events"].append(event("harmony", index * 8, 7.8, root + interval, .50 + .02 * index))
            score["events"].append(event("bass", index * 8, 7.8, root - 12, .5))
        result = evaluate(score)
        self.assertEqual(result["facts"]["focus_roles"], ["harmony"])
        self.assertEqual(result["facts"]["activity_credit_limit_beats"], 16.)
        self.assertEqual(result["facts"]["foreground_drone_share"], 0.)
        self.assertGreater(result["reward"], .80)
        static = copy.deepcopy(score)
        static["events"] = [event("harmony", 0, 64, pitch, .6) for pitch in (48, 52, 55)]
        static["events"].append(event("bass", 0, 64, 36, .5))
        self.assertLess(evaluate(static)["reward"], result["reward"] * .3)

    def test_declared_long_melody_and_groove_focus(self):
        score = flowing()
        score["buzzart"] = {"style": {"genome": {"melody": "long", "spans": [4, 6, 8]}}}
        score["events"] = [e for e in score["events"] if e["layer"] not in ("lead", "answer")]
        score["events"] += [event("lead", n * 8, 7.8, 60 + n, .5 + n * .02) for n in range(8)]
        result = evaluate(score)
        self.assertGreater(result["reward"], .8)
        self.assertEqual(result["facts"]["foreground_drone_share"], 0.)
        score["mix"] = {"bass": 1., "lead": .6, "harmony": .3}
        self.assertEqual(evaluate(score)["facts"]["focus_roles"], ["bass"])

    def test_target_window_exposes_early_ending_and_score_is_untouched(self):
        score = flowing()
        before = copy.deepcopy(score)
        full = evaluate(score)
        longer = evaluate(score, target_seconds=48)
        self.assertEqual(score, before)
        self.assertLess(longer["reward"], full["reward"])
        self.assertEqual(longer["facts"]["window_seconds"], 48)

    def test_empty_invalid_and_ignored_outside_events(self):
        self.assertEqual(evaluate({"duration_seconds": 33, "bpm": 120, "events": []})["reward"], 0.)
        score = flowing()
        score["events"][0]["duration_seconds"] = math.nan
        with self.assertRaises(ValueError):
            evaluate(score)
        score = flowing()
        before = evaluate(score)
        score["events"].append(event("lead", 100, 1, 60))
        self.assertEqual(evaluate(score)["reward"], before["reward"])


if __name__ == "__main__":
    unittest.main()
