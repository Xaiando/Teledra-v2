"""Buzzart command line: train, evaluate, observe and showcase the fly brain that learns.

  .venv\\Scripts\\python.exe buzzart.py train --run first-drills
  .venv\\Scripts\\python.exe buzzart.py train --run first-drills --plan melody:20,integration:20   (resumes)
  .venv\\Scripts\\python.exe buzzart.py train --run arcade-1 --start brains\\buzzart.npz --songbook data\\foundation\\console.json --style data\\foundation\\console.json --plan arcade:60
  .venv\\Scripts\\python.exe buzzart.py evaluate --brain training\\buzzart\\first-drills\\best.npz
  ..\\.venv\\Scripts\\python.exe buzzart.py observe --run first-drills
  .venv\\Scripts\\python.exe buzzart.py showcase --run first-drills --seeds 90003 --untrained

Training writes only under training\\buzzart\\<run>. It never touches the
listening library, so it can run while the Fly Composer server is open.
``showcase`` writes the library and needs the server closed.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys

from flycomposer.buzzart.chip import PALETTES
from flycomposer.buzzart.console_style import TEMPERAMENTS

ROOT = Path(__file__).resolve().parent
RUNS = ROOT / "training" / "buzzart"


def _plan(text):
    from flycomposer.buzzart.trainer import DEFAULT_PLAN, DRILLS
    if not text:
        return DEFAULT_PLAN
    plan = []
    for item in text.split(","):
        name, _, count = item.partition(":")
        if name not in DRILLS or not count.isdigit() or int(count) < 1:
            raise SystemExit(f"Plan items look like drill:generations; drills are {', '.join(DRILLS)}")
        plan.append((name, int(count)))
    return plan


def _foundation_and_style(args):
    from flycomposer.buzzart.console_style import ConsoleStyle
    from flycomposer.buzzart.foundation import Foundation
    foundation = Foundation.load(args.songbook) if getattr(args, "songbook", None) else None
    style = (ConsoleStyle.load(args.style, palette=args.palette,
                               temperament=getattr(args, "temperament", "arcade"))
             if getattr(args, "style", None) else None)
    return foundation, style


def train(args):
    from flycomposer.buzzart.brain import Brain
    from flycomposer.buzzart.trainer import Trainer
    from flycomposer.connectome import load_circuit
    run_dir = RUNS / args.run
    brain = None
    if not (run_dir / "state.json").exists():
        brain = Brain.load(args.start) if args.start else Brain.from_controller(load_circuit(ROOT / "data/circuit.npz"), args.init_seed)
    elif args.songbook or args.style:
        print("Resuming: the songbook and style recorded when this run started are used.", flush=True)
    trainer = Trainer(ROOT, run_dir, brain, population=args.population, sigma=args.sigma, learning_rate=args.learning_rate,
                      seeds_per_candidate=args.seeds, seconds=args.seconds, workers=args.workers,
                      foundation_path=args.songbook, style_path=args.style,
                      temperament=args.temperament, palette=args.palette, naturalness_weight=args.naturalness,
                      log=lambda line: print(line, flush=True))
    trainer.run(_plan(args.plan), validate_every=args.validate_every)
    print(json.dumps({"run": str(run_dir), "generation": trainer.generation, "best_validation": trainer.best_validation}), flush=True)


def evaluate(args):
    import numpy as np
    from flycomposer.autonomous import write_performance
    from flycomposer.buzzart.brain import Brain
    from flycomposer.buzzart.critic import COMPONENTS, fitness
    from flycomposer.buzzart.performer import perform
    from flycomposer.connectome import load_circuit
    circuit = load_circuit(ROOT / "data/circuit.npz")
    foundation, style = _foundation_and_style(args)
    seeds = list(range(args.first_seed, args.first_seed + args.count))
    groups = [seeds[i:i + 4] for i in range(0, len(seeds), 4)]
    options = {"foundation": foundation, "style": style}
    suffix = (" + songbook" if foundation else "") + (" + console style" if style else "")
    untrained = Brain.from_controller(circuit, args.init_seed)
    contenders = {"original Fly (a random brain per seed)": lambda seed: write_performance(circuit, seed, args.seconds),
                  f"untrained Buzzart{suffix}": lambda seed: perform(circuit, untrained, seed, args.seconds, **options)}
    if args.brain:
        trained = Brain.load(args.brain)
        contenders[f"Buzzart {Path(args.brain).name} (gen {trained.metadata.get('generation')}){suffix}"] = (
            lambda seed: perform(circuit, trained, seed, args.seconds, **options))
    report = {}
    for name, author in contenders.items():
        results, natural = [], []
        for group in groups:
            scores = [author(seed) for seed in group]
            results.append(fitness(scores))
            if foundation:
                natural += [foundation.fit(score)["fit"] for score in scores]
        report[name] = {"fitness": float(np.mean([r["fitness"] for r in results])),
                        "reward": float(np.mean([r["mean_reward"] for r in results])),
                        "diversity": float(np.mean([r["diversity"] for r in results])),
                        "naturalness": float(np.mean(natural)) if natural else None,
                        "components": {k: float(np.mean([r["components"][k] for r in results])) for k in COMPONENTS}}
        row = report[name]
        natural_text = f" natural {row['naturalness']:.3f}" if row["naturalness"] is not None else ""
        print(f"{name:60s} fitness {row['fitness']:.3f} reward {row['reward']:.3f} diversity {row['diversity']:.2f}{natural_text} | "
              + " ".join(f"{k[:3]} {v:.2f}" for k, v in row["components"].items()), flush=True)
    if args.json:
        print(json.dumps(report, indent=2))


def observe_run(args):
    from flycomposer.buzzart.observe import observe
    summary = observe(ROOT, RUNS / args.run, seconds=args.seconds)
    print(json.dumps({"plots": summary.get("plots"),
                      "comparison": {name: {key: round(row[key], 3) for key in ("fitness", "reward", "diversity", "naturalness")
                                            if row.get(key) is not None}
                                     for name, row in summary["comparison"].items()}}, indent=2), flush=True)


def showcase(args):
    from flycomposer.buzzart.brain import Brain
    from flycomposer.buzzart.showcase import publish
    foundation, style = _foundation_and_style(args)
    options = {"foundation": foundation, "style": style}
    run_dir = RUNS / args.run
    contenders = [("Original Fly", "The untrained Fly circuit: random input and readout connections for this seed.", None)]
    if args.untrained:
        contenders.append(("Buzzart · untrained", "Buzzart's starting brain, before any drills.",
                           Brain.load(run_dir / "brains" / "gen-0000.npz"), options))
    brain = Brain.load(args.brain or run_dir / "best.npz")
    described = "Buzzart after evolution-strategies drills"
    described += " with a real-music songbook" if foundation else ""
    described += " in console style" if style else ""
    contenders.append((f"Buzzart · generation {brain.metadata.get('generation')}", described + ".", brain, options))
    for seed in args.seeds:
        comparison = publish(ROOT, contenders, seed, args.seconds, progress=lambda message: print(message, flush=True),
                             title=args.title)
        print(json.dumps({"published": comparison["id"], "title": comparison["title"], "tracks": comparison["track_ids"]}), flush=True)


def jam_command(args):
    from flycomposer.buzzart.brain import Brain
    from flycomposer.buzzart.jam import jam
    from flycomposer.buzzart.showcase import publish
    foundation, style = _foundation_and_style(args)
    brain = Brain.load(args.brain)
    options = {"foundation": foundation, "style": style}

    keeper = None
    if not args.audition_only:
        described = "Buzzart composing on its own"
        described += " from the blended songbook" if foundation else ""
        described += " in console style" if style else ""

        def keeper(seed, _score):
            comparison = publish(ROOT, [(f"Buzzart · jam", described + ".", brain, options)],
                                 seed, args.seconds, progress=lambda message: None,
                                 title=args.title or f"Buzzart jam · seed {seed}")
            return {"id": comparison["id"], "tracks": comparison["track_ids"]}

    kept = jam(ROOT, brain, foundation=foundation, style=style, keep=args.keep, attempts=args.attempts,
               threshold=args.threshold, novelty_floor=args.novelty, min_voices=args.min_voices, seconds=args.seconds,
               rng_seed=args.rng_seed, publish_keeper=keeper, log=lambda line: print(line, flush=True))
    print(json.dumps({"kept": [{k: v for k, v in record.items() if k != "components"} for record in kept]},
                     indent=2, default=float), flush=True)


def _add_style_options(parser):
    parser.add_argument("--songbook", help="songbook JSON (data/foundation/songbook.json, console.json or blended.json)")
    parser.add_argument("--style", help="console style JSON (data/foundation/console.json)")
    parser.add_argument("--palette", default="nes", choices=tuple(PALETTES),
                        help="voices when --style is set; indie chains FM lead and bass with PyMusic pad and keys")
    parser.add_argument("--temperament", default="arcade", choices=tuple(TEMPERAMENTS),
                        help="how hard the style drives: arcade is dense and fast, indie is mid-tempo with rests")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    t = commands.add_parser("train", help="run (or resume) evolution-strategies drills")
    t.add_argument("--run", required=True, help="run name under training/buzzart")
    t.add_argument("--plan", help="comma-separated drill:generations, e.g. foundation:30,melody:20")
    t.add_argument("--start", help="start from a saved brain instead of the original Fly projections")
    t.add_argument("--init-seed", type=int, default=2026)
    t.add_argument("--population", type=int, default=40)
    t.add_argument("--sigma", type=float, default=.10)
    t.add_argument("--learning-rate", type=float, default=.03)
    t.add_argument("--seeds", type=int, default=6, help="training seeds each candidate performs per generation")
    t.add_argument("--seconds", type=int, default=75)
    t.add_argument("--workers", type=int, default=None)
    t.add_argument("--validate-every", type=int, default=5)
    t.add_argument("--naturalness", type=float, default=.35, help="how much songbook naturalness scales fitness when a drill does not say")
    _add_style_options(t)
    t.set_defaults(handler=train)
    e = commands.add_parser("evaluate", help="compare original Fly, untrained and trained Buzzart on held-out seeds")
    e.add_argument("--brain")
    e.add_argument("--count", type=int, default=16)
    e.add_argument("--first-seed", type=int, default=90_000)
    e.add_argument("--seconds", type=int, default=75)
    e.add_argument("--init-seed", type=int, default=2026)
    e.add_argument("--json", action="store_true")
    _add_style_options(e)
    e.set_defaults(handler=evaluate)
    o = commands.add_parser("observe", help="learning curves, critic breakdown and piano rolls (needs matplotlib)")
    o.add_argument("--run", required=True)
    o.add_argument("--seconds", type=int, default=75)
    o.set_defaults(handler=observe_run)
    s = commands.add_parser("showcase", help="render Fly and Buzzart on the same seeds into the listening library")
    s.add_argument("--run", required=True)
    s.add_argument("--brain", help="defaults to the run's best brain")
    s.add_argument("--seeds", type=int, nargs="+", default=[90_003])
    s.add_argument("--seconds", type=int, default=75)
    s.add_argument("--untrained", action="store_true", help="also include the untrained starting brain")
    s.add_argument("--title", help="session title (default: Buzzart training check · seed N)")
    _add_style_options(s)
    s.set_defaults(handler=showcase)
    j = commands.add_parser("jam", help="compose at random on its own, keeping only good and distinct tunes")
    j.add_argument("--brain", required=True, help="the brain that composes, e.g. brains/buzzart-blended.npz")
    j.add_argument("--keep", type=int, default=8, help="how many tunes to save before stopping")
    j.add_argument("--attempts", type=int, default=200, help="how many random seeds to audition at most")
    j.add_argument("--threshold", type=float, default=.82, help="critic reward a tune must beat to be saved")
    j.add_argument("--novelty", type=float, default=.55, help="how different from already-kept tunes it must be")
    j.add_argument("--min-voices", type=float, default=None,
                   help="refuse pieces averaging fewer pitched voices at once (indie: try 2.8)")
    j.add_argument("--seconds", type=int, default=75)
    j.add_argument("--rng-seed", type=int, default=None, help="fix which random seeds get auditioned")
    j.add_argument("--audition-only", action="store_true", help="score without rendering audio, to calibrate the bar")
    j.add_argument("--title")
    _add_style_options(j)
    j.set_defaults(handler=jam_command)
    args = parser.parse_args()
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    args.handler(args)


if __name__ == "__main__":
    main()
