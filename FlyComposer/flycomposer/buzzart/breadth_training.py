"""Listener-guided fine tuning across fixed and seed-drawn musical grammars.

This is a new run, not a change to the published brain or legacy trainer. All
selection uses validation; the test suite is evaluated only after selection.
The listener objective is an engineered proxy, not an aesthetic verdict.
"""
from __future__ import annotations

from concurrent.futures import ProcessPoolExecutor
import datetime as dt
import hashlib
import json
from pathlib import Path
import shutil
import time

import numpy as np

from ..connectome import load_circuit
from .brain import Brain
from .console_style import ConsoleStyle
from .critic import fitness
from .foundation import Foundation
from .genome import SONGBOOKS, draw
from .performer import perform
from .trainer import parameter_scales

FIXED = ('indie', 'arcade', 'dusk', 'dreamy', 'overworld', 'complex')
VALID_CONTEXTS = (*FIXED, 'genome')
TRAIN_SEEDS = tuple(range(317000000, 317002000))
VALID_SEEDS = tuple(range(317010000, 317010008))
TEST_SEEDS = tuple(range(317020000, 317020008))
PALETTES = {'indie': 'genesis', 'arcade': 'nes', 'dusk': 'dusk',
            'dreamy': 'dreamy', 'overworld': 'overworld', 'complex': 'complex'}
BOOKS = {'indie': 'indie-blend', 'arcade': 'console', 'dusk': 'blended',
         'dreamy': 'songbook', 'overworld': 'amiga', 'complex': 'indie-blend'}
LISTEN_CASES = [dict(context=c, seed=317020000+i) for i, c in
                enumerate(('indie', 'arcade', 'dusk', 'dreamy', 'overworld', 'genome'))]
_CACHE = {}
_WORKER = {}


