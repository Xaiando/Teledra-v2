"""Execute bounded Python music authoring and export one device-free audio artifact."""
from __future__ import annotations

import argparse
import ast
import json
import math
import os
from pathlib import Path
import sys
import types
import wave

import numpy as np
import teledra_synth as synth


SYNTH_EXPORTS = (
    "synth_note", "instrument_note", "instruments", "INSTRUMENTS", "note_to_freq",
    "generate_wave", "adsr_envelope", "pink_noise", "mix_waves", "stereo_pan",
    "stereo_width", "automation_curve", "apply_automation", "soft_limiter", "delay",
    "lowpass_filter", "reverb", "granular_synthesis", "fit_to_length",
    "make_seamless_loop", "render_song", "play_sound",
)
NUMPY_FUNCTIONS = (
    "array", "asarray", "zeros", "zeros_like", "ones", "ones_like", "full", "full_like",
    "arange", "linspace", "sin", "cos", "tan", "tanh", "exp", "log", "log2", "log10",
    "sqrt", "abs", "absolute", "sign", "floor", "ceil", "round", "clip", "where",
    "minimum", "maximum", "mean", "sum", "max", "min", "std", "var", "any", "all",
    "isfinite", "isnan", "isinf", "diff", "gradient", "cumsum", "interp", "pad",
    "concatenate", "stack", "column_stack", "vstack", "hstack", "tile", "repeat",
    "reshape", "copy", "hanning", "hamming", "convolve", "roll", "sort", "unique",
)
ARRAY_ATTRIBUTES = {
    "shape", "size", "ndim", "dtype", "T", "real", "imag", "astype", "reshape",
    "flatten", "ravel", "copy", "clip", "max", "min", "mean", "sum", "std", "var",
    "any", "all", "tolist", "append", "extend", "items", "keys", "values", "get",
    "pop", "update", "join", "lower", "upper", "replace", "split", "strip",
    "seed", "default_rng", "normal", "uniform", "standard_normal", "rand", "randn",
    "randint", "integers", "choice", "random", "shuffle", "permutation",
    "fft", "ifft", "rfft", "irfft", "fftfreq", "rfftfreq",
}
ALLOWED_ATTRIBUTES = ARRAY_ATTRIBUTES | set(NUMPY_FUNCTIONS) | set(SYNTH_EXPORTS) | {
    "pi", "e", "newaxis", "float32", "float64", "int32", "int64", "bool_", "fft", "random",
} | {name for name in dir(math) if not name.startswith("_")}
MAX_ELEMENTS = synth.MAX_SAMPLES


def _shape_size(shape):
    if isinstance(shape, (int, np.integer)):
        return int(shape)
    return math.prod(int(value) for value in shape)


def _bound_size(size):
    if size < 0 or size > MAX_ELEMENTS:
        raise ValueError("requested array exceeds the audio render size bound")


def _numpy_call(name, function, *args, **kwargs):
    # Bound obvious constructors before allocation. Runtime timeout and the
    # owning process policy remain the limits for arbitrary computation.
    if name in ("zeros", "ones", "full"):
        _bound_size(_shape_size(args[0] if args else kwargs["shape"]))
    elif name in ("linspace",):
        _bound_size(int(args[2] if len(args) > 2 else kwargs.get("num", 50)))
    elif name in ("arange",) and args:
        start, end = (0, args[0]) if len(args) == 1 else args[:2]
        step = args[2] if len(args) > 2 else kwargs.get("step", 1)
        if step == 0:
            raise ValueError("arange step cannot be zero")
        _bound_size(max(0, math.ceil((end - start) / step)))
    elif name == "tile" and len(args) >= 2:
        _bound_size(np.size(args[0]) * _shape_size(args[1]))
    elif name == "repeat" and len(args) >= 2 and np.isscalar(args[1]):
        _bound_size(np.size(args[0]) * int(args[1]))
    result = function(*args, **kwargs)
    if isinstance(result, np.ndarray):
        _bound_size(result.size)
    return result


def _wrap_numpy(name, function):
    return lambda *args, **kwargs: _numpy_call(name, function, *args, **kwargs)


