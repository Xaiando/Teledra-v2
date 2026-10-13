"""Bounded symbolic practice targets derived from the September listener review.

These are *proxies*, not an estimate of listener taste or a replacement for the
existing critic/naturalness checks. Targets are beat-relative and deliberately
broad: sparse, brisk, detached and legato pieces can all pass. No instrument,
genre, prescribed intensity arc, or fixed note density earns extra reward.

Coverage credit follows the declared grammar's phrase length, so a held drone
cannot stand in for continued musical activity. Distinct roles, not chord notes,
measure layering. Phrase identity ignores transposition and small timing
differences; mechanically transposing a repeated loop is still repetition.
"""
from __future__ import annotations

from collections import Counter, defaultdict
import math
import statistics

VERSION = "listener_flow_v1"
FOREGROUND = frozenset(("lead", "answer"))
DRUM_VOICES = frozenset(("chip:noise", "amiga:drums"))
COMPONENTS = ("momentum", "continuity", "interaction", "development", "restraint")


def _clamp(value):
    return max(0., min(1., float(value)))


def _mean(values):
    values = list(values)
    return sum(values) / len(values) if values else 0.


def _band(value, low, high, soft):
    return _clamp(1. - max(low - value, value - high, 0.) / soft)


def _merge(spans):
    result = []
    for start, end in sorted(spans):
        if result and start <= result[-1][1] + 1e-8:
            result[-1][1] = max(result[-1][1], end)
        else:
            result.append([start, end])
    return result


def _coverage(spans, start, end):
    return sum(max(0., min(end, b) - max(start, a)) for a, b in spans) / (end - start)


def _finite(value):
    return type(value) in (int, float) and math.isfinite(value)


def _context(score):
    """Read intent set before composition, never infer intent from its defects."""
    style = score.get("buzzart", {}).get("style") or {}
    traits = style.get("genome") or {}
    mix = traits.get("mix") or score.get("mix") or {}
    focus = FOREGROUND
    pitched_mix = {role: value for role, value in mix.items()
                   if _finite(value) and score.get("voices", {}).get(role) not in DRUM_VOICES}
    if pitched_mix:
        dominant = max(pitched_mix, key=pitched_mix.get)
        if dominant not in FOREGROUND and pitched_mix[dominant] > max(pitched_mix.get("lead", 0), pitched_mix.get("answer", 0)):
            focus = frozenset((dominant,))
    slow = (traits.get("melody") == "long" or style.get("temperament") == "dreamy"
            or (focus == frozenset(("harmony",)) and traits.get("harmony") == "hold"))
    spans = [float(value) for value in traits.get("spans", ()) if _finite(value) and value > 0]
    credit = min(16., max(8., 2. * max(spans, default=4.))) if slow else 4.
    return focus, credit, slow


