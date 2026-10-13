"""A musical grammar drawn fresh for every tune.

Three hand-made styles sounded like one tune in three arrangements. Their
melodies were no more alike than two unrelated seeds; what they shared was
grammar that no style could reach — a straight groove, one drum vocabulary, one
bass pattern, melody always out in front with the chords buried at a fixed
level, and the same instruments' kind of sound. The operator's goal is range: a
system that does not keep landing in the same place.

So each seed draws its own grammar across every axis the ear uses — groove,
drums, bass, how the chords are played and how often they move, who is out in
front, how the melody is articulated, the key colour, the tempo and the
instruments. The brain still composes every note inside that grammar.

Draws are coherent rather than uniform. A piece with no drums gets slower tempos
and drones, boom-bap gets swing and a syncopated bass, and instruments are chosen
to suit what their part is asked to do: a sustaining pad for held chords, a keys
or guitar voice for stabs, a drum voice only where there are drums.

The draw is a pure function of the seed, so a tune is reproducible from its seed
alone, and the grammar is recorded in the score.
"""
from __future__ import annotations

import random

# Drum families and the tempos, swing and bass patterns that belong with them.
# Tempo spans must overlap the chiptune corpus's own tempos (72 to 180), which
# the style draws from; nothing here invents a tempo the corpus lacks.
GROOVES = {
    "none":       {"tempo": (72, 90),   "swing": (0., 0.),   "bass": ("drone", "hold", "walking"), "weight": 1.2},
    "sparse":     {"tempo": (72, 96),   "swing": (0., .12),  "bass": ("hold", "walking", "drone", "syncopated"), "weight": 1.0},
    "half_time":  {"tempo": (80, 112),  "swing": (0., .15),  "bass": ("syncopated", "hold", "drone"), "weight": 1.0},
    "boom_bap":   {"tempo": (72, 96),   "swing": (.20, .36), "bass": ("syncopated", "walking", "hold"), "weight": 1.3},
    "shuffle":    {"tempo": (90, 120),  "swing": (.28, .40), "bass": ("walking", "syncopated", "pulse"), "weight": 1.0},
    "four_floor": {"tempo": (108, 130), "swing": (0., .06),  "bass": ("octave", "pulse", "syncopated"), "weight": 1.0},
    "breakbeat":  {"tempo": (108, 150), "swing": (0., .12),  "bass": ("syncopated", "octave", "pulse"), "weight": 1.0},
    "chip":       {"tempo": (96, 155),  "swing": (0., .08),  "bass": ("pulse", "octave"), "weight": .8},
}

# How the chords are played, and the voices that can play them that way.
HARMONY = {
    "hold":     ("warm_pad", "pad", "choir", "string_ensemble", "organ", "electric_piano", "tremolo_strings", "accordion"),
    "strum":    ("electric_guitar", "jazz_guitar", "steel_guitar", "guitar", "harp", "ukulele", "mandolin"),
    "broken":   ("piano", "electric_piano", "harp", "vibraphone", "celesta", "dulcimer", "koto"),
    "stabs":    ("electric_piano", "clavinet", "piano", "muted_guitar", "synth_brass", "organ", "chip:pulse50"),
    "pulse":    ("piano", "electric_piano", "organ", "honky_tonk_piano", "string_ensemble", "accordion"),
    "arpeggio": ("chip:pulse50", "amiga:chord", "harpsichord", "marimba", "fm:bell", "celesta"),
}
# Stabs and on-beat chord pulses only make sense against a beat.
NEEDS_DRUMS = {"stabs", "pulse"}

LEAD = ("fm:lead", "chip:pulse25", "chip:pulse12", "chip:triangle", "amiga:lead", "electric_piano", "vibraphone",
        "marimba", "flute", "pan_flute", "clarinet", "celesta", "jazz_guitar", "harmonica", "soprano_sax",
        "glockenspiel", "piano", "steel_guitar", "xylophone", "recorder", "muted_guitar", "oboe", "koto")
