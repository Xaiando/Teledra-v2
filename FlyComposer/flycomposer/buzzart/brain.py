"""Buzzart's trainable brain.

The fly connectome coupling never changes. Training may adjust only parts the
original controller already treats as engineered: the 8-channel input
projection, the 8-channel readout, a seed-context projection, a per-cell-type
excitability bias and three bounded dynamics scalars (leak, recurrent gain and
readout gain). One brain is shared by every seed it performs.
"""
from __future__ import annotations

import datetime as dt
import hashlib
import io
import json
import math
from pathlib import Path

import numpy as np

from ..controller import Controller

FEATURES = 8
CONTEXT = 4
MODEL = "buzzart_brain_v1"
# Scalars are trained in an unbounded logit space and mapped into these ranges.
SCALAR_BOUNDS = {"leak": (.08, .60), "gain": (.55, 1.45), "readout_gain": (1.0, 4.0)}


def _logit(value, low, high):
    share = min(1 - 1e-9, max(1e-9, (float(value) - low) / (high - low)))
    return math.log(share / (1 - share))


def _squash(value, low, high):
    # Beyond ±40 the logistic is already flat to double precision; clamping avoids overflow.
    value = max(-40., min(40., float(value)))
    return low + (high - low) / (1 + math.exp(-value))


def _plain(value):
    return value.item() if hasattr(value, "item") else str(value)


