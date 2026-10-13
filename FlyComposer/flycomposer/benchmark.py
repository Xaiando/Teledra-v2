"""Fixed-trial local composer benchmark; retain failures and never supply music."""
from __future__ import annotations

import copy
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import random
import time
from uuid import uuid4

import numpy as np

from .organist import _endpoint, _json, _schema, _stream_request, expand_authoring
from .experiment import atomic_json
from .buzzart.brain import Brain
from .buzzart.chip import CHIP_VOICES, PALETTES
from .buzzart.console_style import ConsoleStyle
from .buzzart.foundation import Foundation
from .buzzart.performer import perform
from .buzzart.showcase import render, server_running
from .connectome import load_circuit
from .render import save_render

SEEDS = (202609171, 202609172, 202609173)
BPM = 96
MUSIC_SECONDS = 40
CAPTURE_SECONDS = 41
PALETTE = 'genesis'
TOKENS = 8192


def sha(data):
    return hashlib.sha256(data).hexdigest()


def write(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False, allow_nan=False), encoding='utf-8')


def file_hash(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def validate_authoring(value, persona, seed):
    score = expand_authoring(value, set(PALETTES[PALETTE].values()), seed, CAPTURE_SECONDS)
    if score['bpm'] != BPM or score['voices'] != PALETTES[PALETTE]:
        raise ValueError('Keep the supplied 96 BPM and all five exact role-to-voice assignments')
    score.update(performer=persona, condition=persona, meter='4/4', beats_per_bar=4,
                 arrangement_version='composer_benchmark_clips_v1',
                 composition_rules='Only model-authored clip notes, placements and transpositions are expanded. No notes are added, stretched, retimed or pitch-corrected.')
    return score


def author_llm(root, directory, persona, seed, model, profile, progress=print):
    """One response and at most one technical repair; musical shortcomings are measured."""
    directory.mkdir(parents=True, exist_ok=False)
    voices = dict(PALETTES[PALETTE])
    schema = _schema(list(voices.values()))
    schema['properties']['bpm'] = {'type': 'number', 'enum': [BPM]}
    schema['properties']['voices'].update(required=list(voices),
        properties={part: {'type': 'string', 'enum': [voice]} for part, voice in voices.items()})
    identity = profile.decode('utf-8-sig')
    system = f'''You are performing as {persona.title()} in a standalone local music benchmark.
The quoted office profile establishes your persona. This task is composing instrumental music, not handling the court or addressing an audience. The studio JSON contract below is the output format for this task; do not emit court tags, Python, commentary, delegation, or playback commands.
<office_profile>
{identity}
</office_profile>
Compose the notes yourself. You have not heard a render and must not claim that you have. The other composer is working independently; none of its notes or results is supplied.'''
    prompt = f'''Compose a complete melodic game instrumental: exactly 16 bars of 4/4 at {BPM} BPM, 64 quarter-note beats = {MUSIC_SECONDS} seconds of music. Establish a recognizable motif, support it with harmony and bass, allow foreground breathing, develop an answering idea and contrasting texture, then return and end deliberately. We want layers and motion, not a repeated ascending scale. Choose the key, harmony, melody, rhythms and dynamics yourself.
Use this exact five-part palette: {json.dumps(voices)}.
Voice descriptions: {json.dumps({v: CHIP_VOICES[v] for v in voices.values()})}.
Noise drums use MIDI 36 for kick, 60 for snare and 80 for hat; do not write a pitched melody on noise. Chords on FM bell need one explicit event per note. Give parts their own simultaneous time lines.
Return a JSON object with exactly title,bpm,voices,clips. Each clip has exactly part,events,placements.
Each event is [MIDI_integer, relative_start_in_quarter_beats, duration_in_quarter_beats, velocity_0_to_1]. Each placement is [absolute_start_in_quarter_beats, semitone_transposition_integer]. A placement repeats exactly that clip, with the transposition you specify. There is no automatic looping, harmony, accompaniment or gap filling. Array order consumes no time. Different parts and chord notes can overlap. At96BPM a length of1 is a quarter note lasting0.625seconds; .25 is a sixteenth lasting0.15625seconds.
Use1..12clips,1..16events perclip and1..12placements perclip. Plan your reusable phrases and their placements to cover the whole64-beat piece, not just its opening. Vary and develop them instead of copying a whole short loop for every bar. Every assigned part needs notes. All MIDI pitches after transposition must be0..127, velocity must be>0 and<=1, lengths at least0.04beats and at most48beats. Every note END (placement_start + relative_start + duration) must be<=64beats. Begin by5seconds and carry the composition toward beat64; an ending slightly before that is fine. The renderer appends one second of release, but will not pad, extend or complete an unfinished composition.
Trial seed: {seed}. Return only the complete compact JSON; no placeholders or prose.'''
    messages = [{'role': 'system', 'content': system}, {'role': 'user', 'content': prompt}]
    result = {'persona': persona, 'model': model, 'seed': seed, 'profile_sha256': sha(profile),
              'token_limit_per_attempt': TOKENS, 'maximum_attempts': 2, 'attempts': []}
    (directory / 'profile.md').write_bytes(profile)
    started = time.monotonic()
    for attempt in range(2):
        payload = {'model': model, 'messages': messages, 'stream': True, 'format': schema,
                   'keep_alive': '10m', 'options': {'seed': seed, 'temperature': .7,
                                                   'num_ctx': 16384, 'num_predict': TOKENS}}
        raw = None
        inference = None
        try:
            raw, inference = _stream_request(_endpoint(), payload, directory, attempt,
                lambda message: progress(f'{persona.title()} {seed} attempt{attempt + 1}: {message}'))
            score = validate_authoring(_json(raw), persona, seed)
            result['attempts'].append({'attempt': attempt + 1, 'status': 'valid', 'inference': inference,
                                       'response_sha256': sha(raw.encode())})
            result.update(status='authored', wall_seconds=time.monotonic() - started)
            score['inference_provenance'] = copy.deepcopy(result)
            write(directory / 'score.json', score)
            write(directory / 'result.json', result)
            return score, result
        except Exception as error:
            result['attempts'].append({'attempt': attempt + 1, 'status': 'failed', 'error': str(error), 'inference': inference})
            # No partial streams are treated as valid notes or silently resumed.
            if raw is None or attempt == 1:
                result.update(status='failed', error=str(error), wall_seconds=time.monotonic() - started)
                write(directory / 'result.json', result)
                return None, result
            messages.extend([{'role': 'assistant', 'content': raw}, {'role': 'user', 'content':
                'Technical contract error: ' + str(error) + '. Return the complete corrected JSON. This is the sole repair attempt. No musical notes are supplied or changed for you.'}])
    raise AssertionError('Bounded authoring did not terminate')


def run(root, persona='orator', progress=print):
    from .benchmark_metrics import measure
    root = Path(root).resolve()
    if persona not in ('orator', 'organist'):
        raise ValueError('Choose orator or organist')
    if server_running():
        raise RuntimeError('Close Fly Composer before publishing a benchmark; its in-memory library must not overwrite new entries')
    model = os.environ.get('TELEDRA_MODEL', 'qwen2.5-coder:14b')
    profile_path = root.parent / 'Agents/profiles' / f'{persona}.md'
    profile = profile_path.read_bytes()
    index_path = root / 'library/index.json'
    baseline_bytes = index_path.read_bytes()
    baseline = json.loads(baseline_bytes)
    output = root / 'reports' / ('composer-benchmark-' + dt.datetime.now().strftime('%Y%m%d') + '-' + uuid4().hex[:8])
    output.mkdir(parents=True, exist_ok=False)
    (output / 'library-before.json').write_bytes(baseline_bytes)
    files = [root / name for name in (
        'brains/buzzart-indie-long.npz', 'brains/buzzart-indie-long.json',
        'data/circuit.npz', 'data/circuit.json', 'data/instruments.json',
        'data/foundation/indie-blend.json', 'data/foundation/indie-blend.manifest.json',
        'data/foundation/console.json', 'data/foundation/console.manifest.json',
        'flycomposer/benchmark.py', 'flycomposer/benchmark_metrics.py', 'flycomposer/render.py',
        'flycomposer/organist.py', 'flycomposer/connectome.py', 'flycomposer/controller.py',
        'flycomposer/autonomous.py')]
    files += sorted((root / 'flycomposer/buzzart').glob('*.py'))
    snapshots = {}
    for path in files:
        relative = path.relative_to(root)
        data = path.read_bytes()
        destination = output / 'inputs' / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(data)
        snapshots[str(relative)] = sha(data)
    (output / 'inputs' / f'{persona}-profile.md').write_bytes(profile)
    prior_audio = {track['id']: file_hash(root / 'library' / track['id'] / 'audio.wav') for track in baseline['tracks']}
    manifest = {'version': 'composer_benchmark_v1', 'created_at': dt.datetime.now(dt.timezone.utc).isoformat(),
                'persona': persona, 'model': model, 'seeds': list(SEEDS), 'bpm': BPM,
                'music_seconds': MUSIC_SECONDS, 'capture_seconds': CAPTURE_SECONDS,
                'palette': PALETTE, 'voices': PALETTES[PALETTE], 'temperament': 'indie',
                'profile_path': str(profile_path), 'profile_sha256': sha(profile), 'source_sha256': snapshots,
                'baseline_index_sha256': sha(baseline_bytes), 'prior_audio_sha256': prior_audio,
                'selection': 'Three seeds fixed before any new output. Every trial retained; no jam filter or best-of selection.',
                'training': 'Frozen generation1750 brain, indie-blend songbook, current indie interpreter. No training.',
                'tempo_override': 'ConsoleStyle.tempos=[96] before generation; no note retiming.',
                'llm_budget': {'max_attempts': 2, 'tokens_per_attempt': TOKENS, 'context_tokens': 16384,
                               'repair': 'One technical-contract repair only. Short/sparse/repetitive valid results are retained without aesthetic retries.'},
                'scope': 'Complete composing systems through the same chip synth, gain/pan rules and pairwise RMS/headroom. Buzzart has corpus-derived motifs and an engineered arrangement interpreter. This does not isolate biological wiring, model intelligence, or training benefit. No training-critic winner or listener verdict is inferred.',
                'trials': [], 'status': 'running'}
    write(output / 'manifest.json', manifest)
    progress(f'BENCHMARK {output}')
    circuit = load_circuit(root / 'data/circuit.npz')
    brain = Brain.load(root / 'brains/buzzart-indie-long.npz')
    foundation = Foundation.load(root / 'data/foundation/indie-blend.json')
    new_tracks, comparisons = [], []
    for round_number, seed in enumerate(SEEDS, 1):
        directory = output / f'round-{round_number}'
        directory.mkdir()
        trial = {'round': round_number, 'seed': seed, 'authors': {}, 'rendered': []}
        manifest['trials'].append(trial)
        write(output / 'manifest.json', manifest)
        scores = {}
        style = ConsoleStyle.load(root / 'data/foundation/console.json', palette=PALETTE, temperament='indie')
        style.tempos = [BPM]
        start = time.monotonic()
        try:
            score = perform(circuit, brain, seed, CAPTURE_SECONDS, voices=style.palette, foundation=foundation, style=style)
            scores['buzzart'] = score
            trial['authors']['buzzart'] = {'status': 'authored', 'wall_seconds': time.monotonic() - start,
                                           'brain': brain.identity(), 'notes': len(score['events'])}
            write(directory / 'buzzart-score.json', score)
        except Exception as error:
            trial['authors']['buzzart'] = {'status': 'failed', 'error': str(error), 'wall_seconds': time.monotonic() - start}
        score, evidence = author_llm(root, directory / persona, persona, seed, model, profile, progress)
        trial['authors'][persona] = evidence
        if score is not None:
            scores[persona] = score
        rendered = []
        for author, score in scores.items():
            try:
                if score['bpm'] != BPM or score['voices'] != PALETTES[PALETTE] or score['seed'] != seed:
                    raise ValueError('Composer did not preserve the shared tempo, voices and trial seed')
                progress(f'Round{round_number}: rendering {author}, {len(score["events"])} notes')
                audio, stems, audit = render(score, progress=lambda message: None)
                metrics = measure(score, audit, target_seconds=MUSIC_SECONDS)
                if not metrics['delivery']['complete']:
                    trial['authors'][author]['failed_delivery_evidence'] = metrics
                    raise ValueError('Independent delivery audit found missing or altered note/audio evidence')
                peak = max([audit['raw_peak']] + [float(np.max(np.abs(stem))) for stem in stems.values()])
                rendered.append((author, score, audio, stems, audit, metrics, .82 * audit['raw_rms'] / peak))
            except Exception as error:
                trial['authors'][author].update(status='render_failed', render_error=str(error))
        target = min(.055, *[r[-1] for r in rendered]) if rendered else None
        comparison_id = uuid4().hex[:16]
        created = dt.datetime.now(dt.timezone.utc).isoformat()
        tracks, variants = [], {}
        for author, score, audio, stems, audit, metrics, _ in rendered:
            track_id = uuid4().hex[:16]
            score['benchmark'] = {'manifest': str(output / 'manifest.json'), 'round': round_number,
                                  'music_seconds': MUSIC_SECONDS, 'bpm': BPM, 'palette': PALETTE}
            audit = save_render(root / 'library' / track_id, score, audio, stems, audit, target)
            display = 'Buzzart · trained Fly' if author == 'buzzart' else f'{persona.title()} · {model}'
            description = ('Frozen generation1750 Fly controller with its songbook and current indie arrangement interpreter.'
                           if author == 'buzzart' else 'The configured local model and office persona, authoring its own clips and placements; no supplied music.')
            track = {'id': track_id, 'title': f'{display} · benchmark {round_number}', 'created_at': created,
                     'duration': audit['duration_seconds'], 'condition': author, 'performer': author,
                     'kind': 'benchmark', 'ensemble': 'benchmark', 'seed': seed, 'voices': score['voices'],
                     'description': description, 'arrangement_version': score['arrangement_version'],
                     'benchmark_metrics': metrics, 'benchmark': score['benchmark'],
                     'metrics': {key: audit[key] for key in ('notes_scheduled', 'notes_mixed', 'layers', 'sections', 'peak', 'rms', 'unique_lead_phrases', 'pcm_sha256')},
                     'verdict': None, 'note': '', 'comparison_id': comparison_id}
            tracks.append(track)
            variants[track_id] = {'display_name': display, 'description': description}
            trial['authors'][author].update(status='recorded', track_id=track_id, audio_seconds=audit['duration_seconds'],
                                             benchmark_metrics=metrics, rms=audit['rms'], peak=audit['peak'])
            trial['rendered'].append(track_id)
            write(directory / f'{author}-metrics.json', metrics)
        random.Random(seed ^ 0xAB2026).shuffle(tracks)
        errors = [f'{author}: {record.get("render_error", record.get("error", "No recording"))}'
                  for author, record in trial['authors'].items() if record['status'] != 'recorded']
        comparison = {'id': comparison_id, 'kind': 'benchmark', 'title': f'Composer benchmark · round {round_number}',
                      'seed': seed, 'ensemble': 'benchmark', 'created_at': created, 'revealed': False,
                      'track_ids': [t['id'] for t in tracks], 'variants': variants, 'feedback': {},
                      'recording_seconds': CAPTURE_SECONDS, 'rms_target': target,
                      'recording_errors': errors, 'performer_status': {a: r['status'] for a, r in trial['authors'].items()},
                      'benchmark': {'music_seconds': MUSIC_SECONDS, 'bpm': BPM, 'palette': PALETTE,
                                    'performers': [persona, 'buzzart'], 'scope': manifest['scope'],
                                    'manifest': str(output / 'manifest.json')}}
        comparisons.append(comparison)
        new_tracks.extend(tracks)
        trial.update(comparison_id=comparison_id, rms_target=target)
        write(output / 'manifest.json', manifest)
        progress(f'Round{round_number} complete: {len(tracks)}/2 recordings')
    # Append only after all trials, against exactly the baseline we observed.
    if server_running() or index_path.read_bytes() != baseline_bytes:
        manifest.update(status='not_published', error='Studio started or library changed; recorded files retained without replacing the index')
        write(output / 'manifest.json', manifest)
        raise RuntimeError(manifest['error'])
    changed = [str(p.relative_to(root)) for p in files if file_hash(p) != snapshots[str(p.relative_to(root))]]
    if changed or profile_path.read_bytes() != profile:
        manifest.update(status='not_published', error='Benchmark source changed during run', changed_inputs=changed)
        write(output / 'manifest.json', manifest)
        raise RuntimeError(manifest['error'])
    preservation = {'tracks': len(baseline['tracks']), 'comparisons': len(baseline['comparisons']),
                    'audio_unchanged': all(file_hash(root / 'library' / track_id / 'audio.wav') == digest for track_id, digest in prior_audio.items())}
    if not preservation['audio_unchanged']:
        raise RuntimeError('An earlier audio file changed during the benchmark')
    candidate = copy.deepcopy(baseline)
    candidate['tracks'].extend(new_tracks)
    candidate['comparisons'].extend(comparisons)
    atomic_json(index_path, candidate)
    preservation.update(records_unchanged=candidate['tracks'][:len(baseline['tracks'])] == baseline['tracks'] and candidate['comparisons'][:len(baseline['comparisons'])] == baseline['comparisons'])
    write(output / 'preservation.json', preservation)
    manifest.update(status='published', new_tracks=len(new_tracks), comparisons=[c['id'] for c in comparisons])
    write(output / 'manifest.json', manifest)
    progress(f'Published {len(new_tracks)} recordings in {len(comparisons)} benchmark rounds. Report: {output}')
    return output