def _json(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    pending = path.with_suffix(path.suffix + '.next')
    pending.write_text(json.dumps(value, indent=2, ensure_ascii=False, allow_nan=False) + '\n', encoding='utf-8')
    pending.replace(path)


def _sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def _foundation(root, name):
    path = (Path(root) / name).resolve()
    key = str(path)
    if key not in _CACHE:
        _CACHE[key] = Foundation.load(path)
    return _CACHE[key]


def make_options(root, context, seed):
    """Same seed-drawn grammar and corpus for every candidate, with no note edits."""
    root = Path(root)
    if context == 'genome':
        traits, palette = draw(seed)
        return {'foundation': _foundation(root, SONGBOOKS[traits['songbook']]),
                'style': ConsoleStyle.load(root / 'data/foundation/console.json',
                                           temperament=traits, palette=palette)}
    if context == 'acoustic':  # Entire unstyled context reserved for final transfer test.
        return {'foundation': _foundation(root, 'data/foundation/songbook.json'), 'style': None}
    if context not in FIXED:
        raise ValueError(f'Unknown training context {context!r}')
    return {'foundation': _foundation(root, f'data/foundation/{BOOKS[context]}.json'),
            'style': ConsoleStyle.load(root / 'data/foundation/console.json',
                                       temperament=context, palette=PALETTES[context])}


def ranks_with_ties(values):
    """Average ranks prevent movement when all candidates have equal reward."""
    values = np.asarray(values, dtype=float)
    ranks = np.zeros(len(values))
    if len(values) < 2:
        return ranks
    order = np.argsort(values, kind='stable')
    start = 0
    while start < len(order):
        end = start + 1
        while end < len(order) and values[order[end]] == values[order[start]]:
            end += 1
        ranks[order[start:end]] = (start + end - 1) / 2
        start = end
    return ranks / (len(values) - 1) - .5


def _init(root, template):
    _WORKER.update(root=Path(root), template=template,
                   circuit=load_circuit(Path(root) / 'data/circuit.npz'))


def _group(brain, context, seeds, seconds, details=False):
    from .listener_objective import evaluate as listener_evaluate
    scores, listener, naturalness = [], [], []
    for seed in seeds:
        options = make_options(_WORKER['root'], context, seed)
        score = perform(_WORKER['circuit'], brain, seed, seconds, **options)
        scores.append(score)
        listener.append(listener_evaluate(score, target_seconds=seconds-1))
        naturalness.append(options['foundation'].fit(score)['fit'])
    old = fitness(scores)
    flow = float(np.mean([item['reward'] for item in listener]))
    natural = float(np.mean(naturalness))
    # Diversity is measured inside a context, never between a quiet song and
    # a fast song merely because the chosen styles differ.
    value = (.50 * old['mean_reward'] + .35 * flow + .15 * natural) * (.85 + .15 * old['diversity'])
    result = {'context': context, 'objective': value, 'listener': flow,
              'critic': old['mean_reward'], 'naturalness': natural,
              'diversity': old['diversity'], 'critic_components': old['components'],
              'listener_components': {k: float(np.mean([r['components'][k] for r in listener]))
                                      for k in listener[0]['components']}}
    if details:
        result['cases'] = [{'seed': s['seed'], 'event_sha256': s['event_sha256'],
                            'bpm': s['bpm'], 'tonic': s['tonic'], 'mode': s['mode'],
                            'voices': s['voices'], 'events': len(s['events']),
                            'style': s.get('buzzart', {}).get('style'),
                            'listener': l, 'critic': c, 'naturalness': n}
                           for s, l, c, n in zip(scores, listener, old['results'], naturalness)]
        result['width'] = {'tempos': sorted({s['bpm'] for s in scores}),
                           'keys': sorted({s['tonic']+' '+s['mode'] for s in scores}),
                           'instruments': sorted({v for s in scores for v in s['voices'].values()}),
                           'distinct_scores': len({s['event_sha256'] for s in scores}),
                           'diversity_facts': old['diversity_facts']}
    return result


def _evaluate(task):
    vector, groups, seconds, details = task
    brain = _WORKER['template'].with_vector(vector, metadata={})
    # An implementation error aborts training; never learn from silent zeros.
    results = [_group(brain, context, seeds, seconds, details) for context, seeds in groups]
    values = [r['objective'] for r in results]
    return {'objective': .8*float(np.mean(values)) + .2*min(values),
            'listener': float(np.mean([r['listener'] for r in results])),
            'critic': float(np.mean([r['critic'] for r in results])),
            'naturalness': float(np.mean([r['naturalness'] for r in results])),
            'diversity': float(np.mean([r['diversity'] for r in results])),
            'contexts': {r['context']: r for r in results}}


def eligible(summary, baseline):
    """Selection guardrails fixed before training; these are proxy tolerances."""
    reasons = []
    for name, row in summary['contexts'].items():
        before = baseline['contexts'][name]
        for metric, tolerance in (('critic', .035), ('naturalness', .04), ('diversity', .08), ('listener', .05)):
            if row[metric] < before[metric] - tolerance:
                reasons.append(f'{name}: {metric} regression exceeds {tolerance}')
    if summary['listener'] < baseline['listener']:
        reasons.append('mean listener proxy below parent')
    return reasons


def snapshot(root, run_dir, parent_path):
    paths = set((root/'flycomposer/buzzart').glob('*.py'))
    paths.update((root/'data/foundation').glob('*.json'))
    paths.update(root/p for p in ('train_wide.py', 'data/circuit.npz', 'data/circuit.json', 'flycomposer/connectome.py',
                 'flycomposer/composition.py', 'flycomposer/autonomous.py', 'flycomposer/render.py',
                 'flycomposer/vendor/teledra_synth.py', 'flycomposer/benchmark_metrics.py'))
    paths.update((parent_path, parent_path.with_suffix('.json')))
    paths.update(root/'reports/composer-benchmark-20260917-aace4266'/p
                 for p in ('listener-feedback.json', 'listener-review.md'))
    hashes = {}
    for path in sorted(paths):
        relative = path.relative_to(root)
        target = run_dir/'inputs'/relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, target)
        hashes[str(relative)] = _sha(path)
    # The native renderer is recorded but not duplicated into every training run.
    hashes['bin/PyMusicSynth.exe'] = _sha(root/'bin/PyMusicSynth.exe')
    return hashes


