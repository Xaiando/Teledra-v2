"""Musical decoding of circuit activity into a complete, explicit arrangement.

Harmony/form and voice-leading rules are human-designed and shared by controls.
Temporal circuit outputs develop pitches, rhythm, answers, texture and dynamics.
"""
from __future__ import annotations

from collections import Counter
import hashlib
import json
import math
import numpy as np

from .controller import Controller

ROLES = ("lead", "answer", "harmony", "bass", "texture")
ENSEMBLES = {
    "keys": {"lead":"classic:keys", "answer":"classic:bell", "harmony":"classic:pad", "bass":"classic:bass", "texture":"classic:pluck"},
    "chamber": {"lead":"piano", "answer":"clarinet", "harmony":"violin", "bass":"double_bass", "texture":"harp"},
    "luminous": {"lead":"electric_piano", "answer":"theremin", "harmony":"cello", "bass":"classic:bass", "texture":"vibraphone"},
}
SCALES = {"major": (0,2,4,5,7,9,11), "minor": (0,2,3,5,7,8,10), "dorian": (0,2,3,5,7,9,10)}
RHYTHMS = (
    ((0,1.),(1.5,.55),(2.5,.8),(4,1.1),(5.5,.6),(7,.55)),
    ((.5,.65),(1.5,1.2),(3,.65),(4.5,.7),(5.5,.6),(7,.55)),
    ((0,1.45),(2,.6),(3,.7),(4.5,1.1),(6,.5),(7,.55)),
    ((0,.65),(1,.75),(2.5,1.),(4,.65),(5.5,1.0),(7,.55)),
    ((.5,1.1),(2,.5),(3,1.),(4.5,.65),(6,.5),(7,.55)),
    ((0,.6),(1.5,.7),(3,.6),(4,1.3),(6,.55),(7,.55)),
)
ENERGY = (.45, .65, .83, .58)
SECTION_NAMES = ("Threshold", "Conversation", "Flight", "Homecoming")


def choose(value, options):
    return options[min(len(options)-1, max(0, int((float(value)+1)*.5*len(options))))]


def midi(degree, tonic, scale, base):
    octave, within = divmod(int(degree), 7)
    return base + tonic + 12*octave + scale[within]


def nearest_chord(degree, root):
    return min((root+d+o*7 for o in range(-3,4) for d in (0,2,4)), key=lambda n: (abs(n-degree),n))


