"""Device-free rendering. Each explicit note is checked at its actual placement."""
from __future__ import annotations

import hashlib
import json
import math
from collections import Counter
from pathlib import Path
import wave
import numpy as np

from .composition import ROLES, assess_score
from .vendor import teledra_synth as synth

ROOT = Path(__file__).resolve().parents[1]
SR = 48000
PAN = dict(lead=-.15,answer=.25,harmony=-.08,bass=0.,texture=.40)
LEVEL = dict(lead=1.,answer=.66,harmony=.19,bass=.72,texture=.30)


def assess_performance(score):
    """Validate the recording contract, without imposing an arrangement or taste."""
    duration=score.get('duration_seconds')
    if type(duration) not in (int,float) or not math.isfinite(duration) or not 10<=duration<=180:
        raise ValueError('Performance duration must be 10–180 finite seconds')
    voices=score.get('voices',{})
    catalog={v['id'] for v in json.loads((ROOT/'data/instruments.json').read_text(encoding='utf8'))}
    catalog|={'classic:'+v for v in ('sine','triangle','pluck','bell','pad','keys','bass')}
    if not isinstance(voices,dict) or not voices or not set(voices)<=set(ROLES) or any(v not in catalog for v in voices.values()):
        raise ValueError('Performance needs supported instruments and part names')
    events=score.get('events')
    if not isinstance(events,list) or not 1<=len(events)<=6000:
        raise ValueError('Performance requires 1–6000 explicit note events')
    for event in events:
        if event.get('layer') not in voices or event.get('voice')!=voices[event['layer']]:
            raise ValueError('Note does not match its authored part and instrument')
        if type(event.get('midi')) is not int or not 0<=event['midi']<=127:
            raise ValueError('Invalid performance MIDI pitch')
        for key in ('start_seconds','duration_seconds','start_beat','length_beats','velocity'):
            if type(event.get(key)) not in (int,float) or not math.isfinite(event[key]):
                raise ValueError('Performance note must have finite timing and velocity')
        if event['start_seconds']<0 or event['duration_seconds']<.025 or event['start_seconds']+event['duration_seconds']>duration-1+1e-6:
            raise ValueError('Note exceeds the complete recording and its release tail')
        if not event['voice'].startswith('classic:') and event['duration_seconds']>30:
            raise ValueError('Native instrument notes cannot exceed 30 seconds')
        if not 0<event['velocity']<=1 or event['start_beat']<0 or event['length_beats']<=0:
            raise ValueError('Invalid performance articulation')
    if len({e.get('id') for e in events})!=len(events):raise ValueError('Performance note IDs must be unique')
    sections=score.get('sections',[])
    if not sections or any(not 0<=s['start_seconds']<s['end_seconds']<=duration for s in sections):
        raise ValueError('Invalid recording segment bounds')
    actual_hash=hashlib.sha256(json.dumps(events,sort_keys=True).encode()).hexdigest()
    if score.get('event_sha256')!=actual_hash:raise ValueError('Performance note ledger identity mismatch')
    counts=Counter(e['layer'] for e in events)
    signatures={p['lead_signature'] for p in score.get('phrases',[]) if p.get('lead_signature')}
    return {'notes_scheduled':len(events),'layers':len(counts),'sections':len(sections),
            'unique_lead_phrases':len(signatures),'notes_by_layer':dict(counts),
            'event_sha256':actual_hash,'listener_verdict':'unreviewed','validation':'Technical recording checks; no arrangement or taste gate'}


NATIVE_RATE = 48000


