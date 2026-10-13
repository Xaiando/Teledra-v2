"""Render one bounded local-practice candidate, without publishing to Studio."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

import numpy as np

from ..connectome import load_circuit
from ..render import save_render
from .autonomy_checkpoint import atomic_json
from .autonomy_critic import assess_audio, evaluate
from .brain import Brain
from .breadth_training import make_options
from .longform import Direction, arrange
from .performer import perform
from .showcase import render


def render_request(request_path):
    request=json.loads(Path(request_path).read_bytes())
    root,job,destination=(Path(request[k]).resolve() for k in ('root','job','destination'))
    brain=Brain.load(job/'brain.npz')
    context,seed=request['context'],request['seed']
    options=make_options(root,context,seed)
    circuit=load_circuit(root/'data/circuit.npz')
    raw=perform(circuit,brain,seed=seed,seconds=180,**options)
    raw_critic=evaluate(raw)
    atomic_json(job/'raw-score.json',raw)
    atomic_json(job/'raw-critic.json',raw_critic)
    if context in ('dusk','overworld'):
        # These are the approved arranging recipes, clearly separate from
        # brain fitness. A fresh piece supplies its own returning motif.
        directed=perform(circuit,brain,seed=seed,seconds=180,direction=Direction(context),**options)
        source=perform(circuit,brain,seed=seed,seconds=61,**options)
        atomic_json(job/'directed-score.json',directed)
        atomic_json(job/'motif-source-score.json',source)
        score=arrange(directed,source,context)
        score['longform']['reference_track_id']=score['longform'].pop('source_track_id')
        score['longform']['scope']='New-seed performance developed with a recipe derived from listener-approved pieces; the arranger is not part of brain fitness and this candidate is unreviewed.'
        score['title']=f'Practice · {context} · {seed}'
        kind='Arranged study'
    else:
        score=raw
        kind='Raw practice capture'
    critic=evaluate(score)
    atomic_json(job/'candidate-critic.json',critic)
    # The archive is a listening queue, not an approval label. Reject musical
    # failures before spending disk space on their complete multi-stem audio.
    if not critic['acceptable']:
        result={'passed':False,'stage':'score','rejection_reasons':critic['rejection_reasons'],
                'raw_critic':raw_critic,'candidate_critic':critic}
        atomic_json(job/'result.json',result)
        return result
    audio,stems,audit=render(score,progress=lambda msg:print(msg,flush=True))
    audio_critic=assess_audio(score,audit,stems,audio)
    if not audio_critic['passed']:
        result={'passed':False,'stage':'audio','rejection_reasons':audio_critic['rejection_reasons'],
                'raw_critic':raw_critic,'candidate_critic':critic,'audio_critic':audio_critic}
        atomic_json(job/'result.json',result)
        return result
    peak=max(float(np.max(np.abs(audio))),*(float(np.max(np.abs(stem))) for stem in stems.values()))
    target=min(.055,.9*audit['raw_rms']/max(peak,1e-12))
    save_render(destination,score,audio,stems,audit,target)
    with (destination/'audio.wav').open('rb') as stream:
        file_hash=hashlib.file_digest(stream,'sha256').hexdigest()
    generation=request['generation']
    title=f'{context.title()} · {request["lane"]} · '+(f'checkpoint generation {generation}' if generation else 'starting brain')
    candidate={'id':request['id'],'title':title,'context':context,'seed':seed,'duration':180,
               'kind':kind,'generation':generation,'lane':request['lane'],'bpm':score['bpm'],
               'metrics':{'raw_reward':raw_critic['reward'],'arranged_reward':critic['reward'] if kind=='Arranged study' else None,
                          'notes':len(score['events']),'peak':audit['peak'],'rms':audit['rms'],
                          'bass_coverage':critic['metrics'].get('bass_coverage'),'audio_verified':True},
               'wav_sha256':file_hash,'listener_verdict':None,
               'scope':'Local proxy and delivery checks passed; awaiting listener review. Not an automatically approved library track.'}
    result={'passed':True,'candidate':candidate,'raw_critic':raw_critic,'candidate_critic':critic,'audio_critic':audio_critic}
    atomic_json(destination/'critic.json',result)
    atomic_json(job/'result.json',result)
    return result