ANSWER = ("electric_piano", "vibraphone", "marimba", "harp", "celesta", "clavinet", "chip:pulse12", "fm:bell",
          "flute", "pizzicato_strings", "muted_guitar", "koto", "dulcimer", "glockenspiel", "clarinet")
BASS = {
    "drone": ("sub_bass", "double_bass", "fm:bass", "cello"),
    "hold": ("double_bass", "fm:bass", "bass", "chip:triangle", "cello"),
    "walking": ("double_bass", "bass", "fm:bass"),
    "syncopated": ("fm:bass", "bass", "chip:triangle", "amiga:bass", "sub_bass"),
    "octave": ("fm:bass", "chip:triangle", "amiga:bass", "bass"),
    "pulse": ("fm:bass", "chip:triangle", "amiga:bass"),
}
DRUM_VOICES = ("chip:noise", "amiga:drums")
SPARKLE = ("glockenspiel", "celesta", "harp", "marimba", "vibraphone", "fm:bell", "tubular_bells", "xylophone",
           "chip:pulse12")

# Who is out in front. The renderer used to fix harmony at 0.19 for every tune.
MIXES = {
    "melody":  {"lead": 1.00, "answer": .66, "harmony": .19, "bass": .72, "texture": .30},
    "chords":  {"lead": .55, "answer": .40, "harmony": .75, "bass": .70, "texture": .35},
    "pad":     {"lead": .45, "answer": .35, "harmony": .90, "bass": .50, "texture": .25},
    "groove":  {"lead": .60, "answer": .45, "harmony": .30, "bass": 1.00, "texture": .75},
    "duet":    {"lead": .90, "answer": .90, "harmony": .28, "bass": .60, "texture": .30},
}
# The melodic language: which real-music corpus the intervals, rhythms and
# motifs are drawn from. Every tune used to speak through one songbook.
SONGBOOKS = {
    "folk": "data/foundation/songbook.json",        # Irish tunes, Bach chorale sopranos
    "chiptune": "data/foundation/console.json",     # console game music
    "tracker": "data/foundation/amiga.json",        # Amiga modules
    "indie": "data/foundation/indie-blend.json",    # the retro-led blend
}

# How the melody moves through time. "flowing" is the old behaviour and only
# one of six; the rest change the rhythm the ear follows first.
MELODY_RHYTHMS = {
    "flowing":    {},
    "running":    {"melody_time": .5, "melody_grid": .25, "melody_offsets": (0., .5), "melody_slots": 40},
    "long":       {"melody_time": 3.0, "melody_grid": 1.0, "melody_offsets": (0.,), "melody_slots": 12, "legato": .95},
    "offbeat":    {"melody_time": 1.0, "melody_grid": .5, "melody_offsets": (.5, .75)},
    "triplet":    {"melody_time": .67, "melody_grid": 1. / 3., "melody_offsets": (0., 1. / 3.), "melody_slots": 30},
    "riff":       {"melody_time": .75, "melody_grid": .5, "melody_offsets": (0., .5), "riff": 4, "melody_slots": 32},
}
# Gesture-length menus: how often phrases turn and chords move.
PACES = {"calm": (4., 6., 8.), "steady": (3., 4., 6.), "busy": (2., 3., 4.), "mixed": (2., 4., 8.)}
MODES = (("minor",), ("dorian",), ("major",), ("minor", "dorian"), ("dorian", "major"))
# Registers keep a C base, which the interpreter needs to spell degrees in key.
LEAD_REGISTERS = {"low": (55, 79, 60), "middle": (64, 88, 72), "high": (72, 96, 84)}


def _pick(rng, options, exclude=()):
    choices = [option for option in options if option not in exclude]
    return rng.choice(choices or list(options))


def _weighted(rng, table):
    names = list(table)
    return rng.choices(names, weights=[table[name]["weight"] for name in names])[0]


