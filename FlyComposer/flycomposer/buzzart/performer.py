"""Perform with Buzzart's brain through Buzzart's own note interpreter."""
from __future__ import annotations

from .brain import Brain
from .circuit import BrainController, seed_context  # noqa: F401  (re-exported for callers)
from .interpreter import write


def perform(circuit, brain, seed=2026, seconds=75, voices=None, condition="connectome", foundation=None, style=None, direction=None):
    """One Buzzart performance.

    ``foundation`` is an optional songbook :class:`Foundation`; ``style`` an
    optional :class:`ConsoleStyle`.
    """
    if not isinstance(brain, Brain):
        raise TypeError("perform needs a Buzzart Brain")
    return write(circuit, brain, seed, seconds, voices, condition, foundation, style, direction)
