"""Evolution-strategies training ("drills") for Buzzart's brain.

Each generation perturbs the brain's parameter vector with mirrored Gaussian
noise, lets every candidate perform the same training seeds, scores them, and
moves the brain toward the better-ranked candidates (OpenAI-style ES with
centered ranks and Adam). Held-out seeds track whether improvement generalizes.

A run may perform with a songbook foundation and a console style. Fitness is
then the critic's score scaled by naturalness, how close the notes are to the
songbook's real music; each drill says how much naturalness counts.
"""
from __future__ import annotations

import datetime as dt
import hashlib
import json
import math
import os
from pathlib import Path
import time

import numpy as np

from ..connectome import load_circuit
from .brain import Brain
from .console_style import ConsoleStyle
from .critic import COMPONENTS, fitness
from .foundation import Foundation
from .performer import perform

DRILLS = {
    "foundation": {"weights": {}, "description": "The full critic, every area weighted evenly."},
    "melody": {"weights": {"melody": 3., "motif": 1.5},
               "description": "Stepwise lines with purposeful leaps, and motifs that return."},
    "rhythm": {"weights": {"rhythm": 3., "breathing": 1.5},
               "description": "Rhythm that lands on the beat as well as off it, with breaths between phrases."},
    "harmony": {"weights": {"harmony": 3.},
                "description": "Chord tones on strong beats, fewer clashes and root motion by fifths."},
    "form": {"weights": {"form": 3., "motif": 1.5},
             "description": "A density and dynamics arc, a recognizable return and an ending on the tonic."},
    "integration": {"weights": {}, "description": "The full critic again, to consolidate every drill."},
    "songbook": {"weights": {"melody": 1.5, "motif": 1.5}, "naturalness": .6,
                 "description": "Sound like real music: lines, rhythms and progressions close to the songbook."},
    "arcade": {"weights": {"rhythm": 1.5, "texture": 1.2, "form": 1.2}, "naturalness": .5,
               "description": "Console idioms: a catchy lead over driving drums, bass and arpeggios."},
    "texture": {"weights": {"texture": 3., "harmony": 1.5}, "naturalness": .4,
                "description": "Layers: parts sounding together, every role present, a full but unclouded texture."},
}
DEFAULT_PLAN = (("foundation", 30), ("melody", 30), ("rhythm", 25), ("harmony", 25), ("form", 25), ("integration", 45))
DEFAULT_NATURALNESS = .35

_WORKER = {}


def _init_worker(root, template, foundation_path=None, style_path=None, temperament="arcade", palette="nes"):
    _WORKER["circuit"] = load_circuit(Path(root) / "data/circuit.npz")
    _WORKER["template"] = template
    _WORKER["foundation"] = Foundation.load(foundation_path) if foundation_path else None
    # The temperament has to reach the workers, or training optimises against
    # arcade's standards while the operator believes they asked for indie.
    _WORKER["style"] = (ConsoleStyle.load(style_path, temperament=temperament, palette=palette)
                        if style_path else None)


def _evaluate(task):
    vector, seeds, seconds, weights, naturalness_weight = task
    try:
        brain = _WORKER["template"].with_vector(vector, metadata={})
        foundation, style = _WORKER.get("foundation"), _WORKER.get("style")
        scores = [perform(_WORKER["circuit"], brain, seed, seconds, foundation=foundation, style=style) for seed in seeds]
        result = fitness(scores, weights)
        naturalness = float(np.mean([foundation.fit(score)["fit"] for score in scores])) if foundation else None
        total = result["fitness"] if naturalness is None else result["fitness"] * ((1. - naturalness_weight) + naturalness_weight * naturalness)
        return {"fitness": total, "critic_fitness": result["fitness"], "naturalness": naturalness,
                "mean_reward": result["mean_reward"], "diversity": result["diversity"], "components": result["components"]}
    except Exception as error:  # An invalid candidate scores zero instead of stopping the generation.
        return {"fitness": 0., "critic_fitness": 0., "naturalness": None, "mean_reward": 0., "diversity": 0.,
                "components": {k: 0. for k in COMPONENTS}, "error": f"{type(error).__name__}: {error}"}


