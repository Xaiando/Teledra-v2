"""Observation for a Buzzart training run: curves, critic breakdowns and piano rolls.

Everything here reads saved evidence (history.jsonl and saved brains) or
re-performs held-out seeds. Plots need matplotlib, which the Teledra
development environment provides; the scores and summary do not.
"""
from __future__ import annotations

import json
from pathlib import Path

import numpy as np

from ..autonomous import write_performance
from ..composition import ROLES
from ..connectome import load_circuit
from .brain import Brain
from .console_style import ConsoleStyle
from .critic import COMPONENTS, fitness
from .foundation import Foundation
from .performer import perform

COLOURS = {"lead": "#1f77b4", "answer": "#ff7f0e", "harmony": "#2ca02c", "bass": "#9467bd", "texture": "#8c564b"}


def read_history(run_dir):
    path = Path(run_dir) / "history.jsonl"
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]


def compare(circuit, authors, seeds, seconds=75, group=4, foundation=None):
    """Critic summary for each author over held-out seeds, grouped so diversity is measured.

    With a songbook, every author's naturalness under it is reported as well.
    """
    groups = [seeds[i:i + group] for i in range(0, len(seeds), group)]
    report = {}
    for name, author in authors.items():
        results, natural = [], []
        for chunk in groups:
            scores = [author(seed) for seed in chunk]
            results.append(fitness(scores))
            if foundation:
                natural += [foundation.fit(score)["fit"] for score in scores]
        report[name] = {"fitness": float(np.mean([r["fitness"] for r in results])),
                        "reward": float(np.mean([r["mean_reward"] for r in results])),
                        "diversity": float(np.mean([r["diversity"] for r in results])),
                        "naturalness": float(np.mean(natural)) if natural else None,
                        "components": {k: float(np.mean([r["components"][k] for r in results])) for k in COMPONENTS},
                        "facts": {key: float(np.mean([res["facts"][key] for r in results for res in r["results"]
                                                      if isinstance(res["facts"].get(key), (int, float))]))
                                  for key in results[0]["results"][0]["facts"]
                                  if isinstance(results[0]["results"][0]["facts"][key], (int, float))
                                  and not isinstance(results[0]["results"][0]["facts"][key], bool)}}
    return report