def native_duration(seconds, sr):
    """A note length the native patch renderer and this renderer agree on, frame for frame.

    The native renderer multiplies in 32-bit floats and the vendored check in
    64-bit. A length meant as 0.986 s is stored as 0.9859999999999999: Python
    makes that 47327.99999999999 frames and truncates to 47327, the native side
    makes exactly 47328, and the note is refused as incomplete. Held chords and
    strums produce enough arbitrary lengths to hit it — four of 207 PyMusic
    notes in one dusk piece did — so a whole piece failed to render.

    A length safely inside a frame is returned unchanged, so every render that
    worked before stays byte-identical. Only a length within float error of a
    frame boundary moves, to mid-frame at 48 kHz, and never so far that its
    frame count at the target rate changes.
    """
    seconds = float(seconds)
    edge = seconds * NATIVE_RATE
    # 32-bit error grows with the length; a 30 s note can be off by ~0.07 frames.
    margin = .01 + 1.5e-7 * edge
    if abs(edge - round(edge)) > margin:
        return seconds
    target = int(seconds * sr)
    low, high = target / sr, (target + 1) / sr
    best = None
    frame = math.floor(low * NATIVE_RATE)
    while frame / NATIVE_RATE < high:
        candidate = (frame + .5) / NATIVE_RATE
        if int(candidate * sr) == target and int(candidate * NATIVE_RATE) == frame:
            distance = abs(candidate - (low + high) / 2)
            if best is None or distance < best[0]:
                best = (distance, candidate)
        frame += 1
    return best[1] if best else seconds


def note_seed(score, event):
    # Independent of insertion order: shared notes receive identical timbre noise.
    identity = [score['seed'], event['layer'], event['start_beat'], event['midi'], event['voice']]
    return int.from_bytes(hashlib.sha256(json.dumps(identity).encode()).digest()[:4], 'little')


def instrument_catalog():
    synth._configure_bridge(ROOT / "bin/PyMusicSynth.exe")
    try:
        catalog = synth.instruments()
    finally:
        synth._close_bridge()
    return catalog


def classic_note(name, frequency, duration, velocity, sr):
    recipes = {"sine":("sine",.025,.12,.70,.18), "triangle":("triangle",.015,.12,.60,.15),
               "pluck":("triangle",.006,.16,.15,.09), "bell":("sine",.006,.20,.20,.18),
               "pad":("sine",.12,.18,.78,.25), "keys":("triangle",.008,.20,.30,.15),
               "bass":("sine",.008,.10,.62,.12)}
    form,attack,decay,sustain,release = recipes[name]
    sound = synth.synth_note(frequency,duration,form,attack,decay,sustain,release,velocity,sr)
    if name in ("bell","keys","pad"):
        harmonic,level = (3,.14) if name=="bell" else (2,.12)
        sound += synth.synth_note(frequency*harmonic,duration,"sine",attack,decay*.7,sustain*.4,release,velocity*level,sr)
    return sound


