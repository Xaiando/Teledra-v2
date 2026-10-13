"""Publish Buzzart recordings into the Fly Composer listening library.

The same seed is rendered through the original Fly and one or more Buzzart
brains with one shared instrument palette and matched session RMS, then added
as a 'buzzart' comparison in library/index.json. Console palettes (NES-style
pulse, triangle and noise, or FM voices) are synthesized by ``chip.py``; the Fly
renderer still places and checks every note. The Fly Composer server keeps the
library in memory and would overwrite this entry, so it must be closed.
"""
from __future__ import annotations

import copy
import datetime as dt
import hashlib
import json
from pathlib import Path
import socket
import uuid

import numpy as np

from ..autonomous import write_performance
from ..experiment import Studio, atomic_json
from ..preferences import summarize_preferences
from ..render import ROOT, assess_performance, render_score, save_render
from ..vendor import teledra_synth as synth
from . import chip
from .critic import evaluate
from .performer import perform


def server_running(port=8789):
    with socket.socket() as probe:
        probe.settimeout(.5)
        return probe.connect_ex(("127.0.0.1", port)) == 0


def render(score, progress=print):
    """Render one performance; any chip voice routes every note through the synth chain."""
    if not chip.uses_chip_voices(score["voices"]):
        assess_performance(score)
        return render_score(score, progress=progress)
    # The Fly renderer only accepts catalog instruments, so validate a copy that
    # names a classic voice, and let the note hook synthesize the real voice —
    # the chip for chip parts, PyMusic for the rest.
    stand_in = copy.deepcopy(score)
    stand_in["voices"] = {role: "classic:sine" for role in stand_in["voices"]}
    for event in stand_in["events"]:
        event["voice"] = "classic:sine"
    stand_in["event_sha256"] = hashlib.sha256(json.dumps(stand_in["events"], sort_keys=True).encode()).hexdigest()
    pymusic = chip.pymusic_voices(score["voices"])
    # With a note hook the renderer never opens the PyMusic bridge itself, which
    # is why a console piece could not reach PyMusic at all. The chain opens it.
    if pymusic:
        synth._configure_bridge(ROOT / "bin/PyMusicSynth.exe")
    try:
        audio, stems, audit = render_score(stand_in, chip.note_factory(score["voices"], score["seed"], score),
                                           progress=progress)
    finally:
        if pymusic:
            synth._close_bridge()
    for entry in audit["events"]:
        entry["voice"] = score["voices"][entry["layer"]]
    audit["event_sha256"] = score["event_sha256"]
    audit["renderer"] = ("fly_explicit_pymusic_v1 chaining Buzzart chip voices with PyMusic "
                         f"({', '.join(sorted(pymusic.values()))})" if pymusic
                         else "fly_explicit_pymusic_v1 with Buzzart console voices")
    return audio, stems, audit


def publish(root, contenders, seed, seconds=75, voices=None, progress=print, port=8789, title=None):
    """Render and save one comparison.

    ``contenders`` is a list of (display name, description, brain or None for the
    original Fly, options). ``options`` may hold a songbook ``foundation`` and a
    console ``style``; the tuple may omit it.
    """
    if server_running(port):
        raise RuntimeError("Close the Fly Composer server first; it would overwrite the library index")
    root = Path(root)
    studio = Studio(root)
    styles = [entry[3].get("style") for entry in contenders if len(entry) > 3 and entry[3].get("style")]
    if voices is None:
        voices = dict(styles[0].palette) if styles else dict(summarize_preferences(studio.library)["preferred_voices"])
    if not chip.uses_chip_voices(voices):
        studio.validate(seed, "luminous", voices)
    else:
        # Chip parts are the chip synth's to check; the parts that fall through
        # the chain must still be real PyMusic patches.
        allowed = {voice["id"] for voice in studio.catalog}
        missing = sorted(v for v in chip.pymusic_voices(voices).values() if v not in allowed)
        if missing:
            raise ValueError(f"Not PyMusic patches: {', '.join(missing)}")
    rendered = []
    for entry in contenders:
        name, description, brain = entry[:3]
        options = entry[3] if len(entry) > 3 else {}
        if brain is None:
            score = write_performance(studio.circuit, seed, seconds, voices)
        else:
            score = perform(studio.circuit, brain, seed, seconds, voices, foundation=options.get("foundation"), style=options.get("style"))
        score["critic"] = evaluate(score)
        progress(f"Rendering {name} (seed {seed})")
        audio, stems, audit = render(score, progress=lambda message, name=name: progress(f"{name}: {message}"))
        peak = max([audit["raw_peak"]] + [float(np.max(np.abs(stem))) for stem in stems.values()])
        rendered.append((name, description, brain, options, score, audio, stems, audit, .82 * audit["raw_rms"] / peak))
    # One session target keeps loudness comparable, with headroom for the loudest contender.
    target = min(.055, *[item[-1] for item in rendered])
    created = dt.datetime.now(dt.timezone.utc).isoformat()
    batch_id = uuid.uuid4().hex[:16]
    tracks, variants = [], {}
    for name, description, brain, options, score, audio, stems, audit, _ in rendered:
        track_id = uuid.uuid4().hex[:16]
        audit = save_render(root / "library" / track_id, score, audio, stems, audit, target)
        critic = score["critic"]
        tracks.append({"id": track_id, "title": f"{name} · {seed}", "created_at": created, "duration": audit["duration_seconds"],
                       "condition": "buzzart" if brain else "connectome", "kind": "buzzart", "performer": score["performer"],
                       "description": description, "seed": seed, "ensemble": "buzzart", "voices": score["voices"],
                       "arrangement_version": score["arrangement_version"], "brain": brain.identity() if brain else None,
                       "foundation": score.get("buzzart", {}).get("foundation"), "style": score.get("buzzart", {}).get("style"),
                       "critic": {"reward": critic["reward"], "components": critic["components"]},
                       "metrics": {k: audit[k] for k in ("notes_scheduled", "notes_mixed", "layers", "sections", "peak", "rms",
                                                         "unique_lead_phrases", "pcm_sha256")},
                       "verdict": None, "note": "", "comparison_id": batch_id})
        variants[track_id] = {"display_name": name, "description": description}
    comparison = {"id": batch_id, "kind": "buzzart", "title": title or f"Buzzart training check · seed {seed}", "seed": seed,
                  "ensemble": "buzzart", "created_at": created, "revealed": True, "track_ids": [t["id"] for t in tracks],
                  "variants": variants, "feedback": {}, "rms_target": target, "recording_seconds": seconds,
                  "scope": "The same seed and instruments through the original Fly and Buzzart brains, at matched session RMS. "
                           "Critic scores are designed measurements; listening decides."}
    with studio.lock:
        candidate = copy.deepcopy(studio.library)
        candidate["tracks"].extend(tracks)
        candidate["comparisons"].append(comparison)
        atomic_json(studio.index, candidate)
    atomic_json(root / "reports" / f"buzzart-{batch_id}.json", comparison)
    return comparison