def write_song(circuit, seed=42, ensemble="keys", condition="connectome", voices=None):
    if ensemble not in ENSEMBLES or type(seed) is not int or not 0 <= seed <= 2**31-1:
        raise ValueError("Choose a supported ensemble and integer seed in 0..2147483647")
    voice_map = dict(ENSEMBLES[ensemble] if voices is None else voices)
    if set(voice_map) != set(ROLES):
        raise ValueError("Exactly five instrumental parts are required")
    rng = np.random.default_rng(seed)
    tonic = int(rng.choice([0,2,5,7,9]))
    mode = str(rng.choice(list(SCALES)))
    scale = SCALES[mode]
    bpm = int(rng.choice([88,96,104]))
    # Four purposeful harmonic regions, each chord held for two bars.
    roots = ([0,3,5,4, 0,5,3,4, 1,4,0,4, 3,1,4,0] if mode == "major" else
             [0,5,2,6, 0,3,5,4, 3,6,2,4, 5,3,4,0])
    motif = [0]
    for i in range(4):
        candidates = [n for n in (motif[-1]-3,motif[-1]-2,motif[-1]+1,motif[-1]+2,motif[-1]+4)
                      if 0 <= n <= 8]
        motif.append(int(rng.choice(candidates)))
    motif.append(int(rng.choice([0,2,4])))
    if len(set(motif)) < 3:
        motif[2], motif[3] = 4, 2
    controller = Controller(circuit, seed, condition)
    events, phrases = [], []
    beat_seconds = 60 / bpm
    last_pitch = {}

    def note(role, beat, degree, length, velocity, section, root, anchor=False):
        if anchor:
            degree = nearest_chord(degree, root)
        base = {"lead":60,"answer":60,"harmony":48,"bass":36,"texture":72}[role]
        pitch = midi(degree, tonic, scale, base)
        # Octave voicing preserves pitch class; melodic parts choose the nearest
        # register to their preceding note, so a motif does not jump at a boundary.
        low, high = {"lead":(60,84),"answer":(60,86),"harmony":(45,72),"bass":(33,55),"texture":(68,93)}[role]
        while pitch < low: pitch += 12
        while pitch > high: pitch -= 12
        if role in ('lead','answer') and role in last_pitch:
            choices = [p for p in range(low,high+1) if p % 12 == pitch % 12]
            pitch = min(choices,key=lambda p:(abs(p-last_pitch[role]),abs(p-pitch)))
        last_pitch[role] = pitch
        length = min(length, 128-beat-.08)
        if length <= .05: return
        events.append({"id":len(events), "layer":role, "voice":voice_map[role], "midi":pitch,
                       "start_beat":round(beat,5), "length_beats":round(length,5),
                       "velocity":round(float(np.clip(velocity,.25,.9)),5), "section":section,
                       "start_seconds":beat*beat_seconds, "duration_seconds":length*beat_seconds})

    previous_voicing = [midi(d,tonic,scale,48) for d in (0,2,4)]
    for phrase in range(16):
        section, local = divmod(phrase, 4)
        start = phrase*8
        root = roots[phrase]
        control = controller.phrase(phrase, ENERGY[section], root)
        rhythm = RHYTHMS[choose(control[0], list(range(len(RHYTHMS))))]
        pitches = motif.copy()
        if phrase % 4 != 0:
            for slot, value in ((1,control[1]),(3,control[2]),(4,control[3])):
                pitches[slot] = int(np.clip(pitches[slot] + choose(value, [-2,-1,0,1,2]),0,10))
        if section == 2:
            pitches[2] = int(np.clip(pitches[2] + choose(control[4], [-2,0,2]),0,10))
        if phrase == 15:
            pitches[-1] = 0
        # Sparse introduction; alternating foreground with overlapping held harmony.
        lead_active = phrase != 0 and not (section == 1 and local == 2)
        if phrase == 0:
            for slot in (0,3,5):
                beat,length = rhythm[slot]
                note("lead",start+beat,pitches[slot],length,.5,section,root,anchor=slot in (0,5))
        elif lead_active:
            for slot,(beat,length) in enumerate(rhythm):
                if phrase == 15 and slot == 4:
                    continue  # A deliberate breath before the held final tonic.
                if phrase == 15 and slot == 5:
                    beat,length = 6.,1.7
                note("lead",start+beat,pitches[slot],length,
                     (.58 + .12*control[5]) * (.8+.3*ENERGY[section]),section,root,
                     anchor=slot in (0,5))
        # Answer speaks after a breath; reverse fragments establish a related voice.
        answer_degrees = [pitches[4],pitches[2],pitches[1],nearest_chord(pitches[0],root)]
        if phrase >= 2 and phrase != 15:
            answer_start = choose(control[6], [3.5,4.,4.5])
            for slot,degree in enumerate(answer_degrees[:3 if lead_active else 4]):
                beat = answer_start+slot*.9
                if beat > 7.4: break
                if slot == 1: degree += choose(control[7], [-2,0,2])
                note("answer",start+beat,degree,.50 if slot<2 else .75,
                     .45+.08*ENERGY[section],section,root,anchor=slot in (0,3))
        elif phrase == 15:
            note("answer",start+1.,4,1.25,.43,section,root,anchor=True)
            note("answer",start+4.,2,1.8,.36,section,root,anchor=True)
        # Closest inversion produces real voice leading, no root-position leaps.
        candidates = []
        chord_degrees = [root,root+2,root+4]
        for inversion in range(3):
            degrees = chord_degrees[inversion:]+[d+7 for d in chord_degrees[:inversion]]
            for shift in (-7,0,7):
                trial = [d+shift for d in degrees]
                pitches_h = [midi(d,tonic,scale,48) for d in trial]
                if min(pitches_h)>=45 and max(pitches_h)<=72:
                    candidates.append((sum(abs(a-b) for a,b in zip(pitches_h,previous_voicing)),trial,pitches_h))
        _,voicing,previous_voicing = min(candidates,key=lambda c:c[0])
        for bar in range(2):
            chord_start = start+bar*4
            if section == 2 and bar == 1 and control[4] > .15:
                # One arpeggiated answer in the harmony, with unchanged chord identity.
                for k,degree in enumerate(voicing):
                    note("harmony",chord_start+k*.4,degree,3.4-k*.4,.49,section,root)
            else:
                for degree in voicing:
                    note("harmony",chord_start,degree,3.75,.44+.1*ENERGY[section],section,root)
            note("bass",chord_start,root,1.4 if phrase<15 else 2.,.65,section,root)
            if phrase != 15:
                bass_degree = root + choose(control[3], [0,4,7])
                bass_beat = choose(control[2], [2.,2.5,3.])
                note("bass",chord_start+bass_beat,bass_degree,.6,.45,section,root)
            if phrase > 1 and phrase < 15 and (bar == 0 or control[7] > .15):
                note("texture",chord_start+choose(control[6],[1.,1.5,2.5]),
                     root+choose(control[4],[2,4,7]),.6,.37,section,root)
        sounding = [e for e in events if e["layer"]=="lead" and start<=e["start_beat"]<start+8]
        phrase_hash = hashlib.sha256(json.dumps([(e["midi"],e["start_beat"]-start,e["length_beats"]) for e in sounding]).encode()).hexdigest()
        phrases.append({"index":phrase,"section":section,"root":root,"lead_signature":phrase_hash,
                        "lead_notes":len(sounding),"activity":control.tolist()})
    # Fixed, quiet release tail; no forced loop and no mid-phrase truncation.
    sections = [{"name":name,"start_seconds":i*32*beat_seconds,
                 "end_seconds":(i+1)*32*beat_seconds,"target_energy":ENERGY[i]} for i,name in enumerate(SECTION_NAMES)]
    signature = hashlib.sha256(json.dumps(events,sort_keys=True).encode()).hexdigest()
    return {"version":1,"title":f"Wing Study {seed}","seed":seed,"condition":condition,
            "tonic":("C","Db","D","Eb","E","F","Gb","G","Ab","A","Bb","B")[tonic],
            "mode":mode,"bpm":bpm,"bars":32,"duration_seconds":128*beat_seconds+1.,
            "voices":voice_map,"motif":motif,"progression":roots,"events":events,"phrases":phrases,
            "sections":sections,"event_sha256":signature,"controller":controller.identity,
            "neural_trace":controller.trace,"circuit_metadata":circuit.get("metadata",{})}