class _RandomFacade:
    def __init__(self, generator=None):
        self.generator = generator if generator is not None else np.random

    def __getattr__(self, name):
        if name == "default_rng":
            return lambda seed=None: _RandomFacade(np.random.default_rng(seed))
        if name not in {"seed", "normal", "uniform", "standard_normal", "rand", "randn",
                        "randint", "integers", "choice", "random", "shuffle", "permutation"}:
            raise AttributeError(name)
        function = getattr(self.generator, name)

        def call(*args, **kwargs):
            size = kwargs.get("size")
            if size is None:
                if name in ("rand", "randn"):
                    size = args
                elif name in ("standard_normal", "random") and args:
                    size = args[0]
                elif name in ("normal", "uniform", "randint", "integers") and len(args) > 2:
                    size = args[2]
            if size is not None:
                _bound_size(_shape_size(size))
            result = function(*args, **kwargs)
            if isinstance(result, np.ndarray):
                _bound_size(result.size)
            return result
        return call


def _module(name, values):
    module = types.ModuleType(name)
    module.__dict__.update(values)
    module.__all__ = tuple(values)
    return module


def _safe_modules():
    numpy = {name: _wrap_numpy(name, getattr(np, name)) for name in NUMPY_FUNCTIONS}
    numpy.update({name: getattr(np, name) for name in ("pi", "e", "newaxis", "float32", "float64", "int32", "int64", "bool_")})
    numpy["random"] = _RandomFacade()
    numpy["fft"] = _module("numpy.fft", {name: _wrap_numpy(name, getattr(np.fft, name))
                                           for name in ("fft", "ifft", "rfft", "irfft", "fftfreq", "rfftfreq")})
    return {
        "numpy": _module("numpy", numpy),
        "math": _module("math", {name: getattr(math, name) for name in dir(math) if not name.startswith("_")}),
        "teledra_synth": _module("teledra_synth", {name: getattr(synth, name) for name in SYNTH_EXPORTS}),
    }


def validate_source(source):
    if len(source.encode("utf-8")) > 256 * 1024:
        raise ValueError("composition source exceeds 256 KiB")
    tree = ast.parse(source, filename="composition.py")
    allowed_modules = {"numpy", "math", "teledra_synth"}
    forbidden = (ast.ClassDef, ast.AsyncFunctionDef, ast.Await, ast.AsyncFor, ast.AsyncWith,
                 ast.With, ast.Global, ast.Nonlocal, ast.Delete, ast.MatMult)
    for node in ast.walk(tree):
        if isinstance(node, forbidden):
            raise ValueError(f"unsupported composition operation: {type(node).__name__}")
        if isinstance(node, ast.Name) and node.id.startswith("__") and node.id != "__name__":
            raise ValueError("private runtime names are not available to compositions")
        if isinstance(node, ast.Attribute) and (node.attr.startswith("_") or node.attr not in ALLOWED_ATTRIBUTES):
            raise ValueError(f"unsupported composition attribute: {node.attr}")
        if isinstance(node, ast.Import):
            if any(alias.name not in allowed_modules for alias in node.names):
                raise ValueError("only numpy, math and teledra_synth imports are supported")
        if isinstance(node, ast.ImportFrom):
            if node.level or node.module not in allowed_modules:
                raise ValueError("only numpy, math and teledra_synth imports are supported")
            if any(alias.name.startswith("_") for alias in node.names):
                raise ValueError("private imports are not available to compositions")
    return tree


def _namespace():
    modules = _safe_modules()

    def importer(name, globals=None, locals=None, fromlist=(), level=0):
        if level or name not in modules:
            raise ImportError("composition imports are limited to numpy, math and teledra_synth")
        return modules[name]

    builtins = {
        "__import__": importer, "range": range, "len": len, "enumerate": enumerate, "zip": zip,
        "min": min, "max": max, "abs": abs, "sum": sum, "round": round, "sorted": sorted,
        "reversed": reversed, "float": float, "int": int, "str": str, "bool": bool,
        "list": list, "tuple": tuple, "dict": dict, "set": set, "isinstance": isinstance,
        "all": all, "any": any, "ValueError": ValueError, "RuntimeError": RuntimeError,
        "Exception": Exception, "print": lambda *args, **kwargs: print(*args, file=sys.stderr),
    }
    return {"__builtins__": builtins, "__name__": "__main__", "np": modules["numpy"]}


def _stereo_48k(array, sample_rate):
    array = synth._audio_array(array)
    if array.ndim == 1:
        array = np.column_stack((array, array))
    elif array.shape[1] == 1:
        array = np.repeat(array, 2, axis=1)
    if not 1000 <= sample_rate <= 192000:
        raise ValueError("captured sample rate must be 1000..192000 Hz")
    seconds = len(array) / sample_rate
    if not 4 <= seconds <= 600:
        raise ValueError(f"completed music must last 4..600 seconds, received {seconds:.3f}")
    if sample_rate != 48000:
        if sample_rate > 48000:
            spectrum = np.fft.rfft(array, axis=0)
            spectrum[np.fft.rfftfreq(len(array), 1 / sample_rate) >= 24000] = 0
            array = np.fft.irfft(spectrum, n=len(array), axis=0)
        # NumPy 1.x uses 32-bit default integers on Windows. Integer frame *
        # sample_rate overflows after about one second, scrambling later audio.
        destination = np.arange(int(seconds * 48000), dtype=np.float64) * (sample_rate / 48000.0)
        origin = np.arange(len(array))
        array = np.column_stack([np.interp(destination, origin, array[:, channel]) for channel in range(2)])
    return synth._protect_playback_peak(array)


