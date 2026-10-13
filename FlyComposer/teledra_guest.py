"""Buzzart plays one set for the Teledra court as its guest musician.

The court owns playback; this only composes and renders. It auditions a few
drawn grammars cheaply as note ledgers, keeps the first that holds together,
renders it to PCM16 stereo at 48 kHz, and prints one JSON line describing it.

    python teledra_guest.py --out DIR [--seed N] [--seconds 90]

Selection follows the listener's verdicts rather than taste invented here:
pieces need real layers (thin two-voice textures were the recurring complaint),
and palettes carrying more than one chip voice outside the drums are skipped,
because every "too noisy / too intense" note fell on chip-heavy renders.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import random
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))

GROOVE_NAMES = {"none": "no drums", "chip": "chip groove"}


def _chip_foreground(palette):
    from flycomposer.buzzart.chip import CHIP_VOICES
    return sum(1 for role, voice in palette.items() if role != "texture" and voice in CHIP_VOICES)


def _loop_safe(audio, rate, fade_in=.012, fade_out=.35):
    """The court loops every set and refuses a hard jump from the last sample
    back to the first. A short fade at each end lets the loop close on silence;
    the out-fade is long enough to sound like a release rather than a cut."""
    import numpy as np
    audio = np.array(audio, dtype=np.float64, copy=True)
    frames_first = audio.shape[0] > 2
    view = audio if frames_first else audio.T
    frames = view.shape[0]
    head = min(frames // 4, max(2, int(fade_in * rate)))
    tail = min(frames // 4, max(2, int(fade_out * rate)))
    view[:head] *= (np.linspace(0., 1., head) ** 2)[:, None]
    view[frames - tail:] *= (np.linspace(1., 0., tail) ** 2)[:, None]
    return audio


def archive_take(wav, report, archive_dir):
    """Keeps a compressed copy of a set: the court keeps only the newest
    twelve full-size takes (~60 MB each), but every set Buzzart plays is
    worth keeping. Writes `<time> - <title>.mp3` and a matching `.json` with
    the seed, so any set can also be rendered again exactly."""
    import re
    import soundfile as sf
    archive_dir.mkdir(parents=True, exist_ok=True)
    stamp = time.strftime("%Y-%m-%d %H%M%S")
    title = report["title"].removeprefix("Buzzart guest set: ")
    safe = re.sub(r"[^A-Za-z0-9 ,.-]+", "", title).strip()[:80]
    stem = archive_dir / f"{stamp} - {safe}"
    audio, rate = sf.read(str(wav), always_2d=True)
    # Highest VBR quality: about 190 kbps, ~2 MB for a 90-second set.
    sf.write(str(stem.with_suffix(".mp3")), audio, rate, format="MP3",
             compression_level=0.0, bitrate_mode="VARIABLE")
    stem.with_suffix(".json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    return stem.with_suffix(".mp3")


def _title(traits, score):
    groove = traits.get("drum_pattern") or ("chip" if traits["drums"] else "none")
    feel = GROOVE_NAMES.get(groove, groove.replace("_", "-"))
    return f"Buzzart guest set: {feel} in {score['tonic']} {score['mode']}, {score['bpm']} BPM"


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", required=True, help="empty or missing directory for this take")
    parser.add_argument("--seed", type=int, default=None)
    parser.add_argument("--seconds", type=int, default=90)
    parser.add_argument("--brain", default="brains/buzzart-indie-long.npz")
    parser.add_argument("--attempts", type=int, default=16)
    parser.add_argument("--threshold", type=float, default=.84)
    parser.add_argument("--min-voices", type=float, default=2.0)
    parser.add_argument("--archive", default=None,
                        help="folder that keeps a compressed MP3 copy of every set")
    args = parser.parse_args()
    started = time.time()

    from flycomposer.buzzart import genome
    from flycomposer.buzzart.brain import Brain
    from flycomposer.buzzart.console_style import ConsoleStyle
    from flycomposer.buzzart.critic import evaluate
    from flycomposer.buzzart.foundation import Foundation
    from flycomposer.buzzart.performer import perform
    from flycomposer.buzzart.showcase import render
    from flycomposer.connectome import load_circuit
    from flycomposer.render import save_render
    import numpy as np

    out = Path(args.out)
    if out.exists() and any(out.iterdir()):
        raise SystemExit(json.dumps({"ok": False, "error": f"{out} is not empty"}))
    circuit = load_circuit(ROOT / "data/circuit.npz")
    brain = Brain.load(ROOT / args.brain)
    console = json.loads((ROOT / "data/foundation/console.json").read_text(encoding="utf-8"))
    books = {}
    rng = random.Random(args.seed if args.seed is not None else time.time_ns())
    seed = args.seed if args.seed is not None else rng.randint(100_000, 999_999_999)

    best = None
    for attempt in range(args.attempts):
        candidate_seed = seed if attempt == 0 else rng.randint(100_000, 999_999_999)
        traits, palette = genome.draw(candidate_seed)
        if _chip_foreground(palette) > 1:
            continue
        book = traits["songbook"]
        if book not in books:
            books[book] = Foundation.load(ROOT / genome.SONGBOOKS[book])
        style = ConsoleStyle(console, palette=palette, temperament=traits)
        score = perform(circuit, brain, candidate_seed, args.seconds, foundation=books[book], style=style)
        critic = evaluate(score)
        voices = critic["facts"].get("mean_voices", 0.)
        record = (critic["reward"], candidate_seed, traits, score, critic, voices)
        if best is None or record[0] > best[0]:
            best = record
        if critic["reward"] >= args.threshold and voices >= args.min_voices:
            best = record
            break
    if best is None:
        raise SystemExit(json.dumps({"ok": False, "error": "no auditioned grammar was playable"}))

    reward, seed, traits, score, critic, voices = best
    score["critic"] = critic
    audio, stems, audit = render(score, progress=lambda message: None)
    audio = _loop_safe(audio, audit["sample_rate"])
    peak = max([audit["raw_peak"]] + [float(np.max(np.abs(stem))) for stem in stems.values()])
    target = min(.055, .82 * audit["raw_rms"] / peak)
    take = out / "take"
    out.mkdir(parents=True, exist_ok=True)
    save_render(take, score, audio, stems, audit, target)
    wav = take / "audio.wav"
    report = {
        "performer": "Buzzart",
        "title": _title(traits, score),
        "description": traits["description"],
        "seed": seed,
        "bpm": score["bpm"],
        "tonic": score["tonic"],
        "mode": score["mode"],
        "seconds": float(score["duration_seconds"]),
        "reward": round(float(reward), 4),
        "mean_voices": round(float(voices), 3),
        "brain_sha256": brain.identity()["brain_sha256"],
        "audio": str(wav),
        "audio_sha256": hashlib.sha256(wav.read_bytes()).hexdigest(),
        "voices": score["voices"],
        "render_seconds": round(time.time() - started, 1),
    }
    (out / "report.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    if args.archive:
        try:
            report["archived"] = str(archive_take(wav, report, Path(args.archive)))
        except Exception as error:  # a failed copy must never cost the court its set
            print(f"archive failed: {type(error).__name__}: {error}", file=sys.stderr, flush=True)
    print(json.dumps({"ok": True, "report": report}), flush=True)


if __name__ == "__main__":
    try:
        main()
    except SystemExit:
        raise
    except Exception as error:  # the court reads one JSON line whatever happens
        print(json.dumps({"ok": False, "error": f"{type(error).__name__}: {error}"}), flush=True)
        raise SystemExit(1)
