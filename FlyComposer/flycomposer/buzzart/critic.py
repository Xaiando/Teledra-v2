"""A designed symbolic music critic: Buzzart's reinforcement signal.

Every component is a transparent measurement with a target range, not a taste
model. Ranges reward balance (breathing room, some repetition, mostly consonant
harmony) rather than pushing one number to an extreme, and a quarter of the
reward is the weakest component, so no single trick can maximize the score.
Listening remains the judge of whether the music is actually better.
"""
from __future__ import annotations

from collections import Counter
import math

import numpy as np

from ..composition import ROLES, SCALES, midi
from .console_style import critic_bands

TONICS = ("C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B")
COMPONENTS = ("melody", "breathing", "rhythm", "motif", "harmony", "texture", "form")
DEFAULT_WEIGHTS = {"melody": 1., "breathing": 1., "rhythm": 1., "motif": 1., "harmony": 1.2, "texture": .8, "form": 1.}
FOREGROUND = ("lead", "answer")


def band(value, low, high, soft):
    """1 inside [low, high], falling linearly to 0 over ``soft`` outside it."""
    if value is None or not math.isfinite(value):
        return 0.
    if low <= value <= high:
        return 1.
    distance = low - value if value < low else value - high
    return max(0., 1. - distance / soft)


def _mean(values):
    values = [float(v) for v in values if v is not None]
    return sum(values) / len(values) if values else 0.


def _entropy(counter, categories):
    total = sum(counter.values())
    if total == 0 or len(counter) < 2:
        return 0.
    return -sum((c / total) * math.log(c / total) for c in counter.values()) / math.log(max(categories, len(counter)))


def _merged(intervals):
    merged = []
    for start, end in sorted(intervals):
        if merged and start <= merged[-1][1]:
            merged[-1][1] = max(merged[-1][1], end)
        else:
            merged.append([start, end])
    return merged


def _cosine(a, b):
    norm = float(np.linalg.norm(a) * np.linalg.norm(b))
    return float(np.dot(a, b)) / norm if norm else 0.


