"""Matched offline comparisons and a persistent, local listening library."""
from __future__ import annotations

import datetime as dt
import copy
import itertools
import hashlib
import json
import os
from pathlib import Path
import threading
import uuid
import numpy as np

from .connectome import load_circuit
from .composition import write_song, assess_score, ENSEMBLES, ROLES
from .controller import CONDITIONS
from .render import render_score, save_render, assess_performance
from .preferences import summarize_preferences


def atomic_json(path, value):
    path.parent.mkdir(parents=True,exist_ok=True)
    pending=path.with_suffix('.next.json')
    pending.write_text(json.dumps(value,ensure_ascii=False,indent=2,allow_nan=False),encoding='utf-8')
    os.replace(pending,path)


def compare_scores(scores):
    comparisons=[]
    for left,right in itertools.combinations(scores,2):
        a,b=scores[left],scores[right]
        rows_a={(e['layer'],round(e['start_beat'],4),e['midi'],e['length_beats']) for e in a['events']}
        rows_b={(e['layer'],round(e['start_beat'],4),e['midi'],e['length_beats']) for e in b['events']}
        phrase_changes=sum(x['lead_signature']!=y['lead_signature'] for x,y in zip(a['phrases'],b['phrases']))
        comparisons.append({'left':left,'right':right,'changed_lead_phrases':phrase_changes,
                            'event_jaccard_distance':1-len(rows_a&rows_b)/len(rows_a|rows_b),
                            'identical_event_ledger':a['event_sha256']==b['event_sha256']})
    return comparisons


