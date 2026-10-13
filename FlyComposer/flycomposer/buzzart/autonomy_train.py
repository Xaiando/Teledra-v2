"""Bounded local practice with a frozen critic, two lineages and held-out checks.

Brain training evaluates unarranged notes. A separate listening stage may use
the approved dusk/overworld arranger; those improvements never enter fitness.
No library ratings, published brains, or existing recordings are overwritten.
"""
from __future__ import annotations

from concurrent.futures import ProcessPoolExecutor, TimeoutError as FutureTimeout
import datetime as dt
import hashlib
import json
import math
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

import numpy as np

from ..connectome import load_circuit
from .autonomy_checkpoint import atomic_json, load_checkpoint, run_lock, save_checkpoint
from .brain import Brain
from .breadth_training import make_options, ranks_with_ties
from .critic import fitness
from .performer import perform
from .trainer import parameter_scales

CONTEXTS=('indie','arcade','dusk','dreamy','overworld','complex','genome')
PARENTS={'wide':'brains/buzzart-wide-candidate.npz','heritage':'brains/buzzart-indie-long.npz'}
SEEDS={'train':(1600000000,1650000000),'validation':1660000000,
       'challenge':1670000000,'final':1680000000,'listening':1690000000}
_WORKER={}


class RunInterrupted(Exception):
    """An expected stop or budget boundary, not a failed musical candidate."""
    def __init__(self, reason):
        self.reason = reason
        super().__init__(reason)


class BoundedPool:
    """Bound each evaluation batch and stop only workers owned by this pool."""
    def __init__(self, root, run_dir, workers, deadline):
        self.run_dir, self.deadline = Path(run_dir), deadline
        self.pool = ProcessPoolExecutor(max_workers=workers, initializer=worker_init,
                                        initargs=(str(root),))

    def __enter__(self):
        return self

    def check(self):
        if (self.run_dir / 'STOP').exists():
            raise RunInterrupted('Stopped by user; checkpoints retained.')
        if time.time() >= self.deadline:
            raise RunInterrupted('Evaluation time budget reached; checkpoints retained.')

    def map(self, function, tasks, chunksize=1):
        self.check()
        futures = [self.pool.submit(function, task) for task in tasks]
        batch_deadline = min(self.deadline, time.time() + 120.)
        results = []
        for future in futures:
            while True:
                self.check()
                remaining = batch_deadline - time.time()
                if remaining <= 0:
                    raise RunInterrupted('Evaluation batch exceeded 120 seconds; checkpoints retained.')
                try:
                    results.append(future.result(timeout=min(1., remaining)))
                    break
                except FutureTimeout:
                    continue
        return iter(results)

    def __exit__(self, kind, error, traceback):
        if kind is None:
            self.pool.shutdown(wait=True)
            return False
        # These are our own Process objects, never a global process/PID search.
        processes = list((getattr(self.pool, '_processes', None) or {}).values())
        for process in processes:
            if process.is_alive():
                process.terminate()
        self.pool.shutdown(wait=False, cancel_futures=True)
        for process in processes:
            process.join(timeout=1.)
            if process.is_alive():
                process.kill()
                process.join(timeout=1.)
        return False


def utc(): return dt.datetime.now(dt.timezone.utc).isoformat()


def sha(path):
    with Path(path).open('rb') as stream: return hashlib.file_digest(stream,'sha256').hexdigest()


def worker_init(root):
    _WORKER.update(root=Path(root),circuit=load_circuit(Path(root)/'data/circuit.npz'),
                   template=Brain.load(Path(root)/PARENTS['wide']))