def evaluate(score, weights=None):
    """Score one performance. Returns reward, per-component values and the measured facts."""
    weights = {**DEFAULT_WEIGHTS, **(weights or {})}
    events = score["events"]
    beat = 60. / score["bpm"]
    window = float(score["duration_seconds"]) - 1.
    tonic, scale = TONICS.index(score["tonic"]), SCALES[score["mode"]]
    decisions = score.get("decision_log", [])
    roots = list(score.get("progression", []))
    # Console style keeps its own idiom. The chiptune songbook's lead lines are legato, with
    # little breathing, and often let one note length dominate; the ranges below admit that.
    style = score.get("buzzart", {}).get("style")
    console = bool(style)
    # What this style asks for. Arcade wants drive and forgives density; indie
    # wants the space the kingdom tunes have. A style recorded before
    # temperaments existed reads as arcade, which is what it was.
    bands = critic_bands(style)
    foreground = sorted((e for e in events if e["layer"] in FOREGROUND), key=lambda e: (e["start_seconds"], e["layer"]))
    components, facts = {}, {"notes": len(events), "foreground_notes": len(foreground)}

    pitches = [e["midi"] for e in foreground]
    intervals = [b - a for a, b in zip(pitches, pitches[1:])]
    if len(intervals) >= 8:
        n = len(intervals)
        moves = [i for i in intervals if i]
        facts.update(step_share=sum(1 <= abs(i) <= 2 for i in intervals) / n,
                     leap_share=sum(abs(i) >= 5 for i in intervals) / n,
                     large_leap_share=sum(abs(i) >= 9 for i in intervals) / n,
                     repeat_share=sum(i == 0 for i in intervals) / n,
                     turn_share=sum((a > 0) != (b > 0) for a, b in zip(moves, moves[1:])) / max(1, len(moves) - 1),
                     foreground_range=max(pitches) - min(pitches))
        components["melody"] = _mean([band(facts["step_share"], .40, .72, .20), band(facts["leap_share"], .08, .30, .15),
                                      band(facts["large_leap_share"], 0, .08, .10), band(facts["repeat_share"], 0, .20, .20),
                                      band(facts["turn_share"], .30, .65, .20), band(facts["foreground_range"], 9, 24, 8)])
    else:
        components["melody"] = 0.

    all_spans = _merged([(e["start_seconds"], e["start_seconds"] + e["duration_seconds"]) for e in events])
    silences = [all_spans[0][0]] + [b[0] - a[1] for a, b in zip(all_spans, all_spans[1:])] + [window - all_spans[-1][1]] if all_spans else [window]
    facts["longest_silence_seconds"] = max(silences)
    if len(foreground) >= 8:
        gaps = [(b["start_seconds"] - (a["start_seconds"] + a["duration_seconds"])) / beat for a, b in zip(foreground, foreground[1:])]
        spans = _merged([(e["start_seconds"], e["start_seconds"] + e["duration_seconds"]) for e in foreground])
        active = spans[-1][1] - spans[0][0]
        covered = sum(end - start for start, end in spans)
        foreground_silences = [spans[0][0]] + [b[0] - a[1] for a, b in zip(spans, spans[1:])] + [window - spans[-1][1]]
        facts.update(breath_share=sum(g >= .5 for g in gaps) / len(gaps),
                     foreground_rest_share=1 - covered / active if active > 0 else 1.,
                     longest_foreground_silence_seconds=max(foreground_silences))
        components["breathing"] = _mean([band(facts["breath_share"], bands["breath_floor"], bands["breath_ceiling"], .15),
                                         band(facts["foreground_rest_share"], bands["rest_floor"], bands["rest_ceiling"], .15),
                                         band(facts["longest_foreground_silence_seconds"], 0, 6, 6),
                                         band(facts["longest_silence_seconds"], 0, 2.5, 3)])
        onsets = sorted({round(e["start_beat"], 3) for e in foreground})
        iois = Counter(min(4., round((b - a) * 4) / 4) for a, b in zip(onsets, onsets[1:]) if b > a)
        facts.update(rhythm_entropy=_entropy(iois, 8),
                     top_gap_share=iois.most_common(1)[0][1] / sum(iois.values()) if iois else 1.,
                     offbeat_share=sum(abs(o - round(o)) > 1e-3 for o in onsets) / len(onsets))
        components["rhythm"] = _mean([band(facts["rhythm_entropy"], bands["entropy_floor"], .85, .25),
                                      band(facts["top_gap_share"], 0, bands["top_gap_ceiling"], .30),
                                      band(facts["offbeat_share"], .20, .60, .20)])
    else:
        components["breathing"] = components["rhythm"] = 0.

    trigrams = [tuple(intervals[i:i + 3]) for i in range(len(intervals) - 2)]
    phrases = [p for p in score.get("phrases", []) if p.get("lead_notes")]
    operations = [d.get("memory_operation") for d in decisions]
    if len(trigrams) >= 6:
        counts = Counter(trigrams)
        facts.update(motif_recurrence=sum(c for c in counts.values() if c > 1) / len(trigrams),
                     distinct_patterns=len(counts) / len(trigrams),
                     unique_phrase_share=len({p["lead_signature"] for p in phrases}) / len(phrases) if phrases else 0.,
                     developed_share=sum(op not in (None, "explore") for op in operations) / len(operations) if operations else 0.)
        components["motif"] = _mean([band(facts["motif_recurrence"], .25, .60, .20), band(facts["distinct_patterns"], .35, .80, .20),
                                     band(facts["unique_phrase_share"], .60, 1., .30), band(facts["developed_share"], .25, .65, .25)])
    else:
        components["motif"] = 0.

    def chord(root):
        return {midi(root + d, tonic, scale, 0) % 12 for d in (0, 2, 4)}
    if foreground:
        tones = [e["midi"] % 12 in chord(e["harmonic_root"]) for e in foreground]
        strong = [t for t, e in zip(tones, foreground) if abs(e["start_beat"] - round(e["start_beat"])) < 1e-6]
        starts = np.array([e["start_seconds"] for e in events])
        ends = starts + np.array([e["duration_seconds"] for e in events])
        notes = np.array([e["midi"] for e in events])
        dissonant = pairs = 0
        for onset in sorted({e["start_seconds"] for e in foreground}):
            heard = sorted(set(notes[(starts <= onset + 1e-6) & (ends > onset + 1e-6)].tolist()))
            for i, low in enumerate(heard):
                for high in heard[i + 1:]:
                    pairs += 1
                    dissonant += (high - low) % 12 in (1, 6, 11)
        steps = max(1, len(roots) - 1)
        facts.update(chord_tone_share=_mean(tones), strong_beat_chord_tone_share=_mean(strong) if strong else _mean(tones),
                     dissonant_pair_share=dissonant / pairs if pairs else 0.,
                     root_change_share=sum(a != b for a, b in zip(roots, roots[1:])) / steps,
                     distinct_roots=len(set(roots)),
                     fifth_motion_share=sum((a - b) % 7 == 4 for a, b in zip(roots, roots[1:])) / steps)
        components["harmony"] = _mean([band(facts["chord_tone_share"], .45, .85, .20),
                                       band(facts["strong_beat_chord_tone_share"], .60, 1., .30),
                                       band(facts["dissonant_pair_share"], 0, .12, .15), band(facts["root_change_share"], .55, .95, .20),
                                       band(facts["distinct_roots"], 4, 7, 2), band(facts["fifth_motion_share"], .12, .50, .12)])
    else:
        components["harmony"] = 0.

    # Console style fills the texture part with drums and the harmony part with
    # arpeggios, dozens of notes a second by design. Density and the form arc are
    # then judged on the lead, answer and bass, with ranges for that music.
    measured = [e for e in events if e["layer"] in ("lead", "answer", "bass")] if console else events
    facts["console_style"] = console
    layer_counts = Counter(e["layer"] for e in events)
    boundaries = sorted([(e["start_seconds"], 1) for e in measured] + [(e["start_seconds"] + e["duration_seconds"], -1) for e in measured],
                        key=lambda item: (item[0], item[1]))
    sounding = polyphony = 0
    for _, change in boundaries:
        sounding += change
        polyphony = max(polyphony, sounding)
    # How many pitched parts are sounding at once, averaged over time. Peak
    # polyphony over lead, answer and bass could never pass 2 — lead and answer
    # are call and response and overlap for 0.00 s — and a brief swell to five
    # voices said nothing about a piece that sat under two for most of its
    # length. Layering is what the listener hears across the whole piece.
    pitched = [e for e in events if e["layer"] in ("lead", "answer", "harmony", "bass")]
    edges = sorted([(e["start_seconds"], 1) for e in pitched] + [(e["start_seconds"] + e["duration_seconds"], -1) for e in pitched],
                   key=lambda item: (item[0], item[1]))
    sounding_now, last_time, voice_seconds = 0, 0., 0.
    for moment, change in edges:
        voice_seconds += sounding_now * (moment - last_time)
        last_time, sounding_now = moment, sounding_now + change
    facts["mean_voices"] = voice_seconds / window if window > 0 else 0.
    facts.update(notes_per_second=len(measured) / window,
                 roles_present_share=sum(layer_counts[r] >= .04 * max(1, len(events)) for r in ROLES) / len(ROLES),
                 roles_per_decision=_mean(len(d.get("sounding_roles", [])) for d in decisions),
                 max_polyphony=polyphony)
    if console:
        density, polyphony_band, roles_at_once = bands["density"], bands["polyphony"], bands["roles_at_once"]
        components["texture"] = _mean([band(facts["notes_per_second"], density[0], density[1], 4.),
                                       band(facts["roles_present_share"], .8, 1., .4),
                                       band(facts["roles_per_decision"], roles_at_once[0], roles_at_once[1], 1.),
                                       # Indie scores layering by the average; arcade keeps its peak band.
                                       band(facts["mean_voices"], *bands["voices"], 1.) if bands.get("voices")
                                       else band(facts["max_polyphony"], polyphony_band[0], polyphony_band[1], 3)])
    else:
        components["texture"] = _mean([band(facts["notes_per_second"], 2., 6.5, 2.), band(facts["roles_present_share"], 1., 1., .4),
                                       band(facts["roles_per_decision"], 2.5, 4.3, 1.), band(facts["max_polyphony"], 0, 9, 4)])

    quarter = window / 4
    quarters = [[e for e in measured if q * quarter <= e["start_seconds"] < (q + 1) * quarter] for q in range(4)]
    counts = np.array([len(q) for q in quarters], dtype=float)
    histograms = []
    for q in quarters:
        histogram = np.zeros(12)
        for e in q:
            histogram[e["midi"] % 12] += e["duration_seconds"]
        histograms.append(histogram)
    registers = [_mean(e["midi"] for e in q if e["layer"] in FOREGROUND) for q in quarters if any(e["layer"] in FOREGROUND for e in q)]
    velocities = [_mean(e["velocity"] for e in q) for q in quarters if q]
    facts.update(density_variation=float(counts.std() / counts.mean()) if counts.mean() else 0.,
                 velocity_range=max(velocities) - min(velocities) if velocities else 0.,
                 register_range=max(registers) - min(registers) if len(registers) > 1 else 0.,
                 return_similarity=_cosine(histograms[0], histograms[3]),
                 ends_on_tonic=bool(roots and roots[-1] == 0))
    components["form"] = _mean([band(facts["density_variation"], .12, .50, .20), band(facts["velocity_range"], .05, .30, .05),
                                band(facts["register_range"], 2, 9, 3), band(facts["return_similarity"], .60, .95, .30),
                                1. if facts["ends_on_tonic"] else .4])

    weighted = sum(weights[k] * components[k] for k in COMPONENTS) / sum(weights[k] for k in COMPONENTS)
    return {"reward": .75 * weighted + .25 * min(components.values()), "components": components, "facts": facts}


