"""Append audited long-form developments without replacing their source recordings."""
from __future__ import annotations

import copy
import datetime as dt
import hashlib
import json
from pathlib import Path
import shutil
from uuid import uuid4
import wave

import numpy as np

from ..benchmark_metrics import measure
from ..experiment import Studio, atomic_json
from .showcase import server_running


def _hash(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def _wav(path, frames, channels, sample_rate):
    with wave.open(str(path), 'rb') as stream:
        if (stream.getnframes(), stream.getnchannels(), stream.getframerate(), stream.getsampwidth()) != (frames, channels, sample_rate, 2):
            raise ValueError(f'Incomplete or unexpected WAV format: {path}')
        pcm = stream.readframes(frames)
    samples = np.frombuffer(pcm, dtype='<i2')
    if len(samples) != frames * channels or not len(samples) or np.max(np.abs(samples.astype(np.int32))) >= 32767:
        raise ValueError(f'Incomplete or clipped WAV: {path}')
    return {'pcm_sha256': hashlib.sha256(pcm).hexdigest(),
            'rms': float(np.mean((samples.astype(float) / 32767.) ** 2)) ** .5,
            'peak': float(np.max(np.abs(samples.astype(float)))) / 32767.}


def _check_recording(directory, source, source_score):
    score = json.loads((directory / 'score.json').read_text(encoding='utf-8'))
    audit = json.loads((directory / 'audit.json').read_text(encoding='utf-8'))
    if score.get('seed') != source['seed'] or score.get('voices') != source['voices'] or score.get('bpm') != source_score['bpm']:
        raise ValueError('A development changed the selected source seed, instruments or tempo')
    brain = score.get('buzzart', {}).get('brain', {})
    if not brain.get('brain_sha256') or brain['brain_sha256'] != source_score.get('buzzart', {}).get('brain', {}).get('brain_sha256'):
        raise ValueError('A development did not retain the selected source brain')
    sr, frames = audit.get('sample_rate'), audit.get('frames')
    if sr != 48000 or frames != 180 * sr or score.get('duration_seconds') != 180 or audit.get('duration_seconds') != 180:
        raise ValueError('A developed recording must be exactly 180 seconds at 48 kHz')
    # The denominator includes the release: coverage is labelled as a recording
    # measurement, not a promise of music occupying every tail sample.
    facts = measure(score, audit, target_seconds=180)
    if not audit.get('complete') or not facts['delivery']['complete']:
        raise ValueError('A developed recording has missing or altered note delivery')
    audio = _wav(directory / 'audio.wav', frames, 2, sr)
    if audio['pcm_sha256'] != audit.get('pcm_sha256') or audio['rms'] <= 1e-8:
        raise ValueError('The mixed WAV does not match its audit or is silent')
    if abs(audio['rms'] - audit['rms']) > 1 / 32767.:
        raise ValueError('The mixed WAV level does not match its audit')
    for role in score['voices']:
        _wav(directory / f'{role}.wav', frames, 1, sr)
    return score, audit, facts


def publish(root, developed, report_dir):
    """Publish both selected developments against a freshly read, closed Studio.

    ``developed`` contains ``context``, ``source_track_id`` and ``directory`` for
    each completed render. Existing source entries are reused, never relabelled
    or rewritten. This function does not render, train, infer listener approval,
    normalize old audio or stop/start the server.
    """
    root, report_dir = Path(root).resolve(), Path(report_dir).resolve()
    developed = list(developed)
    expected = {'dusk': 'd4906349cc154ac1', 'overworld': 'da8dfe5b464b417b'}
    if len(developed) != 2 or {case['context']: case['source_track_id'] for case in developed} != expected:
        raise ValueError('Publish the two selected dusk and overworld developments together')
    if server_running():
        raise RuntimeError('Close the Fly Composer server before publishing developments')
    if (report_dir / 'manifest.json').exists():
        raise ValueError('Publication evidence already exists; choose a fresh report directory')
    index = root / 'library/index.json'
    baseline_bytes = index.read_bytes()
    baseline = json.loads(baseline_bytes)
    if baseline.get('version') != 1 or not isinstance(baseline.get('tracks'), list) or not isinstance(baseline.get('comparisons'), list):
        raise ValueError('Preserve and inspect the existing library: its format is invalid')
    by_id = {track['id']: track for track in baseline['tracks']}
    if any(track_id not in by_id for track_id in expected.values()):
        raise ValueError('The selected source recording is missing from the library')
    prior_files = {str(path.relative_to(root)): _hash(path)
                   for path in sorted((root / 'library').rglob('*')) if path.is_file() and path != index}
    report_dir.mkdir(parents=True, exist_ok=True)
    (report_dir / 'library-before.json').write_bytes(baseline_bytes)
    created = dt.datetime.now(dt.timezone.utc).isoformat()
    manifest = {'version': 'buzzart_longform_publication_v1', 'status': 'validating', 'created_at': created,
                'baseline_index_sha256': hashlib.sha256(baseline_bytes).hexdigest(),
                'prior_file_sha256': prior_files, 'recordings': [],
                'scope': 'Arranged developments of two listener-selected recordings. Original audio and saved feedback are retained; no training or listener approval is inferred.',
                'loudness': 'Existing and developed recordings retain their saved mastering. This publication does not assert matched RMS or perceptual loudness.'}
    atomic_json(report_dir / 'manifest.json', manifest)
    new_tracks, comparisons = [], []
    try:
        for case in developed:
            context, source_id = case['context'], case['source_track_id']
            directory, source = Path(case['directory']).resolve(), by_id[source_id]
            source_score = json.loads((root / 'library' / source_id / 'score.json').read_text(encoding='utf-8'))
            score, audit, facts = _check_recording(directory, source, source_score)
            filenames = ['score.json', 'audit.json', 'audio.wav'] + [f'{role}.wav' for role in score['voices']]
            input_hashes = {name: _hash(directory / name) for name in filenames}
            track_id, comparison_id = uuid4().hex[:16], uuid4().hex[:16]
            destination = root / 'library' / track_id
            destination.mkdir(exist_ok=False)
            for name in filenames:
                shutil.copyfile(directory / name, destination / name)
            if any(_hash(destination / name) != digest or _hash(directory / name) != digest for name, digest in input_hashes.items()):
                raise RuntimeError('A developed recording changed during publication')
            longform = {'context': context, 'source_track_id': source_id, 'bpm': score['bpm'],
                        'recording_seconds': 180, 'manifest': str(report_dir / 'manifest.json'),
                        'development': copy.deepcopy(score.get('longform', {}))}
            description = (f'Three-minute development of the selected {context} recording; '
                           f'{score["bpm"]} BPM, seed {score["seed"]}. The source brain and instrument palette are retained. '
                           'Bass continuity, phrase development and balance were revised; listener approval remains open.')
            track = {'id': track_id, 'title': f'Developed · {context} · {score["seed"]}', 'created_at': created,
                     'kind': 'buzzart', 'condition': 'buzzart', 'ensemble': 'buzzart', 'performer': score['performer'],
                     'duration': audit['duration_seconds'], 'description': description, 'seed': score['seed'],
                     'voices': score['voices'], 'brain': score['buzzart']['brain'], 'longform': longform,
                     'arrangement_version': score['arrangement_version'],
                     'foundation': score.get('buzzart', {}).get('foundation'), 'style': score.get('buzzart', {}).get('style'),
                     'benchmark_metrics': facts,
                     'metrics': {key: audit[key] for key in ('notes_scheduled', 'notes_mixed', 'layers', 'sections', 'peak', 'rms', 'unique_lead_phrases', 'pcm_sha256')},
                     'verdict': None, 'note': '', 'comparison_id': comparison_id}
            original_description = f'Your selected source: {source["title"]}. This original recording and its existing comments are unchanged.'
            comparison = {'id': comparison_id, 'kind': 'buzzart', 'title': f'Three-minute development · {context}',
                          'seed': score['seed'], 'ensemble': 'buzzart', 'created_at': created, 'revealed': True,
                          'track_ids': [source_id, track_id],
                          'variants': {source_id: {'display_name': 'Original (1:01)', 'description': original_description},
                                       track_id: {'display_name': 'Developed (3:00)', 'description': description}},
                          'feedback': {}, 'recording_seconds': 180, 'longform': longform,
                          'scope': ('The selected one-minute recording and its three-minute development share the source brain, seed, tempo and instruments. '
                                    'The arrangement builds over a continuing bass foundation. Only one recording plays at a time; neither is marked as approved.')}
            new_tracks.append(track)
            comparisons.append(comparison)
            manifest['recordings'].append({'context': context, 'source_track_id': source_id, 'track_id': track_id,
                                           'comparison_id': comparison_id, 'input_directory': str(directory),
                                           'files_sha256': input_hashes, 'brain': score['buzzart']['brain'],
                                           'delivery': facts['delivery'], 'notes': len(score['events'])})
            atomic_json(report_dir / 'manifest.json', manifest)
        if server_running() or index.read_bytes() != baseline_bytes:
            raise RuntimeError('Server started or library changed; existing records were preserved')
        if any(not (root / name).is_file() or _hash(root / name) != digest for name, digest in prior_files.items()):
            raise RuntimeError('An earlier library file changed; developments were not published')
        studio = Studio(root)
        with studio.lock:
            if server_running() or index.read_bytes() != baseline_bytes or studio.library != baseline:
                raise RuntimeError('Library changed before publication; existing records were preserved')
            updated = copy.deepcopy(studio.library)
            updated['tracks'].extend(new_tracks)
            updated['comparisons'].extend(comparisons)
            atomic_json(index, updated)
        preservation = {'tracks': len(baseline['tracks']), 'comparisons': len(baseline['comparisons']),
                        'files': len(prior_files), 'wav_files': sum(name.lower().endswith('.wav') for name in prior_files),
                        'files_unchanged': True, 'audio_unchanged': True, 'source_records_reused_unchanged': list(expected.values()),
                        'records_unchanged': updated['tracks'][:len(baseline['tracks'])] == baseline['tracks']
                        and updated['comparisons'][:len(baseline['comparisons'])] == baseline['comparisons']}
        atomic_json(report_dir / 'preservation.json', preservation)
        manifest.update(status='published', new_tracks=2, new_comparisons=2)
        atomic_json(report_dir / 'manifest.json', manifest)
        return report_dir
    except Exception as error:
        manifest.update(status='not_published', error=str(error))
        atomic_json(report_dir / 'manifest.json', manifest)
        raise