def evaluate_task(task):
    from .autonomy_critic import evaluate
    vector,groups,details=task
    brain=_WORKER['template'].with_vector(vector,metadata={})
    rows={}
    for context,seeds,seconds in groups:
        scores=[]; local=[]; natural=[]
        for seed in seeds:
            options=make_options(_WORKER['root'],context,int(seed))
            score=perform(_WORKER['circuit'],brain,seed=int(seed),seconds=seconds,**options)
            scores.append(score); local.append(evaluate(score))
            natural.append(options['foundation'].fit(score)['fit'])
        legacy=fitness(scores)
        critic=float(np.mean([r['reward'] for r in local]))
        corpus=float(np.mean(natural))
        variety=float(legacy['diversity'])
        objective=(.65*critic+.20*legacy['mean_reward']+.15*corpus)*(.70+.30*variety)
        metrics={k:float(np.mean([r['metrics'][k] for r in local])) for k in local[0]['metrics']
                 if all(type(r['metrics'][k]) in (int,float) for r in local)}
        row={'objective':objective,'critic':critic,'legacy':legacy['mean_reward'],'naturalness':corpus,
             'diversity':variety,'metrics':metrics,
             'acceptable_fraction':float(np.mean([r['acceptable'] for r in local]))}
        if details:
            row['cases']=[{'seed':s['seed'],'seconds':seconds,'event_sha256':s['event_sha256'],
                           'bpm':s['bpm'],'tonic':s['tonic'],'mode':s['mode'],'voices':s['voices'],
                           'reward':c['reward'],'rejection_reasons':c['rejection_reasons']}
                          for s,c in zip(scores,local)]
        rows[context]=row
    values=[r['objective'] for r in rows.values()]
    return {'objective':.8*float(np.mean(values))+.2*min(values),
            'critic':float(np.mean([r['critic'] for r in rows.values()])),
            'contexts':rows}


def combine(results):
    rows={k:v for result in results for k,v in result['contexts'].items()}
    values=[r['objective'] for r in rows.values()]
    return {'objective':.8*float(np.mean(values))+.2*min(values),
            'critic':float(np.mean([r['critic'] for r in rows.values()])), 'contexts':rows}


def validation_groups(kind='validation',offset=0,count=16,seconds=91):
    if kind not in ('validation','challenge','final') or type(offset) is not int or type(count) is not int:
        raise ValueError('Invalid evaluation seed group')
    if not 0 <= offset <= 200000 or not 1 <= count <= 1000 or offset + count >= 1000000:
        raise ValueError('Evaluation seed group exceeds its reserved context range')
    contexts=(*CONTEXTS,'acoustic') if kind=='final' else CONTEXTS
    return [(context,[SEEDS[kind]+offset+i*1000000+j for j in range(count)],seconds)
            for i,context in enumerate(contexts)]


def evaluate_groups(pool,vector,groups,details=True):
    return combine(list(pool.map(evaluate_task,[(vector,[g],details) for g in groups],chunksize=1)))


def eligible(candidate,baseline):
    """Never accept broad improvement by sacrificing an original capability."""
    reasons=[]
    if set(candidate.get('contexts', {})) != set(baseline.get('contexts', {})):
        return ['evaluation context inventory changed']
    for summary in (candidate, baseline):
        if not all(math.isfinite(summary[key]) for key in ('objective', 'critic')):
            return ['nonfinite evaluation summary']
    for name,row in candidate['contexts'].items():
        base=baseline['contexts'][name]
        for key,tolerance in (('critic',.025),('legacy',.045),('naturalness',.05),('diversity',.06)):
            if not math.isfinite(row[key]):
                reasons.append(f'{name}: nonfinite {key}')
                continue
            if row[key]<base[key]-tolerance: reasons.append(f'{name}: {key} regression')
        for key,tolerance in (('bass_credited_coverage',.035),('two_pitched_roles_share',.06)):
            if not math.isfinite(row['metrics'].get(key,0)):
                reasons.append(f'{name}: nonfinite {key}')
                continue
            if row['metrics'].get(key,0)<base['metrics'].get(key,0)-tolerance:
                reasons.append(f'{name}: {key} regression')
    if candidate['critic'] < baseline['critic']-.002: reasons.append('mean local critic regression')
    return reasons


def frozen_inputs(root,run_dir):
    paths=set((root/'flycomposer').rglob('*.py'))
    paths.update((root/'data/foundation').glob('*.json'))
    paths.update(root/p for p in ('data/circuit.npz','data/circuit.json','data/instruments.json','bin/PyMusicSynth.exe','train_autonomous.py'))
    for path in PARENTS.values(): paths.update((root/path,(root/path).with_suffix('.json')))
    for context in ('dusk','overworld'):
        paths.update(root/'reports/longform-development'/f'final-{context}'/name for name in ('score.json','audit.json','audio.wav'))
    hashes={}
    for path in sorted(paths):
        relative=path.relative_to(root)
        target=run_dir/'inputs'/relative
        target.parent.mkdir(parents=True,exist_ok=True)
        shutil.copy2(path,target)
        hashes[str(relative)]=sha(path)
    return hashes