class Studio:
    def __init__(self, root):
        self.root=Path(root).resolve()
        self.index=self.root/'library/index.json'
        self.lock=threading.RLock()
        self.busy=False;self.progress='Ready';self.error=None
        self.circuit=load_circuit(self.root/'data/circuit.npz')
        self.library=json.loads(self.index.read_text(encoding='utf-8')) if self.index.exists() else {'version':1,'tracks':[],'comparisons':[]}
        if self.library.get('version')!=1 or not isinstance(self.library.get('tracks'),list):
            raise ValueError('Library is invalid; preserve and inspect the original index')
        self.catalog=json.loads((self.root/'data/instruments.json').read_text(encoding='utf-8'))

    def status(self, message):
        with self.lock:self.progress=message

    def state(self):
        with self.lock:
            state=json.loads(json.dumps(self.library))
            preferences=summarize_preferences(self.library)
            by_id={t['id']:t for t in state['tracks']}
            visible=set()
            for comparison in state['comparisons']:
                ids=set(comparison['track_ids'])
                if comparison['revealed']:visible|=ids
                comparison['tracks']=[{'id':track_id,'label':chr(65+i),
                                       'audio_url':f'/audio/{track_id}/audio.wav',
                                       'duration':next(t['duration'] for t in state['tracks'] if t['id']==track_id),
                                       'condition':next(t['condition'] for t in state['tracks'] if t['id']==track_id) if comparison['revealed'] else None,
                                       'direction':next(t.get('direction') for t in state['tracks'] if t['id']==track_id),
                                       'direction_name':next(t.get('direction_name') for t in state['tracks'] if t['id']==track_id),
                                       'description':comparison.get('variants',{}).get(track_id,{}).get('description',by_id[track_id].get('description')),
                                       'display_name':comparison.get('variants',{}).get(track_id,{}).get('display_name')}
                                      for i,track_id in enumerate(comparison['track_ids'])]
                comparison.pop('evaluation',None)  # Detailed metrics include unblinded controller names.
                if comparison.get('kind')=='benchmark' and not comparison['revealed']:
                    # Blind listening is a presentation aid, not an access boundary.
                    # Preserve the saved evidence and listener notes; mask author labels in this copy.
                    for track in comparison['tracks']:
                        track['direction']=None;track['direction_name']=None
                        track['display_name']=f"Sample {track['label']}"
                        track['description']='Identity hidden until Reveal.'
                    comparison['variants']={track['id']:{'display_name':track['display_name'],
                                            'description':track['description']} for track in comparison['tracks']}
                    if comparison.get('recording_errors'):
                        comparison['recording_errors']=[
                            'A performer did not produce a valid recording. Reveal identities to inspect the error details.'
                            for _ in comparison['recording_errors']]
                    comparison.pop('performer_status',None)
                    if isinstance(comparison.get('benchmark'),dict):
                        comparison['benchmark'].pop('performers',None)
            for track in state['tracks']:
                if track['id'] not in visible:track['condition']='hidden'
                if track.get('kind')=='benchmark' and track['id'] not in visible:
                    comparison=next((c for c in state['comparisons'] if track['id'] in c['track_ids']),None)
                    label=next((t['label'] for t in comparison['tracks'] if t['id']==track['id']),'') if comparison else ''
                    track['title']=f"{comparison['title']} · Sample {label}" if comparison else 'Benchmark sample'
                    track['description']='Identity hidden until Reveal.'
                    track['performer']='hidden'
                    for key in ('display_name','direction_name','brain','critic'):
                        track.pop(key,None)
                track['audio_url']=f"/audio/{track['id']}/audio.wav"
            metadata=self.circuit['metadata']
            # A revision references its parent without duplicating the saved piece.
            ordered_ids=dict.fromkeys(track_id for comparison in reversed(state['comparisons']) for track_id in comparison['track_ids'])
            return {'busy':self.busy,'progress':self.progress,'error':self.error,
                    'capabilities':{'organist_study_paging':True},
                    'graph':{**{k:metadata[k] for k in ('nodes','edges','region','provenance_url')},
                             'node_kind':metadata['node_kind'],'represented_neurons':metadata.get('represented_neurons')},
                    'preferences':preferences,'catalog':self.catalog,'library':[by_id[track_id] for track_id in ordered_ids],
                    'comparisons':list(reversed(state['comparisons']))}

    def validate(self, seed, ensemble, voices=None):
        if type(seed)is not int or not 0<=seed<=2**31-1 or ensemble not in ENSEMBLES:
            raise ValueError('Choose a supported ensemble and seed')
        if voices is not None:
            allowed={v['id'] for v in self.catalog}|{'classic:'+v for v in ('sine','triangle','pluck','bell','pad','keys','bass')}
            if not isinstance(voices,dict) or set(voices)!=set(ROLES) or any(not isinstance(v,str) or v not in allowed for v in voices.values()):
                raise ValueError('Choose one catalog instrument for each of the five parts')

    def compose(self, seed, ensemble='keys', blind=True, voices=None, *, _reserved=False):
        self.validate(seed,ensemble,voices)
        with self.lock:
            if self.busy and not _reserved:raise ValueError('A comparison is already being rendered')
            self.busy=True;self.error=None
        try:
            self.status('Developing four sections through the selected circuit')
            scores={condition:write_song(self.circuit,seed,ensemble,condition,voices) for condition in (*CONDITIONS,'disconnected')}
            evaluation=compare_scores(scores)
            for score in scores.values():assess_score(score)
            ablation=next(r for r in evaluation if {r['left'],r['right']}=={'connectome','disconnected'})
            if ablation['identical_event_ledger']:
                raise ValueError('Wiring had no measurable effect after musical decoding; experiment not published')
            rendered={}
            for i,condition in enumerate(CONDITIONS):
                self.status(f'Rendering comparison {i+1}/3')
                rendered[condition]=render_score(scores[condition],progress=lambda text,i=i:self.status(f'Comparison {i+1}/3: {text}'))
            target=min(.055,*[.82*x[2]['raw_rms']/x[2]['raw_peak'] for x in rendered.values()])
            comparison_id=uuid.uuid4().hex[:16]
            created=dt.datetime.now(dt.timezone.utc).isoformat()
            tracks=[]
            ids={condition:uuid.uuid4().hex[:16] for condition in CONDITIONS}
            order=list(CONDITIONS)
            np.random.default_rng(seed+61991).shuffle(order)
            labels={condition:chr(65+order.index(condition)) for condition in CONDITIONS}
            for condition,(audio,stems,audit) in rendered.items():
                track_id=ids[condition]
                audit=save_render(self.root/'library'/track_id,scores[condition],audio,stems,audit,target)
                tracks.append({'id':track_id,'title':f'Wing Study {seed} · {labels[condition]}',
                               'created_at':created,'duration':audit['duration_seconds'],'condition':condition,
                               'seed':seed,'ensemble':ensemble,'voices':scores[condition]['voices'],
                               'metrics':{k:audit[k] for k in ('notes_scheduled','notes_mixed','layers','sections','peak','rms','unique_lead_phrases','pcm_sha256')},
                               'verdict':None,'note':'','comparison_id':comparison_id})
            comparison={'id':comparison_id,'title':f'Wing Study {seed} / {ensemble}',
                        'seed':seed,'ensemble':ensemble,'created_at':created,'revealed':not blind,
                        'track_ids':[ids[c] for c in order],'feedback':{},'evaluation':evaluation,
                        'scope':'Technical comparison only. Connectivity causes measured differences; musical superiority requires listening.',
                        'rms_target':target}
            with self.lock:
                candidate=copy.deepcopy(self.library)
                candidate['tracks'].extend(tracks);candidate['comparisons'].append(comparison)
                atomic_json(self.index,candidate)
                self.library=candidate
            atomic_json(self.root/'reports'/f'comparison-{comparison_id}.json',comparison)
            self.status('Three complete pieces saved. Ready for blind listening.')
            return comparison
        except Exception as error:
            with self.lock:self.error=str(error)
            raise
        finally:
            with self.lock:self.busy=False

    def start(self, seed, ensemble, blind=True, voices=None):
        self.validate(seed,ensemble,voices)
        # Reserve the worker synchronously to avoid simultaneous POST races.
        with self.lock:
            if self.busy:raise ValueError('A comparison is already rendering')
            self.busy=True;self.error=None;self.progress='Starting the comparison'
        def worker():
            try:
                self.compose(seed,ensemble,blind,voices,_reserved=True)
            except Exception:
                pass  # Error is retained in the state for the UI.
        threading.Thread(target=worker,daemon=True,name='fly-composition').start()

    def develop(self, seed, ensemble=None, voices=None, *, _reserved=False):
        """Three contrasting musical studies using a shared palette and controller.

        These intentionally vary arrangement and meter, so are not wiring controls.
        """
        from .development import write_development, DIRECTIONS
        with self.lock:
            preference=summarize_preferences(self.library)
            if ensemble is None:
                ensemble=preference['preferred_ensemble']
                if voices is None:voices=preference['preferred_voices']
            self.validate(seed,ensemble,voices)
            if self.busy and not _reserved:raise ValueError('A comparison is already being rendered')
            self.busy=True;self.error=None
            active_voices=voices or ENSEMBLES[ensemble]
            parents=[t['id'] for t in self.library['tracks'] if t.get('verdict')=='keep' and t.get('voices')==active_voices]
        try:
            preferred=[d['id'] for d in preference['direction_ranking'] if d['mean_verdict']>0 and d['id'] in DIRECTIONS]
            order=preferred+[d for d in DIRECTIONS if d not in preferred]
            self.status('Developing contrasting forms from the listening feedback')
            revisions=preference.get('preferred_revisions',{})
            scores={direction:write_development(self.circuit,seed,ensemble,'connectome',active_voices,direction,
                                               revision=revisions.get(direction)) for direction in order}
            for score in scores.values():
                assess_score(score)
                score['learning_context']={'preference_version':preference['version'],
                    'evidence_sha256':preference['evidence_sha256'],'parent_track_ids':parents,
                    'scope':'Reuses a listener-favored palette with engineered new arrangements; no neural retraining.'}
            evaluation=compare_scores(scores)
            if any(e['identical_event_ledger'] or e['event_jaccard_distance']<.1 for e in evaluation):
                raise ValueError('Development directions collapsed into similar note plans')
            rendered={}
            for i,(direction,score) in enumerate(scores.items()):
                self.status(f"Rendering {DIRECTIONS[direction]['name']}")
                rendered[direction]=render_score(score,progress=lambda message,i=i:self.status(f'Study {i+1}/3: {message}'))
            target=min(.055,*[.82*x[2]['raw_rms']/x[2]['raw_peak'] for x in rendered.values()])
            batch_id=uuid.uuid4().hex[:16];created=dt.datetime.now(dt.timezone.utc).isoformat();tracks=[]
            for direction,(audio,stems,audit) in rendered.items():
                track_id=uuid.uuid4().hex[:16];score=scores[direction]
                audit=save_render(self.root/'library'/track_id,score,audio,stems,audit,target)
                tracks.append({'id':track_id,'title':f"{DIRECTIONS[direction]['name']} · {seed}",
                    'created_at':created,'duration':audit['duration_seconds'],'condition':'connectome',
                    'kind':'development','direction':direction,'direction_name':DIRECTIONS[direction]['name'],
                    'description':DIRECTIONS[direction]['description'],'seed':seed,'ensemble':ensemble,
                    'voices':score['voices'],'parent_track_ids':parents,
                    'meter':score.get('meter'),'arrangement_version':score.get('arrangement_version'),'revision':score.get('revision'),
                    'metrics':{k:audit[k] for k in ('notes_scheduled','notes_mixed','layers','sections','peak','rms','unique_lead_phrases','pcm_sha256')},
                    'verdict':None,'note':'','comparison_id':batch_id})
            comparison={'id':batch_id,'kind':'development','title':f'{ensemble.title()} development · {seed}',
                'seed':seed,'ensemble':ensemble,'created_at':created,'revealed':True,
                'track_ids':[t['id'] for t in tracks],'feedback':{},'evaluation':evaluation,'rms_target':target,
                'learning_context':{'evidence_sha256':preference['evidence_sha256'],'parent_track_ids':parents,
                                    'supporting_batch_ids':preference['supporting_batch_ids']},
                'scope':'Three contrasting arrangements with the same palette and real circuit. This is a development session, not a matched wiring comparison.'}
            with self.lock:
                candidate=copy.deepcopy(self.library)
                candidate['tracks'].extend(tracks);candidate['comparisons'].append(comparison)
                atomic_json(self.index,candidate);self.library=candidate
            atomic_json(self.root/'reports'/f'development-{batch_id}.json',comparison)
            self.status('Three new studies saved; your kept pieces and feedback are preserved.')
            return comparison
        except Exception as error:
            with self.lock:self.error=str(error)
            raise
        finally:
            with self.lock:self.busy=False

    def start_development(self, seed, ensemble=None, voices=None):
        with self.lock:
            resolved=ensemble or summarize_preferences(self.library)['preferred_ensemble']
            self.validate(seed,resolved,voices)
            if self.busy:raise ValueError('A comparison is already rendering')
            self.busy=True;self.error=None;self.progress='Starting musical development'
        def worker():
            try:self.develop(seed,ensemble,voices,_reserved=True)
            except Exception as error:
                # Includes import/validation failures before develop's try/finally.
                with self.lock:self.error=str(error);self.busy=False
        threading.Thread(target=worker,daemon=True,name='fly-development').start()

    def revision_source(self, track_id, revision):
        if revision!='connected_phrases_v1':raise ValueError('Unknown musical revision')
        source=next((t for t in self.library['tracks'] if t['id']==track_id),None)
        if not source or source.get('kind')!='development' or source.get('direction')!='sparks' or source.get('arrangement_version')!='development_v1' or source.get('revision'):
            raise ValueError('This revision requires an original Crossing Sparks development study')
        if source.get('condition')!='connectome':raise ValueError('Unsupported source controller')
        self.validate(source['seed'],source['ensemble'],source['voices'])
        return source

    def revise(self, track_id, revision='connected_phrases_v1', *, _reserved=False):
        """Render one opt-in musical revision; retain its original for comparison."""
        from .development import write_development
        with self.lock:
            source=copy.deepcopy(self.revision_source(track_id,revision))
            if self.busy and not _reserved:raise ValueError('A comparison is already being rendered')
            preference=summarize_preferences(self.library)
            source_comparison=next(c for c in self.library['comparisons'] if c['id']==source['comparison_id'])
            context={'revision':revision,'source_track_id':track_id,'source_comparison_id':source['comparison_id'],
                     'source_feedback':copy.deepcopy(source_comparison.get('feedback',{})),
                     'source_track_note':source.get('note',''),'evidence_sha256':preference['evidence_sha256'],
                     'scope':'An explicit compositional revision prompted by listening feedback; no inferred Keep or neural retraining.'}
            self.busy=True;self.error=None
        try:
            folder=self.root/'library'/track_id
            original=json.loads((folder/'score.json').read_text(encoding='utf8'))
            source_audit=json.loads((folder/'audit.json').read_text(encoding='utf8'))
            if original['seed']!=source['seed'] or original['voices']!=source['voices'] or original['condition']!=source['condition']:
                raise ValueError('Original score does not match its library record')
            reference=write_development(self.circuit,source['seed'],source['ensemble'],source['condition'],
                                        source['voices'],source['direction'])
            identity=('events','event_sha256','tonic','mode','bpm','meter','bars','sections','duration_seconds','controller')
            if any(reference[key]!=original[key] for key in identity):
                raise ValueError('Current composer cannot reproduce this exact saved original; revision refused')
            self.status('Connecting the phrases in Crossing Sparks')
            score=write_development(self.circuit,source['seed'],source['ensemble'],source['condition'],
                                    source['voices'],source['direction'],revision=revision)
            assess_score(score)
            if score['event_sha256']==original['event_sha256']:raise ValueError('Revision did not change the music')
            score['revision_context']=context
            audio,stems,audit=render_score(score,progress=self.status)
            target=source_audit['rms']
            if audit['raw_peak']*target/audit['raw_rms']>.82:
                raise ValueError('Revision cannot match original RMS with the required peak headroom')
            batch_id=uuid.uuid4().hex[:16];new_id=uuid.uuid4().hex[:16]
            created=dt.datetime.now(dt.timezone.utc).isoformat()
            audit=save_render(self.root/'library'/new_id,score,audio,stems,audit,target)
            description='The same theme and syncopation, with connected phrases and a clearer handoff through the quiet section.'
            track={'id':new_id,'title':f"Crossing Sparks — connected · {source['seed']}",
                   'created_at':created,'duration':audit['duration_seconds'],'condition':source['condition'],
                   'kind':'revision','direction':source['direction'],'direction_name':'Crossing Sparks — connected',
                   'description':description,'seed':source['seed'],'ensemble':source['ensemble'],'voices':score['voices'],
                   'parent_track_ids':[track_id],'source_track_id':track_id,'revision':revision,
                   'meter':score.get('meter'),'arrangement_version':score.get('arrangement_version'),
                   'metrics':{k:audit[k] for k in ('notes_scheduled','notes_mixed','layers','sections','peak','rms','unique_lead_phrases','pcm_sha256')},
                   'verdict':None,'note':'','comparison_id':batch_id}
            comparison={'id':batch_id,'kind':'revision','title':'Crossing Sparks · smoother phrasing',
                        'created_at':created,'seed':source['seed'],'ensemble':source['ensemble'],'revealed':True,
                        'source_track_id':track_id,'track_ids':[track_id,new_id],'feedback':{},'rms_target':target,
                        'variants':{track_id:{'display_name':'Original Crossing Sparks','description':source.get('description','Original study.')},
                                    new_id:{'display_name':'Revised Crossing Sparks','description':description}},
                        'revision_context':context,'evaluation':compare_scores({'original':original,'revision':score}),
                        'scope':'Original and targeted revision: the same seed, instruments, circuit and whole-piece RMS.'}
            with self.lock:
                candidate=copy.deepcopy(self.library)
                candidate['tracks'].append(track);candidate['comparisons'].append(comparison)
                atomic_json(self.index,candidate);self.library=candidate
            atomic_json(self.root/'reports'/f'revision-{batch_id}.json',comparison)
            self.status('Crossing Sparks revision saved beside its original. Listening decides whether it improves the piece.')
            return comparison
        except Exception as error:
            with self.lock:self.error=str(error)
            raise
        finally:
            with self.lock:self.busy=False

    def start_revision(self, track_id, revision='connected_phrases_v1'):
        with self.lock:
            self.revision_source(track_id,revision)
            if self.busy:raise ValueError('A comparison is already rendering')
            self.busy=True;self.error=None;self.progress='Starting the musical revision'
        def worker():
            try:self.revise(track_id,revision,_reserved=True)
            except Exception as error:
                with self.lock:self.error=str(error);self.busy=False
        threading.Thread(target=worker,daemon=True,name='fly-revision').start()

    def _performance_sources(self, source_id, seconds):
        source=next((c for c in self.library['comparisons'] if c['id']==source_id),None)
        if not source or source.get('kind')!='performance':
            raise ValueError('Choose a saved independent performance session to continue')
        tracks={t['id']:t for t in self.library['tracks']}
        sources={}
        for track_id in source['track_ids']:
            track=tracks.get(track_id)
            if not track or track.get('performer') not in ('fly','organist') or track['performer'] in sources:
                raise ValueError('The source session has inconsistent author identities')
            folder=self.root/'library'/track_id
            if not all((folder/name).is_file() for name in ('audio.wav','score.json','audit.json')):
                raise ValueError('A source recording has missing files; continuation refused')
            try:raw=(self.root/'library'/track_id/'score.json').read_bytes()
            except OSError as error:raise ValueError('A source score is unavailable; original recordings were not changed') from error
            score=json.loads(raw)
            assess_performance(score)
            if score.get('performer')!=track['performer'] or score.get('seed')!=source['seed']:
                raise ValueError('The source score does not match its saved performer')
            if seconds<score['duration_seconds']:
                raise ValueError('Choose a target at least as long as both source recordings')
            finished=score['duration_seconds']>=seconds-2
            sources[track['performer']]={'track_id':track_id,'score':score,'score_sha256':hashlib.sha256(raw).hexdigest(),'finished':finished}
        if set(sources)!={'fly','organist'}:
            raise ValueError('Both source recordings must exist before continuing the pair')
        return source['seed'],sources

    def _performance_request(self, seed, seconds, retry_id, source_id=None):
        """Resolve a new capture or missing authors from a saved session.

        Caller holds the studio lock. Saved track identity, rather than a stale
        progress label, determines whether an author has already succeeded.
        """
        comparison=None
        missing=['fly','organist']
        if retry_id is not None:
            if source_id is not None:raise ValueError('Choose either a retry or a new continuation')
            if not isinstance(retry_id,str) or not retry_id:
                raise ValueError('Retry must identify an existing performance session')
            source=next((c for c in self.library['comparisons'] if c['id']==retry_id),None)
            if not source or source.get('kind')!='performance':
                raise ValueError('Unknown performance session for retry')
            if source.get('performance_mode')=='study':
                raise ValueError('Retry this feedback study through the Organist study action')
            comparison=copy.deepcopy(source)
            seed=comparison.get('seed');seconds=comparison.get('recording_seconds')
            source_id=comparison.get('source_comparison_id')
            tracks={track['id']:track for track in self.library['tracks']}
            recorded=set()
            for track_id in comparison['track_ids']:
                track=tracks.get(track_id)
                if not track or track.get('kind')!='performance' or track.get('performer') not in missing or track['performer'] in recorded:
                    raise ValueError('Saved performance references are inconsistent; retry refused')
                recorded.add(track['performer'])
                folder=self.root/'library'/track_id
                if not all((folder/name).is_file() for name in ('audio.wav','score.json','audit.json')):
                    raise ValueError('A previously recorded performance has missing files; retry refused')
            if any(status=='recorded' and performer not in recorded for performer,status in comparison.get('performer_status',{}).items()):
                raise ValueError('Saved recorded status has no corresponding track; retry refused')
            missing=[performer for performer in missing if performer not in recorded]
            if not missing:raise ValueError('This performance session is already complete')
        if type(seconds) is not int or not 30<=seconds<=120:
            raise ValueError('Choose a recording length of 30–120 seconds')
        if source_id is not None:
            seed,sources=self._performance_sources(source_id,seconds)
            if comparison is None:
                missing=[name for name,value in sources.items() if not value['finished']]
                if not missing:raise ValueError('Both performers have already reached this target')
            expected=comparison.get('source_score_sha256') if comparison else None
            if expected and expected!={name:value['score_sha256'] for name,value in sources.items()}:
                raise ValueError('A source score changed since this continuation began; retry refused')
        self.validate(seed,'luminous')
        return seed,seconds,comparison,missing

    def _persist_performance(self, comparison, track=None):
        """Publish a status or verified recording against the latest feedback."""
        with self.lock:
            candidate=copy.deepcopy(self.library)
            proposed=copy.deepcopy(comparison)
            previous=next((c for c in candidate['comparisons'] if c['id']==proposed['id']),None)
            if track is not None:
                if any(t['id']==track['id'] for t in candidate['tracks']):
                    raise ValueError('Recording identity is already published')
                candidate['tracks'].append(copy.deepcopy(track))
            if previous:
                proposed['feedback']=copy.deepcopy(previous.get('feedback',{}))
                candidate['comparisons'][candidate['comparisons'].index(previous)]=proposed
            else:candidate['comparisons'].append(proposed)
            atomic_json(self.index,candidate)
            self.library=candidate
            self.error='; '.join(proposed.get('recording_errors',[])) or None
            return copy.deepcopy(proposed)

    def perform(self, seed=2026, seconds=75, *, retry_id=None, source_id=None, _reserved=False):
        """Independent authors, recorded through one common note renderer.

        Publish each successful recording immediately. One author's failure must
        not discard the other or replace it with a template.
        """
        from .autonomous import write_performance, extend_performance
        from .organist import generate_performance
        with self.lock:
            seed,seconds,comparison,missing=self._performance_request(seed,seconds,retry_id,source_id)
            source_id=comparison.get('source_comparison_id') if comparison else source_id
            sources=self._performance_sources(source_id,seconds)[1] if source_id else None
            if self.busy and not _reserved:raise ValueError('A session is already recording')
            self.busy=True;self.error=None
        created=dt.datetime.now(dt.timezone.utc).isoformat()
        if comparison is None:
            comparison={'id':uuid.uuid4().hex[:16],'kind':'performance','title':f'Fly & Organist · independent session {seed}',
                        'seed':seed,'ensemble':'independent','created_at':created,'revealed':True,
                        'track_ids':[],'variants':{},'feedback':{},'recording_seconds':seconds,
                        'performer_status':{'fly':'waiting','organist':'waiting'},
                        'scope':'Independent note authors using the same PyMusic renderer. Recorded sessions, not live Court control or a matched wiring experiment.'}
            if sources:
                parent=next(c for c in self.library['comparisons'] if c['id']==source_id)
                round_number=parent.get('continuation_round',1 if parent.get('performance_mode')=='continuation' else 0)+1
                comparison.update(title=f'Fly & Organist · continuation {round_number} · {seconds}s target',performance_mode='continuation',continuation_round=round_number,
                    source_comparison_id=source_id,source_track_ids={name:value['track_id'] for name,value in sources.items()},
                    source_score_sha256={name:value['score_sha256'] for name,value in sources.items()},
                    continuation_status={},scope='Each performer continues its own prior notes toward the same duration target. Original recordings are retained; extended recordings are remastered separately.')
                for name,value in sources.items():
                    if value['finished']:
                        prior=next(t for t in self.library['tracks'] if t['id']==value['track_id'])
                        comparison['track_ids'].append(prior['id'])
                        comparison['variants'][prior['id']]={'display_name':prior['title'],'description':'Previously completed recording retained for comparison.'}
                        comparison['performer_status'][name]='recorded'
                        comparison['continuation_status'][name]={'status':'complete','target_reached':True,'actual_seconds':prior['duration'],'reused':True}
        else:
            comparison.setdefault('retry_history',[]).append({
                'retried_at':created,'performers':list(missing),
                'previous_performer_status':copy.deepcopy(comparison.get('performer_status',{})),
                'recording_errors':copy.deepcopy(comparison.get('recording_errors',[]))})
            comparison['performer_status']={performer:'waiting' if performer in missing else 'recorded' for performer in ('fly','organist')}
        comparison['recording_errors']=[]
        batch_id=comparison['id']
        try:
            comparison=self._persist_performance(comparison)
            if sources:
                from .organist_continuation import generate_continuation
                authors=(('fly',lambda:extend_performance(self.circuit,sources['fly']['score'],seconds=seconds)),
                         ('organist',lambda:generate_continuation(self.root,sources['organist']['score'],seconds=seconds,progress=self.status)))
            else:
                authors=(('fly',lambda:write_performance(self.circuit,seed=seed,seconds=seconds)),
                         ('organist',lambda:generate_performance(self.root,seed=seed,seconds=seconds,progress=self.status)))
            for performer,author in authors:
                if performer not in missing:continue
                name='Fly' if performer=='fly' else 'Organist'
                self.status(f'{name} is creating its independent performance')
                comparison['performer_status'][performer]='generating'
                comparison=self._persist_performance(comparison)
                try:
                    score=author();assess_performance(score)
                    if score.get('performer')!=performer:raise ValueError('Recording author identity mismatch')
                    if sources:
                        source_score=sources[performer]['score']
                        if any(score.get(key)!=source_score.get(key) for key in ('seed','bpm','voices')):
                            raise ValueError('Continuation changed the source seed, tempo or instruments')
                        old=sources[performer]['score']['events'];by_id={e['id']:e for e in score['events']}
                        if any(by_id.get(e['id'])!=e for e in old):raise ValueError('Continuation changed an original authored note')
                        if len(score['events'])<=len(old):raise ValueError('No new material was authored; the source recording remains available')
                    audio,stems,audit=render_score(score,progress=lambda message:self.status(f'{name}: {message}'))
                    peak=max([audit['raw_peak']]+[float(np.max(np.abs(stem))) for stem in stems.values()])
                    target=min(.055,.82*audit['raw_rms']/peak)
                    track_id=uuid.uuid4().hex[:16]
                    audit=save_render(self.root/'library'/track_id,score,audio,stems,audit,target)
                    description=('Evolving fly-circuit activity controls its musical sequence through a designed note interpreter.' if performer=='fly' else
                                 'The configured local model writes its own note clips and placements as the Organist persona, with no supplied song arrangement.')
                    if sources:
                        description=('The same circuit state and musical memory continue beyond the earlier capture.' if performer=='fly' else
                                     'The local Organist develops its own prior phrases over successive authoring turns.')
                        if score.get('continuation',{}).get('status')=='partial':
                            description+=f" Partial continuation: {score['duration_seconds']:.1f}s of the {seconds}s target."
                    track={'id':track_id,'title':f"{name} · {score['title']}",'created_at':created,
                           'duration':audit['duration_seconds'],'condition':performer,'kind':'performance','performer':performer,
                           'description':description,'seed':seed,'ensemble':'independent','voices':score['voices'],
                           'arrangement_version':score.get('arrangement_version','independent_performance_v1'),
                           'metrics':{k:audit[k] for k in ('notes_scheduled','notes_mixed','layers','sections','peak','rms','unique_lead_phrases','pcm_sha256')},
                           'verdict':None,'note':'','comparison_id':batch_id}
                    if sources:
                        track.update(source_track_id=sources[performer]['track_id'],parent_track_ids=[sources[performer]['track_id']],
                                     performance_mode='continuation',continuation=copy.deepcopy(score.get('continuation',{})))
                    proposed=copy.deepcopy(comparison)
                    proposed['track_ids'].append(track_id)
                    proposed['variants'][track_id]={'display_name':f'{name} · {score["title"]}','description':description}
                    proposed['performer_status'][performer]='recorded'
                    if sources:
                        proposed.setdefault('continuation_status',{})[performer]=copy.deepcopy(score.get('continuation',{}))
                    comparison=self._persist_performance(proposed,track)
                except Exception as error:
                    comparison['performer_status'][performer]='failed'
                    comparison['recording_errors'].append(f'{name}: {error}')
                    comparison=self._persist_performance(comparison)
            comparison=self._persist_performance(comparison)
            atomic_json(self.root/'reports'/f'performance-{batch_id}.json',comparison)
            self.status('Independent recordings are ready.' if not comparison['recording_errors'] else 'Session ended; successful recordings are saved and failed authors are reported.')
            return comparison
        except Exception as error:
            with self.lock:self.error=str(error)
            raise
        finally:
            with self.lock:self.busy=False

    def start_performance(self, seed=2026, seconds=75, *, retry_id=None, source_id=None):
        with self.lock:
            seed,seconds,_,_=self._performance_request(seed,seconds,retry_id,source_id)
            if self.busy:raise ValueError('A session is already recording')
            self.busy=True;self.error=None
            self.progress='Retrying missing performance recordings' if retry_id else 'Starting independent Fly and Organist sessions'
        def worker():
            try:self.perform(seed,seconds,retry_id=retry_id,source_id=source_id,_reserved=True)
            except Exception as error:
                with self.lock:self.error=str(error);self.busy=False
        threading.Thread(target=worker,daemon=True,name='fly-organist-performance').start()

    def _study_request(self, source_id, feedback, seconds):
        if type(seconds) is not int or not 30<=seconds<=120:
            raise ValueError('Choose a recording limit of 30–120 seconds')
        if not isinstance(feedback,str) or not feedback.strip() or len(feedback)>2000:
            raise ValueError('Describe what Organist should change in 1–2000 characters')
        seed,sources=self._performance_sources(source_id,180)
        for value in sources.values():
            track=next(t for t in self.library['tracks'] if t['id']==value['track_id'])
            if value['score']['voices']!=track['voices']:
                raise ValueError('The source instruments do not match their library record')
        return seed,sources

    def study_organist(self, source_id, feedback, seconds=75, *, _reserved=False):
        """Let Organist plan, author and review a new take from listener criticism."""
        with self.lock:
            seed,sources=self._study_request(source_id,feedback,seconds)
            if self.busy and not _reserved:raise ValueError('A session is already recording')
            self.busy=True;self.error=None
        created=dt.datetime.now(dt.timezone.utc).isoformat()
        fly_id=sources['fly']['track_id'];original=sources['organist']['score']
        context={'feedback':feedback,'source_comparison_id':source_id,
                 'source_track_ids':{name:s['track_id'] for name,s in sources.items()},
                 'source_score_sha256':{name:s['score_sha256'] for name,s in sources.items()},
                 'scope':'A fresh Organist take prompted by listener criticism. Source recordings are preserved; notes may be replaced in the new take. Fly is retained as a reference. No listening verdict is inferred.'}
        comparison={'id':uuid.uuid4().hex[:16],'kind':'performance','performance_mode':'study',
                    'title':'Fly & Organist · feedback study','seed':seed,'ensemble':'independent',
                    'created_at':created,'revealed':True,'track_ids':[fly_id],
                    'variants':{fly_id:{'display_name':'Fly · existing reference','description':'The saved Fly recording, retained for comparison with Organist’s new study.'}},
                    'feedback':{},'recording_seconds':seconds,'performer_status':{'fly':'recorded','organist':'generating'},
                    'recording_errors':[],'study_context':context,'scope':context['scope']}
        def musical_notes(score):
            return sorted(tuple(event[k] for k in ('layer','voice','midi','start_seconds','duration_seconds','velocity')) for event in score['events'])
        try:
            comparison=self._persist_performance(comparison)
            try:
                from .organist_study import generate_study
                score=copy.deepcopy(generate_study(self.root,copy.deepcopy(original),feedback,seconds=seconds,progress=self.status))
                assess_performance(score)
                if score.get('performer')!='organist' or score.get('condition')!='organist' or score.get('kind')!='performance':
                    raise ValueError('Study author identity mismatch')
                if any(score.get(key)!=original.get(key) for key in ('seed','bpm','voices')):
                    raise ValueError('Study changed the source seed, tempo or instruments')
                if musical_notes(score)==musical_notes(original):
                    raise ValueError('Organist returned the same musical notes; no duplicate recording was published')
                if score['duration_seconds']>seconds:
                    raise ValueError('Study exceeds its requested recording limit')
                for value in sources.values():
                    raw=(self.root/'library'/value['track_id']/'score.json').read_bytes()
                    if hashlib.sha256(raw).hexdigest()!=value['score_sha256']:
                        raise ValueError('A source score changed during study; publication refused')
                score['study_context']=copy.deepcopy(context)
                audio,stems,audit=render_score(score,progress=self.status)
                peak=max([audit['raw_peak']]+[float(np.max(np.abs(stem))) for stem in stems.values()])
                target=min(.055,.82*audit['raw_rms']/peak)
                track_id=uuid.uuid4().hex[:16]
                audit=save_render(self.root/'library'/track_id,score,audio,stems,audit,target)
                description='A fresh composition from Organist’s own plan, followed by a review of the authored notes and one opportunity to revise. Listening decides whether it improves.'
                track={'id':track_id,'title':f"Organist · {score['title']}",'created_at':created,
                       'duration':audit['duration_seconds'],'kind':'performance','condition':'organist','performer':'organist',
                       'performance_mode':'study','description':description,'seed':seed,'ensemble':'independent','voices':score['voices'],
                       'arrangement_version':score.get('arrangement_version'),'source_track_id':sources['organist']['track_id'],
                       'parent_track_ids':[sources['organist']['track_id']],'study_context':copy.deepcopy(context),
                       'metrics':{k:audit[k] for k in ('notes_scheduled','notes_mixed','layers','sections','peak','rms','unique_lead_phrases','pcm_sha256')},
                       'verdict':None,'note':'','comparison_id':comparison['id']}
                proposed=copy.deepcopy(comparison)
                proposed['track_ids'].append(track_id)
                proposed['variants'][track_id]={'display_name':f"Organist · {score['title']}",'description':description}
                proposed['performer_status']['organist']='recorded'
                comparison=self._persist_performance(proposed,track)
            except Exception as error:
                comparison['performer_status']['organist']='failed'
                comparison['recording_errors'].append(f'Organist study: {error}')
                comparison=self._persist_performance(comparison)
            atomic_json(self.root/'reports'/f"organist-study-session-{comparison['id']}.json",comparison)
            self.status('Organist study is ready beside the saved Fly reference.' if not comparison['recording_errors'] else 'Organist study failed; the earlier recordings remain available.')
            return comparison
        except Exception as error:
            with self.lock:self.error=str(error)
            raise
        finally:
            with self.lock:self.busy=False

    def start_organist_study(self, source_id, feedback, seconds=75):
        with self.lock:
            self._study_request(source_id,feedback,seconds)
            if self.busy:raise ValueError('A session is already recording')
            self.busy=True;self.error=None;self.progress='Organist is studying the listening feedback'
        def worker():
            try:self.study_organist(source_id,feedback,seconds,_reserved=True)
            except Exception as error:
                with self.lock:self.error=str(error);self.busy=False
        threading.Thread(target=worker,daemon=True,name='organist-study').start()

    def feedback(self, data):
        with self.lock:
            candidate=copy.deepcopy(self.library)
            track=next((t for t in candidate['tracks'] if t['id']==data.get('track_id')),None)
            if track is None:raise ValueError('Unknown track')
            verdict=data.get('verdict')
            if verdict not in ('keep','reject','clear'):raise ValueError('Unknown listening verdict')
            track['verdict']=None if verdict=='clear' else verdict
            track['note']=str(data.get('note',''))[:2000]
            atomic_json(self.index,candidate)
            self.library=candidate

    def comparison_action(self, data, reveal=False):
        with self.lock:
            candidate=copy.deepcopy(self.library)
            comparison=next((c for c in candidate['comparisons'] if c['id']==data.get('comparison_id')),None)
            if comparison is None:raise ValueError('Unknown comparison')
            if reveal:comparison['revealed']=True
            else:
                preferred=data.get('preferred_track_id')
                if preferred and preferred not in comparison['track_ids']:raise ValueError('Preference must refer to this comparison')
                feedback={'preferred_track_id':preferred,'note':str(data.get('note',''))[:2000],
                          'identities_revealed_at_rating':comparison['revealed']}
                for key in ('coherence','motion','fatigue'):
                    value=data.get(key)
                    if value not in (None,'') and (type(value)is not int or not 1<=value<=5):raise ValueError('Ratings must be integers 1..5')
                    feedback[key]=value
                comparison['feedback']=feedback
            atomic_json(self.index,candidate)
            self.library=candidate
