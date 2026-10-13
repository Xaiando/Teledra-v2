"""Audit the actual saved WAVs, exported stems and matched-note synthesis."""
from pathlib import Path
import hashlib
import itertools
import json
import wave
import numpy as np
from flycomposer.render import PAN

ROOT=Path(__file__).resolve().parent


def read_wav(path):
    with wave.open(str(path)) as wav:
        assert wav.getsampwidth()==2
        raw=wav.readframes(wav.getnframes())
        samples=np.frombuffer(raw,dtype='<i2').reshape(-1,wav.getnchannels()).astype(float)/32767
        return samples,wav.getframerate(),hashlib.sha256(raw).hexdigest()


def audit():
    index=json.loads((ROOT/'library/index.json').read_text(encoding='utf-8'))
    tracks={};scores={};rows=[]
    for track in index['tracks']:
        folder=ROOT/'library'/track['id']
        report=json.loads((folder/'audit.json').read_text(encoding='utf8'))
        score=json.loads((folder/'score.json').read_text(encoding='utf8'))
        audio,sr,digest=read_wav(folder/'audio.wav')
        assert sr==report['sample_rate'] and digest==report['pcm_sha256']
        assert len(audio)==report['frames'] and audio.shape[1]==2
        assert report['notes_mixed']==report['notes_scheduled']==len(score['events'])==len(report['events'])
        assert all(e['mixed'] and e['frames']==int(e['duration_seconds']*sr) and e['end_frame']<=len(audio) for e in report['events'])
        rebuilt=np.zeros_like(audio)
        for role in PAN:
            stem,stem_sr,_=read_wav(folder/f'{role}.wav')
            assert stem_sr==sr and len(stem)==len(audio)
            active=any(e['layer']==role for e in score['events'])
            if score.get('kind')!='performance':assert float(np.sqrt(np.mean(stem**2)))>1e-5
            elif not active:assert np.max(np.abs(stem))==0
            angle=(PAN[role]+1)*np.pi/4
            rebuilt+=stem*np.array([np.cos(angle),np.sin(angle)])
        # Independently quantized five stems can differ by at most six half-LSBs.
        assert np.max(np.abs(rebuilt-audio))<6/32767
        assert float(np.max(np.abs(audio)))<.821
        assert np.max(np.abs(audio[-int(sr*.75):]))==0
        actual_rms=float(np.sqrt(np.mean(audio**2)))
        assert abs(actual_rms-report['rms'])<1e-6
        tracks[track['id']]=report
        scores[track['id']]=score
        rows.append({'id':track['id'],'duration':len(audio)/sr,'notes_fired':len(report['events']),
                     'parts':len({e['layer'] for e in score['events']}),'peak':float(np.max(np.abs(audio))),'rms':actual_rms,'pcm_sha256':digest})
    comparisons=[]
    def note_key(e):return tuple(e[k] for k in ('layer','voice','midi','start_beat','length_beats','duration_seconds','velocity'))
    for comparison in index['comparisons']:
        reports=[tracks[i] for i in comparison['track_ids']]
        revision=comparison.get('kind')=='revision'
        performance=comparison.get('kind')=='performance'
        expected=len(reports) if performance else (2 if revision else 3)
        if performance:assert 0<=expected<=2
        assert len(reports)==len({r['pcm_sha256'] for r in reports})==expected
        rms_matched=bool(reports) and max(r['rms'] for r in reports)-min(r['rms'] for r in reports)<1e-12
        if not performance:assert rms_matched
        development=comparison.get('kind')=='development'
        if development:
            batch_scores=[scores[i] for i in comparison['track_ids']]
            assert len({s['direction'] for s in batch_scores})==3
            assert all(s['condition']=='connectome' for s in batch_scores)
            assert len({json.dumps(s['voices'],sort_keys=True) for s in batch_scores})==1
        if revision:
            original,updated=[scores[i] for i in comparison['track_ids']]
            assert all(original[k]==updated[k] for k in ('seed','voices','condition','direction','meter','bpm'))
            assert updated['revision_context']['source_track_id']==comparison['track_ids'][0]
            assert updated['event_sha256']!=original['event_sha256']
        if performance:
            batch_scores=[scores[i] for i in comparison['track_ids']]
            assert all(s['kind']=='performance' for s in batch_scores)
            authors={s['performer'] for s in batch_scores}
            assert len(authors)==len(batch_scores) and authors<=set(('fly','organist'))
            assert authors=={name for name,status in comparison['performer_status'].items() if status=='recorded'}
            if comparison.get('performance_mode')=='study':
                context=comparison['study_context']
                assert isinstance(context['feedback'],str) and context['feedback'].strip()
                assert comparison['track_ids'][0]==context['source_track_ids']['fly']
                for name,parent_id in context['source_track_ids'].items():
                    assert hashlib.sha256((ROOT/'library'/parent_id/'score.json').read_bytes()).hexdigest()==context['source_score_sha256'][name]
                for score in batch_scores:
                    if score['performer']=='organist':
                        parent=scores[context['source_track_ids']['organist']]
                        assert all(score[key]==parent[key] for key in ('seed','bpm','voices'))
                        assert score['study_context']==context
                        assert sorted(map(note_key,score['events']))!=sorted(map(note_key,parent['events']))
                        if score.get('arrangement_version')=='organist_planned_study_v2':
                            child_id=next(i for i in comparison['track_ids'] if scores[i]['performer']=='organist')
                            ledger=tracks[child_id]['events'];sr=tracks[child_id]['sample_rate']
                            limit=score['study']['continuity_contract']['max_silence_seconds']
                            def largest_gap(events,start,end):
                                cursor=start;maximum=0
                                for event in sorted(events,key=lambda e:e['start_frame']):
                                    left=max(start,event['start_frame']);right=min(end,event['end_frame'])
                                    if right<=left:continue
                                    maximum=max(maximum,left-cursor);cursor=max(cursor,right)
                                return max(maximum,end-cursor)/sr
                            end=max(int(max(p['end_beat'] for p in score['plan']['passages'])*60/score['bpm']*sr),
                                    max(e['end_frame'] for e in ledger))
                            actual=largest_gap(ledger,0,end)
                            assert actual<=limit
                            assert abs(actual-score['study']['continuity_evidence']['global_max_gap_seconds'])<1/sr
                            for n,passage in enumerate(score['plan']['passages']):
                                start=int(passage['start_beat']*60/score['bpm']*sr)
                                end=int(passage['end_beat']*60/score['bpm']*sr)
                                assert largest_gap([e for e in ledger if e['source_passage']==n],start,end)<=limit
            if comparison.get('performance_mode')=='continuation':
                for score in batch_scores:
                    performer=score['performer'];parent_id=comparison['source_track_ids'][performer]
                    parent=scores[parent_id]
                    assert all(score[key]==parent[key] for key in ('seed','bpm','voices'))
                    expected=comparison['source_score_sha256'][performer]
                    assert hashlib.sha256((ROOT/'library'/parent_id/'score.json').read_bytes()).hexdigest()==expected
                    continued={event['id']:event for event in score['events']}
                    assert all(continued.get(event['id'])==event for event in parent['events'])
                    child_id=next(i for i in comparison['track_ids'] if scores[i]['performer']==performer)
                    child_pcm={event['id']:event['note_pcm_sha256'] for event in tracks[child_id]['events']}
                    assert all(child_pcm[event['id']]==event['note_pcm_sha256'] for event in tracks[parent_id]['events'])
        common_counts=[]
        for left,right in itertools.combinations(reports,2):
            a={note_key(e):e['note_pcm_sha256'] for e in left['events']}
            b={note_key(e):e['note_pcm_sha256'] for e in right['events']}
            common=a.keys()&b.keys()
            if not development and not revision and not performance:assert len(common)>100
            assert all(a[k]==b[k] for k in common),'Shared note timbre changed between controllers'
            common_counts.append(len(common))
        comparisons.append({'id':comparison['id'],'seed':comparison['seed'],'ensemble':comparison['ensemble'],
                            'kind':comparison.get('kind','wiring'),'distinct_audio_files':expected,'shared_notes_with_identical_synthesis_per_pair':common_counts,
                            'rms_matched':rms_matched})
    result={'status':'passed','pieces':rows,'comparisons':comparisons,'total_notes_fired':sum(r['notes_fired'] for r in rows),
            'scope':'Actual PCM and stem consistency, note completeness and controlled timbres. No human listening verdict inferred.'}
    path=ROOT/'reports/audio-validation.json';path.parent.mkdir(exist_ok=True)
    path.write_text(json.dumps(result,indent=2),encoding='utf8')
    print(json.dumps({'status':result['status'],'pieces':len(rows),'comparisons':len(comparisons),'total_notes_fired':result['total_notes_fired']}))
    return result


if __name__=='__main__':audit()
