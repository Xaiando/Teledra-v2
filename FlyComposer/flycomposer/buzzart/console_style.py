"""Console idioms learned from the chiptune songbook, for Buzzart's console style.

Supplies what the Fly interpreter has no notion of: faster tempos, broken-chord
arpeggios in place of held chords, a pulsing bass, a kick/snare/hat grid, and
the registers and voices of classic sound chips. The circuit's drive still makes
every choice; the idiom tables say only what this style usually does.
"""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

from .chip import PALETTES

# (lowest, highest, base). A base must be a C: the interpreter spells scale degrees
# upward from it, and any other base would shift every pitch out of the key.
REGISTERS = {"lead": (64, 88, 72), "answer": (60, 84, 72), "harmony": (55, 86, 60),
             "bass": (28, 52, 36), "texture": (36, 84, 60)}
DRUM_PITCH = {"kick": 36, "snare": 60, "hat": 80}
DRUM_LENGTH = {"kick": .22, "snare": .2, "hat": .12}
DRUM_VELOCITY = {"kick": .62, "snare": .52, "hat": .34}
# Which drive channel sets each drum's density, and hits per bar from sparse to busy.
DRUM_DRIVE = {"kick": 7, "snare": 6, "hat": 5}
DRUM_DENSITY = {"kick": (2, 4), "snare": (1, 3), "hat": (4, 8)}

# How hard the style drives, and what the critic will accept from it.
#
# "Console style" began as one setting, and it was the loud one: it told the
# critic to require no breathing at all and to allow a single note length to
# fill 70% of a piece. Those are true of Mega Man and untrue of most game
# music, so everything trained under it came out relentless.
#
# ``indie`` is measured against the court's own kingdom tunes, which are the
# reference for this sound: 96 BPM, and a little over half of every part at
# rest. So it keeps the slower half of the chiptune corpus, thins the drum
# grid, slows the arpeggios, and asks the critic to *require* the space that
# arcade let drop to zero.
TEMPERAMENTS = {
    "arcade": {
        "tempo": None,              # the whole corpus, 72 to 180
        "arpeggio_max": None,
        "harmony": "arpeggio",
        "drum_scale": 1.,
        "description": "Driving console music: dense, fast, relentless by design.",
        "critic": {"breath_floor": 0., "breath_ceiling": .40, "rest_floor": 0., "rest_ceiling": .45,
                   "entropy_floor": .35, "top_gap_ceiling": .70,
                   "density": (3., 10.), "polyphony": (0, 6), "roles_at_once": (3., 5.)},
    },
    "indie": {
        "tempo": (72, 120),         # the melodic half; drops the 144-180 tail
        "arpeggio_max": 3,
        # Held chords, not arpeggios. Under arpeggios the harmony part's median
        # note was 0.30 beats, so five parts existed but sounded together only
        # 1.96 voices deep on average. A pad that rings under the melody is what
        # makes a slow piece feel layered.
        "harmony": "hold",
        "drum_scale": .6,
        "description": "Between retro games and lofi: mid tempo, warm, and full of rests.",
        # Centred on the kingdom tunes, not merely above the arcade floor. The
        # first attempt used .18/.30 and trained brains sat at .14/.32 — already
        # inside the band, so the critic had nothing left to ask for and
        # retraining moved nothing. A target has to be somewhere the music is
        # not yet standing.
        # Density is a range, not a target: the operator judged the current
        # busy-ness right and asked to keep "slightly less to slightly more"
        # available, so the band is wide enough to let seeds differ rather than
        # pushing every tune to one number.
        #
        # Polyphony is the real change. Arcade's band starts at 0, so two voices
        # sounding together scored a perfect 1.00 and the critic never once
        # asked for a third. Measured brains sat at exactly 2. Slow music is
        # carried by layers, so indie requires them.
        "critic": {"breath_floor": .25, "breath_ceiling": .60, "rest_floor": .32, "rest_ceiling": .62,
                   "entropy_floor": .45, "top_gap_ceiling": .50,
                   "density": (4.5, 9.), "polyphony": (3, 9), "roles_at_once": (3.5, 5.),
                   # Average pitched voices sounding, which replaces peak polyphony
                   # for indie. Held chords with the current brain measure 2.76,
                   # ranging 1.33 to 3.83 by seed: the floor sits just above the
                   # average so thin pieces are pushed up, and the top of the band
                   # is already reachable because the fullest seeds are there.
                   "voices": (2.8, 4.5)},
    },
    # Critic bands for each style are set from its own measurement with the
    # indie-long brain (twelve seeds each): floors a little above the average
    # and ceilings inside the range already reached, so a band both asks for
    # something and is reachable. Dreamy plays 1.9 notes a second; indie's
    # density floor of 4.5 would have scored it as a failure for being itself.
    "dusk": {
        "spans": (3., 4., 6., 8.), "presence": .05,
        "tempo": (72, 96),
        "arpeggio_max": 3,
        "harmony": "hold", "sevenths": True, "roll": (0., .04, .10), "hold_share": .75,
        "modes": ("dorian", "minor"),
        "drum_scale": .55,
        "description": "Lofi study beat: slow, jazzy seventh chords under an electric piano.",
        "critic": {"breath_floor": .18, "breath_ceiling": .55, "rest_floor": .30, "rest_ceiling": .60,
                   "entropy_floor": .45, "top_gap_ceiling": .50,
                   "density": (3., 6.), "polyphony": (3, 9), "roles_at_once": (3.5, 5.), "voices": (3.1, 5.)},
    },
    "overworld": {
        "spans": (2., 3., 4., 6.), "presence": .10,
        "tempo": (104, 130),
        "arpeggio_max": 3,
        "harmony": "hold", "roll": (.18, .26, .34),
        "modes": ("major",),
        "drum_scale": .95,
        "description": "Bright adventure in the Sonic manner: strummed guitar chords, marimba, a driving groove.",
        "critic": {"breath_floor": .22, "breath_ceiling": .58, "rest_floor": .32, "rest_ceiling": .60,
                   "entropy_floor": .45, "top_gap_ceiling": .50,
                   "density": (4.2, 8.), "polyphony": (3, 9), "roles_at_once": (3.5, 5.), "voices": (2.3, 4.)},
    },
    "dreamy": {
        "spans": (4., 6., 8.), "presence": -.05,
        "tempo": (72, 80),
        "arpeggio_max": 3,
        "harmony": "hold", "roll": (.10, .18, .28), "hold_share": .85,
        "modes": ("dorian", "major"),
        "bass": "hold", "drums": False,
        "drum_scale": 0.,
        "description": "Crystal cave: very slow, a sung pad, celesta and glockenspiel, no drums.",
        "critic": {"breath_floor": .16, "breath_ceiling": .55, "rest_floor": .30, "rest_ceiling": .62,
                   "entropy_floor": .40, "top_gap_ceiling": .55,
                   "density": (1.5, 3.5), "polyphony": (3, 9), "roles_at_once": (3.3, 5.), "voices": (2.7, 4.5)},
    },
    "complex": {
        "spans": (2., 3., 4.), "presence": .35,
        "tempo": (96, 150),
        "arpeggio_max": None,
        "harmony": "hold", "sevenths": True, "roll": (.06, .14, .22),
        "drum_scale": 1.3,
        "description": "Stress test: roughly twice indie's density and layering.",
        "critic": {"breath_floor": .20, "breath_ceiling": .55, "rest_floor": .25, "rest_ceiling": .55,
                   "entropy_floor": .50, "top_gap_ceiling": .45,
                   "density": (9., 18.), "polyphony": (5, 14), "roles_at_once": (4.5, 5.), "voices": (5.6, 9.)},
    },
}
DEFAULT_TEMPERAMENT = "arcade"
# What the critic asks of a performance with no style at all.
NO_STYLE_CRITIC = {"breath_floor": .12, "breath_ceiling": .40, "rest_floor": .15, "rest_ceiling": .45,
                   "entropy_floor": .45, "top_gap_ceiling": .45,
                   "density": (2., 6.5), "polyphony": (0, 5), "roles_at_once": (3., 5.)}