def evaluate(score, target_seconds=None):
    """Return ``version``, ``reward`` (0..1), ``components`` and raw ``facts``.

    Default measurement window is ``duration_seconds - 1`` (the renderer's
    release tail). Pass the requested musical duration to expose early endings.
    Invalid numbers raise ValueError; callers can reject an invalid candidate.
    Event timing is read from seconds, not potentially stale beat metadata.
    Runtime is O(N log N), with no synthesis and no mutation of the score.
    """
    seconds = score.get("duration_seconds", 0) - 1. if target_seconds is None else target_seconds
    bpm = score.get("bpm")
    if not _finite(seconds) or seconds <= 0 or not _finite(bpm) or bpm <= 0:
        raise ValueError("A finite positive measurement duration and tempo are required")
    beat_seconds = 60. / bpm
    window = seconds / beat_seconds
    focus, credit_limit, slow = _context(score)
    rows, ignored = [], 0
    voices = score.get("voices", {})
    for event in score.get("events", []):
        values = [event.get(name) for name in ("start_seconds", "duration_seconds", "midi", "velocity")]
        if not all(_finite(value) for value in values):
            raise ValueError("Event timing, pitch and velocity must be finite numbers")
        start, length, pitch, velocity = values
        if start < 0 or length <= 0 or not 0 <= velocity <= 1 or not 0 <= pitch <= 127:
            raise ValueError("Event timing, pitch or velocity is outside its valid range")
        role = event.get("layer")
        voice = event.get("voice", voices.get(role))
        if not isinstance(role, str) or not role:
            raise ValueError("Every event needs a named layer")
        if voice in DRUM_VOICES or velocity == 0 or start >= seconds:
            ignored += 1
            continue
        onset = start / beat_seconds
        end = min(window, (start + length) / beat_seconds)
        rows.append({"role": role, "start": onset, "end": end,
                     "credit_end": min(end, onset + credit_limit), "pitch": pitch, "velocity": velocity})
    rows.sort(key=lambda e: (e["start"], e["role"], e["pitch"]))
    foreground = [e for e in rows if e["role"] in focus]
    spans = _merge((e["start"], e["credit_end"]) for e in foreground)
    quarters = [(q * window / 4., (q + 1) * window / 4.) for q in range(4)]
    quarter_coverage = [_coverage(spans, a, b) for a, b in quarters]
    quarter_onsets, quarter_pitches = [], []
    for a, b in quarters:
        part = [e for e in foreground if a <= e["start"] < b]
        quarter_onsets.append(len({round(e["start"], 6) for e in part}))
        quarter_pitches.append(len({e["pitch"] for e in part}))
    # Two changing onsets in each quarter is a broad lower bound, not a target
    # density. No penalty for a sparse middle that continues to develop.
    onset_floor = 1. if slow else 2.
    active_quarters = _mean(min(1., n / onset_floor) * min(1., p / onset_floor)
                            for n, p in zip(quarter_onsets, quarter_pitches))
    coverage = _coverage(spans, 0., window)
    gaps = ([spans[0][0]] + [b[0] - a[1] for a, b in zip(spans, spans[1:])]
            + [window - spans[-1][1]]) if spans else [window]
    interior = [b[0] - a[1] for a, b in zip(spans, spans[1:])]
    typical_gap = statistics.median(interior) if interior else 0.
    # Compare interruptions with this piece's own articulation. Consistent
    # staccato has no cost here; irregular gaps well beyond its usual breath
    # provide a modest, separate target. The critic still protects rhythm
    # variety, so this must not be used as a standalone optimization score.
    free_breath = max(.5, credit_limit / 8., 2. * typical_gap)
    interruption_share = sum(max(0., gap - free_breath) for gap in interior) / window
    # Up to two beats of breath is free. Longer pauses are measured by their
    # accumulated excess, so many stalling gaps cannot hide behind one average.
    stalled_share = sum(max(0., gap - 2.) for gap in gaps) / window
    components = {
        "momentum": _mean((_band(coverage, .25, 1., .25),
                            _band(min(quarter_coverage), .12, 1., .12), active_quarters)),
        "continuity": _mean((_band(stalled_share, 0., .04, .25),
                              _band(max(gaps), 0., 6., max(8., window / 4.)),
                              _band(interruption_share, 0., .04, .20))),
    }

    # Sweep all note boundaries together. Three simultaneous chord notes in
    # harmony remain ONE role, and percussion cannot count as a pitched layer.
    edges = defaultdict(list)
    edges[0.]
    edges[window]
    for e in rows:
        edges[e["start"]].append((e["role"], 1))
        edges[e["credit_end"]].append((e["role"], -1))
    active, previous = Counter(), 0.
    part_beats = foreground_beats = supported_beats = support_beats = 0.
    for moment in sorted(edges):
        width = moment - previous
        sounding = {role for role, count in active.items() if count > 0}
        part_beats += width * len(sounding)
        if sounding & focus:
            foreground_beats += width
            supported_beats += width * (len(sounding) >= 2)
            # A chord-led piece may be supported by bass or answering melody;
            # a melody-led piece's harmonic support is any pitched non-bass part.
            excluded = focus if focus != FOREGROUND else focus | {"bass"}
            support_beats += width * bool(sounding - excluded)
        for role, delta in edges[moment]:
            active[role] += delta
        previous = moment
    supported_share = supported_beats / foreground_beats if foreground_beats else 0.
    harmonic_share = support_beats / foreground_beats if foreground_beats else 0.
    # Harmony/bell/other pitched texture is a supporting role, not a required
    # timbre. Partial participation and changing entrances receive full credit.
    moving_support = []
    for a, b in quarters:
        support = [e for e in rows if e["role"] not in focus and a <= e["start"] < b]
        moving_support.append(len({(e["role"], e["pitch"]) for e in support}) >= 2
                              and len({round(e["start"], 6) for e in support}) >= onset_floor)
    components["interaction"] = _mean((_band(supported_share, .65, 1., .65),
                                       _band(harmonic_share, .35, 1., .35),
                                       _band(_mean(moving_support), .5, 1., .5)))

    # Non-overlapping two-bar cells: retain register contour, role, rhythm and
    # duration, but ignore transposition, absolute loudness and tiny jitters.
    fingerprints, cells = [], defaultdict(list)
    for e in foreground:
        index = int(e["start"] // 8.)
        if index < int(window // 8.):
            cells[index].append(e)
    for index, part in sorted(cells.items()):
        if len(part) < 2:
            continue
        origin = part[0]["pitch"]
        fingerprints.append(tuple((e["role"], e["pitch"] - origin,
                                   round((e["start"] - index * 8.) * 4) / 4.,
                                   round((e["end"] - e["start"]) * 4) / 4.) for e in part))
    dominant = max(Counter(fingerprints).values(), default=0) / max(1, len(fingerprints))
    repeat_score = _band(dominant, 0., .60, .40) if len(fingerprints) >= 3 else .5
    pitch_change = []
    for role in focus:
        gestures = defaultdict(set)
        for e in foreground:
            if e["role"] == role:
                gestures[round(e["start"], 6)].add(e["pitch"])
        pitches = [tuple(sorted(pitches)) for _, pitches in sorted(gestures.items())]
        pitch_change.extend(a != b for a, b in zip(pitches, pitches[1:]))
    components["development"] = _mean((repeat_score, _band(_mean(pitch_change), .30, 1., .30)))

    # A ceiling, not an incentive for busyness. Onsets are unique per role, so
    # neither chord voicings nor doubling can earn extra activity credit.
    density = len({(e["role"], round(e["start"], 6)) for e in foreground}) / window
    lengths = [e["end"] - e["start"] for e in foreground]
    drone_share = sum(max(0., length - credit_limit) for length in lengths) / max(1e-9, sum(lengths))
    velocities = [e["velocity"] for e in foreground]
    velocity_sd = statistics.pstdev(velocities) if velocities else 0.
    velocity_mean = _mean(velocities)
    # Relative expression survives a uniform gain change; gain itself earns no
    # reward. Near-maximum velocities and flat velocity streams are guardrails.
    relative_expression = velocity_sd / max(.05, velocity_mean)
    components["restraint"] = _mean((_band(density, 0., 4., 4.),
                                     _band(drone_share, 0., .35, .50),
                                     _band(relative_expression, .025, .50, .10),
                                     _band(velocity_mean, 0., .88, .12)))
    reward = (.30 * components["momentum"] + .22 * components["continuity"]
              + .22 * components["interaction"] + .16 * components["development"]
              + .10 * components["restraint"])
    # Stop non-musical cheats from compensating with perfect coverage/layers.
    safeguards = min(_band(density, 0., 4., 4.), _band(drone_share, 0., .35, .50),
                     _band(velocity_mean, 0., .88, .12))
    # Count distinct articulated gestures, not three notes of one chord.
    gestures = len({(e["role"], round(e["start"], 6)) for e in foreground})
    reward *= (.35 + .65 * safeguards) * min(1., gestures / (4. if slow else 8.))
    facts = {
        "window_seconds": seconds, "window_beats": window, "pitched_notes": len(rows),
        "focus_roles": sorted(focus), "activity_credit_limit_beats": credit_limit,
        "declared_long_phrase_context": slow,
        "foreground_notes": len(foreground), "ignored_percussion_silent_or_outside_notes": ignored,
        "credited_foreground_coverage": coverage, "quarter_foreground_coverage": quarter_coverage,
        "quarter_foreground_onsets": quarter_onsets, "quarter_foreground_distinct_pitches": quarter_pitches,
        "active_quarter_share": active_quarters, "longest_foreground_gap_beats": max(gaps),
        "interior_gap_beats": interior, "stall_excess_share": stalled_share,
        "typical_gap_beats": typical_gap, "interruption_excess_share": interruption_share,
        "mean_distinct_pitched_roles": part_beats / window,
        "supported_foreground_share": supported_share, "harmonic_support_share": harmonic_share,
        "moving_support_quarter_share": _mean(moving_support),
        "two_bar_cells": len(fingerprints), "dominant_two_bar_phrase_share": dominant,
        "foreground_pitch_change_share": _mean(pitch_change), "foreground_onsets_per_beat": density,
        "foreground_drone_share": drone_share, "foreground_velocity_mean": velocity_mean,
        "foreground_velocity_sd": velocity_sd, "relative_velocity_expression": relative_expression,
        "safeguard_multiplier": .35 + .65 * safeguards,
    }
    return {"version": VERSION, "reward": _clamp(reward), "components": components, "facts": facts}
