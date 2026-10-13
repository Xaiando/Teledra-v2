"""Train the Organist's genome: learn which choices make good pieces in his style.

The Organist's melodies come from the court's language model, but every round
is framed by a genome drawn from Music/organist_style.json: key, mode, tempo,
pulse, chord progression, a coherent ensemble, a form and phrase shapes. This
learns a weight for each of those choices.

Each generation draws a batch of genomes with the court's own binary (so
drawing, expansion and validation are exactly what the court plays), renders
them with the court's PyMusic worker, measures every render with
critic_harness.py, and turns the measurements into one reward. Choice weights
follow each choice's average reward relative to the rest, bounded to
[WEIGHT_FLOOR, WEIGHT_CEILING] so no ensemble, key or form ever disappears:
the Organist should get better without collapsing into one sound.

At the deadline both the untrained and the trained style render the same fixed
held-out seeds; the trained style is promoted into the court only if it scores
clearly higher.

    .venv\\Scripts\\python.exe Music\\organist_train.py --run organist-1 --hours 8

Everything runs at idle CPU priority, so it only uses cycles nothing else wants.
"""
from __future__ import annotations

import argparse
import concurrent.futures as futures
import json
import math
import os
import shutil
import subprocess
import sys
import time
from collections import defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
WORKSPACE = HERE.parent
STYLE = HERE / "organist_style.json"
WEIGHT_FLOOR, WEIGHT_CEILING = 0.35, 3.0
# How strongly reward differences move weights, in reward units.
TEMPERATURE = 0.06
# A choice needs this many rendered pieces before its weight moves at all.
MIN_EVIDENCE = 4
HOLDOUT_FIRST = 9_000_000_000
KEEP_LISTENING = 12


def idle_priority():
    if os.name == "nt":
        import ctypes
        from ctypes import wintypes
        kernel32 = ctypes.windll.kernel32
        # Declared types matter: an undeclared HANDLE is truncated to 32 bits
        # on 64-bit Windows and the call silently changes nothing.
        kernel32.GetCurrentProcess.restype = wintypes.HANDLE
        kernel32.SetPriorityClass.argtypes = (wintypes.HANDLE, wintypes.DWORD)
        kernel32.SetPriorityClass(kernel32.GetCurrentProcess(), 0x40)


def clip(value, low=0.0, high=1.0):
    return max(low, min(high, value))


def bell(value, low, high, soft):
    """1 inside [low, high], falling to 0 over `soft` outside it."""
    if value is None:
        return 0.5
    if value < low:
        return clip(1 - (low - value) / soft)
    if value > high:
        return clip(1 - (value - high) / soft)
    return 1.0


def reward(symbolic: dict, audio: dict, notes: list[str]) -> tuple[float, dict]:
    """One number from the critic's measurements, and its parts.

    Weighted toward what makes the Organist distinct from Buzzart's grooves:
    harmony under the melody, a clear key, breathing phrases, sections that
    contrast and return, and pieces that keep their energy to the end."""
    s, a = symbolic, audio
    melody = s.get("melody", {})
    counts = s.get("quarter_note_counts") or []
    development = (min(counts[2:]) / max(counts)) if len(counts) == 4 and max(counts) else 0.0
    gap = s.get("most_common_gap") or {}
    parts = {
        "key": clip(((s.get("in_key_share") or 0) - 0.75) / 0.2),
        "consonance": 1 - clip(((s.get("dissonant_pair_share") or 0) - 0.12) / 0.25),
        "rhythm": 1 - clip(((gap.get("share") or 0) - 0.45) / 0.4),
        "harmony": bell(s.get("mean_polyphony_while_sounding"), 2.0, 4.5, 1.5),
        "breath": bell(s.get("silence_share"), 0.03, 0.25, 0.15),
        "contrast": clip((s.get("quarter_pitch_contrast") or 0) / 0.12),
        "return": clip(((s.get("return_similarity") or 0) - 0.3) / 0.5),
        "development": clip((development - 0.4) / 0.4),
        "melody": 1 - clip(((melody.get("longest_scale_run") or 0) - 4) / 4),
        # Silence every 2-3 s sounds like the piece stalling; a gap every 10 s
        # or more is phrasing. Level is a mastering matter, not a genome choice.
        "continuity": clip(((audio.get("seconds_per_gap") or 60) - 3) / 7),
        "dynamics": bell(audio.get("dynamic_range_db"), 8, 24, 10),
        "clean": 1 - clip((audio.get("clipped_share") or 0) / 0.002),
        "noise": 1 - clip(((audio.get("spectral_flatness") or 0) - 0.05) / 0.2),
    }
    weights = {"key": 1.0, "consonance": 1.2, "rhythm": 0.8, "harmony": 1.0, "breath": 0.8,
               "contrast": 1.4, "return": 0.6, "development": 1.2, "melody": 0.8,
               "continuity": 1.4, "dynamics": 0.8, "clean": 1.0, "noise": 0.6}
    score = sum(parts[k] * w for k, w in weights.items()) / sum(weights.values())
    # Level and gaps are already scored above; other flags cost a little each.
    flagged = [n for n in notes if not n.startswith(("quiet mix", "audio is below")) and "silent gaps" not in n]
    score = clip(score - 0.03 * len(flagged))
    return round(score, 4), {k: round(v, 3) for k, v in parts.items()}


