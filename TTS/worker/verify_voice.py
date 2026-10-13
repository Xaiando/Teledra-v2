"""Render and transcribe real worker output; never plays sound or starts a court.

Run with the repository venv. Reports and WAVs go to the supplied directory.
--compare-legacy reproduces the five-second clean-reference defect first.
"""
import argparse
import contextlib
import io
import json
import os
from pathlib import Path
import struct
import sys

import librosa
import numpy as np
import soundfile as sf

from voice_worker import AudioOut, REFERENCES, SAMPLE_RATE, Voice, reference_for


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--compare-legacy", action="store_true")
    parser.add_argument("--modes", nargs="+", default=["clean", "custom"])
    parser.add_argument("--voices", nargs="+", choices=sorted(REFERENCES), default=["queen"])
    parser.add_argument("--cpu", action="store_true", help="Leave the running court's GPU available")
    args = parser.parse_args()
    if args.cpu:
        # An empty environment value is removed by the Windows C runtime.
        os.environ["CUDA_VISIBLE_DEVICES"] = "-1"
    args.output.mkdir(parents=True, exist_ok=True)
    voice = Voice(str(Path(__file__).resolve().parent))
    text = "The silver flute carries the melody. The quiet organ leaves room for every phrase."
    cases = []
    for name in args.voices:
        modes = [None]
        if name == "queen":
            modes = (["legacy_clean"] if args.compare_legacy else []) + args.modes
        cases.extend((name, mode) for mode in modes)
    report = []
    for name, case in cases:
        if case is not None:
            os.environ["TELEDRA_QUEEN_VOICE"] = "clean" if case == "legacy_clean" else case
        voice._prompts.clear()
        if case == "legacy_clean":
            with contextlib.redirect_stdout(sys.stderr):
                voice._prompts["queen"] = voice._engine.encode_prompt(reference_for("queen", voice._base), duration=5)
        wire = io.BytesIO()
        voice.speak(name, text, AudioOut(wire))
        wire.seek(0)
        assert struct.unpack("<i", wire.read(4))[0] == SAMPLE_RATE
        frames = []
        while count := struct.unpack("<i", wire.read(4))[0]:
            frames.append(np.frombuffer(wire.read(count * 4), dtype="<f4"))
        assert not wire.read(), "unexpected extra protocol bytes"
        samples = np.concatenate(frames)
        assert np.isfinite(samples).all() and np.max(np.abs(samples)) > 0
        path = args.output / f"{case if name == 'queen' else name}.wav"
        sf.write(path, samples, SAMPLE_RATE)
        downsampled = librosa.resample(samples, orig_sr=SAMPLE_RATE, target_sr=16000)
        with contextlib.redirect_stdout(sys.stderr):
            transcript = voice._engine.transcriber(downsampled)["text"].strip()
        row = {"voice": name, "mode": case, "requested": text, "transcribed": transcript,
               "seconds": len(samples) / SAMPLE_RATE, "path": str(path.resolve())}
        report.append(row)
        print(json.dumps(row), flush=True)
    (args.output / "report.json").write_text(json.dumps(report, indent=2), encoding="utf-8")


if __name__ == "__main__":
    main()
