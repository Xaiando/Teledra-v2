import copy
import hashlib
import json
import math
import unittest

import numpy as np

from flycomposer.buzzart.autonomy_critic import evaluate, assess_audio
from flycomposer.render import PAN


def note(role, start, length, midi, velocity=.55, voice=None):
    return {'layer':role,'voice':voice or {'lead':'piano','answer':'vibraphone','bass':'fm:bass',
            'harmony':'warm_pad','texture':'chip:noise'}[role],
            'midi':midi,'start_seconds':start*.5,'duration_seconds':length*.5,'velocity':velocity,
            'start_beat':start,'length_beats':length}


def fixture():
    events=[]
    roots=(0,5,7,2)
    chords=((48,52,55),(53,57,60),(55,59,62),(50,53,57))
    melody=(60,64,67,62,65,69,71,67)
    for bar in range(16):
        for pulse in range(4):
            events.append(note('bass',bar*4+pulse,.96,36+roots[bar%4],.50+.04*(pulse%3)))
        for pitch in chords[bar%4]:
            events.append(note('harmony',bar*4,3.85,pitch,.39))
        for step in range(2):
            i=bar*2+step
            events.append(note('lead' if i%3 else 'answer',i*2,1.75,
                               melody[(i+bar//4*3)%8]+12*(bar//8),.48+.025*(i%7)))
    score={'events':events,'bpm':120,'duration_seconds':33.,'tonic':'C','mode':'major',
           'voices':{role:voice for role,voice in (('lead','piano'),('answer','vibraphone'),
                     ('harmony','warm_pad'),('bass','fm:bass'),('texture','chip:noise'))}}
    return score


def audio_fixture(score):
    score=copy.deepcopy(score)
    sr=4000
    frames=int(score['duration_seconds']*sr)
    stems={r:np.zeros(frames) for r in PAN}
    ledger=[]
    for i,e in enumerate(score['events']):
        e['id']=i
        left,size=int(e['start_seconds']*sr),int(e['duration_seconds']*sr)
        t=np.arange(size)/sr
        signal=.003*np.sin(2*np.pi*(110+i%7*13)*t)
        stems[e['layer']][left:left+size] += signal
        ledger.append({**e,'start_frame':left,'frames':size,'end_frame':left+size,'mixed':True,
                       'placed_rms':float(np.sqrt(np.mean(signal**2))),
                       'note_pcm_sha256':hashlib.sha256(signal.astype('<f4').tobytes()).hexdigest()})
    sha=hashlib.sha256(json.dumps(score['events'],sort_keys=True).encode()).hexdigest()
    score['event_sha256']=sha
    audit={'sample_rate':sr,'frames':frames,'channels':2,'events':ledger,'event_sha256':sha,'complete':True,
           'notes_scheduled':len(ledger),'notes_mixed':len(ledger)}
    return score,audit,stems,mix(stems)


def mix(stems):
    out=np.zeros((len(next(iter(stems.values()))),2))
    for role,stem in stems.items():
        angle=(PAN[role]+1)*np.pi/4
        out += stem[:,None]*np.array([np.cos(angle),np.sin(angle)])
    return out


class AutonomyCriticTests(unittest.TestCase):
    def test_layered_music_beats_missing_bass_with_learning_gradient(self):
        good=fixture(); a=evaluate(good)
        missing=copy.deepcopy(good);missing['events']=[e for e in missing['events'] if e['layer']!='bass']
        sparse=copy.deepcopy(good);sparse['events']=[e for e in sparse['events'] if e['layer']!='bass' or e['start_seconds']<8]
        b,c=evaluate(missing),evaluate(sparse)
        self.assertTrue(a['acceptable'],a['rejection_reasons'])
        self.assertGreater(a['reward'],.80)
        self.assertGreater(a['reward'],c['reward'])
        self.assertGreater(c['reward'],b['reward'])
        self.assertGreater(b['reward'],0.)
        self.assertIn('bass_coverage_below_60_percent',b['rejection_reasons'])

    def test_drones_cannot_simulate_drive_or_layers(self):
        score=fixture()
        score['events']=[note(r,0,64,p) for r,p in (('lead',60),('harmony',64),('bass',36))]
        r=evaluate(score)
        self.assertFalse(r['acceptable'])
        self.assertIn('bass_lacks_repeated_activity',r['rejection_reasons'])
        self.assertLess(r['reward'],evaluate(fixture())['reward']*.60)

    def test_repeated_scale_is_rejected_despite_full_backing(self):
        score=fixture();score['events']=[e for e in score['events'] if e['layer'] not in ('lead','answer')]
        scale=(60,62,64,65,67,69,71,72)
        score['events'] += [note('lead',i,.94,scale[i%8],.5+.02*(i%3)) for i in range(64)]
        r=evaluate(score)
        self.assertIn('extended_scalar_runs',r['rejection_reasons'])
        self.assertGreater(r['metrics']['scalar_run_note_share'],.8)
        self.assertLess(r['components']['development'],.6)
        self.assertLess(r['reward'],evaluate(fixture())['reward']-.15)

    def test_transposed_jittered_loop_not_saved_by_velocity(self):
        score=fixture();score['events']=[e for e in score['events'] if e['layer'] not in ('lead','answer')]
        for phrase in range(8):
            for i,pitch in enumerate((60,64,67,62)):
                score['events'].append(note('lead',phrase*8+i*2+.003*(phrase%2),1.7,pitch+12*(phrase%2),.4+.03*phrase))
        r=evaluate(score)
        self.assertAlmostEqual(r['metrics']['dominant_two_bar_share'],1.)
        self.assertIn('repetitive_foreground_cells',r['rejection_reasons'])

    def test_loud_noise_cannot_buy_depth(self):
        score=fixture();before=evaluate(score)
        score['events'] += [note('texture',i/4,.2,80,1.) for i in range(256)]
        score['mix']={'texture':3.}
        r=evaluate(score)
        self.assertIn('noise_dominates_score_balance',r['rejection_reasons'])
        self.assertEqual(before['metrics']['distinct_pitched_roles_mean'],r['metrics']['distinct_pitched_roles_mean'])
        self.assertLess(r['reward'],before['reward']-.15)

    def test_duplicate_chord_tones_are_not_more_roles(self):
        a=fixture();b=copy.deepcopy(a)
        b['events'] += [{**e,'midi':e['midi']+12} for e in a['events'] if e['layer']=='harmony']
        self.assertEqual(evaluate(a)['metrics']['distinct_pitched_roles_mean'],evaluate(b)['metrics']['distinct_pitched_roles_mean'])

    def test_tempo_instrument_and_gain_width(self):
        a=fixture();before=copy.deepcopy(a);b=copy.deepcopy(a)
        ratio=120/76
        b['bpm']=76;b['duration_seconds']=(b['duration_seconds']-1)*ratio+1
        for e in b['events']:
            e['start_seconds']*=ratio;e['duration_seconds']*=ratio;e['velocity']*=.5
            e['voice']='cello' if e['layer']=='bass' else 'harp'
        b['voices']={r:'cello' if r=='bass' else 'harp' for r in b['voices']}
        self.assertAlmostEqual(evaluate(a)['reward'],evaluate(b)['reward'],places=10)
        self.assertEqual(a,before)

    def test_declared_slow_bass_has_room_for_changing_held_notes(self):
        score=fixture();score['events']=[e for e in score['events'] if e['layer']!='bass']
        score['buzzart']={'style':{'temperament':'dreamy'}}
        score['events'] += [note('bass',i*8,7.8,p,.5) for i,p in enumerate((36,41,43,38,41,43,38,36))]
        r=evaluate(score)
        self.assertTrue(r['acceptable'],r['rejection_reasons'])
        self.assertGreater(r['components']['drive'],.90)

    def test_invalid_numbers_and_early_ending(self):
        a=fixture();a['events'][0]['duration_seconds']=math.nan
        with self.assertRaises(ValueError):evaluate(a)
        a=fixture();r=evaluate(a);a['duration_seconds']=65
        self.assertLess(evaluate(a)['reward'],r['reward'])
        self.assertFalse(evaluate(a)['acceptable'])

    def test_actual_arrays_pass_without_mutation(self):
        s,a,stems,out=audio_fixture(fixture());before=out.copy()
        r=assess_audio(s,a,stems,out)
        self.assertTrue(r['passed'],r['rejection_reasons'])
        np.testing.assert_array_equal(before,out)

    def test_silent_gates_and_quiet_bass_are_detected(self):
        s,a,stems,out=audio_fixture(fixture())
        stems['bass'][len(stems['bass'])//3:]=0
        r=assess_audio(s,a,stems,mix(stems))
        self.assertIn('near_silent_authored_gates',r['rejection_reasons'])
        s,a,stems,out=audio_fixture(fixture());stems['bass']*=.00001
        self.assertIn('bass_level_negligible',assess_audio(s,a,stems,mix(stems))['rejection_reasons'])

    def test_pcm_clipping_missing_delivery_and_noise_fail(self):
        s,a,stems,out=audio_fixture(fixture());out[0,0]=1.01
        self.assertIn('mix_peak_unsafe',assess_audio(s,a,stems,out)['rejection_reasons'])
        s,a,stems,out=audio_fixture(fixture());a['events']=a['events'][:-1]
        self.assertIn('incomplete_note_delivery',assess_audio(s,a,stems,out)['rejection_reasons'])
        s,a,stems,out=audio_fixture(fixture());stems['texture'][:]=.03*np.sin(np.arange(len(out)))
        self.assertIn('noise_dominates_actual_stems',assess_audio(s,a,stems,mix(stems))['rejection_reasons'])

    def test_frame_claims_and_false_pcm_evidence_do_not_pass(self):
        s,a,stems,out=audio_fixture(fixture());a['frames']-=100
        self.assertIn('audit_recording_shape_mismatch',assess_audio(s,a,stems,out)['rejection_reasons'])
        s,a,stems,out=audio_fixture(fixture());a['events'][0]['note_pcm_sha256']='z'*64
        self.assertIn('missing_note_pcm_evidence',assess_audio(s,a,stems,out)['rejection_reasons'])


if __name__=='__main__':
    unittest.main()