def observe(root, run_dir, seeds=tuple(range(90_000, 90_016)), seconds=75, roll_seed=90_003):
    root, run_dir = Path(root), Path(run_dir)
    out = run_dir / "observe"
    out.mkdir(exist_ok=True)
    circuit = load_circuit(root / "data/circuit.npz")
    history = read_history(run_dir)
    config = json.loads((run_dir / "state.json").read_text(encoding="utf-8")).get("config", {})
    # Buzzart performs the way the run trained: with its songbook and console style, if it had them.
    foundation = Foundation.load(config["foundation_path"]) if config.get("foundation_path") else None
    style = ConsoleStyle.load(config["style_path"]) if config.get("style_path") else None
    options = {"foundation": foundation, "style": style}
    start = Brain.load(run_dir / "brains" / "gen-0000.npz")
    best = Brain.load(run_dir / "best.npz")
    authors = {"Original Fly": lambda seed: write_performance(circuit, seed, seconds),
               "Buzzart at generation 0": lambda seed: perform(circuit, start, seed, seconds, **options),
               f"Buzzart best (gen {best.metadata.get('generation')})": lambda seed: perform(circuit, best, seed, seconds, **options)}
    report = compare(circuit, authors, list(seeds), seconds, foundation=foundation)
    scope = "Designed symbolic critic on held-out seeds. Listening decides musical quality."
    if style:
        scope += " Original Fly plays in its own style; Buzzart plays in console style, judged with the critic's console ranges."
    summary = {"run": run_dir.name, "generations": max((h["generation"] for h in history), default=0),
               "best_brain": best.identity(), "held_out_seeds": list(seeds),
               "songbook": foundation.identity() if foundation else None, "style": style.identity() if style else None,
               "comparison": report, "scope": scope}
    (out / "summary.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
    try:
        import matplotlib
        matplotlib.use("Agg")
        import matplotlib.pyplot as plt
    except ImportError:
        summary["plots"] = "matplotlib unavailable"
        return summary
    _plot_curves(plt, history, out / "learning_curves.png")
    _plot_components(plt, report, out / "critic_breakdown.png")
    rolls = {"Buzzart at generation 0": perform(circuit, start, roll_seed, seconds, **options),
             f"Buzzart best (gen {best.metadata.get('generation')})": perform(circuit, best, roll_seed, seconds, **options)}
    _plot_rolls(plt, rolls, out / f"piano_rolls_seed_{roll_seed}.png")
    summary["plots"] = [str(p) for p in sorted(out.glob("*.png"))]
    (out / "summary.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
    return summary


def _plot_curves(plt, history, path):
    steps = [h for h in history if "fitness_mean" in h]
    validation = [(h["generation"], h["validation"]["fitness"]) for h in history if "validation" in h]
    figure, (top, bottom) = plt.subplots(2, 1, figsize=(13, 8), sharex=True)
    if steps:
        generations = [h["generation"] for h in steps]
        top.plot(generations, [h["fitness_mean"] for h in steps], label="population mean (training seeds)", linewidth=1)
        top.plot(generations, [h["fitness_max"] for h in steps], label="best candidate", linewidth=.8, alpha=.6)
        for name in COMPONENTS:
            bottom.plot(generations, [h["component_means"][name] for h in steps], label=name, linewidth=1)
        bottom.plot(generations, [h["diversity_mean"] for h in steps], label="diversity", linestyle="--", linewidth=1)
        if any(h.get("naturalness_mean") is not None for h in steps):
            bottom.plot(generations, [np.nan if h.get("naturalness_mean") is None else h["naturalness_mean"] for h in steps],
                        label="naturalness", linestyle=":", color="black", linewidth=1.2)
        drill_starts = [(h["generation"], h["drill"]) for i, h in enumerate(steps) if i == 0 or steps[i - 1]["drill"] != h["drill"]]
        for axis in (top, bottom):
            for generation, drill in drill_starts:
                axis.axvline(generation - .5, color="grey", alpha=.3)
        for generation, drill in drill_starts:
            top.annotate(drill, xy=(generation, 1.), xycoords=("data", "axes fraction"), xytext=(3, -12),
                         textcoords="offset points", fontsize=8, color="dimgray")
    if validation:
        top.plot(*zip(*validation), "o-", color="black", label="held-out validation", markersize=3)
    top.set_ylabel("fitness")
    top.legend(fontsize=8, loc="lower right")
    top.grid(alpha=.3)
    bottom.set_ylabel("critic component (training mean)")
    bottom.set_xlabel("generation")
    bottom.legend(fontsize=7, ncol=4, loc="lower right")
    bottom.grid(alpha=.3)
    figure.suptitle("Buzzart training")
    figure.tight_layout()
    figure.savefig(path, dpi=90)
    plt.close(figure)


def _plot_components(plt, report, path):
    names = list(report)
    natural = any(row.get("naturalness") is not None for row in report.values())
    labels = [*COMPONENTS, "diversity", *(["naturalness"] if natural else []), "fitness"]
    width = .8 / len(names)
    figure, axis = plt.subplots(figsize=(13, 5))
    for i, name in enumerate(names):
        row = report[name]
        values = ([row["components"][k] for k in COMPONENTS] + [row["diversity"]]
                  + ([row.get("naturalness") or 0.] if natural else []) + [row["fitness"]])
        axis.bar(np.arange(len(labels)) + i * width, values, width, label=name)
    axis.set_xticks(np.arange(len(labels)) + width * (len(names) - 1) / 2, labels)
    axis.set_ylim(0, 1.05)
    axis.set_ylabel("critic score (held-out seeds)")
    axis.legend(fontsize=9)
    axis.grid(axis="y", alpha=.3)
    figure.tight_layout()
    figure.savefig(path, dpi=90)
    plt.close(figure)


def _plot_rolls(plt, scores, path):
    figure, axes = plt.subplots(len(scores), 1, figsize=(14, 3.8 * len(scores)), sharex=True, squeeze=False)
    for axis, (name, score) in zip(axes[:, 0], scores.items()):
        for role in ROLES:
            notes = [e for e in score["events"] if e["layer"] == role]
            for e in notes:
                axis.broken_barh([(e["start_seconds"], e["duration_seconds"])], (e["midi"] - .4, .8), facecolors=COLOURS[role])
            axis.plot([], [], color=COLOURS[role], linewidth=6, label=f"{role} ({len(notes)})")
        axis.set_title(f"{name} · seed {score['seed']} · {score['tonic']} {score['mode']} · {score['bpm']} BPM", fontsize=10)
        axis.set_ylabel("MIDI pitch")
        axis.legend(fontsize=8, ncol=5, loc="upper right")
        axis.grid(alpha=.25)
    axes[-1, 0].set_xlabel("seconds")
    figure.tight_layout()
    figure.savefig(path, dpi=80)
    plt.close(figure)