class Brain:
    def __init__(self, inputs, readout, context, bias, leak=.28, gain=.97, readout_gain=2.2, metadata=None):
        self.inputs = np.array(inputs, dtype=float)
        self.readout = np.array(readout, dtype=float)
        self.context = np.array(context, dtype=float)
        self.bias = np.array(bias, dtype=float).reshape(-1)
        n = len(self.bias)
        if (n < 8 or self.inputs.shape != (n, FEATURES) or self.readout.shape != (FEATURES, n)
                or self.context.shape != (n, CONTEXT)):
            raise ValueError("Brain arrays do not describe one circuit")
        if not all(np.all(np.isfinite(array)) for array in (self.inputs, self.readout, self.context, self.bias)):
            raise ValueError("Brain contains non-finite values")
        for name, value in (("leak", leak), ("gain", gain), ("readout_gain", readout_gain)):
            low, high = SCALAR_BOUNDS[name]
            if not math.isfinite(float(value)) or not low <= float(value) <= high:
                raise ValueError(f"{name} must lie in {low}..{high}")
        self.leak, self.gain, self.readout_gain = float(leak), float(gain), float(readout_gain)
        self.metadata = dict(metadata or {})

    @property
    def nodes(self):
        return len(self.bias)

    @classmethod
    def from_controller(cls, circuit, seed=2026, context_scale=.3):
        """The untrained starting point: the original Fly projections for one seed.

        One shared brain gives every performance seed the same dynamics, and a
        seed's tiny initial-state noise alone converged to identical pieces. A
        sparse random context projection lets each seed's mood vector move the
        circuit elsewhere from the first generation. ``context_scale=0``
        reproduces the original Fly performance for ``seed`` exactly.
        """
        controller = Controller(circuit, seed)
        n = len(controller.ids)
        rng = np.random.default_rng(int(seed) + 104729)
        context = rng.normal(0., context_scale, (n, CONTEXT)) * (rng.random((n, CONTEXT)) < .5) if context_scale else np.zeros((n, CONTEXT))
        return cls(controller.inputs, controller.readout, context, np.zeros(n),
                   metadata={"initialized_from": {"controller": "leaky_tanh_rate_v1", "seed": int(seed),
                                                  "context_scale": float(context_scale)},
                             "generation": 0})

    def layout(self):
        n = self.nodes
        return (("inputs", (n, FEATURES)), ("readout", (FEATURES, n)), ("context", (n, CONTEXT)),
                ("bias", (n,)), ("scalars", (len(SCALAR_BOUNDS),)))

    def group_slices(self):
        slices, start = {}, 0
        for name, shape in self.layout():
            size = int(np.prod(shape))
            slices[name] = slice(start, start + size)
            start += size
        return slices

    def vector(self):
        scalars = [_logit(getattr(self, name), *SCALAR_BOUNDS[name]) for name in SCALAR_BOUNDS]
        return np.concatenate([self.inputs.ravel(), self.readout.ravel(), self.context.ravel(),
                               self.bias, np.array(scalars)])

    def with_vector(self, vector, metadata=None):
        vector = np.asarray(vector, dtype=float)
        slices = self.group_slices()
        if vector.shape != (slices["scalars"].stop,):
            raise ValueError("Parameter vector has the wrong size for this brain")
        n = self.nodes
        scalars = [_squash(u, *SCALAR_BOUNDS[name]) for u, name in zip(vector[slices["scalars"]], SCALAR_BOUNDS)]
        return Brain(vector[slices["inputs"]].reshape(n, FEATURES), vector[slices["readout"]].reshape(FEATURES, n),
                     vector[slices["context"]].reshape(n, CONTEXT), vector[slices["bias"]], *scalars,
                     metadata=dict(self.metadata) if metadata is None else metadata)

    def sha256(self):
        digest = hashlib.sha256()
        for name in ("inputs", "readout", "context", "bias"):
            digest.update(name.encode())
            digest.update(np.ascontiguousarray(getattr(self, name), dtype="<f8").tobytes())
        digest.update(json.dumps([self.leak, self.gain, self.readout_gain]).encode())
        return digest.hexdigest()

    def identity(self):
        return {"model": MODEL, "brain_sha256": self.sha256(), "trainable_parameters": int(self.vector().size),
                "leak": self.leak, "recurrent_gain": self.gain, "readout_gain": self.readout_gain,
                "generation": self.metadata.get("generation"),
                "fixed": "Connectome coupling, transmitter signs and normalization are unchanged; only engineered "
                         "projections, per-type bias and bounded dynamics scalars are trained."}

    def save(self, path, **metadata):
        path = Path(path)
        path.parent.mkdir(parents=True, exist_ok=True)
        buffer = io.BytesIO()
        np.savez(buffer, inputs=self.inputs, readout=self.readout, context=self.context, bias=self.bias,
                 scalars=np.array([self.leak, self.gain, self.readout_gain]))
        data = buffer.getvalue()
        pending = path.with_name(path.stem + ".next.npz")
        pending.write_bytes(data)
        pending.replace(path)
        self.metadata.update(metadata)
        manifest = {"model": MODEL, "file": path.name, "npz_sha256": hashlib.sha256(data).hexdigest(),
                    "brain_sha256": self.sha256(), "saved_at": dt.datetime.now(dt.timezone.utc).isoformat(),
                    "identity": self.identity(), "metadata": self.metadata}
        manifest_path = path.with_suffix(".json")
        pending = manifest_path.with_name(manifest_path.stem + ".next.json")
        pending.write_text(json.dumps(manifest, indent=2, allow_nan=False, default=_plain), encoding="utf-8")
        pending.replace(manifest_path)
        return manifest

    @classmethod
    def load(cls, path):
        path = Path(path)
        manifest = json.loads(path.with_suffix(".json").read_text(encoding="utf-8"))
        data = path.read_bytes()
        if hashlib.sha256(data).hexdigest() != manifest["npz_sha256"]:
            raise ValueError("Brain file does not match its manifest")
        with np.load(io.BytesIO(data), allow_pickle=False) as arrays:
            leak, gain, readout_gain = arrays["scalars"].tolist()
            brain = cls(arrays["inputs"], arrays["readout"], arrays["context"], arrays["bias"],
                        leak, gain, readout_gain, metadata=manifest.get("metadata"))
        if brain.sha256() != manifest["brain_sha256"]:
            raise ValueError("Brain values do not match their recorded identity")
        return brain