def verify_inputs(root,state):
    changed=[p for p,digest in state['input_hashes'].items() if not(root/p).is_file() or sha(root/p)!=digest]
    if changed: raise RuntimeError('Frozen inputs changed; checkpoint retained. '+', '.join(changed[:8]))


def directory_bytes(path): return sum(p.stat().st_size for p in Path(path).rglob('*') if p.is_file())


def configuration(hours=8.,workers=4,population=24,max_generations=20000,
                  validate_every=32,preview_minutes=30.,max_candidates=16,disk_gb=4.):
    if not math.isfinite(hours) or not .001<=hours<=24: raise ValueError('Hours must be between .001 and 24')
    if type(workers)!=int or not 1<=workers<=6: raise ValueError('Use one to six workers')
    if type(population)!=int or population<4 or population>64 or population%2: raise ValueError('Use even population 4..64')
    if (type(max_generations) is not int or not 1<=max_generations<=20000
        or type(validate_every) is not int or validate_every<1
        or not math.isfinite(preview_minutes) or preview_minutes<=0
        or type(max_candidates) is not int or not 0<=max_candidates<=32
        or not math.isfinite(disk_gb) or not .25<=disk_gb<=8):
        raise ValueError('Invalid run bounds')
    return dict(hours=hours,workers=workers,population=population,max_generations=max_generations,
                validate_every=validate_every,preview_seconds=preview_minutes*60,max_candidates=max_candidates,
                disk_limit_bytes=int(disk_gb*1024**3),sigma=.07,learning_rate=.009,anchor_decay=.003,
                rng_seed=2026091713,checkpoint_every=4,validation_count=16)


def initialize(root,run_dir,config):
    run_dir.mkdir(parents=True,exist_ok=True)
    if (run_dir/'protocol.json').exists(): raise FileExistsError('Existing run requires --resume')
    hashes=frozen_inputs(root,run_dir)
    state={'version':'buzzart_local_practice_v1','config':config,'started_at':utc(),'updated_at':utc(),
           'deadline_at':(dt.datetime.now(dt.timezone.utc)+dt.timedelta(hours=config['hours'])).isoformat(),
           'elapsed_seconds':0.,'generation':0,'lanes':{},'candidates':[],'preview_attempts':0,
           'next_preview_at':0.,'input_hashes':hashes,'status':'initializing','phase':'baseline',
           'accepted':0,'rejected':0,'rng_state':np.random.default_rng(config['rng_seed']).bit_generator.state,
           'approval':{'message':'This is awesome','scope':'Two explicitly arranged final recordings approved by listener; not automatic approval of their raw brains or future candidates.'}}
    arrays={}
    for name,path in PARENTS.items():
        parent=Brain.load(root/path)
        vector=parent.vector()
        for key in ('theta','anchor','best'): arrays[f'{name}_{key}']=vector.copy()
        for key in ('m','v'): arrays[f'{name}_{key}']=np.zeros_like(vector)
        state['lanes'][name]={'generation':0,'best_generation':0,'optimizer_step':0,'parent':parent.identity(),
                              'baseline':None,'best_validation':None,'promotions':0,'rejected':0}
    atomic_json(run_dir/'protocol.json',{'config':config,'parents':PARENTS,'seeds':SEEDS,'contexts':CONTEXTS,
                'training':'Unarranged brain outputs; 61s, 91s, 121s and 180s captures. Fixed and drawn grammars.',
                'selection':'Fixed 16 validation seeds per context, immutable own-parent regression guardrails, fresh paired challenges before promotion.',
                'objective':'65% local critic +20% legacy critic +15% corpus fit, with within-context diversity; 80% mean +20% weakest context.',
                'final_test':'Fresh untouched final seeds once on ordinary completion; never used for promotion.',
                'listening':'New-seed candidates; explicitly arranged dusk/overworld separated from unarranged training. No automatic library approval.',
                'approval':state['approval'],'input_hashes':hashes})
    return state,arrays


