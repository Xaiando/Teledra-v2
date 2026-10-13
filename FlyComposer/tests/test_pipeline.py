import copy
import hashlib
import json
from pathlib import Path
import shutil
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch
import urllib.request
import urllib.error
import wave

import numpy as np

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT))
from flycomposer.connectome import load_circuit, table_edges
from flycomposer.controller import Controller, shuffled_edges
from flycomposer.composition import write_song, assess_score, ROLES
from flycomposer.render import render_score, save_render, LEVEL, PAN, note_seed, instrument_catalog
from flycomposer.vendor import teledra_synth as synth
from flycomposer.experiment import Studio, atomic_json, compare_scores
from flycomposer.server import make_server


class CircuitTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):cls.c=load_circuit()

    def test_provenance_and_exact_known_counts(self):
        metadata=self.c['metadata']
        self.assertEqual((metadata['nodes'],metadata['edges'],metadata['represented_neurons']),(51,1309,623))
        self.assertEqual(int(self.c['weights'].sum()),847131)
        self.assertEqual(len(set(zip(self.c['src'],self.c['dst']))),1309)
        for source in metadata['sources']:
            raw=(ROOT/'data'/source['file']).read_bytes()
            self.assertEqual(hashlib.sha256(raw).hexdigest(),source['sha256'])
        parsed={e['target']:e['weight'] for e in table_edges((ROOT/'data/type_pages/EPG.html').read_text(encoding='utf8'))}
        self.assertEqual({k:parsed[k] for k in ('Delta7','EPG','PEG')},{'Delta7':19896,'EPG':11486,'PEG':6154})
        names={n['name']:n['index'] for n in metadata['neuron_types']}
        for target in ('Delta7','EPG','PEG'):
            edge=(self.c['src']==names['EPG'])&(self.c['dst']==names[target])
            self.assertEqual(self.c['weights'][edge].tolist(),[parsed[target]])

    def test_corrupt_graph_refused(self):
        with tempfile.TemporaryDirectory() as folder:
            p=Path(folder)/'circuit.npz';shutil.copy(ROOT/'data/circuit.npz',p);shutil.copy(ROOT/'data/circuit.json',p.with_suffix('.json'))
            p.write_bytes(p.read_bytes()+b'corrupt')
            with self.assertRaisesRegex(ValueError,'provenance'):load_circuit(p)

    def test_shuffle_retains_degrees_source_weights_and_within_type_edges(self):
        src,dst=self.c['src'],self.c['dst']
        shuffled,swaps=shuffled_edges(src,dst,42)
        self.assertGreater(swaps,100)
        np.testing.assert_array_equal(np.bincount(dst),np.bincount(shuffled))
        np.testing.assert_array_equal(shuffled[src==dst],dst[src==dst])
        self.assertEqual(int(np.sum(src==shuffled)),51)
        self.assertEqual(len(set(zip(src,shuffled))),len(src))
        self.assertGreater(np.count_nonzero(shuffled!=dst),700)
        a,b=Controller(self.c,42),Controller(self.c,42,'shuffled')
        np.testing.assert_array_equal(a.src,b.src)
        np.testing.assert_array_equal(a.inputs,b.inputs);np.testing.assert_array_equal(a.readout,b.readout)
        np.testing.assert_array_equal(a.state,b.state)
        np.testing.assert_array_equal(a.weights[src==dst],b.weights[src==dst])

    def test_several_seeds_complete_and_circuit_survives_decoding(self):
        for seed in (1,2,7,42,73,77,99,2026):
            scores={c:write_song(self.c,seed,'keys',c) for c in ('connectome','shuffled','baseline','disconnected')}
            for score in scores.values():
                facts=assess_score(score)
                self.assertEqual(facts['layers'],5);self.assertEqual(facts['sections'],4)
                self.assertGreater(facts['lead_rest_fraction'],.3)
                self.assertGreaterEqual(facts['unique_lead_phrases'],6)
                last=max((e for e in score['events'] if e['layer']=='lead'),key=lambda e:e['start_beat'])
                first=next(e for e in score['events'] if e['layer']=='bass')
                self.assertEqual(last['midi']%12,first['midi']%12)
                self.assertGreater(score['duration_seconds']-max(e['start_seconds']+e['duration_seconds'] for e in score['events']),1)
            for row in compare_scores(scores):
                self.assertFalse(row['identical_event_ledger']);self.assertGreater(row['event_jaccard_distance'],.02)
            self.assertEqual(scores['connectome']['event_sha256'],write_song(self.c,seed)['event_sha256'])
            for key in ('input_projection_sha256','readout_sha256'):
                self.assertEqual(len({s['controller'][key] for s in scores.values()}),1)


class RenderTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):cls.score=write_song(load_circuit(),42)

    def test_every_note_independently_reconstructed_and_rate_preserved(self):
        sr=1000
        def factory(event,rate):
            return 1+np.arange(int(event['duration_seconds']*rate))%7+event['id']%3
        audio,stems,audit=render_score(self.score,factory,sr)
        expected={role:np.zeros(int(self.score['duration_seconds']*sr)) for role in ROLES}
        for event in self.score['events']:
            raw=factory(event,sr).astype(float)
            amplitude=.08*LEVEL[event['layer']]*event['velocity']
            values=raw/np.sqrt(np.dot(raw,raw)/len(raw))*amplitude
            positions=np.arange(len(values))+int(event['start_seconds']*sr)
            np.add.at(expected[event['layer']],positions,values)
        wanted=np.zeros_like(audio)
        for role in ROLES:
            np.testing.assert_allclose(stems[role],expected[role],atol=1e-14)
            wanted+=expected[role][:,None]*np.array([np.cos((PAN[role]+1)*np.pi/4),np.sin((PAN[role]+1)*np.pi/4)])
        np.testing.assert_allclose(audio,wanted,atol=1e-14)
        self.assertEqual(audit['notes_mixed'],len(self.score['events']))
        with tempfile.TemporaryDirectory() as folder:
            save_render(Path(folder)/'track',self.score,audio,stems,audit,.035)
            with wave.open(str(Path(folder)/'track/audio.wav')) as wav:
                self.assertEqual(wav.getframerate(),sr);self.assertEqual(wav.getnframes(),len(audio))

    def test_silent_truncated_nonfinite_notes_refused(self):
        for factory in (lambda e,s:np.zeros(int(e['duration_seconds']*s)),lambda e,s:np.ones(int(e['duration_seconds']*s)-1),lambda e,s:np.full(int(e['duration_seconds']*s),np.nan)):
            with self.assertRaises(ValueError):render_score(self.score,factory,1000)

    def test_semantic_synthesis_seed_ignores_event_insertion_id(self):
        event=self.score['events'][0]
        self.assertEqual(note_seed(self.score,event),note_seed(self.score,{**event,'id':event['id']+50}))

    def test_all_72_packaged_instruments_fire(self):
        catalog=instrument_catalog();self.assertGreaterEqual(len(catalog),72)
        self.assertTrue({'piano','theremin'}<={v['id'] for v in catalog})
        synth._configure_bridge(ROOT/'bin/PyMusicSynth.exe')
        try:
            for voice in catalog:
                pitch=sum(voice['range'])/2
                a=synth.instrument_note(440*2**((pitch-69)/12),.375,voice['id'],volume=.5,sr=48000,seed=77)
                self.assertEqual(len(a),18000,voice['id']);self.assertTrue(np.all(np.isfinite(a)),voice['id'])
                self.assertGreater(float(np.sqrt(np.mean(a*a))),1e-7,voice['id'])
                self.assertLess(float(np.max(np.abs(a))),1,voice['id'])
        finally:synth._close_bridge()


class LibraryTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.root=Path(self.tmp.name)
        shutil.copytree(ROOT/'data',self.root/'data',ignore=shutil.ignore_patterns('raw','type_pages','*.html','*.js','probe_download.py'))
        self.studio=Studio(self.root)
        self.tracks=[{'id':str(i)*16,'title':f'Test {i}','condition':condition,'duration':40,'verdict':None,'note':''} for i,condition in zip((1,2,3),('connectome','shuffled','baseline'))]
        self.comparison={'id':'test','track_ids':['2'*16,'1'*16,'3'*16],'revealed':False,'feedback':{},'evaluation':[{'left':'connectome'}]}
        self.studio.library={'version':1,'tracks':self.tracks,'comparisons':[self.comparison]}
        atomic_json(self.studio.index,self.studio.library)

    def tearDown(self):self.tmp.cleanup()

    def test_blind_order_reveal_and_feedback_survive_restart(self):
        state=self.studio.state()
        self.assertEqual([t['id'] for t in state['library']],self.comparison['track_ids'])
        self.assertEqual({t['condition'] for t in state['library']},{'hidden'})
        self.assertNotIn('evaluation',state['comparisons'][0])
        self.studio.feedback({'track_id':'2'*16,'verdict':'keep','note':'Actual listener verdict'})
        self.studio.comparison_action({'comparison_id':'test','preferred_track_id':'2'*16,'motion':4})
        reloaded=Studio(self.root)
        self.assertEqual(reloaded.state()['library'][0]['verdict'],'keep')
        self.assertFalse(reloaded.library['comparisons'][0]['feedback']['identities_revealed_at_rating'])
        reloaded.comparison_action({'comparison_id':'test'},reveal=True)
        self.assertEqual(Studio(self.root).state()['library'][0]['condition'],'shuffled')
        with self.assertRaises(ValueError):reloaded.comparison_action({'comparison_id':'test','preferred_track_id':'nope'})

    def test_failed_save_does_not_change_live_state(self):
        original=copy.deepcopy(self.studio.library)
        with patch('flycomposer.experiment.atomic_json',side_effect=OSError('disk full')):
            with self.assertRaises(OSError):self.studio.feedback({'track_id':'1'*16,'verdict':'keep'})
            with self.assertRaises(OSError):self.studio.comparison_action({'comparison_id':'test'},reveal=True)
        self.assertEqual(self.studio.library,original)

    def test_one_background_worker_and_failures_release_it(self):
        entered=threading.Event();release=threading.Event()
        def develop(*args,**kwargs):entered.set();release.wait(5);raise ValueError('deliberate test failure')
        with patch('flycomposer.experiment.write_song',side_effect=develop):
            self.studio.start(42,'keys');self.assertTrue(entered.wait(2))
            with self.assertRaises(ValueError):self.studio.start(43,'keys')
            release.set()
            for _ in range(100):
                if not self.studio.busy:break
                time.sleep(.01)
            self.assertFalse(self.studio.busy);self.assertEqual(self.studio.error,'deliberate test failure')
        self.assertEqual(len(self.studio.library['tracks']),3)

    def test_http_audio_ranges_and_request_validation(self):
        track=self.root/'library'/('1'*16);track.mkdir();(track/'audio.wav').write_bytes(bytes(range(100)))
        server=make_server(self.studio,0);thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
        url=f'http://127.0.0.1:{server.server_port}'
        try:
            request=urllib.request.Request(url+'/audio/'+('1'*16)+'/audio.wav',headers={'Range':'bytes=10-19'})
            with urllib.request.urlopen(request) as response:
                self.assertEqual(response.status,206);self.assertEqual(response.read(),bytes(range(10,20)))
            for body,origin,status in [({'seed':True},url,400),({'seed':42},'https://unrelated.example',403)]:
                request=urllib.request.Request(url+'/api/compose',data=json.dumps(body).encode(),headers={'Content-Type':'application/json','Origin':origin})
                with self.assertRaises(urllib.error.HTTPError) as error:urllib.request.urlopen(request)
                self.assertEqual(error.exception.code,status)
            with urllib.request.urlopen(url+'/api/state') as response:self.assertEqual(json.load(response)['library'][0]['condition'],'hidden')
        finally:server.shutdown();server.server_close();thread.join()


if __name__=='__main__':unittest.main(verbosity=2)