def run(root, run_dir, *, generations=72, population=32, workers=6, seconds=61,
        validate_every=8, sigma=.07, learning_rate=.012, anchor_decay=.003, log=print):
    root, run_dir = Path(root).resolve(), Path(run_dir).resolve()
    if generations < 1 or population < 4 or population % 2 or workers < 1 or validate_every < 1:
        raise ValueError('Need positive generations/workers/validation interval and even population >=4')
    if run_dir.exists():
        raise ValueError('Use a new run directory; existing evidence is never resumed or overwritten')
    run_dir.mkdir(parents=True)
    (run_dir/'brains').mkdir()
    parent_path = root/'brains/buzzart-indie-long.npz'
    parent = Brain.load(parent_path)
    parent_identity = parent.identity()
    hashes = snapshot(root, run_dir, parent_path)
    config = dict(generations=generations, population=population, workers=workers, seconds=seconds,
                  validate_every=validate_every, sigma=sigma, learning_rate=learning_rate, anchor_decay=anchor_decay,
                  rng_seed=2026091702, parent=parent_identity, source_hashes=hashes,
                  train_seed_range=[TRAIN_SEEDS[0], TRAIN_SEEDS[-1]], validation_seeds=VALID_SEEDS,
                  test_seeds=TEST_SEEDS, validation_contexts=VALID_CONTEXTS, transfer_context='acoustic',
                  listening_cases=LISTEN_CASES,
                  objective='(.50 critic + .35 listener proxy + .15 corpus fit) * (.85+.15 within-context diversity); .8 mean + .2 weakest context',
                  selection='highest eligible validation objective above parent; otherwise retain parent and report candidate separately',
                  width_limit='Drawn grammar diversity is partly supplied by the existing grammar system; it is not learned stylistic invention. Four fixed validation and eight test seeds per context constitute a pilot.',
                  started_at=dt.datetime.now(dt.timezone.utc).isoformat())
    _json(run_dir/'protocol.json', config)
    # Clear inherited old validation scores and generation counts from child metadata.
    template = parent.with_vector(parent.vector(), metadata={'parent': parent_identity, 'run': run_dir.name})
    theta, anchor = template.vector(), template.vector()
    scales = parameter_scales(template)
    adam_m, adam_v = np.zeros_like(theta), np.zeros_like(theta)
    rng = np.random.default_rng(config['rng_seed'])
    valid_groups = [(c, list(VALID_SEEDS[:4])) for c in FIXED] + [('genome', list(VALID_SEEDS))]
    test_groups = [(c, list(TEST_SEEDS)) for c in (*VALID_CONTEXTS, 'acoustic')]
    history = run_dir/'history.jsonl'
    def record(row):
        with history.open('a', encoding='utf-8') as stream:
            stream.write(json.dumps(row, allow_nan=False)+'\n')
    def saved(vector, generation, path, **metadata):
        brain = template.with_vector(vector, metadata={**template.metadata, 'generation': generation,
                                                       'fine_tune_generations': generation, **metadata})
        brain.save(path)
        return brain
    started = time.monotonic()
    with ProcessPoolExecutor(max_workers=workers, initializer=_init, initargs=(str(root), template)) as pool:
        baseline = pool.submit(_evaluate, (anchor, valid_groups, seconds, True)).result()
        _json(run_dir/'validation-parent.json', baseline)
        saved(anchor, 0, run_dir/'parent.npz')
        best_value, best_gen, best_vector = baseline['objective'], 0, anchor.copy()
        candidate_value, candidate_gen, candidate_vector = -1., 0, anchor.copy()
        log(f"Parent validation: objective {best_value:.4f}, listener {baseline['listener']:.4f}, critic {baseline['critic']:.4f}", flush=True)
        record({'generation': 0, 'validation': baseline})
        for generation in range(1, generations+1):
            tick = time.monotonic()
            seeds = [int(s) for s in rng.choice(TRAIN_SEEDS, 4, replace=False)]
            offset = ((generation-1)*2) % len(FIXED)
            contexts = [FIXED[offset], FIXED[(offset+1) % len(FIXED)]]
            groups = [(c, seeds[:2]) for c in contexts] + [('genome', seeds)]
            noise = rng.standard_normal((population//2, theta.size))
            deltas = sigma*scales*noise
            candidates = [theta+d for d in deltas]+[theta-d for d in deltas]
            results = list(pool.map(_evaluate, [(v, groups, seconds, False) for v in candidates], chunksize=1))
            values = np.array([r['objective'] for r in results])
            ranks = ranks_with_ties(values)
            gradient = ((ranks[:population//2]-ranks[population//2:])[:,None]*noise).sum(axis=0)/(population*sigma)
            adam_m = .9*adam_m+.1*gradient
            adam_v = .999*adam_v+.001*gradient**2
            update = learning_rate*(adam_m/(1-.9**generation))/(np.sqrt(adam_v/(1-.999**generation))+1e-8)
            theta += scales*update-anchor_decay*(theta-anchor)
            row = {'generation': generation, 'groups': groups, 'objective_mean': float(values.mean()),
                   'objective_max': float(values.max()), 'seconds': time.monotonic()-tick,
                   'distance_from_parent': float(np.linalg.norm((theta-anchor)/scales))}
            if generation % validate_every == 0 or generation == generations:
                summary = pool.submit(_evaluate, (theta, valid_groups, seconds, True)).result()
                reasons = eligible(summary, baseline)
                row.update(validation=summary, selection_rejections=reasons)
                saved(theta, generation, run_dir/'brains'/f'gen-{generation:04d}.npz', validation=summary)
                if summary['objective'] > candidate_value:
                    candidate_value, candidate_gen, candidate_vector = summary['objective'], generation, theta.copy()
                if not reasons and summary['objective'] > best_value:
                    best_value, best_gen, best_vector = summary['objective'], generation, theta.copy()
                    saved(theta, generation, run_dir/'best.npz', validation=summary)
                log(f"Generation {generation}: validation {summary['objective']:.4f}, listener {summary['listener']:.4f}, best eligible {best_gen}; {len(reasons)} guardrail regressions", flush=True)
            else:
                log(f"Generation {generation}/{generations}: training {values.mean():.4f}, {row['seconds']:.1f}s, contexts {','.join(contexts)} + drawn grammars", flush=True)
            record(row)
            saved(theta, generation, run_dir/'latest.npz')
            np.savez(run_dir/'optimizer.npz', theta=theta, anchor=anchor, adam_m=adam_m, adam_v=adam_v)
            _json(run_dir/'state.json', {'generation': generation, 'best_generation': best_gen,
                  'rng_state': rng.bit_generator.state, 'elapsed_seconds': time.monotonic()-started})
        if best_gen == 0:
            saved(anchor, 0, run_dir/'best.npz', validation=baseline)
        saved(candidate_vector, candidate_gen, run_dir/'best_candidate.npz')
        # Select before touching test seeds. A non-improving candidate is reported honestly.
        selected_vector = best_vector if best_gen else candidate_vector
        selected_gen = best_gen or candidate_gen
        selected = saved(selected_vector, selected_gen, run_dir/'listening-candidate.npz', eligible_validation=bool(best_gen))
        log(f'Final tests: parent vs candidate generation {selected_gen}; all eight contexts, including unstyled transfer.', flush=True)
        before, after = list(pool.map(_evaluate, [(v, test_groups, seconds, True) for v in (anchor, selected_vector)]))
    changed = [p for p, digest in hashes.items() if _sha(root/p) != digest]
    if changed:
        raise RuntimeError(f'Inputs changed during training: {changed}')
    result = {'completed_at': dt.datetime.now(dt.timezone.utc).isoformat(),
              'elapsed_seconds': time.monotonic()-started, 'generations': generations,
              'best_eligible_generation': best_gen, 'candidate_generation': selected_gen,
              'parent': parent.identity(), 'candidate': selected.identity(),
              'source_hashes_unchanged': True, 'test_parent': before, 'test_candidate': after,
              'test_guardrail_regressions': eligible(after, before),
              'test_listener_delta': after['listener']-before['listener'],
              'test_objective_delta': after['objective']-before['objective'],
              'scope': 'Proxy training and unseen-seed measurements, not listener approval. Published parent remains unchanged.'}
    _json(run_dir/'result.json', result)
    log(f"Finished: test objective {before['objective']:.4f} -> {after['objective']:.4f}, listener {before['listener']:.4f} -> {after['listener']:.4f}", flush=True)
    return result