def status(run_dir,state,**changes):
    state.update(changes,updated_at=utc())
    # The dashboard does not need optimizer arrays or the large frozen-input list.
    public={k:v for k,v in state.items() if k not in ('rng_state','input_hashes','config')}
    public.update(config=state['config'],best={k:{'generation':v['best_generation'],
                       'objective':(v.get('best_validation') or {}).get('objective'),
                       'baseline_objective':(v.get('baseline') or {}).get('objective')} for k,v in state['lanes'].items()})
    public.pop('lanes',None)
    atomic_json(run_dir/'status.json',public)


def record(run_dir,row):
    with (run_dir/'history.jsonl').open('a',encoding='utf-8') as stream:
        stream.write(json.dumps({'at':utc(),**row},allow_nan=False)+'\n')


def preview(root,run_dir,state,arrays):
    """Rendering is a bounded subprocess; it cannot hang the optimizer."""
    index=state['preview_attempts']; state['preview_attempts']+=1
    # Allocate the attempt durably before creating its job. A crash in rendering
    # must not make resume reuse an existing job directory and fail the trainer.
    save_checkpoint(run_dir,state,arrays)
    context=('dusk','overworld','indie','dreamy','genome','arcade','dusk','overworld','complex','genome')[index%10]
    lane='heritage' if index%2 else 'wide'
    seed=SEEDS['listening']+index
    name=f'{index+1:03d}-{context}-{lane}-g{state["lanes"][lane]["best_generation"]}'
    job=run_dir/'jobs'/name; job.mkdir(parents=True,exist_ok=False)
    template=Brain.load(root/PARENTS[lane])
    generation=state['lanes'][lane]['best_generation']
    template.with_vector(arrays[f'{lane}_best'],metadata={'generation':generation,'run':run_dir.name,'lineage':lane}).save(job/'brain.npz')
    request={'root':str(root),'job':str(job),'destination':str(run_dir/'listening'/name),
             'context':context,'seed':seed,'lane':lane,'generation':generation,'id':name}
    atomic_json(job/'request.json',request)
    status(run_dir,state,phase='rendering',message=f'Checking {context} listening candidate; no autoplay.')
    try:
        with (job/'render.log').open('w',encoding='utf-8') as output:
            result=subprocess.run([sys.executable,str(root/'train_autonomous.py'),'--render',str(job/'request.json')],
                                  cwd=root,stdout=output,stderr=subprocess.STDOUT,timeout=180,
                                  creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0))
        if result.returncode: raise RuntimeError(f'Render exited {result.returncode}; see {job.name}/render.log')
        summary=json.loads((job/'result.json').read_bytes())
        if summary['passed']: state['candidates'].append(summary['candidate'])
        record(run_dir,{'type':'listening','candidate':name,**summary})
    except (subprocess.TimeoutExpired,RuntimeError,OSError,ValueError) as error:
        record(run_dir,{'type':'listening_failure','candidate':name,'error':str(error)})
    state['next_preview_at']=state['elapsed_seconds']+state['config']['preview_seconds']