# Drum pattern families as 16-step grids per bar. The chiptune corpus gave one
# pattern vocabulary to every style, which is part of why they all grooved alike.
DRUM_PATTERNS = {
    "boom_bap": ({"kick": "1000000100100000", "snare": "0000100000001000", "hat": "1010101010101010"},
                 {"kick": "1000000100000100", "snare": "0000100000001000", "hat": "1010101010101011"}),
    "four_floor": ({"kick": "1000100010001000", "snare": "0000100000001000", "hat": "0010001000100010"},
                   {"kick": "1000100010001000", "snare": "0000100000001001", "hat": "1111111111111111"}),
    "breakbeat": ({"kick": "1000001000100000", "snare": "0000100100001001", "hat": "1010101010101010"},
                  {"kick": "1010000000100100", "snare": "0000100000001010", "hat": "1011101110111011"}),
    "half_time": ({"kick": "1000000000100000", "snare": "0000000010000000", "hat": "1000100010001000"},
                  {"kick": "1000000000000000", "snare": "0000000010000000", "hat": "1010101010101010"}),
    "shuffle": ({"kick": "1000001010000010", "snare": "0000100000001000", "hat": "1001001010010010"},
                {"kick": "1000000010000000", "snare": "0000100000001000", "hat": "1001101010011010"}),
    "sparse": ({"kick": "1000000000000000", "snare": "0000000000001000", "hat": "0000000010000000"},
               {"kick": "1000000000000000", "snare": "0000000000000000", "hat": "0010000000100000"}),
}


def critic_bands(style):
    """The critic's ranges for a performance, given its recorded style block."""
    if not style:
        return NO_STYLE_CRITIC
    if isinstance(style, dict) and isinstance(style.get("critic"), dict):
        # A drawn grammar carries its own expectations: an ambient piece with no
        # drums is not a failed dance track.
        return style["critic"]
    name = style.get("temperament") if isinstance(style, dict) else None
    return TEMPERAMENTS.get(name or DEFAULT_TEMPERAMENT, TEMPERAMENTS[DEFAULT_TEMPERAMENT])["critic"]


