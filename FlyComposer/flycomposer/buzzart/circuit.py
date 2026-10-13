"""Buzzart's controller: the Fly rate circuit driven and read through a trained brain."""
from __future__ import annotations

import numpy as np

from ..controller import Controller, fingerprint
from .brain import CONTEXT


def seed_context(seed):
    """A per-seed 'mood' vector; a brain with zero context weights ignores it."""
    return np.random.default_rng(int(seed) + 50311).normal(0., 1., CONTEXT)


class BrainController(Controller):
    def __init__(self, circuit, seed, condition, brain):
        if condition not in ("connectome", "shuffled", "disconnected"):
            raise ValueError("Buzzart needs a simulated circuit condition")
        super().__init__(circuit, seed, condition)
        if brain.nodes != len(self.ids):
            raise ValueError("Brain was trained for a different circuit")
        self.brain = brain
        self.inputs, self.readout = brain.inputs, brain.readout
        self.context_input = brain.context @ seed_context(seed)
        # Rounding keeps the untrained leak's retention bit-identical to the original .72.
        self.retain = round(1. - brain.leak, 12)
        self.identity = {**self.identity, "model": "buzzart_rate_v1",
                         "input_projection_sha256": fingerprint(self.inputs),
                         "readout_sha256": fingerprint(self.readout),
                         "leak": brain.leak, "recurrent_gain": brain.gain, "readout_gain": brain.readout_gain,
                         "brain": brain.identity(), "seed_context": seed_context(seed).tolist()}

    def phrase(self, index, section_energy, root):
        brain = self.brain
        phase = index / 16
        feature = np.array([np.sin(2*np.pi*phase), np.cos(2*np.pi*phase),
                            np.sin(2*np.pi*index/4), np.cos(2*np.pi*index/4),
                            section_energy, root/6, 1., (index % 4)/3])
        recurrent_rms = 0.
        for step in range(32):
            drive = feature.copy()
            drive[0] += .12*np.sin(2*np.pi*step/16)
            recurrent = np.bincount(self.dst, weights=self.weights*self.state[self.src], minlength=len(self.state))
            self.state = self.retain*self.state + brain.leak*np.tanh(
                brain.gain*recurrent + self.inputs@drive + self.context_input + brain.bias)
            recurrent_rms += float(np.mean(recurrent**2))**.5 / 32
        output = np.tanh(brain.readout_gain*(self.readout@self.state))
        self.states.append(self.state.copy())
        self.trace.append({"phrase": index, "inputs": feature.tolist(), "outputs": output.tolist(),
                           "state_rms": float(np.mean(self.state**2))**.5,
                           "recurrent_rms": recurrent_rms})
        return output