def draw(seed):
    """The grammar and voices for one seed. Deterministic in the seed."""
    rng = random.Random(f"buzzart-genome-{int(seed)}")
    groove = _weighted(rng, GROOVES)
    spec = GROOVES[groove]
    drums = groove != "none"

    harmony_options = [kind for kind in HARMONY if drums or kind not in NEEDS_DRUMS]
    harmony = rng.choice(harmony_options)
    bass = rng.choice(spec["bass"])
    pace = rng.choice(list(PACES)) if drums else rng.choice(("calm", "steady", "mixed"))
    mix = rng.choice(list(MIXES) if drums else ["melody", "chords", "pad", "duet"])

    traits = {
        "name": f"genome-{int(seed)}",
        "tempo": spec["tempo"],
        "swing": round(rng.uniform(*spec["swing"]), 3),
        "drums": drums,
        "drum_scale": round(rng.uniform(.6, 1.1), 2) if groove == "chip" else 0.,
        "arpeggio_max": rng.choice((2, 3, None)) if harmony == "arpeggio" else 3,
        "harmony": "hold" if harmony == "strum" else harmony,
        "sevenths": rng.random() < (.75 if groove in ("boom_bap", "shuffle", "sparse") else .40),
        "roll": (.18, .26, .34) if harmony == "strum" else rng.choice(((0., .04, .10), (0., .06, .18), (.06, .12, .2))),
        "hold_share": round(rng.uniform(.6, .9), 2),
        "bass": bass,
        "spans": PACES[pace],
        "presence": round(rng.uniform(-.15, .3), 2),
        "lead_gate": rng.choice((.5, .75, 1., 1.35)),
        "modes": rng.choice(MODES),
        "mix": MIXES[mix],
        "registers": {"lead": LEAD_REGISTERS[rng.choice(list(LEAD_REGISTERS))]},
    }
    if groove not in ("none", "chip"):
        traits["drum_pattern"] = groove

    melody = rng.choice(list(MELODY_RHYTHMS))
    traits.update(MELODY_RHYTHMS[melody])
    # Articulation has to agree with the rhythm: a long-note melody clipped to
    # half length measured 0.40 beats, and a running line held legato smears.
    if melody == "long":
        traits["lead_gate"] = max(traits["lead_gate"], 1.35)
    elif melody in ("running", "triplet"):
        traits["lead_gate"] = min(traits["lead_gate"], .75)

    lead = _pick(rng, LEAD)
    palette = {
        "lead": lead,
        "answer": _pick(rng, ANSWER, exclude={lead}),
        "harmony": _pick(rng, HARMONY[harmony], exclude={lead}),
        "bass": _pick(rng, BASS[bass]),
        "texture": _pick(rng, DRUM_VOICES) if drums else _pick(rng, SPARKLE, exclude={lead}),
    }

    # The critic judges whether the music is good on its own terms; it no longer
    # asks every grammar for one density. An ambient drone and a breakbeat are
    # both allowed to be what they are.
    thin_harmony = harmony in ("arpeggio", "stabs")
    traits["critic"] = {"breath_floor": .10, "breath_ceiling": .65, "rest_floor": .20, "rest_ceiling": .70,
                        "entropy_floor": .40, "top_gap_ceiling": .60,
                        "density": (.8, 12.), "polyphony": (0, 12), "roles_at_once": (2.5, 5.),
                        "voices": (1.2, 6.) if thin_harmony else (2.0, 6.)}
    traits["melody"] = melody
    traits["songbook"] = rng.choice(list(SONGBOOKS))
    traits["description"] = describe(groove, harmony, bass, pace, mix, traits, palette)
    return traits, palette


def describe(groove, harmony, bass, pace, mix, traits, palette):
    feel = f"{groove.replace('_', ' ')} groove" if groove != "none" else "no drums"
    if traits["swing"] >= .15:
        feel += f", swung {traits['swing']:.2f}"
    return (f"{feel}; {harmony} chords on {palette['harmony']}"
            f"{' with sevenths' if traits['sevenths'] else ''}; {bass} bass on {palette['bass']}; "
            f"{traits['melody']} melody on {palette['lead']}, {palette['answer']} answer; {mix}-led, {pace} pace; "
            f"{'/'.join(traits['modes'])}; {traits['songbook']} phrasing")