class _InlinePool:
    """Serial evaluation for tests and single-core machines."""

    def __init__(self, *initargs):
        _init_worker(*initargs)

    def map(self, function, tasks, chunksize=1):
        return map(function, tasks)

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False


def parameter_scales(brain):
    """Per-parameter noise scale, matched to how each group is initialized."""
    slices = brain.group_slices()
    scales = np.empty(slices["scalars"].stop)
    scales[slices["inputs"]] = .30
    scales[slices["readout"]] = 1. / math.sqrt(brain.nodes)
    scales[slices["context"]] = .30
    scales[slices["bias"]] = .20
    scales[slices["scalars"]] = .50
    return scales


def centered_ranks(values):
    ranks = np.empty(len(values))
    ranks[np.argsort(values, kind="stable")] = np.arange(len(values))
    return ranks / max(1, len(values) - 1) - .5


def _atomic_json(path, value):
    pending = path.with_name(path.name + ".next")
    pending.write_text(json.dumps(value, indent=2, allow_nan=False), encoding="utf-8")
    os.replace(pending, path)


def _file_sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest() if path else None


def _mean_or_none(values):
    values = [v for v in values if v is not None]
    return float(np.mean(values)) if values else None


class Trainer:
    def __init__(self, root, run_dir, brain=None, *, population=40, sigma=.10, learning_rate=.03, seeds_per_candidate=6,
                 seconds=75, workers=None, anchor_decay=.001, train_seeds=range(10_000, 12_000),
                 validation_seeds=range(90_000, 90_016), validation_group=4, rng_seed=20260914,
                 foundation_path=None, style_path=None, temperament="arcade", palette="nes",
                 naturalness_weight=DEFAULT_NATURALNESS, log=print):
        self.root, self.run_dir, self.log = Path(root), Path(run_dir), log
        self.run_dir.mkdir(parents=True, exist_ok=True)
        (self.run_dir / "brains").mkdir(exist_ok=True)
        if population < 2 or population % 2:
            raise ValueError("Population must be an even number of at least 2")
        self.population, self.sigma, self.learning_rate = int(population), float(sigma), float(learning_rate)
        self.seeds_per_candidate, self.seconds, self.anchor_decay = int(seeds_per_candidate), seconds, float(anchor_decay)
        self.workers = max(1, (os.cpu_count() or 2) - 2) if workers is None else int(workers)
        self.train_seeds, self.validation_seeds = list(train_seeds), list(validation_seeds)
        self.validation_group = int(validation_group)
        state_path = self.run_dir / "state.json"
        if state_path.exists():
            state = json.loads(state_path.read_text(encoding="utf-8"))
            self.template = Brain.load(self.run_dir / "latest.npz")
            arrays = np.load(self.run_dir / "optimizer.npz", allow_pickle=False)
            # The saved brain's scalars pass through a logit round trip; the optimizer keeps the exact vector.
            self.theta, self.anchor = arrays["theta"], arrays["anchor"]
            self.adam_m, self.adam_v = arrays["adam_m"], arrays["adam_v"]
            self.step, self.generation = state["step"], state["generation"]
            self.best_validation = state["best_validation"]
            self.rng = np.random.default_rng()
            self.rng.bit_generator.state = state["rng_state"]
            self.config = state["config"]
            self.foundation_path = self.config.get("foundation_path")
            self.style_path = self.config.get("style_path")
            self.temperament = self.config.get("temperament", "arcade")
            self.palette = self.config.get("palette", "nes")
            self.naturalness_weight = self.config.get("naturalness_weight", naturalness_weight)
            for label, path, recorded in (("songbook", self.foundation_path, self.config.get("foundation_sha256")),
                                          ("style", self.style_path, self.config.get("style_sha256"))):
                if path and _file_sha256(path) != recorded:
                    raise ValueError(f"The {label} file changed since this run started; start a new run instead")
            self.log(f"Resumed {self.run_dir.name} at generation {self.generation}")
        else:
            if brain is None:
                raise ValueError("A new run needs a starting brain")
            self.template = brain
            self.theta = brain.vector()
            self.anchor = self.theta.copy()
            self.adam_m = np.zeros_like(self.theta)
            self.adam_v = np.zeros_like(self.theta)
            self.step, self.generation, self.best_validation = 0, 0, -math.inf
            self.rng = np.random.default_rng(rng_seed)
            self.foundation_path = str(Path(foundation_path).resolve()) if foundation_path else None
            self.style_path = str(Path(style_path).resolve()) if style_path else None
            self.temperament, self.palette = temperament, palette
            self.naturalness_weight = float(naturalness_weight)
            self.config = {"population": self.population, "sigma": self.sigma, "learning_rate": self.learning_rate,
                           "seeds_per_candidate": self.seeds_per_candidate, "seconds": self.seconds,
                           "anchor_decay": self.anchor_decay, "train_seeds": [self.train_seeds[0], self.train_seeds[-1]],
                           "validation_seeds": self.validation_seeds, "rng_seed": rng_seed,
                           "foundation_path": self.foundation_path, "foundation_sha256": _file_sha256(self.foundation_path),
                           "style_path": self.style_path, "style_sha256": _file_sha256(self.style_path),
                           "temperament": self.temperament, "palette": self.palette,
                           "naturalness_weight": self.naturalness_weight,
                           "algorithm": "mirrored-sampling evolution strategies, centered ranks, Adam, decay toward the starting brain",
                           "started_at": dt.datetime.now(dt.timezone.utc).isoformat()}
        self.scales = parameter_scales(self.template)

    def brain(self, vector=None, **metadata):
        values = self.theta if vector is None else vector
        return self.template.with_vector(values, metadata={**self.template.metadata, "generation": self.generation, **metadata})

    def _pool(self):
        initargs = (str(self.root), self.template, self.foundation_path, self.style_path,
                    self.temperament, self.palette)
        if self.workers <= 1:
            return _InlinePool(*initargs)
        from concurrent.futures import ProcessPoolExecutor
        return ProcessPoolExecutor(max_workers=self.workers, initializer=_init_worker, initargs=initargs)

    def validate(self, pool):
        seeds = self.validation_seeds
        groups = [seeds[i:i + self.validation_group] for i in range(0, len(seeds), self.validation_group)]
        results = list(pool.map(_evaluate, [(self.theta, group, self.seconds, {}, self.naturalness_weight) for group in groups]))
        summary = {"fitness": float(np.mean([r["fitness"] for r in results])),
                   "critic_fitness": float(np.mean([r["critic_fitness"] for r in results])),
                   "naturalness": _mean_or_none(r["naturalness"] for r in results),
                   "mean_reward": float(np.mean([r["mean_reward"] for r in results])),
                   "diversity": float(np.mean([r["diversity"] for r in results])),
                   "components": {k: float(np.mean([r["components"][k] for r in results])) for k in COMPONENTS},
                   "errors": [r["error"] for r in results if "error" in r]}
        brain = self.brain(validation=summary)
        brain.save(self.run_dir / "brains" / f"gen-{self.generation:04d}.npz")
        if summary["fitness"] > self.best_validation:
            self.best_validation = summary["fitness"]
            brain.save(self.run_dir / "best.npz", best_validation=summary)
        return summary

    def _save_state(self, drill):
        self.brain(last_drill=drill).save(self.run_dir / "latest.npz")
        pending = self.run_dir / "optimizer.next.npz"
        with open(pending, "wb") as stream:
            np.savez(stream, theta=self.theta, anchor=self.anchor, adam_m=self.adam_m, adam_v=self.adam_v)
        os.replace(pending, self.run_dir / "optimizer.npz")
        _atomic_json(self.run_dir / "state.json", {
            "generation": self.generation, "step": self.step, "best_validation": self.best_validation,
            "rng_state": self.rng.bit_generator.state, "config": self.config, "last_drill": drill,
            "saved_at": dt.datetime.now(dt.timezone.utc).isoformat()})

    def _record(self, entry):
        with open(self.run_dir / "history.jsonl", "a", encoding="utf-8") as stream:
            stream.write(json.dumps(entry, allow_nan=False) + "\n")

    @staticmethod
    def _summary_text(summary):
        natural = f" natural {summary['naturalness']:.3f}" if summary.get("naturalness") is not None else ""
        return (f"fitness {summary['fitness']:.3f} reward {summary['mean_reward']:.3f} diversity {summary['diversity']:.2f}{natural} | "
                + " ".join(f"{k[:3]} {v:.2f}" for k, v in summary["components"].items()))

    def run(self, plan=DEFAULT_PLAN, validate_every=5):
        with self._pool() as pool:
            if self.generation == 0:
                summary = self.validate(pool)
                self._record({"generation": 0, "drill": "start", "validation": summary})
                self.log(f"gen    0 validation {self._summary_text(summary)}")
            for drill, generations in plan:
                if drill not in DRILLS:
                    raise ValueError(f"Unknown drill: {drill}")
                self.log(f"--- drill {drill}: {DRILLS[drill]['description']} ({generations} generations)")
                for _ in range(generations):
                    self._generation(pool, drill, validate_every)
                summary = self.validate(pool)
                self._record({"generation": self.generation, "drill": drill, "validation": summary, "drill_end": True})
                self.log(f"=== {drill} finished at gen {self.generation}: validation {self._summary_text(summary)} (best {self.best_validation:.3f})")
                self._save_state(drill)
        return self.brain()

    def _generation(self, pool, drill, validate_every):
        started = time.perf_counter()
        weights = DRILLS[drill]["weights"]
        naturalness_weight = DRILLS[drill].get("naturalness", self.naturalness_weight)
        half = self.population // 2
        noise = self.rng.standard_normal((half, self.theta.size))
        seeds = self.rng.choice(self.train_seeds, self.seeds_per_candidate, replace=False).tolist()
        offsets = self.sigma * self.scales * noise
        candidates = [self.theta + offset for offset in offsets] + [self.theta - offset for offset in offsets]
        results = list(pool.map(_evaluate, [(c, seeds, self.seconds, weights, naturalness_weight) for c in candidates], chunksize=2))
        values = np.array([r["fitness"] for r in results])
        ranks = centered_ranks(values)
        gradient = ((ranks[:half] - ranks[half:])[:, None] * noise).sum(axis=0) / (self.population * self.sigma)
        self.step += 1
        self.adam_m = .9 * self.adam_m + .1 * gradient
        self.adam_v = .999 * self.adam_v + .001 * gradient ** 2
        m_hat = self.adam_m / (1 - .9 ** self.step)
        v_hat = self.adam_v / (1 - .999 ** self.step)
        update = self.learning_rate * m_hat / (np.sqrt(v_hat) + 1e-8)
        self.theta = self.theta + self.scales * update - self.anchor_decay * (self.theta - self.anchor)
        self.generation += 1
        best = results[int(np.argmax(values))]
        entry = {"generation": self.generation, "drill": drill, "seeds": seeds,
                 "fitness_mean": float(values.mean()), "fitness_max": float(values.max()), "fitness_min": float(values.min()),
                 "reward_mean": float(np.mean([r["mean_reward"] for r in results])),
                 "diversity_mean": float(np.mean([r["diversity"] for r in results])),
                 "naturalness_mean": _mean_or_none(r["naturalness"] for r in results),
                 "naturalness_weight": naturalness_weight,
                 "best_components": best["components"],
                 "component_means": {k: float(np.mean([r["components"][k] for r in results])) for k in COMPONENTS},
                 "errors": sum("error" in r for r in results),
                 "update_norm": float(np.linalg.norm(update)),
                 "distance_from_start": float(np.linalg.norm((self.theta - self.anchor) / self.scales)),
                 "seconds": time.perf_counter() - started}
        if self.generation % validate_every == 0:
            entry["validation"] = self.validate(pool)
        self._record(entry)
        self._save_state(drill)
        natural = f" nat {entry['naturalness_mean']:.2f}" if entry["naturalness_mean"] is not None else ""
        line = (f"gen {self.generation:4d} [{drill}] fit {entry['fitness_mean']:.3f} (max {entry['fitness_max']:.3f}) "
                f"reward {entry['reward_mean']:.3f} div {entry['diversity_mean']:.2f}{natural} | "
                + " ".join(f"{k[:3]} {v:.2f}" for k, v in entry["component_means"].items())
                + f" | {entry['seconds']:.1f}s")
        if "validation" in entry:
            line += f" | VAL {entry['validation']['fitness']:.3f} (best {self.best_validation:.3f})"
        if entry["errors"]:
            line += f" | {entry['errors']} candidate errors"
        self.log(line)