def _features(score):
    beat_onsets = sorted({round(e["start_beat"], 3) for e in score["events"] if e["layer"] in FOREGROUND})
    gaps = Counter(min(8, int(round((b - a) * 2))) for a, b in zip(beat_onsets, beat_onsets[1:]) if b > a)
    gap_histogram = np.array([gaps[i] for i in range(9)], dtype=float)
    contour = Counter(max(-7, min(7, b["midi"] - a["midi"])) for a, b in
                      zip(*(lambda line: (line, line[1:]))(sorted((e for e in score["events"] if e["layer"] in FOREGROUND),
                                                                   key=lambda e: e["start_seconds"]))))
    contour_histogram = np.array([contour[i] for i in range(-7, 8)], dtype=float)
    roles = np.array([sum(e["layer"] == r for e in score["events"]) for r in ROLES], dtype=float)
    parts = [gap_histogram, contour_histogram, roles, np.array([score["bpm"] / 120.])]
    return np.concatenate([p / (np.linalg.norm(p) or 1.) for p in parts])


def diversity(scores):
    """How different the performances for different seeds are, in 0..1.

    Uses key-independent features, so a transposed copy does not count as new music.
    """
    if len(scores) < 2:
        return 1., {"mean_similarity": 0., "shared_note_share": 0.}
    vectors = [_features(s) for s in scores]
    ledgers = [Counter((e["layer"], e["midi"] - TONICS.index(s["tonic"]), round(e["start_beat"], 2)) for e in s["events"])
               for s in scores]
    similarities, shared = [], []
    for i in range(len(scores)):
        for j in range(i + 1, len(scores)):
            similarities.append(_cosine(vectors[i], vectors[j]))
            common = sum((ledgers[i] & ledgers[j]).values())
            shared.append(common / max(1, min(sum(ledgers[i].values()), sum(ledgers[j].values()))))
    facts = {"mean_similarity": float(np.mean(similarities)), "shared_note_share": float(np.mean(shared))}
    return _mean([band(1 - facts["mean_similarity"], .06, 1., .06), band(facts["shared_note_share"], 0, .15, .30)]), facts


def fitness(scores, weights=None):
    """Mean critic reward across seeds, cut by up to 40% when the seeds all sound alike.

    A composer that plays one piece for every seed is a failure however well
    that piece scores, so variety carries real weight.
    """
    results = [evaluate(score, weights) for score in scores]
    variety, variety_facts = diversity(scores)
    mean_reward = _mean(r["reward"] for r in results)
    components = {k: _mean(r["components"][k] for r in results) for k in COMPONENTS}
    return {"fitness": mean_reward * (.6 + .4 * variety), "mean_reward": mean_reward, "diversity": variety,
            "diversity_facts": variety_facts, "components": components, "results": results}