def render_source(source, *, synth_exe=None):
    synth._note_requests = synth._note_renders = synth._play_requests = 0
    synth._song_requests = synth._song_renders = 0
    synth._song_audit = None
    tree = validate_source(source)
    captures = []
    calls = 0

    def capture(array, sr, loop):
        nonlocal calls
        calls += 1
        if calls > 1:
            raise ValueError("a composition must submit exactly one final play_sound array")
        captures.append((array.copy(), sr, loop))

    synth._capture_callback = capture
    namespace = _namespace()
    try:
        synth._configure_bridge(synth_exe)
        exec(compile(tree, "composition.py", "exec"), namespace, namespace)
        if calls != 1 or len(captures) != 1 or synth._play_requests != 1:
            raise ValueError("composition must finish by calling play_sound exactly once")
        if synth._note_requests != synth._note_renders:
            raise ValueError(f"incomplete note synthesis: {synth._note_renders}/{synth._note_requests} helper calls completed")
        array, sr, loop = captures[0]
        song_audit = synth._song_audit
        if synth._song_requests:
            if synth._song_requests != 1 or synth._song_renders != 1 or song_audit is None:
                raise ValueError("a song composition must successfully render exactly one complete song")
            from pymusic_score import audio_identity
            if sr != song_audit["sample_rate"] or audio_identity(array) != song_audit["pcm_sha256"]:
                raise ValueError("render_song audio was replaced or modified before capture; submit its complete returned array")
        completed = _stereo_48k(array, sr)
        peak = float(np.max(np.abs(completed)))
        rms = float(np.sqrt(np.mean(completed * completed)))
        if rms < 0.0001:
            raise ValueError("completed composition is effectively silent")
        metadata = namespace.get("TELEDRA_SCORE", {})
        if not isinstance(metadata, dict):
            raise ValueError("TELEDRA_SCORE metadata must be a dictionary")
        if song_audit is not None:
            # The renderer supplies this evidence; declarations in authored
            # Python cannot forge completion, title, sections or note counts.
            metadata = dict(title=song_audit["title"], key=song_audit["key"],
                            bpm=song_audit["bpm"], bars=song_audit["bars"],
                            sections=[section["name"] for section in song_audit["sections"]])
        report = {
            "ok": True, "title": str(metadata.get("title", namespace.get("TITLE", "Python composition")))[:160],
            "sample_rate": 48000, "channels": 2, "frames": len(completed),
            "duration": len(completed) / 48000, "duration_seconds": len(completed) / 48000,
            "peak": peak, "rms": rms, "loop": loop,
            "notes_requested": synth._note_requests, "notes_rendered": synth._note_renders,
            "note_audit_scope": "note helper calls completed (including explicit rests); arbitrary NumPy oscillators and final mix placement are not counted",
            "metadata": {key: metadata[key] for key in ("key", "bpm", "bars", "sections") if key in metadata},
        }
        if song_audit is not None:
            report["composition_audit"] = song_audit
        json.dumps(report, allow_nan=False)
        return completed, report
    finally:
        synth._capture_callback = None
        synth._close_bridge()


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--code", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--synth-exe", type=Path)
    args = parser.parse_args(argv)
    try:
        array, report = render_source(args.code.read_text(encoding="utf-8-sig"), synth_exe=args.synth_exe)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        temporary = args.output.with_suffix(args.output.suffix + ".partial")
        with wave.open(str(temporary), "wb") as output:
            output.setnchannels(2)
            output.setsampwidth(2)
            output.setframerate(48000)
            output.writeframes(np.round(array * 32767).astype("<i2").tobytes())
        os.replace(temporary, args.output)
        report["output"] = str(args.output.resolve())
        status = 0
    except Exception as error:
        report = {"ok": False, "error": f"{type(error).__name__}: {error}",
                  "notes_requested": synth._note_requests, "notes_rendered": synth._note_renders}
        status = 1
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2, allow_nan=False), encoding="utf-8")
    print(json.dumps(report, allow_nan=False))
    return status


if __name__ == "__main__":
    raise SystemExit(main())
