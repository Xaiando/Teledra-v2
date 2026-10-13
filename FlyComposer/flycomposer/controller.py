"""Explicitly chosen leaky-rate dynamics over real fly connectivity.

This is a repurposed circuit controller, not an emulation of a fly's cognition.
Input/readout projections, gains, leak, and musical decoding are engineered.
"""
from __future__ import annotations

import hashlib
import numpy as np

CONDITIONS = ("connectome", "shuffled", "baseline")


def fingerprint(array):
    return hashlib.sha256(np.ascontiguousarray(array).tobytes()).hexdigest()


def shuffled_edges(src, dst, seed):
    """Keep within-type recurrence; swap inter-type edges preserving degrees."""
    src, result = np.asarray(src, dtype=np.int32), np.array(dst, dtype=np.int32, copy=True)
    rng = np.random.default_rng(seed)
    occupied = set(zip(src.tolist(), result.tolist()))
    eligible = np.flatnonzero(src != result)
    if len(eligible) < 2:
        raise ValueError('Insufficient inter-type edges for a wiring control')
    swaps = 0
    for _ in range(len(src) * 12):
        i, j = rng.choice(eligible, 2, replace=False)
        a, b, c, d = int(src[i]), int(result[i]), int(src[j]), int(result[j])
        if a == c or b == d or a == d or c == b or (a, d) in occupied or (c, b) in occupied:
            continue
        occupied.remove((a, b)); occupied.remove((c, d))
        occupied.add((a, d)); occupied.add((c, b))
        result[i], result[j] = d, b
        swaps += 1
    if swaps == 0:
        raise ValueError("Circuit cannot support a nontrivial directed wiring control")
    return result, swaps


class Controller:
    def __init__(self, circuit, seed, condition="connectome"):
        if condition not in (*CONDITIONS, "disconnected"):
            raise ValueError("Unknown controller")
        self.condition, self.seed = condition, int(seed)
        self.ids = np.asarray(circuit["node_ids"])
        self.src = np.asarray(circuit["src"], dtype=np.int32)
        self.dst = np.asarray(circuit["dst"], dtype=np.int32)
        weights = np.asarray(circuit["weights"], dtype=float)
        signs = np.asarray(circuit["signs"], dtype=float)
        n = len(self.ids)
        if n < 8 or len(self.src) < n or not np.all(weights > 0):
            raise ValueError("A connected, positive-count biological circuit is required")
        self.swaps = 0
        original_dst = self.dst.copy()
        if condition == "shuffled":
            self.dst, self.swaps = shuffled_edges(self.src, self.dst, seed + 19073)
        # One calibration from the REAL graph, used unchanged by all conditions.
        incoming = np.bincount(original_dst, weights=weights, minlength=n)
        scale = np.maximum(incoming, max(1., np.percentile(incoming, 30)))
        self.weights = weights * signs[self.src] / scale[self.dst]
        if condition == "disconnected":
            self.weights[:] = 0.
        rng = np.random.default_rng(seed + 8309)
        self.inputs = rng.normal(0, .35, (n, 8)) * (rng.random((n, 8)) < .30)
        self.readout = rng.normal(0, 1 / np.sqrt(n), (8, n))
        self.state = rng.normal(0, .015, n)
        self.trace, self.states = [], []
        self.identity = {
            "condition": condition, "model": "leaky_tanh_rate_v1", "seed": int(seed),
            "input_projection_sha256": fingerprint(self.inputs),
            "readout_sha256": fingerprint(self.readout), "node_ids_sha256": fingerprint(self.ids),
            "edge_destination_sha256": fingerprint(self.dst), "directed_edge_swaps": self.swaps,
            "shuffle_policy": "Inter-type directed degree-preserving swaps; original within-type edges and source weights fixed",
            "fixed_within_type_edges": int(np.sum(self.src == original_dst)),
            "leak": .28, "recurrent_gain": .97, "substeps_per_phrase": 32,
            "assumptions": "Synapse counts scaled into rate coupling; predicted transmitter signs; chosen tanh dynamics, input/readout and music decoder. No claim of biological musical ability.",
        }

    def phrase(self, index, section_energy, root):
        phase = index / 16
        feature = np.array([np.sin(2*np.pi*phase), np.cos(2*np.pi*phase),
                            np.sin(2*np.pi*index/4), np.cos(2*np.pi*index/4),
                            section_energy, root/6, 1., (index % 4)/3])
        if self.condition == "baseline":
            output = np.array([np.sin((index+1)*(j+1)*.63 + self.seed*.17)
                               for j in range(8)])
            recurrent_rms = 0.
        else:
            recurrent_rms = 0.
            for step in range(32):
                drive = feature.copy()
                drive[0] += .12*np.sin(2*np.pi*step/16)
                recurrent = np.bincount(self.dst, weights=self.weights*self.state[self.src],
                                        minlength=len(self.state))
                self.state = .72*self.state + .28*np.tanh(.97*recurrent + self.inputs@drive)
                recurrent_rms += float(np.mean(recurrent**2))**.5 / 32
            output = np.tanh(2.2*(self.readout@self.state))
        self.states.append(self.state.copy())
        self.trace.append({"phrase": index, "inputs": feature.tolist(), "outputs": output.tolist(),
                           "state_rms": float(np.mean(self.state**2))**.5,
                           "recurrent_rms": recurrent_rms})
        return output
