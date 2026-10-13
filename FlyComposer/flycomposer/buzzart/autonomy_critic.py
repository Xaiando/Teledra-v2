"""Local, transparent practice critic; never an estimate of listener approval.

The score path is O(N log N), synthesizes nothing and has no network/model
dependency. Beat-relative plateaus preserve different tempos, instruments and
declared long-phrase idioms. Rejection gates do not zero the training reward.
The separate audio path checks actual arrays and renderer evidence before a
candidate can be retained as a technically complete recording.
"""
from __future__ import annotations

from collections import Counter, defaultdict
import hashlib
import json
import math
import statistics

import numpy as np

from ..composition import SCALES
from ..render import LEVEL, PAN
from .console_style import TEMPERAMENTS

VERSION = 'autonomy_critic_v1'
COMPONENTS = ('drive', 'flow', 'depth', 'development', 'restraint', 'harmony')
DRUMS = frozenset(('chip:noise', 'amiga:drums'))
TONICS = ('C', 'Db', 'D', 'Eb', 'E', 'F', 'Gb', 'G', 'Ab', 'A', 'Bb', 'B')
SCOPE = 'Designed local measurement proxy, not listening, taste prediction, or listener approval.'


def _finite(value):
    return type(value) in (int, float) and math.isfinite(value)


def _clip(value):
    return max(0., min(1., float(value)))


def _mean(values):
    values = list(values)
    return sum(values)/len(values) if values else 0.


def _band(value, low, high, soft):
    return _clip(1.-max(low-value, value-high, 0.)/soft)


def _merge(intervals):
    result = []
    for start, end in sorted(intervals):
        if start >= end:
            continue
        if result and start <= result[-1][1]+1e-8:
            result[-1][1] = max(end, result[-1][1])
        else:
            result.append([start, end])
    return result


def _cover(intervals, begin, end):
    return sum(max(0., min(b,end)-max(a,begin)) for a,b in intervals)/(end-begin)


def _gaps(intervals, end):
    return ([intervals[0][0]]+[b[0]-a[1] for a,b in zip(intervals,intervals[1:])]
            +[end-intervals[-1][1]]) if intervals else [end]


def _intent(score):
    style = score.get('buzzart',{}).get('style') or {}
    traits = {**TEMPERAMENTS.get(style.get('temperament'),{}), **(style.get('genome') or {})}
    # Arranger/mastering balance is not evidence of a compositional idiom.
    # Only the predeclared genome can nominate a chord-led focus.
    declared_mix = (style.get('genome') or {}).get('mix') or {}
    chord_focus = declared_mix.get('harmony',0) > max(declared_mix.get('lead',0),declared_mix.get('answer',0))
    focus = frozenset(('harmony',)) if chord_focus else frozenset(('lead','answer'))
    long_phrase = traits.get('melody') == 'long' or style.get('temperament') == 'dreamy' or chord_focus
    slow_bass = traits.get('bass') in ('hold','drone')
    span = max([float(v) for v in traits.get('spans',()) if _finite(v) and v>0] or [4.])
    return focus, min(16., max(8., 2*span)) if long_phrase else 8., 16. if slow_bass else 4., slow_bass


def _rows(score):
    seconds, bpm = score.get('duration_seconds'), score.get('bpm')
    if not _finite(seconds) or seconds <= 1 or not _finite(bpm) or bpm<=0:
        raise ValueError('Critic requires a positive musical window and tempo')
    beat, window = 60./bpm, (seconds-1)*bpm/60.
    voices, rows, outside = score.get('voices') or {}, [], 0
    if not isinstance(score.get('events'),list):
        raise ValueError('Critic requires an explicit event list')
    for event in score['events']:
        start, length, pitch, velocity = (event.get(k) for k in ('start_seconds','duration_seconds','midi','velocity'))
        if not all(_finite(v) for v in (start,length,pitch,velocity)) or start<0 or length<=0 or not 0<=pitch<=127 or not 0<=velocity<=1:
            raise ValueError('Invalid finite note timing, MIDI pitch or velocity')
        role = event.get('layer')
        if not isinstance(role,str) or not role:
            raise ValueError('Critic requires named note layers')
        if start+length > seconds-1+1/48000:
            outside += 1
        if velocity == 0 or start >= seconds-1:
            continue
        rows.append({'role':role,'start':start/beat,'end':min(window,(start+length)/beat),
                     'pitch':pitch,'velocity':velocity,'voice':event.get('voice',voices.get(role))})
    rows.sort(key=lambda e:(e['start'],e['role'],e['pitch']))
    return rows, beat, window, outside


