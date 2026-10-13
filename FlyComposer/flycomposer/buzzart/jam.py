"""Let Buzzart iterate on its own: audition random seeds, keep the good ones.

Every seed is a different tune, so composing at random is free — but most of
what comes out is ordinary, and a loop that saved everything would bury the
good pieces in a library nobody wants to scroll. So the loop auditions
cheaply and keeps rarely.

Auditioning a seed means performing it as a note ledger and scoring it; that
costs a fraction of a second. Rendering it to audio costs far more. So nothing
is rendered until it has earned a place: a tune has to beat the critic's bar,
and then it has to be different enough from what is already saved. The second
test matters as much as the first — a composer that writes the same good tune
twenty times is not composing.

    ..\\.venv\\Scripts\\python.exe buzzart.py jam --brain brains\\buzzart-blended.npz --keep 8
"""
from __future__ import annotations

import random
from pathlib import Path

from ..connectome import load_circuit
from .critic import diversity, evaluate
from .performer import perform


def audition(circuit, brain, seed, seconds, foundation=None, style=None):
    """Perform one seed and score it, without rendering any audio."""
    score = perform(circuit, brain, seed, seconds, foundation=foundation, style=style)
    critic = evaluate(score)
    natural = foundation.fit(score)["fit"] if foundation else None
    return score, critic, natural


def novelty(score, kept):
    """How new this tune is against everything kept so far, in 0..1.

    The lowest pairwise score wins, so one near-duplicate is enough to fail it
    however fresh the rest of the library looks.
    """
    if not kept:
        return 1.
    return min(diversity([score, earlier])[0] for earlier in kept)


def jam(root, brain, *, foundation=None, style=None, keep=8, attempts=200, threshold=.82,
        novelty_floor=.55, min_voices=None, seconds=75, seed_range=(100_000, 999_999), rng_seed=None,
        publish_keeper=None, log=print):
    """Audition random seeds until ``keep`` tunes have been saved or attempts run out.

    Returns the kept records. ``publish_keeper`` renders and saves one keeper;
    leaving it None auditions only, which is the cheap way to calibrate the bar
    before spending render time.
    """
    root = Path(root)
    circuit = load_circuit(root / "data/circuit.npz")
    rng = random.Random(rng_seed)
    kept, kept_scores, tried = [], [], 0
    best_missed = None

    while len(kept) < keep and tried < attempts:
        tried += 1
        seed = rng.randint(*seed_range)
        score, critic, natural = audition(circuit, brain, seed, seconds, foundation, style)
        reward = critic["reward"]
        weakest = min(critic["components"].items(), key=lambda kv: kv[1])

        if reward < threshold:
            if best_missed is None or reward > best_missed[1]:
                best_missed = (seed, reward)
            log(f"  seed {seed}  reward {reward:.3f}  — below the bar (weakest {weakest[0]} {weakest[1]:.2f})")
            continue

        # Layering is one sub-score of one component, so a thin piece can clear
        # the overall bar: the first indie jam kept pieces averaging 1.49 and
        # 1.96 voices next to one at 3.62. When layers are the point, a thin
        # piece is refused however well the rest of it scores.
        voices = critic["facts"].get("mean_voices")
        if min_voices is not None and voices is not None and voices < min_voices:
            log(f"  seed {seed}  reward {reward:.3f}  voices {voices:.2f} — too thin")
            continue

        fresh = novelty(score, kept_scores)
        if fresh < novelty_floor:
            log(f"  seed {seed}  reward {reward:.3f}  novelty {fresh:.2f} — too close to one already kept")
            continue

        natural_text = f" natural {natural:.2f}" if natural is not None else ""
        voices_text = f" voices {voices:.2f}" if voices is not None else ""
        log(f"* seed {seed}  reward {reward:.3f}  novelty {fresh:.2f}{voices_text}{natural_text}  KEEP "
            f"({len(kept) + 1}/{keep})")
        record = {"seed": seed, "reward": reward, "novelty": fresh, "naturalness": natural, "mean_voices": voices,
                  "components": critic["components"]}
        if publish_keeper is not None:
            record["published"] = publish_keeper(seed, score)
        kept.append(record)
        kept_scores.append(score)

    log(f"auditioned {tried}, kept {len(kept)}")
    if not kept and best_missed:
        log(f"nothing cleared {threshold:.2f}; the best was seed {best_missed[0]} at {best_missed[1]:.3f}")
    return kept