def choices(genome: dict) -> dict[str, str]:
    """The weight-table key of every choice this genome made."""
    ensemble = genome["ensemble"]
    theme, answer = genome["theme"], genome["answer"]
    picked = {
        "mode": genome["mode"], "tempo": str(genome["bpm"]), "pulse": genome["pulse"],
        "tonic": genome["tonic"], "ensemble": ensemble, "form": str(genome["form_index"]),
        f"progression:{genome['mode']}": "-".join(map(str, genome["progression"])),
        "rhythm:theme": theme["rhythm"], "rhythm:answer": answer["rhythm"],
        "theme_start": str(theme["degrees"][0]), "theme_end": str(theme["degrees"][-1]),
        "answer_start": str(answer["degrees"][0]), "answer_end": str(answer["degrees"][-1]),
    }
    for role, voice in genome["voices"].items():
        picked[f"voice:{ensemble}:{role}"] = voice
    return picked


def phrase_steps(genome: dict) -> list[str]:
    steps = []
    for phrase in (genome["theme"], genome["answer"]):
        degrees = phrase["degrees"]
        steps += [str(b - a) for a, b in zip(degrees[:5], degrees[1:5])]
    return steps


def draw(synth: Path, first: int, count: int, style: Path) -> list[dict]:
    out = subprocess.run([str(synth), "--organist-genomes", str(first), str(count), str(style)],
                         capture_output=True, text=True, timeout=120,
                         creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    if out.returncode != 0:
        raise RuntimeError(f"genome draw failed: {out.stderr[-500:]}")
    return [json.loads(line) for line in out.stdout.splitlines() if line.strip()]


def render_and_score(item: dict, directory: str, synth: str, python: str) -> dict:
    """Render one genome with the court's worker and score it. Runs in a worker process."""
    idle_priority()
    sys.path.insert(0, str(HERE))
    import critic_harness as critic
    folder = Path(directory)
    folder.mkdir(parents=True, exist_ok=True)
    (folder / "code.py").write_text(item["code"], encoding="utf-8")
    started = time.time()
    with open(folder / "error.log", "w", encoding="utf-8") as log:
        done = subprocess.run(
            [python, str(HERE / "python_music_worker.py"), "--code", str(folder / "code.py"),
             "--output", str(folder / "audio.wav"), "--report", str(folder / "report.json"),
             "--synth-exe", synth],
            cwd=str(HERE), stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=log,
            timeout=180, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    result = {"seed": item["seed"], "genome": item["genome"], "dir": str(folder),
              "render_seconds": round(time.time() - started, 1)}
    if done.returncode != 0 or not (folder / "audio.wav").is_file():
        result.update(ok=False, reward=0.0, error=(folder / "error.log").read_text(encoding="utf-8")[-400:])
        return result
    report = json.loads((folder / "report.json").read_text(encoding="utf-8"))
    audio = critic.audio_metrics(folder / "audio.wav")
    audio.pop("_plot", None)
    duration = audio.get("seconds") or 0
    events = critic.events_from_audit((report.get("composition_audit") or {}).get("events") or [], duration)
    symbolic = critic.symbolic_metrics(events, duration) if events else {}
    notes = critic.observations({"symbolic": symbolic, "audio": audio, "report": {"loop": True},
                                 "capture": {"ok": True}})
    score, parts = reward(symbolic, audio, notes)
    result.update(ok=True, reward=score, parts=parts, observations=notes,
                  seconds=round(duration, 1), title=report.get("title"))
    return result


class Weights:
    """Running reward per choice, turned into bounded draw weights."""

    def __init__(self):
        self.total = defaultdict(lambda: defaultdict(float))
        self.count = defaultdict(lambda: defaultdict(int))
        self.all_total = 0.0
        self.all_count = 0

    def add(self, result: dict):
        if not result.get("ok"):
            return
        score = result["reward"]
        self.all_total += score
        self.all_count += 1
        genome = result["genome"]
        for dimension, key in choices(genome).items():
            self.total[dimension][key] += score
            self.count[dimension][key] += 1
        for step in phrase_steps(genome):
            self.total["step"][step] += score
            self.count["step"][step] += 1

    def table(self) -> dict:
        if not self.all_count:
            return {}
        mean = self.all_total / self.all_count
        table = {}
        for dimension, totals in self.total.items():
            row = {}
            for key, total in totals.items():
                n = self.count[dimension][key]
                if n < MIN_EVIDENCE:
                    continue
                # Shrink small samples toward the overall mean before comparing.
                shrunk = (total + mean * MIN_EVIDENCE) / (n + MIN_EVIDENCE)
                row[key] = round(clip(math.exp((shrunk - mean) / TEMPERATURE),
                                      WEIGHT_FLOOR, WEIGHT_CEILING), 4)
            if row:
                table[dimension] = row
        return table


def write_style(path: Path, weights: dict, meta: dict):
    style = json.loads(STYLE.read_text(encoding="utf-8"))
    style["weights"] = weights
    style["training"] = meta
    tmp = path.with_suffix(".tmp")
    tmp.write_text(json.dumps(style, indent=1) + "\n", encoding="utf-8")
    tmp.replace(path)


def keep_for_listening(run: Path, best: list[dict]):
    """The best pieces so far as MP3s with their scores, for a human to judge."""
    import soundfile as sf
    listening = run / "listening"
    listening.mkdir(exist_ok=True)
    wanted = set()
    for rank, result in enumerate(best[:KEEP_LISTENING], 1):
        genome = result["genome"]
        name = f"{result['reward']:.3f} {genome['ensemble']} {genome['tonic']} {genome['mode']} {genome['bpm']} - seed {result['seed']}"
        wanted.add(name)
        mp3 = listening / f"{name}.mp3"
        wav = Path(result["dir"]) / "audio.wav"
        if not mp3.exists() and wav.is_file():
            data, rate = sf.read(str(wav), always_2d=True)
            sf.write(str(mp3), data, rate, format="MP3", compression_level=0.0, bitrate_mode="VARIABLE")
            (listening / f"{name}.json").write_text(json.dumps(result, indent=1), encoding="utf-8")
    for old in listening.glob("*.mp3"):
        if old.stem not in wanted:
            old.unlink(missing_ok=True)
            old.with_suffix(".json").unlink(missing_ok=True)


def evaluate(pool, synth, python, run, style_path, label, count) -> float:
    items = []
    for seed in range(HOLDOUT_FIRST, HOLDOUT_FIRST + count):
        items += draw(synth, seed, 1, style_path)
    jobs = [pool.submit(render_and_score, item, str(run / "holdout" / label / str(item["seed"])),
                        str(synth), python) for item in items]
    scores = [job.result()["reward"] for job in jobs]
    return round(sum(scores) / max(len(scores), 1), 4)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--run", required=True)
    parser.add_argument("--hours", type=float, default=8.0)
    parser.add_argument("--workers", type=int, default=2)
    parser.add_argument("--batch", type=int, default=16)
    parser.add_argument("--synth", default=str(WORKSPACE / "target/release/teledra.exe"))
    parser.add_argument("--promote-to", default=str(Path(os.environ.get("LOCALAPPDATA", "."))
                                                      / "Teledra V2/music/organist_style.json"))
    parser.add_argument("--holdout", type=int, default=48, help="fixed seeds for the final comparison")
    parser.add_argument("--margin", type=float, default=0.015,
                        help="how much the trained style must beat the untrained one on held-out seeds")
    args = parser.parse_args()
    idle_priority()
    run = HERE / "training" / "organist" / args.run
    if run.exists():
        parser.error(f"{run} exists; choose a new --run name")
    (run / "bin").mkdir(parents=True)
    # A private copy: the court's launcher replaces its own binary on rebuild.
    synth = run / "bin" / "teledra-synth.exe"
    shutil.copy2(args.synth, synth)
    python = sys.executable
    deadline = time.time() + args.hours * 3600
    # Leave time at the end for the held-out comparison.
    train_until = deadline - min(1800, args.hours * 3600 * 0.12)
    weights = Weights()
    trained = run / "organist_style.trained.json"
    write_style(trained, {}, {"generation": 0})
    history = (run / "history.jsonl").open("a", encoding="utf-8")
    best: list[dict] = []
    status = run / "status.json"
    generation, seed = 0, 1_000_000_000
    with futures.ProcessPoolExecutor(max_workers=args.workers) as pool:
        while time.time() < train_until:
            if (run / "STOP").exists():
                break
            generation += 1
            items = draw(synth, seed, args.batch, trained)
            seed += args.batch
            jobs = [pool.submit(render_and_score, item, str(run / "renders" / f"g{generation:05d}-{item['seed']}"),
                                str(synth), python) for item in items]
            results = [job.result() for job in jobs]
            for result in results:
                weights.add(result)
                history.write(json.dumps({k: result.get(k) for k in
                                          ("seed", "ok", "reward", "parts", "observations", "seconds", "render_seconds", "error")}
                                         | {"generation": generation, "choices": choices(result["genome"]) if result.get("genome") else None}) + "\n")
            history.flush()
            best = sorted([r for r in best + results if r.get("ok")], key=lambda r: -r["reward"])[:KEEP_LISTENING]
            try:
                keep_for_listening(run, best)
            except Exception as error:  # listening copies never stop training
                print(f"listening copy failed: {error}", file=sys.stderr)
            # Renders are large; keep only the ones still worth hearing.
            keep_dirs = {r["dir"] for r in best}
            for result in results:
                if result["dir"] not in keep_dirs:
                    shutil.rmtree(result["dir"], ignore_errors=True)
            table = weights.table()
            write_style(trained, table, {"generation": generation, "pieces": weights.all_count,
                                         "mean_reward": round(weights.all_total / max(weights.all_count, 1), 4)})
            batch_mean = sum(r["reward"] for r in results) / max(len(results), 1)
            status.write_text(json.dumps({
                "phase": "training", "generation": generation, "pieces": weights.all_count,
                "failed": sum(not r.get("ok") for r in results),
                "batch_mean_reward": round(batch_mean, 4),
                "overall_mean_reward": round(weights.all_total / max(weights.all_count, 1), 4),
                "best": [{"reward": r["reward"], "seed": r["seed"], "ensemble": r["genome"]["ensemble"]} for r in best[:5]],
                "deadline": time.strftime("%Y-%m-%d %H:%M", time.localtime(deadline)),
                "updated": time.strftime("%Y-%m-%d %H:%M:%S"),
            }, indent=1), encoding="utf-8")
            print(f"generation {generation}: batch {batch_mean:.3f}, overall "
                  f"{weights.all_total / max(weights.all_count, 1):.3f}, pieces {weights.all_count}", flush=True)
        status.write_text(json.dumps({"phase": "held-out comparison", "generation": generation,
                                      "updated": time.strftime("%Y-%m-%d %H:%M:%S")}), encoding="utf-8")
        untrained = evaluate(pool, synth, python, run, STYLE, "untrained", args.holdout)
        learned = evaluate(pool, synth, python, run, trained, "trained", args.holdout)
    promoted = learned >= untrained + args.margin
    final = {"generations": generation, "pieces": weights.all_count, "held_out_seeds": args.holdout,
             "untrained_reward": untrained, "trained_reward": learned, "margin": args.margin,
             "promoted": promoted, "promoted_to": args.promote_to if promoted else None,
             "finished": time.strftime("%Y-%m-%d %H:%M:%S")}
    if promoted:
        Path(args.promote_to).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(trained, args.promote_to)
    (run / "final.json").write_text(json.dumps(final, indent=1), encoding="utf-8")
    status.write_text(json.dumps({"phase": "finished", **final}, indent=1), encoding="utf-8")
    print(json.dumps(final), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
