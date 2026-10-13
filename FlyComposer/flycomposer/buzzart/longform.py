"""Piece-specific, disclosed development of two listener-selected performances.

This is arrangement work, not a new trained brain or a global style setting.
The brain keeps state throughout; a slowly varying phrase clock after minute
one releases its periodic input cycle. The arranger supplies a continuous
foundation, connected articulation, a return and an ending.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import math
from pathlib import Path

import numpy as np

from ..composition import ROLES, SCALES, midi
from ..connectome import load_circuit
from ..render import save_render
from .brain import Brain
from .breadth_training import make_options
from .performer import perform
from .showcase import render

CASES = {
    'dusk': {'source': 'd4906349cc154ac1', 'brain': 'brains/buzzart-wide-candidate.npz',
             'bounds': [0, 24, 48, 72, 96, 120, 144, 168, 179],
             'names': ['Recognition', 'Deepening', 'Conversation', 'Shade', 'Expansion', 'Remembered light', 'Crest', 'Arrival'],
             'energy': [.46, .57, .67, .51, .65, .70, .82, .54],
             'mix': dict(lead=.83, answer=.82, harmony=.30, bass=.85, texture=.16),
             'return': 160.},
    'overworld': {'source': 'da8dfe5b464b417b', 'brain': 'brains/buzzart-indie-long.npz',
                  'bounds': [0, 160/9, 160/3, 800/9, 320/3, 1280/9, 160, 179],
                  'names': ['Open road', 'Theme', 'Travelling', 'Wide horizon', 'Varied return', 'Ascent', 'Home'],
                  'energy': [.50, .65, .76, .53, .72, .86, .66],
                  'mix': dict(lead=.87, answer=.70, harmony=.18, bass=.88, texture=.14),
                  'return': 192.},
}
TONICS = ('C','Db','D','Eb','E','F','Gb','G','Ab','A','Bb','B')


def dump(path, obj):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(obj, indent=2, ensure_ascii=False, allow_nan=False)+'\n', encoding='utf-8')


class Direction:
    """Keep minute one intact, then drift the phrase clock without resetting state."""
    def __init__(self, context):
        self.case = CASES[context]

    def __call__(self, index, seconds, energy):
        if seconds <= 60:
            return index, energy
        share = min(1., (seconds-60)/96.)
        smooth = share*share*(3-2*share)
        # A continuous offset, not jumps or repeated fresh controller captures.
        clock = index + 5.25*smooth + .24*smooth*math.sin(seconds/19.)
        centers = [(a+b)/2 for a,b in zip(self.case['bounds'], self.case['bounds'][1:])]
        target = float(np.interp(seconds, centers, self.case['energy']))
        return clock, (1-.40*smooth)*energy + .40*smooth*target


def arrange(raw, source, context):
    score = copy.deepcopy(raw)
    case = CASES[context]
    beat = 60./score['bpm']
    stop = 179./beat
    cadence = math.floor((stop-4)/4)*4
    tonic, scale = TONICS.index(score['tonic']), SCALES[score['mode']]
    decisions = score['decision_log']
    boundaries = [d['at_seconds']/beat for d in decisions] + [stop]
    original = copy.deepcopy(score['events'])
    for event in original:
        event['source_event_id'] = event['id']
        event['arrangement_origin'] = 'directed_brain'
    edits = {'bass_pulses_added': 0, 'foreground_gates_extended': 0, 'harmony_notes_added': 0,
             'hats_removed': 0, 'return_notes': 0, 'counterline_notes': 0}

    def decision_at(start):
        i = min(len(decisions)-1, max(0, int(np.searchsorted(boundaries, start+1e-7, side='right')-1)))
        return i, decisions[i]['selected_root'], boundaries[i+1]

    def section_at(seconds):
        return min(len(case['names'])-1, max(0, int(np.searchsorted(case['bounds'], seconds, side='right')-1)))

    def pitch(degree, base=60):
        return int(midi(degree, tonic, scale, base))

    def degree_for(p):
        return next(d for d in range(-42, 64) if pitch(d) == p)

    def event(role, start, length, p, velocity, root, origin, decision=None):
        start = float(start)
        length = float(min(length, stop-start))
        if length*beat < .025:
            return None
        return {'id': -1, 'layer': role, 'voice': score['voices'][role], 'midi': int(p),
                'start_beat': round(start, 6), 'length_beats': round(length, 6),
                'start_seconds': start*beat, 'duration_seconds': length*beat,
                'velocity': float(velocity), 'section': section_at(start*beat),
                'harmonic_root': int(root), 'scale_degree': degree_for(p) if role != 'texture' else 0,
                'decision': decision_at(start)[0] if decision is None else decision,
                'arrangement_origin': origin}

    def resize(e, length):
        e['length_beats'] = round(float(length), 6)
        e['duration_seconds'] = float(length)*beat

    # Retain the actual pitches and starts of the selected opening, then
    # continue its recurrent brain rather than looping its minute-long WAV.
    events = [e for e in original if e['layer'] != 'bass' and e['start_beat'] < cadence]
    for role in ('lead', 'answer'):
        line = sorted([e for e in events if e['layer'] == role], key=lambda e:e['start_seconds'])
        for i, e in enumerate(line):
            start = e['start_seconds']/beat
            _, _, boundary = decision_at(start)
            following = line[i+1]['start_seconds']/beat if i+1 < len(line) else cadence
            cap = min(following-.025, boundary-.012, cadence)-start
            desired = min(2.5 if context == 'dusk' else 1.6, (following-start)*.94)
            length = min(cap, max(e['duration_seconds']/beat, desired))
            if length > e['duration_seconds']/beat + .001:
                resize(e, length)
                edits['foreground_gates_extended'] += 1
            elif cap >= .05 and e['duration_seconds']/beat > cap:
                resize(e, cap)

    # Continuous eighth-note bass: keep existing pitches where a pulse was
    # played; fill inactive decisions from the same harmonic root. Accents
    # breathe by bar, while the underlying attack rate remains constant.
    raw_bass = [e for e in original if e['layer'] == 'bass']
    swing = float(score['buzzart']['style'].get('traits', {}).get('swing', 0.))
    starts = [k*.5 + (.5*swing if k%2 else 0.) for k in range(int(math.ceil(cadence*2)))]
    for k, start in enumerate(starts):
        if start >= cadence:
            break
        di, root, _ = decision_at(start)
        nearest = min(raw_bass, key=lambda e:abs(e['start_seconds']/beat-start), default=None)
        retained = nearest and abs(nearest['start_seconds']/beat-start) < .12
        p = nearest['midi'] if retained else pitch(root, 36)
        if not retained:
            edits['bass_pulses_added'] += 1
        next_start = starts[k+1] if k+1<len(starts) else cadence
        length = min(next_start, cadence)-start-.008
        accent = (.62, .50, .55, .51, .58, .50, .55, .52)[k%8]
        accent *= 1 + .035*math.sin(start*math.pi/16)
        events.append(event('bass', start, length, p, accent, root, 'continuous_bass', di))

    # Dusk needs a sustained harmonic floor. Voice-lead quiet seventh chords;
    # keep the selected original voicing when the brain actually supplied one.
    if context == 'dusk':
        events = [e for e in events if e['layer'] != 'harmony']
        previous = [pitch(2,48), pitch(4,48), pitch(6,48)]
        for i, d in enumerate(decisions):
            begin, end = boundaries[i], min(boundaries[i+1], cadence)
            if begin >= cadence:
                break
            root = d['selected_root']
            old = [e for e in original if e['layer']=='harmony' and e['decision']==i]
            if len(old)>=3:
                notes = sorted(set(e['midi'] for e in old))
            else:
                options = []
                for inv in range(3):
                    # The continuing bass supplies the root. Keep dusk's
                    # source seventh colour in the upper three voices.
                    ds = [root+2,root+4,root+6]
                    ds = ds[inv:] + [x+7 for x in ds[:inv]]
                    for shift in (-7,0,7):
                        notes = [pitch(x+shift,48) for x in ds]
                        if min(notes)>=45 and max(notes)<=72:
                            options.append(notes)
                notes = min(options, key=lambda ns:sum(abs(a-b) for a,b in zip(ns,previous)))
            previous = notes
            for j,p in enumerate(notes):
                e=event('harmony',begin+j*.014,max(.06,end-begin-j*.014-.018),p,.34+(j==1)*.025,root,'connected_harmonic_bed',i)
                if e: events.append(e); edits['harmony_notes_added']+=1

    # Noise has a supporting role. Retain kick/snare drive, with restrained
    # hats on a regular grid rather than washing over the pitched instruments.
    cleaned=[]
    for e in events:
        if e['layer']=='texture':
            grid=round(e['start_seconds']/beat*4)%16
            allowed=(4,12) if context=='dusk' else (2,6,10,14)
            if e['midi']>=72 and grid not in allowed:
                edits['hats_removed']+=1
                continue
            e['velocity']=min(e['velocity'], .23 if e['midi']>=72 else .46)
        cleaned.append(e)
    events=cleaned

    # A recognizable eight-bar return, re-voiced into the current harmony.
    # Only the principal line is replaced here; accompaniment keeps moving.
    principal = 'answer' if context=='dusk' else 'lead'
    return_start=case['return']
    motif=[e for e in source['events'] if e['layer']==principal and e['start_seconds']/beat < 32]
    events=[e for e in events if not(e['layer']==principal and return_start<=e['start_seconds']/beat<return_start+32)]
    for old in motif:
        start=return_start+old['start_seconds']/beat
        if start>=cadence: continue
        di,root,boundary=decision_at(start)
        degree=old['scale_degree']+root-old['harmonic_root']
        p=pitch(degree)
        while p>86: degree-=7; p=pitch(degree)
        while p<57: degree+=7; p=pitch(degree)
        # Source contour returns with longer breathing notes, never across a
        # new chord or the next onset of the motif.
        following=min([return_start+x['start_seconds']/beat for x in motif if x['start_seconds']>old['start_seconds']]+[return_start+32])
        length=min(max(old['duration_seconds']/beat,.55), following-start-.025, boundary-start-.012, cadence-start)
        e=event(principal,start,max(.05,length),p,old['velocity']*.91,root,'varied_opening_return',di)
        if e: events.append(e); edits['return_notes']+=1

    # A sparse answering voice adds depth in the development. It occupies
    # actual rests of its own part and avoids the principal pitch/register.
    other='lead' if context=='dusk' else 'answer'
    for start in np.arange(math.ceil(64./beat/4)*4, min(158./beat,cadence), 4.):
        line=[e for e in events if e['layer']==other]
        di,root,boundary=decision_at(start)
        length=min(1.75 if context=='dusk' else 1.35, boundary-start-.035)
        if length<.5 or any(e['start_seconds']/beat < start+length+.12 and (e['start_seconds']+e['duration_seconds'])/beat>start-.12 for e in line):
            continue
        degree=root+(2 if int(start//4)%3 else 4)
        p=pitch(degree,48 if context=='dusk' else 60)
        while p<57: p+=12
        e=event(other,float(start),length,p,.34,root,'sparse_answer',di)
        if e: events.append(e); edits['counterline_notes']+=1

    # Resolve the last bars rather than stopping an arbitrary brain gesture.
    for e in events:
        remaining=cadence-e['start_seconds']/beat
        if e['duration_seconds']/beat>remaining:
            resize(e, remaining)
    for k,start in enumerate(np.arange(cadence, stop-1., .5)):
        root=4 if start<cadence+2 else 0
        e=event('bass',float(start),min(.492,stop-start),pitch(root,36),.55-.07*(start-cadence)/(stop-cadence),root,'cadence_pulse')
        if e: events.append(e)
    # Last tonic has one full beat of weight before the one-second release.
    final_start=stop-1.
    events=[e for e in events if e['start_seconds']/beat<final_start or e['layer']!='bass']
    for e in events:
        if e['layer']=='bass' and (e['start_seconds']+e['duration_seconds'])/beat>final_start:
            resize(e,final_start-e['start_seconds']/beat-.006)
    events.append(event('bass',final_start,1.,pitch(0,36),.52,0,'final_tonic'))
    for begin,end,root in ((cadence,cadence+2,4),(cadence+2,stop,0)):
        for degree in (root,root+2,root+4):
            events.append(event('harmony',begin,end-begin,pitch(degree,48),.31,root,'composed_cadence'))
    for n,(offset,degree) in enumerate(((0,4),(.85,2),(2,0))):
        start=cadence+offset
        length=(.82 if n<2 else stop-start)
        events.append(event(principal,start,length,pitch(degree,72 if context=='overworld' else 60),.53-n*.025,4 if n<2 else 0,'composed_cadence'))
    for start in np.arange(cadence,stop-1,1.):
        e=event('texture',float(start),.22,36 if int(start)%2==0 else 60,.30,0,'cadence_groove')
        if e: events.append(e)

    # Critical revision: a contrast must actually change musical activity.
    # Clear space ABOVE the continuing groove and consolidate very short
    # ornaments after minute one instead of merely stretching their gates.
    if context=='overworld':
        contrast_begin,contrast_end=case['bounds'][3:5]
        events=[e for e in events if not(e['layer']=='texture' and e['midi']>=72 and contrast_begin<=e['start_seconds']<contrast_end)]
        removed=set()
        for role in ('lead','answer'):
            last=None
            for e in sorted([x for x in events if x['layer']==role],key=lambda x:x['start_seconds']):
                t=e['start_seconds']
                contrast=contrast_begin<=t<contrast_end
                minimum=1.25*beat if contrast else .55*beat
                if last is not None and 60<=t<cadence*beat and t-last['start_seconds']<minimum and (contrast or e['duration_seconds']<.48*beat):
                    removed.add(id(e))
                else:
                    last=e
        events=[e for e in events if id(e) not in removed]
        edits['ornaments_consolidated']=len(removed)
    else:
        # Dusk's shade is a genuine spacious passage above the same drive.
        # Keep kick and a restrained backbeat, with no hat wash.
        removed=set()
        for role in ('lead','answer'):
            last=None
            for e in sorted([x for x in events if x['layer']==role],key=lambda x:x['start_seconds']):
                t=e['start_seconds']
                if 72<=t<96 and last is not None and t-last['start_seconds']<1.5*beat:
                    removed.add(id(e))
                else: last=e
        for e in events:
            t=e['start_seconds']
            if 72<=t<96 and e['layer']=='texture':
                if e['midi']>=72 or (48<=e['midi']<72 and int(e['start_seconds']/beat)//4%2):
                    removed.add(id(e))
            if 72<=t<96 and e['layer'] in ('lead','answer'):
                e['velocity']*=.87
            if 144<=t<168 and e['layer']=='answer' and e['midi']<72:
                e['midi']+=12
                e['scale_degree']+=7
                e['arrangement_origin']='crest_register_development'
        events=[e for e in events if id(e) not in removed]
        edits['shade_notes_thinned']=len(removed)

    # Reconnect the final melodic ledger after return/contrast edits. This
    # also prevents a return's shorter source gates reintroducing fragmentation.
    for role in ('lead','answer'):
        line=sorted([e for e in events if e['layer']==role],key=lambda e:e['start_seconds'])
        for i,e in enumerate(line):
            start=e['start_seconds']/beat
            if start>=cadence: continue
            following=line[i+1]['start_seconds']/beat if i+1<len(line) else cadence
            _,_,boundary=decision_at(start)
            cap=min(following-.022,boundary-.012,cadence)-start
            length=min(cap,max(e['duration_seconds']/beat,min(2.5 if context=='dusk' else 2.,.94*(following-start))))
            if length>=.045:
                resize(e,length)

    centers=[(a+b)/2 for a,b in zip(case['bounds'],case['bounds'][1:])]
    for e in events:
        if e is None: continue
        t=e['start_seconds']
        energy=float(np.interp(t,centers,case['energy']))
        if e['layer']!='bass':
            gain=.78+.32*energy
            # Alternating emphasis is gradual and does not mute the foundation.
            if e['layer'] in ('lead','answer'):
                turn=.07*math.sin(t*math.pi/24)
                gain += turn if e['layer']=='lead' else -turn
            e['velocity']=round(min(.86,e['velocity']*gain),6)
        e['section']=section_at(t)
    events=[e for e in events if e and e['duration_seconds']>=.025]
    events.sort(key=lambda e:(e['start_seconds'],ROLES.index(e['layer']),e['midi']))
    for i,e in enumerate(events): e['id']=i
    score['events']=events
    score['event_sha256']=hashlib.sha256(json.dumps(events,sort_keys=True).encode()).hexdigest()
    score['mix']=case['mix']
    score['sections']=[{'name':name,'start_seconds':a,'end_seconds':b,'scope':'Explicit longform arrangement'}
                       for name,a,b in zip(case['names'],case['bounds'],case['bounds'][1:])]
    score['title']=f'Developed · {context} · {score["seed"]} · 3:00'
    score['arrangement_version']='buzzart_selected_longform_v1'
    score['longform']={'context':context,'source_track_id':case['source'], 'source_event_sha256':source['event_sha256'],
                       'raw_event_sha256':raw['event_sha256'],'edits':edits,
                       'scope':'Same selected brain/seed/key/tempo/palette, with explicit arrangement; not new training or listener approval.',
                       'direction':'First 60 seconds retain controller inputs; afterwards continuous phrase-clock drift and section energy, without state resets.',
                       'cadence_start_seconds':cadence*beat}
    score['decoder_provenance']['planned_form']=case['names']
    score['decoder_provenance']['longform_arranger']=score['longform']
    score['decoder_provenance']['prewritten_melody']='Selected opening motif returns once over current harmony; final cadence and sparse answers explicitly arranged.'
    # Recompute phrase statistics from what is actually heard, retaining the
    # original brain decisions in their separate provenance ledger.
    for p in score['phrases']:
        notes=[e for e in events if e['layer']=='lead' and p['start_beat']<=e['start_seconds']/beat<p['start_beat']+p['length_beats']]
        p['lead_notes']=len(notes)
        p['lead_signature']=hashlib.sha256(json.dumps([(e['midi'],round(e['start_beat']-p['start_beat'],6),e['length_beats']) for e in notes]).encode()).hexdigest()
        p['section']=section_at(p['start_beat']*beat)
    return score


def develop(root, context, destination):
    root,destination=Path(root).resolve(),Path(destination).resolve()
    if destination.exists(): raise FileExistsError(destination)
    case=CASES[context]
    source=json.loads((root/'library'/case['source']/'score.json').read_bytes())
    brain=Brain.load(root/case['brain'])
    options=make_options(root,context,source['seed'])
    options['style'].tempos=[source['bpm']]
    raw=perform(load_circuit(),brain,seed=source['seed'],seconds=180,voices=source['voices'],direction=Direction(context),**options)
    score=arrange(raw,source,context)
    print(f'{context}: rendering {len(score["events"])} notes, {len(score["sections"])} sections',flush=True)
    audio,stems,audit=render(score,progress=lambda msg:print(msg,flush=True))
    # Full-piece target retained where both mix and individual stems have
    # safe headroom. Do not silently clip a stem while the mix passes.
    peak=max(float(np.max(np.abs(audio))),*(float(np.max(np.abs(s))) for s in stems.values()))
    target=min(.055,.90*audit['raw_rms']/peak)
    save_render(destination,score,audio,stems,audit,target)
    dump(destination/'raw-directed-score.json',raw)
    dump(destination/'development.json',score['longform'])
    print(json.dumps({'directory':str(destination),'notes':len(score['events']),'peak':audit['peak'],'rms':audit['rms'],'edits':score['longform']['edits']}),flush=True)
    return destination


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('context',choices=CASES)
    parser.add_argument('destination',type=Path)
    args=parser.parse_args()
    develop(Path(__file__).resolve().parents[2],args.context,args.destination)