def run(root,run_dir,config=None,resume=False):
    root,run_dir=Path(root).resolve(),Path(run_dir).resolve()
    run_dir.mkdir(parents=True,exist_ok=True)
    with run_lock(run_dir):
        if resume:
            state,arrays=load_checkpoint(run_dir)
            if state['status'] in ('completed','completed_with_incomplete_test'):
                raise ValueError('Completed runs are immutable; start another run')
            if config is not None and config != state['config']:
                raise ValueError('Resume configuration differs from the frozen run configuration')
            verify_inputs(root,state)
        else: state,arrays=initialize(root,run_dir,config or configuration())
        cfg=state['config']; rng=np.random.default_rng(); rng.bit_generator.state=state['rng_state']
        template=Brain.load(root/PARENTS['wide']); scales=parameter_scales(template)
        deadline=dt.datetime.fromisoformat(state['deadline_at']).timestamp()
        training_deadline=deadline-min(120.,cfg['hours']*3600*.10)
        initial_elapsed=state['elapsed_seconds']; clock_start=time.monotonic()
        def elapsed(): state['elapsed_seconds']=initial_elapsed+time.monotonic()-clock_start
        def checkpoint():
            elapsed(); state['rng_state']=rng.bit_generator.state; save_checkpoint(run_dir,state,arrays)
        checkpoint()
        status(run_dir,state,pid=os.getpid(),status='running')
        try:
            if (run_dir/'STOP').exists():
                raise RunInterrupted('Stopped by user; checkpoints retained.')
            if time.time() >= deadline:
                raise RunInterrupted('Run deadline already elapsed; final test was not started.')
            with BoundedPool(root,run_dir,cfg['workers'],deadline) as pool:
                while (run_dir/'PAUSE').exists():
                    pool.check()
                    status(run_dir,state,status='paused',phase='paused',message='Paused before baseline; deadline still applies.')
                    checkpoint()
                    time.sleep(min(2., max(.01, deadline-time.time())))
                groups=validation_groups(count=cfg['validation_count'])
                for lane,info in state['lanes'].items():
                    if info['baseline'] is None:
                        status(run_dir,state,phase='baseline',lane=lane,message=f'Measuring immutable {lane} baseline across seven contexts.')
                        info['baseline']=evaluate_groups(pool,arrays[f'{lane}_anchor'],groups)
                        info['best_validation']=info['baseline']
                        atomic_json(run_dir/f'{lane}-baseline.json',info['baseline']); checkpoint()
                while state['generation']<cfg['max_generations'] and time.time()<training_deadline:
                    elapsed()
                    if (run_dir/'STOP').exists():
                        status(run_dir,state,status='stopped',phase='checkpoint',message='Stopped by user; checkpoints retained.')
                        break
                    if directory_bytes(run_dir)>cfg['disk_limit_bytes'] or shutil.disk_usage(run_dir).free<2*1024**3:
                        status(run_dir,state,status='stopped',phase='disk_limit',message='Storage bound reached; checkpoints retained.')
                        break
                    if (run_dir/'PAUSE').exists():
                        status(run_dir,state,status='paused',phase='paused',message='Paused; the eight-hour deadline still applies.')
                        checkpoint(); time.sleep(2); continue
                    lane=('wide','heritage')[state['generation']%2]; info=state['lanes'][lane]
                    theta=arrays[f'{lane}_theta']; g=info['generation']+1
                    # Longer captures expose post-minute cycles and dropouts.
                    seconds=(61,91,121,180)[(g//8)%4]
                    contexts=list(dict.fromkeys((CONTEXTS[(g-1)%6],CONTEXTS[g%6],'genome')))
                    seeds=[int(x) for x in rng.integers(*SEEDS['train'],size=2)]
                    train_groups=[(c,seeds,seconds) for c in contexts]
                    sigma=cfg['sigma']*(1 if (g//256)%2==0 else .75)
                    noise=rng.standard_normal((cfg['population']//2,theta.size))
                    deltas=sigma*scales*noise
                    candidates=[theta+d for d in deltas]+[theta-d for d in deltas]
                    status(run_dir,state,status='running',phase='training',lane=lane,
                           message=f'{lane}: generation {g}, {seconds}s raw performances across {", ".join(contexts)}.')
                    tick=time.monotonic()
                    results=list(pool.map(evaluate_task,[(v,train_groups,False) for v in candidates],chunksize=1))
                    values=np.array([r['objective'] for r in results]); ranks=ranks_with_ties(values)
                    gradient=((ranks[:len(noise)]-ranks[len(noise):])[:,None]*noise).sum(axis=0)/(cfg['population']*sigma)
                    info['optimizer_step']+=1; step=info['optimizer_step']
                    m=arrays[f'{lane}_m']; v=arrays[f'{lane}_v']
                    m*=.9; m+=.1*gradient; v*=.999; v+=.001*gradient**2
                    update=cfg['learning_rate']*(m/(1-.9**step))/(np.sqrt(v/(1-.999**step))+1e-8)
                    theta+=scales*update-cfg['anchor_decay']*(theta-arrays[f'{lane}_anchor'])
                    info['generation']=g; state['generation']+=1
                    row={'type':'generation','generation':state['generation'],'lane':lane,'lane_generation':g,
                         'seconds':seconds,'contexts':contexts,'seeds':seeds,'objective_mean':float(values.mean()),
                         'objective_max':float(values.max()),'compute_seconds':time.monotonic()-tick}
                    if g%cfg['validate_every']==0:
                        status(run_dir,state,phase='validation',message=f'Checking {lane} against its frozen parent and all styles.')
                        candidate=evaluate_groups(pool,theta,groups)
                        reasons=eligible(candidate,info['baseline'])
                        if candidate['objective']<=info['best_validation']['objective']+.0002: reasons.append('no new validation improvement')
                        if not reasons:
                            fresh=validation_groups('challenge',offset=state['generation']*8,count=4,seconds=180)
                            parent_fresh=evaluate_groups(pool,arrays[f'{lane}_anchor'],fresh)
                            child_fresh=evaluate_groups(pool,theta,fresh)
                            reasons+=eligible(child_fresh,parent_fresh)
                            if child_fresh['objective']<parent_fresh['objective']+.0001: reasons.append('no fresh longform challenge gain')
                            row['fresh_challenge']={'parent':parent_fresh,'candidate':child_fresh}
                        row.update(validation=candidate,rejection_reasons=reasons)
                        if not reasons:
                            arrays[f'{lane}_best']=theta.copy(); info['best_validation']=candidate
                            info['best_generation']=g; info['promotions']+=1; state['accepted']+=1
                            archive=run_dir/'accepted'/f'{lane}-{g:06d}.npz'
                            template.with_vector(theta,metadata={'generation':g,'run':run_dir.name,'lineage':lane}).save(archive)
                        else: info['rejected']+=1; state['rejected']+=1
                        verify_inputs(root,state)
                    record(run_dir,row)
                    # Bounded exploration around the best accepted checkpoint,
                    # avoiding eight hours of drift from a rejected optimizer.
                    if g%128==0:
                        arrays[f'{lane}_theta']=arrays[f'{lane}_best'].copy()
                        arrays[f'{lane}_m'].fill(0); arrays[f'{lane}_v'].fill(0); info['optimizer_step']=0
                    elapsed()
                    if state['generation']>=2 and state['elapsed_seconds']>=state['next_preview_at'] and state['preview_attempts']<cfg['max_candidates'] and time.time()<deadline-190:
                        checkpoint(); preview(root,run_dir,state,arrays)
                    if state['generation']%cfg['checkpoint_every']==0: checkpoint()
                    status(run_dir,state,phase='training')
                if state['status'] not in ('stopped',):
                    if (run_dir/'STOP').exists():
                        raise RunInterrupted('Stopped by user; final test was not started.')
                    if (run_dir/'PAUSE').exists():
                        raise RunInterrupted('Run paused at its training deadline; final test was not started.')
                    pool.deadline=deadline
                    pool.check()
                    verify_inputs(root,state)
                    status(run_dir,state,phase='final_test',message='One final untouched-seed evaluation; never used for selection.')
                    final={}
                    for lane,info in state['lanes'].items():
                        final_groups=validation_groups('final',count=8,seconds=180)
                        before=evaluate_groups(pool,arrays[f'{lane}_anchor'],final_groups)
                        after=evaluate_groups(pool,arrays[f'{lane}_best'],final_groups)
                        final[lane]={'parent':before,'candidate':after,'guardrail_regressions':eligible(after,before)}
                    atomic_json(run_dir/'final-test.json',final)
                    status(run_dir,state,status='completed',phase='complete',message='Bounded practice complete; candidates await your listening.')
                checkpoint()
        except RunInterrupted as error:
            expired=time.time()>=deadline
            terminal='completed_with_incomplete_test' if expired else 'stopped'
            status(run_dir,state,status=terminal,phase='incomplete',message=error.reason,
                   final_test_complete=False)
            record(run_dir,{'type':'interrupted','reason':error.reason})
            checkpoint()
        except BaseException as error:
            status(run_dir,state,status='failed',phase='error',message=f'{type(error).__name__}: {error}')
            record(run_dir,{'type':'failure','error':state['message']})
            checkpoint()
            raise
        finally:
            elapsed(); status(run_dir,state)
        return state