def assess_score(score):
    events = score["events"]
    duration = score["duration_seconds"]
    if duration < 30 or len(events) < 100 or set(e["layer"] for e in events) != set(ROLES):
        raise ValueError("Arrangement is incomplete")
    for event in events:
        if not all(math.isfinite(event[k]) for k in ("start_seconds","duration_seconds","velocity")):
            raise ValueError("Nonfinite event")
        if event["start_seconds"]<0 or event["duration_seconds"]<=0 or event["start_seconds"]+event["duration_seconds"]>duration:
            raise ValueError("Event outside complete arrangement")
    lead = sorted((e for e in events if e["layer"]=="lead"), key=lambda e:e["start_seconds"])
    for a,b in zip(lead,lead[1:]):
        if a["start_seconds"]+a["duration_seconds"] > b["start_seconds"]+1e-8:
            raise ValueError("Foreground phrase overlaps itself")
    signatures = [p["lead_signature"] for p in score["phrases"] if p["lead_notes"]]
    unique = len(set(signatures))
    if unique < 6:
        raise ValueError("Insufficient development across the full piece")
    intervals = np.diff([e["midi"] for e in lead])
    scalar_runs = sum(all(x in (1,2) for x in intervals[i:i+6]) or all(x in (-1,-2) for x in intervals[i:i+6])
                      for i in range(max(0,len(intervals)-5)))
    if scalar_runs:
        raise ValueError("A long scalar run survived musical decoding")
    return {"notes_scheduled":len(events),"layers":len(ROLES),"sections":len(score["sections"]),
            "unique_lead_phrases":unique,"foreground_phrases":len(signatures),"scalar_runs":scalar_runs,
            "notes_by_layer":dict(Counter(e["layer"] for e in events)),
            "lead_rest_fraction":1-sum(e["duration_seconds"] for e in lead)/(duration-1),
            "event_sha256":score["event_sha256"],"listener_verdict":"unreviewed"}