def render_score(score, note_factory=None, sr=SR, progress=lambda value:None):
    performance=score.get('kind')=='performance'
    facts = assess_performance(score) if performance else assess_score(score)
    frames = int(score["duration_seconds"]*sr)
    stems = {role:np.zeros(frames) for role in ROLES}
    ledger = []
    # A score may carry its own balance. Every part used to sit at one fixed
    # level, harmony at 0.19, so every tune was melody-led whatever it wanted.
    levels = {**LEVEL, **(score.get("mix") or {})}
    native = any(not voice.startswith("classic:") for voice in score["voices"].values())
    if native and note_factory is None:
        synth._configure_bridge(ROOT / "bin/PyMusicSynth.exe")
    try:
        for i,event in enumerate(score["events"]):
            frequency = 440.*2**((event["midi"]-69)/12)
            seconds = event["duration_seconds"]
            if note_factory is not None:
                sound = note_factory(event, sr)
            elif event["voice"].startswith("classic:"):
                sound = classic_note(event["voice"].split(":")[1],frequency,seconds,1.,sr)
            else:
                sound = synth.instrument_note(frequency,native_duration(seconds,sr),event["voice"],volume=1.,sr=sr,
                                               velocity=event["velocity"],seed=note_seed(score,event))
            sound = np.asarray(sound,dtype=float)
            start = int(event["start_seconds"]*sr)
            expected = int(seconds*sr)
            if sound.ndim!=1 or len(sound)!=expected or not np.all(np.isfinite(sound)):
                raise ValueError(f"Note {i} returned incomplete or invalid audio")
            rms = float(np.mean(sound*sound))**.5
            if rms <= 1e-8 or start<0 or start+expected>frames:
                raise ValueError(f"Note {i} was silent or outside the complete piece")
            # A shared musical gain rule; identical voices/conditions use the same rule.
            gain = .08*levels[event["layer"]]*event["velocity"] / rms
            sound = sound * gain
            stems[event["layer"]][start:start+expected] += sound
            ledger.append({**event,"synthesis_seed":note_seed(score,event),"start_frame":start,"frames":expected,"end_frame":start+expected,
                           "mixed":True,"placed_rms":float(np.mean(sound*sound))**.5,
                           "note_pcm_sha256":hashlib.sha256(sound.astype('<f4').tobytes()).hexdigest()})
            if i % 30 == 0:
                progress(f"Mixed {i+1}/{len(score['events'])} notes")
    finally:
        if native and note_factory is None:
            synth._close_bridge()
    mixed = np.zeros((frames,2))
    for role,stem in stems.items():
        angle = (PAN[role]+1)*np.pi/4
        mixed[:,0] += stem*np.cos(angle)
        mixed[:,1] += stem*np.sin(angle)
    layer_stats = [{"name":role,"rms":float(np.mean(stem**2))**.5,"peak":float(np.max(np.abs(stem))),
                    "notes":sum(e["layer"]==role for e in ledger)} for role,stem in stems.items()]
    if any((item['peak']==0 if performance else item['rms']<1e-5) for item in layer_stats if not performance or item['notes']):
        raise ValueError("An instrumental layer is silent")
    section_stats = []
    for section in score["sections"]:
        segment = mixed[int(section["start_seconds"]*sr):int(section["end_seconds"]*sr)]
        section_stats.append({**section,"rms":float(np.mean(segment**2))**.5})
    audit = {**facts,"renderer":"fly_explicit_pymusic_v1","complete":True,"notes_mixed":len(ledger),
             "sample_rate":sr,"channels":2,"frames":frames,"duration_seconds":frames/sr,
             "layer_stats":layer_stats,"section_stats":section_stats,"events":ledger,
             "raw_rms":float(np.mean(mixed**2))**.5,"raw_peak":float(np.max(np.abs(mixed))),
             "playback":"finite independent recording with release tail" if performance else "finite piece with composed tonic ending and release tail"}
    return mixed, stems, audit


def save_wav(path, audio, sr=SR):
    if not np.all(np.isfinite(audio)) or np.max(np.abs(audio)) >= 1:
        raise ValueError("Clipping/nonfinite audio cannot be saved")
    pcm = np.rint(audio*32767).astype('<i2')
    with wave.open(str(path),'wb') as output:
        output.setnchannels(1 if pcm.ndim==1 else 2)
        output.setsampwidth(2); output.setframerate(sr); output.writeframes(pcm.tobytes())
    return hashlib.sha256(pcm.tobytes()).hexdigest()


def save_render(directory, score, audio, stems, audit, target_rms):
    directory.mkdir(parents=True,exist_ok=False)
    gain = target_rms/audit["raw_rms"]
    mastered = audio*gain
    audit.update(rms=float(np.mean(mastered**2))**.5,peak=float(np.max(np.abs(mastered))),
                 matching="Full-piece RMS at the session target; not a perceptual loudness or taste judgment",
                 master_gain=gain,comparison_target_rms=target_rms)
    audit["pcm_sha256"] = save_wav(directory/'audio.wav',mastered,audit['sample_rate'])
    for role,stem in stems.items():
        save_wav(directory/f'{role}.wav',stem*gain,audit['sample_rate'])
    (directory/'score.json').write_text(json.dumps(score,indent=2),encoding='utf-8')
    (directory/'audit.json').write_text(json.dumps(audit,indent=2),encoding='utf-8')
    return audit
