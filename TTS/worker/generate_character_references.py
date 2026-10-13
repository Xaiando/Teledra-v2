"""Bake court voice references from explicit local Kokoro assets.

This is an offline asset-authoring tool, not a runtime dependency. It needs
kokoro-onnx and onnxruntime, and never downloads models. Pass --model and
--voices to the local Kokoro v1.0 model and voice-bank files, and --roles to
choose which references to bake (all by default). An existing reference is
kept unless --replace is given; a replaced recording is moved to
assets/replaced/ rather than deleted.
"""

from __future__ import annotations

import argparse
import datetime
import hashlib
import importlib.metadata
import json
from pathlib import Path
import wave


SPECS = {
    "malthus": {
        "speaker": "am_eric",
        "language": "en-us",
        # 0.90 spoke at 18.7 characters a second, and LuxTTS carries a
        # reference's pace into every line: Malthus took 3.9 s for a line the
        # Queen speaks in 5.9 s. A measured prophet of scarcity should not rush.
        "speed": 0.80,
        "text": (
            "Every promise has a cost. Show me the evidence, name the weakest "
            "assumption, and choose a test that could prove us wrong."
        ),
    },
    "wizard": {
        "speaker": "bm_lewis",
        "language": "en-gb",
        "speed": 0.92,
        "text": (
            "The pattern is clear, but the mechanism still needs a careful test. "
            "I will measure the result before bringing my findings to the court."
        ),
    },
    # The Treasurer shared the Archivist's recording byte for byte, so the two
    # offices spoke with one voice.
    "treasurer": {
        "speaker": "bm_george",
        "language": "en-gb",
        "speed": 0.86,
        "text": (
            "The accounts are balanced, but only just. Before we spend a single "
            "coin, tell me what it buys and what it will cost us next season."
        ),
    },
}


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", required=True, type=Path)
    parser.add_argument("--voices", required=True, type=Path)
    parser.add_argument(
        "--output", type=Path,
        default=Path(__file__).resolve().parents[2] / "assets",
    )
    parser.add_argument("--roles", nargs="+", choices=sorted(SPECS), default=sorted(SPECS))
    parser.add_argument("--replace", action="store_true",
                        help="Move existing references to assets/replaced/ and bake new ones")
    args = parser.parse_args()
    if not args.model.is_file() or not args.voices.is_file():
        parser.error("Both local model and voice-bank files must exist")
    specs = {role: SPECS[role] for role in args.roles}
    outputs = [args.output / f"{role}_ref_clean.wav" for role in specs]
    manifest_path = args.output / "additional_voice_references.json"
    existing = [path for path in outputs if path.exists()]
    if existing and not args.replace:
        parser.error(f"Refusing to overwrite existing references without --replace: {existing}")

    import numpy as np
    import onnxruntime as ort
    from kokoro_onnx import Kokoro

    options = ort.SessionOptions()
    options.intra_op_num_threads = 2
    options.inter_op_num_threads = 1
    session = ort.InferenceSession(
        str(args.model), sess_options=options, providers=["CPUExecutionProvider"],
    )
    engine = Kokoro.from_session(session, str(args.voices))
    previous = (
        json.loads(manifest_path.read_text(encoding="utf-8")).get("references", {})
        if manifest_path.exists() else {}
    )
    manifest = {
        "generator": "TTS/worker/generate_character_references.py",
        "model": {"file": args.model.name, "sha256": digest(args.model)},
        "voice_bank": {"file": args.voices.name, "sha256": digest(args.voices)},
        "kokoro_onnx_version": importlib.metadata.version("kokoro-onnx"),
        "onnxruntime_version": ort.__version__,
        "provider": "CPUExecutionProvider",
        "source": "Kokoro v1.0 synthetic speaker presets; original reference text",
        "model_source": "https://huggingface.co/hexgrad/Kokoro-82M",
        "onnx_source": "https://github.com/thewh1teagle/kokoro-onnx",
        "processing": "Remove DC, normalize peak to 0.72, 5 ms edge fade, 100 ms silence each end; mono PCM16",
        "references": previous,
    }
    args.output.mkdir(parents=True, exist_ok=True)
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    for path in existing:
        backup = args.output / "replaced" / f"{path.stem}-{stamp}{path.suffix}"
        backup.parent.mkdir(exist_ok=True)
        path.replace(backup)
        print(f"kept the previous recording at {backup}", flush=True)
    for (role, spec), output in zip(specs.items(), outputs):
        audio, rate = engine.create(
            spec["text"], voice=spec["speaker"], speed=spec["speed"],
            lang=spec["language"], trim=True,
        )
        audio = np.asarray(audio, dtype=np.float32)
        if audio.ndim != 1 or not np.isfinite(audio).all():
            raise ValueError(f"Invalid reference audio for {role}")
        duration = audio.size / rate
        if not 2.0 <= duration <= 25.0:
            raise ValueError(f"Unexpected reference duration for {role}: {duration}")
        audio -= float(audio.mean())
        peak = float(np.max(np.abs(audio)))
        if peak < 0.001:
            raise ValueError(f"Silent reference for {role}")
        audio *= 0.72 / peak
        fade = round(rate * 0.005)
        ramp = np.linspace(0.0, 1.0, fade, dtype=np.float32)
        audio[:fade] *= ramp
        audio[-fade:] *= ramp[::-1]
        audio = np.pad(audio, round(rate * 0.10))
        pcm = (audio * 32767.0).astype("<i2")
        with wave.open(str(output), "wb") as recording:
            recording.setnchannels(1)
            recording.setsampwidth(2)
            recording.setframerate(rate)
            recording.writeframes(pcm.tobytes())
        manifest["references"][role] = {
            **spec, "file": output.name, "sha256": digest(output),
            "sample_rate": rate, "duration_seconds": round(audio.size / rate, 4),
        }
        print(f"{role}: {spec['speaker']}, {audio.size / rate:.2f}s -> {output}", flush=True)
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(f"Provenance: {manifest_path}", flush=True)


if __name__ == "__main__":
    main()
