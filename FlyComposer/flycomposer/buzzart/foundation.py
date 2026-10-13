"""Buzzart's musical foundation: a songbook of statistics learned from real music.

Built once from a public-domain corpus (see ``songbook_build.py``) and saved as
``data/foundation/songbook.json`` with a SHA-256 manifest. At performance time it
gives Buzzart's interpreter three things:

- vocabulary: melodic intervals, note lengths and rests keep the songbook's
  proportions. Options are laid out from smallest to largest along their
  cumulative frequencies, and each drive's rank among the performance's recent
  drives says where a note falls, so the circuit steers every choice while a
  whole piece keeps real practice, whatever range its drives happen to use;
- harmony: chord-root choices lean toward the transitions real chorales use;
- memory: each performance opens with a few real motifs to recall and develop.

It also measures how natural a performance's lines, rhythms and progressions
are under the same statistics, which the foundation drill uses as its reward.
"""
from __future__ import annotations

import bisect
from collections import deque
import hashlib
import json
import math
from pathlib import Path

import numpy as np

DEFAULT_ROOT_WEIGHT = .6
ROOT_FLOOR = .02
# How far below the corpus's own average log-probability a performance may fall before an area scores zero.
FIT_TOLERANCE = 1.5
# A drive is ranked against this many of the performance's latest drives. The window
# starts from evenly spread drives so the opening choices are defined.
RANK_WINDOW = 64
RANK_PRIOR = tuple(float(value) for value in np.linspace(-.9, .9, 9))


class _Choice:
    """Songbook options from smallest to largest, each owning a band of 0..1 as wide as its frequency.

    Shares spread evenly over 0..1 therefore reproduce the songbook's proportions.
    The first rule weighed log-frequency against distance from the drive; drives
    crowd toward -1 and 1, so rare long notes and long rests took over the lead.
    """

    def __init__(self, probabilities):
        items = sorted((float(value), float(weight)) for value, weight in probabilities.items() if float(weight) > 0)
        if not items:
            raise ValueError("A foundation choice table is empty")
        total = sum(weight for _, weight in items)
        self.values = [value for value, _ in items]
        self.edges, running = [], 0.
        for _, weight in items:
            running += weight / total
            self.edges.append(running)

    def at(self, share):
        index = bisect.bisect_right(self.edges, min(1., max(0., float(share))))
        return self.values[min(index, len(self.values) - 1)]


class _Ranker:
    """Where a drive falls among the performance's recent drives, as a share of 0..1."""

    def __init__(self):
        self.order = deque(RANK_PRIOR, maxlen=RANK_WINDOW)
        self.ranked = sorted(RANK_PRIOR)

    def share(self, drive):
        drive = float(drive)
        below, through = bisect.bisect_left(self.ranked, drive), bisect.bisect_right(self.ranked, drive)
        share = (below + .5 * (through - below) + .5) / (len(self.ranked) + 1)
        if len(self.order) == self.order.maxlen:
            del self.ranked[bisect.bisect_left(self.ranked, self.order[0])]
        self.order.append(drive)
        bisect.insort(self.ranked, drive)
        return share


class Choices:
    """One performance's vocabulary choices, calibrated to that performance's own drives."""

    def __init__(self, foundation):
        self.foundation = foundation
        self.rankers = {"interval": _Ranker(), "step": _Ranker(), "rest": _Ranker()}

    def interval(self, drive):
        return int(self.foundation.intervals.at(self.rankers["interval"].share(drive)))

    def step(self, drive):
        return self.foundation.steps.at(self.rankers["step"].share(drive))

    def rest(self, drive):
        return self.foundation.rests.at(self.rankers["rest"].share(drive))