class ConsoleStyle:
    def __init__(self, data, palette="nes", temperament=DEFAULT_TEMPERAMENT, sha256=None):
        idioms = data["idioms"]
        self.data = data
        self.sha256 = sha256 or hashlib.sha256(json.dumps(data, sort_keys=True).encode()).hexdigest()
        if isinstance(temperament, dict):
            # A grammar drawn for one tune rather than a named preset.
            self.temperament = temperament.get("name", "genome")
            self.traits = dict(temperament)
        elif temperament not in TEMPERAMENTS:
            raise ValueError(f"Unknown temperament {temperament!r}; choose from {', '.join(TEMPERAMENTS)}")
        else:
            self.temperament = temperament
            self.traits = TEMPERAMENTS[temperament]
        self.tempos = sorted(idioms["tempo_bpm"]) or [120]
        span = self.traits["tempo"]
        if span:
            # Keep the corpus's own tempos, just the ones this temperament uses.
            kept = [t for t in self.tempos if span[0] <= t <= span[1]]
            self.tempos = kept or [max(span[0], min(span[1], self.tempos[len(self.tempos) // 2]))]
        profiles = idioms["drums"]["step_probability"]
        # Each drum's step profile relative to its busiest step, so thresholds mean the same for every drum.
        self.profiles = {name: [value / max(max(values), 1e-9) for value in values] for name, values in profiles.items()}
        rates = sorted(int(rate) for rate in idioms["arpeggio"].get("notes_per_beat", {})) or [4]
        cap = self.traits["arpeggio_max"]
        if cap:
            # A 4-notes-per-beat arpeggio under a slow tune is still a fast tune.
            rates = [min(rate, cap) for rate in rates]
        self.arpeggio_rates = (rates[0], rates[-1])
        if isinstance(palette, dict):
            self.palette_name = self.temperament
            self.palette = dict(palette)
        else:
            self.palette_name = palette
            self.palette = dict(PALETTES[palette])
        self.registers = REGISTERS

    @classmethod
    def load(cls, path, **options):
        path = Path(path)
        raw = path.read_bytes()
        manifest = json.loads(path.with_name(path.stem + ".manifest.json").read_text(encoding="utf-8"))
        digest = hashlib.sha256(raw).hexdigest()
        if digest != manifest["sha256"]:
            raise ValueError("Console songbook does not match its manifest")
        return cls(json.loads(raw), sha256=digest, **options)

    def tempo(self, drive):
        share = min(.999999, max(0., (float(drive) + 1.) / 2.))
        return int(round(self.tempos[int(share * len(self.tempos))]))

    def arpeggio_rate(self, drive):
        return self.arpeggio_rates[1] if float(drive) > 0 else self.arpeggio_rates[0]

    def bar_grid(self, drive):
        """One bar's kick/snare/hat grid, step -> drums.

        Each drum takes a count of hits within the density range of the source
        music, on the steps where it most often played. A threshold on the
        profiles alone let a syncopated kick profile fire on most steps.
        """
        grid = {}
        family = self.traits.get("drum_pattern")
        if family:
            # A named groove: the drive picks between its two bars and thins the hats.
            choice = DRUM_PATTERNS[family][0 if float(drive[DRUM_DRIVE["kick"]]) < 0 else 1]
            keep_hats = float(drive[DRUM_DRIVE["hat"]]) > -.5
            for name, pattern in choice.items():
                if name == "hat" and not keep_hats:
                    pattern = "".join(c if i % 4 == 0 else "0" for i, c in enumerate(pattern))
                for step, hit in enumerate(pattern):
                    if hit == "1":
                        grid.setdefault(step, []).append(name)
            return grid
        scale = self.traits["drum_scale"]
        for name, profile in self.profiles.items():
            low, high = DRUM_DENSITY[name]
            count = int(round(scale * (low + (high - low) * (float(drive[DRUM_DRIVE[name]]) + 1.) / 2.)))
            count = max(1 if name != "hat" else 0, count)
            for step in sorted(range(16), key=lambda s: (-profile[s], s))[:count]:
                grid.setdefault(step, []).append(name)
        return grid

    def identity(self):
        identity = {"model": "buzzart_console_style_v1", "console_songbook_sha256": self.sha256,
                    "palette": self.palette_name, "temperament": self.temperament, "voices": self.palette,
                    "tempo_range": [self.tempos[0], self.tempos[-1]], "arpeggio_rates": list(self.arpeggio_rates)}
        if self.temperament not in TEMPERAMENTS:
            # A drawn grammar is recorded whole, so the tune can be reproduced and
            # the critic can read the standards it was composed against.
            identity["genome"] = {k: (list(v) if isinstance(v, tuple) else v)
                                  for k, v in self.traits.items() if k != "critic"}
            identity["critic"] = self.traits["critic"]
        return identity
