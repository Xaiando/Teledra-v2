"""Publish predeclared, paired before/after training captures without selecting jams.

Both brains hear the same seed, songbook, grammar, palette and tempo.  A natural
parent performance chooses the tempo; both recorded performances are then
composed afresh with that tempo fixed.  No notes are retimed or supplied here.
"""
from __future__ import annotations

import copy
import datetime as dt
import hashlib
import json
import math
from pathlib import Path
from uuid import uuid4

import numpy as np

from ..benchmark_metrics import measure
from ..connectome import load_circuit
from ..experiment import Studio, atomic_json
from ..render import save_render
from .brain import Brain
from .critic import evaluate
from .performer import perform
from .showcase import render, server_running


def _hash(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def _write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    atomic_json(path, value)


def _cases(cases):
    result = []
    for case in cases:
        if isinstance(case, dict):
            context, seed = case['context'], case['seed']
        else:
            context, seed = case
        if not isinstance(context, str) or not context.strip():
            raise ValueError('Each case needs a named musical context')
        if type(seed) is not int or not 0 <= seed < 2**31:
            raise ValueError('Each case needs an integer performance seed')
        result.append({'context': context, 'seed': seed})
    if not result or len({(x['context'], x['seed']) for x in result}) != len(result):
        raise ValueError('Declare at least one case, with no duplicate context/seed pairs')
    return result


def _source_hashes(root):
    files = sorted((root / 'flycomposer').rglob('*.py'))
    files += sorted((root / 'data').rglob('*.json'))
    files += sorted((root / 'data').glob('*.npz'))
    return {str(path.relative_to(root)): _hash(path) for path in files}


def _brain(value):
    return value if isinstance(value, Brain) else Brain.load(value)


def paired_scores(circuit, parent, candidate, case, seconds, options):
    """Return two independently composed scores under identical fixed controls."""
    style = options.get('style')
    if style is None:
        raise ValueError('Paired publication needs a style with an explicit tempo control')
    original = copy.deepcopy(options)
    probe = perform(circuit, parent, case['seed'], seconds,
                    voices=dict(original['style'].palette), **original)
    shared = copy.deepcopy(options)
    shared['style'].tempos = [probe['bpm']]
    voices = dict(shared['style'].palette)
    scores = []
    for brain in (parent, candidate):
        score = perform(circuit, brain, case['seed'], seconds, voices=dict(voices),
                        **copy.deepcopy(shared))
        if (score['bpm'] != probe['bpm'] or score['seed'] != case['seed']
                or score['voices'] != voices or score['duration_seconds'] != seconds):
            raise ValueError('A paired performance changed its shared controls')
        scores.append(score)
    return scores


def publish_pairs(root, parent, candidate, cases, seconds=61, *, run_dir,
                  options_factory=None, progress=print, port=8789):
    """Append all declared pairs, retaining the existing library byte-for-byte in evidence.

    ``parent`` and ``candidate`` accept Brain objects or saved brain paths.
    ``cases`` is a sequence of ``{'context': name, 'seed': integer}`` (or pairs).
    Evidence goes in a new ``run_dir/listening-comparisons`` directory.  All
    captures must pass note-delivery checks before any comparison is published.
    On failure, evidence and completed new recordings remain available, but the
    old library index is never replaced.  The listening-room server must stay
    closed for the entire operation.
    """
    root = Path(root).resolve()
    cases = _cases(cases)
    if (isinstance(seconds, bool) or not isinstance(seconds, (int, float))
            or not math.isfinite(seconds) or not 10 <= seconds <= 180):
        raise ValueError('Capture duration must be between 10 and 180 seconds')
    if server_running(port):
        raise RuntimeError('Close the Fly Composer server before publishing training comparisons')
    if options_factory is None:
        from .breadth_training import make_options
        options_factory = make_options
    parent, candidate = _brain(parent), _brain(candidate)
    before_identity, after_identity = parent.identity(), candidate.identity()
    output = Path(run_dir).resolve() / 'listening-comparisons'
    output.mkdir(parents=True, exist_ok=False)
    index = root / 'library/index.json'
    baseline_bytes = index.read_bytes()
    baseline = json.loads(baseline_bytes)
    if baseline.get('version') != 1 or not isinstance(baseline.get('tracks'), list) or not isinstance(baseline.get('comparisons'), list):
        raise ValueError('Preserve and inspect the existing library: its format is invalid')
    (output / 'library-before.json').write_bytes(baseline_bytes)
    prior_wavs = {str(path.relative_to(root)): _hash(path)
                  for path in sorted((root / 'library').rglob('*.wav'))}
    source_hashes = _source_hashes(root)
    copy.deepcopy(parent).save(output / 'parent.npz')
    copy.deepcopy(candidate).save(output / 'trained.npz')
    created = dt.datetime.now(dt.timezone.utc).isoformat()
    manifest = {'version': 'buzzart_breadth_listening_v1', 'status': 'rendering',
                'created_at': created, 'cases': cases, 'capture_seconds': seconds,
                'music_seconds': seconds - 1, 'parent': before_identity, 'trained': after_identity,
                'baseline_index_sha256': hashlib.sha256(baseline_bytes).hexdigest(),
                'prior_wav_sha256': prior_wavs, 'source_sha256': source_hashes,
                'selection': 'All declared cases, one performance per brain; no jam filter or best-of selection.',
                'tempo': 'Parent chooses a natural tempo first. Both recorded scores are composed afresh with that tempo pinned.',
                'loudness': 'Shared full-recording RMS target per pair, limited by stereo and individual-stem headroom; not perceptual loudness matching.',
                'scope': 'Before and trained Buzzart with the same interpreter, seed, songbook, grammar, palette and tempo. Designed critic values are measurements, not listener preferences.',
                'pairs': []}
    _write(output / 'manifest.json', manifest)
    new_tracks, comparisons = [], []
    try:
        circuit = load_circuit(root / 'data/circuit.npz')
        for number, case in enumerate(cases, 1):
            context, seed = case['context'], case['seed']
            progress(f'Pair {number}/{len(cases)}: {context}, seed {seed}')
            options = options_factory(root, context, seed)
            scores = paired_scores(circuit, parent, candidate, case, seconds, options)
            pair_dir = output / f'pair-{number}'
            pair_dir.mkdir()
            pair = {'case': case, 'bpm': scores[0]['bpm'], 'voices': scores[0]['voices'], 'recordings': []}
            manifest['pairs'].append(pair)
            _write(output / 'manifest.json', manifest)
            rendered = []
            for label, brain, score in zip(('Before', 'Trained'), (parent, candidate), scores):
                _write(pair_dir / f'{label.lower()}-score.json', score)
                audio, stems, audit = render(score, progress=lambda message: None)
                facts = measure(score, audit, target_seconds=seconds - 1)
                _write(pair_dir / f'{label.lower()}-delivery.json', facts)
                if not facts['delivery']['complete']:
                    raise RuntimeError(f'{context} {label}: missing or altered note-delivery evidence')
                peak = max([audit['raw_peak']] + [float(np.max(np.abs(stem))) for stem in stems.values()])
                if not math.isfinite(peak) or peak <= 0 or not math.isfinite(audit['raw_rms']) or audit['raw_rms'] <= 0:
                    raise RuntimeError(f'{context} {label}: recording is silent or non-finite')
                score['critic'] = evaluate(score)
                rendered.append((label, brain, score, audio, stems, audit, facts, .82 * audit['raw_rms'] / peak))
            target_rms = min(.055, *(item[-1] for item in rendered))
            comparison_id = uuid4().hex[:16]
            variants, tracks = {}, []
            for label, brain, score, audio, stems, audit, facts, _ in rendered:
                track_id = uuid4().hex[:16]
                destination = root / 'library' / track_id
                if destination.exists():
                    raise RuntimeError('New recording ID collision; refusing to replace a directory')
                score['training_comparison'] = {'context': context, 'stage': label.lower(),
                                               'manifest': str(output / 'manifest.json')}
                audit = save_render(destination, score, audio, stems, audit, target_rms)
                description = f'{label} broad training; {context}, {score["bpm"]} BPM, seed {seed}. Shared controls; no listener verdict inferred.'
                critic = score['critic']
                track = {'id': track_id, 'title': f'{label} · {context} · {seed}', 'created_at': created,
                         'kind': 'buzzart', 'condition': 'buzzart', 'ensemble': 'buzzart',
                         'performer': score['performer'], 'duration': audit['duration_seconds'],
                         'description': description, 'seed': seed, 'voices': score['voices'],
                         'brain': brain.identity(), 'arrangement_version': score['arrangement_version'],
                         'foundation': score.get('buzzart', {}).get('foundation'),
                         'style': score.get('buzzart', {}).get('style'),
                         'training_comparison': score['training_comparison'], 'benchmark_metrics': facts,
                         'critic': {'reward': critic['reward'], 'components': critic['components']},
                         'metrics': {key: audit[key] for key in ('notes_scheduled', 'notes_mixed', 'layers', 'sections', 'peak', 'rms', 'unique_lead_phrases', 'pcm_sha256')},
                         'verdict': None, 'note': '', 'comparison_id': comparison_id}
                tracks.append(track)
                variants[track_id] = {'display_name': label, 'description': description}
                pair['recordings'].append({'stage': label.lower(), 'track_id': track_id,
                                           'brain': brain.identity(), 'event_sha256': score['event_sha256'],
                                           'audio_sha256': _hash(destination / 'audio.wav'),
                                           'rms': audit['rms'], 'peak': audit['peak'], 'facts': facts})
            comparison = {'id': comparison_id, 'kind': 'buzzart', 'title': f'Broad training · {context}',
                          'seed': seed, 'ensemble': 'buzzart', 'created_at': created, 'revealed': True,
                          'track_ids': [track['id'] for track in tracks], 'variants': variants, 'feedback': {},
                          'recording_seconds': seconds, 'rms_target': target_rms,
                          'training_comparison': {'context': context, 'bpm': scores[0]['bpm'],
                                                  'manifest': str(output / 'manifest.json')},
                          'scope': (f'Before and trained Buzzart: {context}, {scores[0]["bpm"]} BPM, '
                                    f'{seconds - 1:g} seconds of music plus release. Same seed, songbook, '
                                    'grammar, instruments and tempo; paired RMS with stem headroom. '
                                    'Every scheduled note passed the delivery audit. The designed critic does not choose your preference.')}
            new_tracks.extend(tracks)
            comparisons.append(comparison)
            pair.update(comparison_id=comparison_id, rms_target=target_rms)
            _write(output / 'manifest.json', manifest)

        if server_running(port) or index.read_bytes() != baseline_bytes:
            raise RuntimeError('Server started or library changed; new recordings retained without replacing the index')
        if _source_hashes(root) != source_hashes:
            raise RuntimeError('Source or corpus changed during rendering; comparisons not published')
        if parent.identity() != before_identity or candidate.identity() != after_identity:
            raise RuntimeError('A brain changed during rendering; comparisons not published')
        if any(_hash(root / relative) != digest for relative, digest in prior_wavs.items()):
            raise RuntimeError('An existing WAV changed; comparisons not published')
        # Read a fresh Studio only after all rendering, so no stale in-memory
        # library can overwrite listener comments or unrelated new recordings.
        studio = Studio(root)
        with studio.lock:
            if server_running(port) or index.read_bytes() != baseline_bytes or studio.library != baseline:
                raise RuntimeError('Library changed before publication; existing records were preserved')
            updated = copy.deepcopy(studio.library)
            updated['tracks'].extend(new_tracks)
            updated['comparisons'].extend(comparisons)
            atomic_json(index, updated)
        preservation = {'tracks': len(baseline['tracks']), 'comparisons': len(baseline['comparisons']),
                        'wav_files': len(prior_wavs), 'audio_unchanged': True,
                        'records_unchanged': updated['tracks'][:len(baseline['tracks'])] == baseline['tracks']
                        and updated['comparisons'][:len(baseline['comparisons'])] == baseline['comparisons']}
        _write(output / 'preservation.json', preservation)
        manifest.update(status='published', new_tracks=len(new_tracks), comparisons=[c['id'] for c in comparisons])
        _write(output / 'manifest.json', manifest)
        progress(f'Published {len(comparisons)} before/trained pairs: {output}')
        return output
    except Exception as error:
        manifest.update(status='not_published', error=str(error))
        _write(output / 'manifest.json', manifest)
        raise