class Foundation:
    def __init__(self, data, root_weight=DEFAULT_ROOT_WEIGHT, sha256=None):
        self.data = data
        self.root_weight = float(root_weight)
        self.sha256 = sha256 or hashlib.sha256(json.dumps(data, sort_keys=True).encode()).hexdigest()
        self.intervals = _Choice(data["intervals"])
        self.steps = _Choice(data["steps"])
        self.rests = _Choice(data["rests"])
        self.roots = data["root_transitions"]
        self.motifs = data["motifs"]
        if not self.motifs:
            raise ValueError("The songbook has no motifs")
        self._unigram = {int(k): float(v) for k, v in data["intervals"].items()}
        self._unigram_total = sum(self._unigram.values())
        self._bigrams = {int(a): {int(b): float(c) for b, c in row.items()} for a, row in data["interval_bigrams"].items()}
        self._bigram_totals = {a: sum(row.values()) for a, row in self._bigrams.items()}
        self._iois = sorted((float(k), float(v)) for k, v in data["iois"].items())
        self._ioi_keys = [k for k, _ in self._iois]
        self._ioi_total = sum(v for _, v in self._iois)

    @classmethod
    def load(cls, path, **options):
        path = Path(path)
        raw = path.read_bytes()
        manifest = json.loads(path.with_name(path.stem + ".manifest.json").read_text(encoding="utf-8"))
        digest = hashlib.sha256(raw).hexdigest()
        if digest != manifest["sha256"]:
            raise ValueError("Songbook does not match its manifest")
        return cls(json.loads(raw), sha256=digest, **options)

    # Choices for the interpreter.
    def session(self):
        """Fresh choices for one performance."""
        return Choices(self)

    # One-off choices read a drive of -1..1 directly as a share of 0..1.
    def interval(self, drive):
        return int(self.intervals.at((float(drive) + 1.) / 2.))

    def step(self, drive):
        return self.steps.at((float(drive) + 1.) / 2.)

    def rest(self, drive):
        return self.rests.at((float(drive) + 1.) / 2.)

    def root_bias(self, previous, candidate, mode):
        table = self.roots.get("major" if mode == "major" else "minor", {})
        row = table.get(str(int(previous)), {})
        return self.root_weight * math.log(float(row.get(str(int(candidate)), 0.)) + ROOT_FLOOR)

    def opening_memory(self, seed, mode, count=4):
        family = "major" if mode == "major" else "minor"
        pool = [motif for motif in self.motifs if motif.get("family") == family] or self.motifs
        rng = np.random.default_rng(int(seed) + 7919)
        chosen = sorted(rng.choice(len(pool), size=min(count, len(pool)), replace=False).tolist())
        return [{"phrase": None, "root": 0, "degrees": [int(d) for d in pool[i]["degrees"]],
                 "steps": [float(s) for s in pool[i]["steps"]], "uses": 0, "source": pool[i]["id"]} for i in chosen]

    # Token-level likelihoods, shared by the songbook builder and the foundation drill.
    def interval_logprobs(self, intervals):
        values = []
        for previous, current in zip(intervals, intervals[1:]):
            row = self._bigrams.get(previous, {})
            prior = self._unigram.get(current, 0.) / self._unigram_total
            values.append(math.log((row.get(current, 0.) + 2. * prior + 1e-4) / (self._bigram_totals.get(previous, 0.) + 2. + 1e-3)))
        return values

    def ioi_logprobs(self, iois):
        values = []
        for ioi in iois:
            at = bisect.bisect_left(self._ioi_keys, ioi)
            candidates = [i for i in (at - 1, at) if 0 <= i < len(self._iois)]
            nearest = min(candidates, key=lambda i: abs(self._ioi_keys[i] - ioi))
            values.append(math.log((self._iois[nearest][1] + .5) / (self._ioi_total + .5 * len(self._iois))))
        return values

    def root_logprobs(self, roots, family):
        table = self.roots.get(family, {})
        return [math.log(float(table.get(str(a), {}).get(str(b), 0.)) + ROOT_FLOOR) for a, b in zip(roots, roots[1:])]

    def identity(self):
        source = self.data.get("source", {})
        return {"model": "buzzart_songbook_v2", "songbook_sha256": self.sha256,
                "works": source.get("works"), "melodies": source.get("melodies"), "notes": source.get("notes"),
                "corpora": source.get("corpora"), "root_weight": self.root_weight,
                "choice": {"rule": "drive rank among the performance's recent drives, read on cumulative songbook frequencies",
                           "window": RANK_WINDOW}}

    def fit(self, score):
        """How natural a performance is under the songbook, per area and overall, in 0..1.

        Average log-probability per token of the foreground's scale-degree
        intervals (bigram), onset gaps and chord-root motion, compared with the
        corpus's own held-out average. Matching real music scores 1; falling
        ``FIT_TOLERANCE`` nats per token below it scores 0.
        """
        reference = self.data["reference"]
        foreground = sorted((e for e in score["events"] if e["layer"] in ("lead", "answer")), key=lambda e: e["start_seconds"])
        degrees = [e["scale_degree"] for e in foreground]
        intervals = [max(-7, min(7, b - a)) for a, b in zip(degrees, degrees[1:])]
        onsets = [e["start_beat"] for e in foreground]
        iois = [b - a for a, b in zip(onsets, onsets[1:]) if b > a]
        family = "major" if score.get("mode") == "major" else "minor"

        def closeness(values, expected):
            if not values:
                return 0.
            gap = expected - float(np.mean(values))
            return max(0., min(1., 1. - max(0., gap) / FIT_TOLERANCE))
        parts = {"intervals": closeness(self.interval_logprobs(intervals), reference["interval_logprob"]),
                 "rhythm": closeness(self.ioi_logprobs(iois), reference["ioi_logprob"]),
                 "harmony": closeness(self.root_logprobs(score.get("progression", []), family), reference["root_logprob"])}
        return {"fit": float(np.mean(list(parts.values()))), "parts": parts}