def _role_sweep(rows, window, focus, credit):
    edges = defaultdict(list)
    edges[0.]; edges[window]
    for e in rows:
        edges[e['start']].append((e['role'],1))
        # Sustained harmony can support long melodic phrases; a single chord
        # lasting the whole piece still cannot simulate changing activity.
        edges[min(e['end'],e['start']+credit)].append((e['role'],-1))
    active, before = Counter(), 0.
    parts = two = three = fg = supported = harmonic = 0.
    for now in sorted(edges):
        width = now-before
        roles = {r for r,n in active.items() if n>0}
        parts += width*len(roles)
        two += width*(len(roles)>=2)
        three += width*(len(roles)>=3)
        if roles & focus:
            fg += width
            supported += width*bool(roles-focus)
            harmonic += width*bool(roles-focus-{'bass'})
        for role,delta in edges[now]:
            active[role] += delta
        before = now
    return {'distinct_pitched_roles_mean':parts/window,'two_pitched_roles_share':two/window,
            'three_pitched_roles_share':three/window,'foreground_supported_share':supported/max(fg,1e-9),
            'foreground_harmonic_support_share':harmonic/max(fg,1e-9)}


def _phrase_facts(rows, window, beats):
    cells = defaultdict(list)
    for e in rows:
        i = int(e['start']//beats)
        if (i+1)*beats <= window+1e-8:
            cells[i].append(e)
    signatures = []
    for i, notes in sorted(cells.items()):
        if len(notes)<2:
            continue
        origin = notes[0]['pitch']
        # Ignore gain, transposition and timing jitter below an eighth beat.
        signatures.append(tuple((e['role'],e['pitch']-origin,
                                 round((e['start']-i*beats)*8),round((e['end']-e['start'])*8)) for e in notes))
    dominant = max(Counter(signatures).values(),default=0)/max(1,len(signatures))
    return dominant, len(signatures)


def _scalar_share(rows):
    flagged, total = 0, 0
    for role in sorted({e['role'] for e in rows}):
        groups = defaultdict(list)
        for e in rows:
            if e['role']==role:
                groups[round(e['start'],6)].append(e)
        run, direction, prior = [], 0, None
        def count():
            return len(run) if len(run)>=5 else 0
        for _, group in sorted(groups.items()):
            total += len(group)
            if len(group)!=1:
                flagged += count(); run, direction, prior = [], 0, None
                continue
            e = group[0]
            step = e['pitch']-prior['pitch'] if prior else 0
            sign = 1 if step>0 else -1
            links = prior is not None and 1<=abs(step)<=2 and e['start']-prior['end']<=2.
            if links and (direction==0 or sign==direction):
                run = run+[e] if run else [prior,e]
                direction = sign
            else:
                flagged += count(); run, direction = [e], 0
            prior = e
        flagged += count()
    return min(1.,flagged/max(1,total))


def _motif_reuse(rows):
    """Sliding onset groups expose loops that do not align to bar windows."""
    fingerprints = []
    for role in sorted({e['role'] for e in rows}):
        notes = [e for e in rows if e['role']==role]
        for i in range(0,len(notes)-7,4):
            part = notes[i:i+8]
            origin, onset = part[0]['pitch'], part[0]['start']
            fingerprints.append((role,tuple((e['pitch']-origin,round((e['start']-onset)*8),
                                             round((e['end']-e['start'])*8)) for e in part)))
    return 1.-len(set(fingerprints))/max(1,len(fingerprints)) if fingerprints else 0., len(fingerprints)


def evaluate(score):
    """Smooth training reward plus separate, conservative publication gates.

    The score is not mutated. Full musical-window denominators include rests
    and early endings. Rejected music still produces a graded learning signal.
    ``metrics`` is a stable JSON object; all timings ending in ``_beats`` are
    tempo-relative. No universal genre, instrument, loudness or density target.
    """
    rows, beat, window, outside = _rows(score)
    focus, fg_credit, bass_credit, slow_bass = _intent(score)
    pitched = [e for e in rows if e['voice'] not in DRUMS]
    foreground = [e for e in pitched if e['role'] in focus]
    bass = [e for e in pitched if e['role']=='bass']
    bass_spans = _merge((e['start'],e['end']) for e in bass)
    bass_active = _merge((e['start'],min(e['end'],e['start']+bass_credit)) for e in bass)
    fg_spans = _merge((e['start'],min(e['end'],e['start']+fg_credit)) for e in foreground)
    windows = [(a,min(window,a+32)) for a in np.arange(0,window,32.)]
    bass_windows = [_cover(bass_active,a,b) for a,b in windows]
    fg_windows = [_cover(fg_spans,a,b) for a,b in windows]
    bass_onsets = len({round(e['start'],6) for e in bass})/window
    bgaps, fgaps = _gaps(bass_spans,window), _gaps(fg_spans,window)
    fg_lengths = [e['end']-e['start'] for e in foreground]
    phrase_rows = [e for e in pitched if e['role'] in ('lead','answer')]
    if not phrase_rows:
        phrase_rows = foreground
    two_dom, two_count = _phrase_facts(phrase_rows,window,8.)
    eight_dom, eight_count = _phrase_facts(phrase_rows,window,32.)
    scalar_share = _scalar_share(phrase_rows)
    motif_reuse, motif_windows = _motif_reuse(phrase_rows)
    levels = {**LEVEL, **(score.get('mix') or {})}
    if any(not _finite(v) or v<0 for v in levels.values()):
        raise ValueError('Mix levels must be finite and nonnegative')
    power = defaultdict(float)
    for e in rows:
        power[e['voice'] in DRUMS] += (e['end']-e['start'])*(levels.get(e['role'],.3)*e['velocity'])**2
    noise_share = power[True]/max(sum(power.values()),1e-12)
    hats = sum(e['voice'] in DRUMS and e['pitch']>=72 for e in rows)/window
    velocities = [e['velocity'] for e in foreground]
    expression = statistics.pstdev(velocities)/max(_mean(velocities),.01) if velocities else 0.
    gesture_density = len({(e['role'],round(e['start'],6)) for e in foreground})/window
    metrics = {'window_seconds':window*beat,'window_beats':window,'notes':len(rows),'focus_roles':sorted(focus),
        'declared_slow_bass':slow_bass,'bass_activity_credit_beats':bass_credit,'foreground_activity_credit_beats':fg_credit,
        'bass_coverage':_cover(bass_spans,0,window),'bass_credited_coverage':_cover(bass_active,0,window),
        'bass_longest_gap_beats':max(bgaps),'bass_window_min_coverage':min(bass_windows,default=0.),
        'bass_onsets_per_beat':bass_onsets,'bass_window_coverages':bass_windows,
        'foreground_coverage':_cover(fg_spans,0,window),'foreground_longest_gap_beats':max(fgaps),
        'foreground_stall_excess_share':sum(max(0.,g-2.) for g in fgaps)/window,
        'foreground_window_min_coverage':min(fg_windows,default=0.),
        'foreground_short_note_share':sum(x<.28 for x in fg_lengths)/max(1,len(fg_lengths)),
        'foreground_onsets_per_beat':gesture_density,'relative_velocity_expression':expression,
        'dominant_two_bar_share':two_dom,'two_bar_cells':two_count,
        'dominant_eight_bar_share':eight_dom,'eight_bar_cells':eight_count,
        'scalar_run_note_share':scalar_share,'noise_energy_share_proxy':noise_share,'hat_onsets_per_beat':hats,
        'transposition_normalized_motif_reuse_share':motif_reuse,'motif_windows':motif_windows,
        'notes_outside_music_window':outside, **_role_sweep(pitched,window,focus,fg_credit)}
    # The onset floor is intentionally broad: held bass can change each two
    # bars; a pulse/walking part gets full credit by one onset each two beats.
    onset_goal = .125 if slow_bass else .50
    drive = (.26*min(1.,metrics['bass_coverage']/.88)+.24*min(1.,metrics['bass_credited_coverage']/.80)
             +.15*min(1.,metrics['bass_window_min_coverage']/.65)+.20*_band(bass_onsets,onset_goal,3.,3.)
             +.15*_clip(1.-sum(max(0.,g-1.) for g in bgaps)/max(window*.25,1.)))
    # _band's lower tail is unsuitable for missing onsets: explicitly grade
    # low-density activity proportionally, while keeping a wide upper plateau.
    drive -= .20*_band(bass_onsets,onset_goal,3.,3.)
    drive += .20*min(1.,bass_onsets/onset_goal)*_band(bass_onsets,0.,3.,3.)
    flow = (.40*min(1.,metrics['foreground_coverage']/(.5 if fg_credit>8 else .72))
            +.20*min(1.,metrics['foreground_window_min_coverage']/.30)
            +.25*_clip(1.-metrics['foreground_stall_excess_share']/.25)
            +.15*_band(metrics['foreground_short_note_share'],0.,.30,.55))
    depth = _mean((min(1.,metrics['two_pitched_roles_share']/.75),
                   min(1.,metrics['foreground_supported_share']/.80),
                   min(1.,metrics['foreground_harmonic_support_share']/.40)))
    repeat2 = _band(two_dom,0.,.65,.35) if two_count>=3 else .65
    repeat8 = _band(eight_dom,0.,.70,.30) if eight_count>=3 else .65
    development = (.30*repeat2+.15*repeat8+.25*_band(motif_reuse,0.,.40,.50)
                   +.30*_band(scalar_share,0.,.25,.60))
    restraint = _mean((_band(noise_share,0.,.15,.30),_band(hats,0.,2.5,4.),
                       _band(gesture_density,0.,3.5,4.),_band(expression,.015,.45,.15)))
    harmony = 1.
    if score.get('tonic') in TONICS and score.get('mode') in SCALES:
        pcs = {(TONICS.index(score['tonic'])+p)%12 for p in SCALES[score['mode']]}
        total = sum(e['end']-e['start'] for e in pitched)
        outside_key = sum(e['end']-e['start'] for e in pitched if e['pitch']%12 not in pcs)/max(total,1e-9)
        metrics['out_of_key_duration_share'] = outside_key
        harmony = _band(outside_key,0.,.10,.30)
    else:
        metrics['out_of_key_duration_share'] = None
    components = dict(drive=_clip(drive),flow=_clip(flow),depth=_clip(depth),development=_clip(development),
                      restraint=_clip(restraint),harmony=_clip(harmony))
    base = sum(components[k]*w for k,w in zip(COMPONENTS,(.28,.24,.18,.17,.08,.05)))
    safeguard = min(components[k] for k in ('drive','flow','development','restraint'))
    reward = base*(.45+.55*safeguard)*min(1.,len(rows)/8.)
    reasons = []
    if metrics['bass_coverage']<.60: reasons.append('bass_coverage_below_60_percent')
    if metrics['bass_longest_gap_beats']>max(4.,bass_credit): reasons.append('bass_stalls')
    if bass_onsets<onset_goal/2: reasons.append('bass_lacks_repeated_activity')
    if metrics['bass_credited_coverage']<.35: reasons.append('bass_gate_without_development')
    if metrics['foreground_coverage']<.30: reasons.append('foreground_missing_or_static')
    if metrics['foreground_stall_excess_share']>.15: reasons.append('foreground_stalls')
    if metrics['two_pitched_roles_share']<.35: reasons.append('insufficient_independent_layers')
    if two_count>=3 and two_dom>.85: reasons.append('repetitive_foreground_cells')
    if eight_count>=3 and eight_dom>.80: reasons.append('repetitive_eight_bar_blocks')
    if motif_windows>=5 and motif_reuse>.75: reasons.append('repetitive_foreground_motifs')
    if len(phrase_rows)>=24 and scalar_share>.55: reasons.append('extended_scalar_runs')
    if noise_share>.30: reasons.append('noise_dominates_score_balance')
    if gesture_density>6.: reasons.append('excess_foreground_attacks')
    if metrics['out_of_key_duration_share'] is not None and metrics['out_of_key_duration_share']>.30:
        reasons.append('widespread_out_of_key_notes')
    if outside: reasons.append('notes_outside_music_window')
    return {'version':VERSION,'reward':_clip(reward),'components':components,'metrics':metrics,
            'acceptable':not reasons,'rejection_reasons':reasons,'scope':SCOPE}


def assess_audio(score, audit, stems, audio):
    """Read-only technical/level checks on a render's actual float/PCM arrays.

    Pass either raw float mix+stems or equally mastered mix+stems. No frame is
    edited. Note hashes are renderer evidence; this function does not recreate
    individual waveforms or claim perceptual audibility through overlapping
    notes. Silent authored gates are checked against short actual-stem windows.
    """
    reasons, metrics = [], {}
    result = lambda: {'version':VERSION,'passed':not reasons,'rejection_reasons':sorted(set(reasons)),
                      'metrics':metrics,'scope':'Actual array/ledger integrity and level proxies; no listening or taste approval.'}
    sr = audit.get('sample_rate')
    if type(sr) is not int or sr<=0:
        reasons.append('invalid_sample_rate'); return result()
    frames = int(score.get('duration_seconds',0)*sr)
    music_frames = max(1,frames-sr)
    mix = np.asarray(audio,dtype=float)
    if mix.shape != (frames,2) or not np.all(np.isfinite(mix)):
        reasons.append('invalid_or_incomplete_mix'); return result()
    peak, rms = float(np.max(np.abs(mix))), float(np.sqrt(np.mean(mix**2)))
    metrics.update(frames=frames,sample_rate=sr,peak=peak,rms=rms)
    if audit.get('frames')!=frames or audit.get('channels')!=2:
        reasons.append('audit_recording_shape_mismatch')
    if peak>=1.: reasons.append('mix_peak_unsafe')
    if rms<=1e-8: reasons.append('silent_mix')
    events = score.get('events',[])
    ledger = audit.get('events',[])
    digest = hashlib.sha256(json.dumps(events,sort_keys=True).encode()).hexdigest()
    originals = {e.get('id'):e for e in events}
    ids = [e.get('id') for e in ledger]
    if not events or None in originals or len(originals)!=len(events) or len(ids)!=len(set(ids)) or set(ids)!=set(originals):
        reasons.append('note_ledger_identity_mismatch')
    if score.get('event_sha256')!=digest or audit.get('event_sha256')!=digest:
        reasons.append('event_hash_mismatch')
    if not audit.get('complete') or audit.get('notes_mixed')!=len(events) or audit.get('notes_scheduled')!=len(events) or len(ledger)!=len(events):
        reasons.append('incomplete_note_delivery')
    for e in ledger:
        source = originals.get(e.get('id'),{})
        if any(e.get(k)!=v for k,v in source.items()) or not e.get('mixed'):
            reasons.append('altered_or_unmixed_note')
        start, length = e.get('start_seconds'), e.get('duration_seconds')
        if not _finite(start) or not _finite(length) or start<0 or length<=0:
            reasons.append('invalid_ledger_timing'); continue
        left, size = int(start*sr), int(length*sr)
        if size<=0 or e.get('start_frame')!=left or e.get('frames')!=size or e.get('end_frame')!=left+size or left+size>frames:
            reasons.append('incorrect_note_frame_placement')
        if left+size>music_frames:
            reasons.append('note_overruns_musical_window')
        pcm_hash = e.get('note_pcm_sha256')
        if (not _finite(e.get('placed_rms')) or e['placed_rms']<=1e-8 or not isinstance(pcm_hash,str)
                or len(pcm_hash)!=64 or any(c not in '0123456789abcdef' for c in pcm_hash)):
            reasons.append('missing_note_pcm_evidence')
    rebuilt = np.zeros_like(mix)
    stem_power, suspect = {}, {}
    for role in PAN:
        if role not in stems:
            reasons.append('missing_stem'); continue
        part = np.asarray(stems[role],dtype=float).reshape(-1)
        if len(part)!=frames or not np.all(np.isfinite(part)):
            reasons.append('invalid_or_incomplete_stem'); continue
        power = float(np.mean(part[:music_frames]**2))
        stem_power[role] = power
        if float(np.max(np.abs(part)))>=1.: reasons.append('stem_peak_unsafe')
        notes = [e for e in events if e.get('layer')==role]
        if notes and power<=1e-16: reasons.append('silent_authored_stem')
        angle = (PAN[role]+1)*np.pi/4
        rebuilt[:,0] += part*np.cos(angle)
        rebuilt[:,1] += part*np.sin(angle)
        # An envelope may decay, but a gate cannot earn coverage for long
        # stretches whose measured power is effectively zero. Use 1/2 beat
        # windows and require 60% declared gate coverage before flagging one.
        step = max(1,int(.5*60/score['bpm']*sr))
        spans = _merge((max(0,int(e['start_seconds']*sr)),min(music_frames,int((e['start_seconds']+e['duration_seconds'])*sr))) for e in notes)
        threshold = max(1e-14,power*1e-5)
        checked = silent = 0
        for left in range(0,music_frames,step):
            right = min(music_frames,left+step)
            if _cover(spans,left,right)>=.60:
                checked += 1
                silent += float(np.mean(part[left:right]**2))<threshold
        # The approved guitar and mallet phrases naturally decay before their
        # notated gates finish. Do not require a marimba to behave like a pad.
        # Bass continuity is stricter; all roles still require actual attacks.
        absent_attacks = 0
        for event in notes:
            left = int(event['start_seconds']*sr)
            right = min(frames,left+max(1,int(min(.12,event['duration_seconds'])*sr)))
            absent_attacks += float(np.mean(part[left:right]**2))<threshold
        suspect[role] = {'covered_windows':checked,'near_silent_covered_windows':silent,
                         'authored_note_attacks':len(notes),'near_silent_attacks':absent_attacks}
        if notes and absent_attacks/max(1,len(notes))>.05:
            reasons.append('near_silent_authored_attacks')
        if checked and silent/checked>(.08 if role=='bass' else .35):
            reasons.append('near_silent_authored_gates')
    error = float(np.max(np.abs(rebuilt-mix)))
    metrics.update(stereo_reconstruction_error=error,stem_rms={r:p**.5 for r,p in stem_power.items()},gate_signal_windows=suspect)
    if error>4/32767.: reasons.append('mix_stem_reconstruction_mismatch')
    noise_power = sum(p for role,p in stem_power.items() if score.get('voices',{}).get(role) in DRUMS)
    ratio = noise_power/max(sum(stem_power.values()),1e-20)
    metrics['noise_stem_energy_share'] = ratio
    if ratio>.35: reasons.append('noise_dominates_actual_stems')
    # Relative level protection: nonzero but almost inaudible entire bass is
    # not accepted as continuous drive. This is not per-note perceptual proof.
    bass_rms = stem_power.get('bass',0.)**.5
    metrics['bass_to_mix_rms_ratio'] = bass_rms/max(rms,1e-12)
    if bass_rms/max(rms,1e-12)<.025: reasons.append('bass_level_negligible')
    return result()
