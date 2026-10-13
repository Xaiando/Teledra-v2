import copy
import json
from pathlib import Path
import shutil
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch
import numpy as np

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT))
from flycomposer.composition import ENSEMBLES,assess_score
from flycomposer.experiment import Studio,atomic_json


class DevelopmentIntegrationTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.root=Path(self.tmp.name)
        (self.root/'data').mkdir()
        for name in ('circuit.npz','circuit.json','instruments.json'):shutil.copy2(ROOT/'data'/name,self.root/'data'/name)
        self.studio=Studio(self.root)
        self.originals=[{'id':str(i)*16,'title':f'Kept {i}','condition':condition,'duration':88,
                         'ensemble':'luminous','voices':dict(ENSEMBLES['luminous']),'verdict':'keep','note':'Original opinion'}
                        for i,condition in zip((1,2,3),('connectome','shuffled','baseline'))]
        self.studio.library={'version':1,'tracks':copy.deepcopy(self.originals),
                             'comparisons':[{'id':'parent','track_ids':[t['id'] for t in self.originals],'revealed':False,'feedback':{}}]}
        atomic_json(self.studio.index,self.studio.library)

    def tearDown(self):self.tmp.cleanup()

    @staticmethod
    def rendered(score,progress=lambda value:None):
        facts=assess_score(score)
        audit={**facts,'notes_mixed':facts['notes_scheduled'],'duration_seconds':score['duration_seconds'],
               'raw_rms':.1,'raw_peak':.5,'peak':.2,'rms':.055,'pcm_sha256':score['event_sha256']}
        return np.ones((10,2)),{},audit

    def test_publishes_new_studies_and_keeps_concurrent_feedback(self):
        calls=[]
        def render(score,progress):
            calls.append(score)
            if len(calls)==1:self.studio.feedback({'track_id':'1'*16,'verdict':'keep','note':'Updated while composing'})
            return self.rendered(score,progress)
        with patch('flycomposer.experiment.render_score',side_effect=render),patch('flycomposer.experiment.save_render',side_effect=lambda d,s,a,st,audit,t:audit):
            batch=self.studio.develop(2026)
        self.assertEqual(len(calls),3)
        self.assertEqual({s['condition'] for s in calls},{'connectome'})
        self.assertEqual({s['direction'] for s in calls},{'nocturne','tide','sparks'})
        self.assertTrue(all(s['voices']==ENSEMBLES['luminous'] for s in calls))
        self.assertEqual(batch['kind'],'development');self.assertTrue(batch['revealed'])
        reloaded=Studio(self.root)
        self.assertEqual(len(reloaded.library['tracks']),6)
        self.assertEqual(reloaded.library['tracks'][0]['note'],'Updated while composing')
        for track in reloaded.library['tracks'][3:]:
            self.assertEqual(track['verdict'],None)
            self.assertEqual(set(track['parent_track_ids']),{t['id'] for t in self.originals})
        self.assertEqual(reloaded.library['tracks'][1:3],self.originals[1:3])
        state=reloaded.state()
        self.assertEqual(state['comparisons'][0]['kind'],'development')
        self.assertTrue(all(t['direction_name'] for t in state['comparisons'][0]['tracks']))

    def test_failed_development_does_not_publish_or_change_opinions(self):
        original=copy.deepcopy(self.studio.library)
        with patch('flycomposer.experiment.render_score',side_effect=ValueError('incomplete note')):
            with self.assertRaisesRegex(ValueError,'incomplete note'):self.studio.develop(2026)
        self.assertFalse(self.studio.busy)
        self.assertEqual(self.studio.library,original)
        self.assertEqual(Studio(self.root).library,original)

    def test_shared_worker_gate_between_wiring_and_development(self):
        entered=threading.Event();release=threading.Event()
        def develop(*args,**kwargs):entered.set();release.wait(5);raise ValueError('deliberate failure')
        with patch('flycomposer.development.write_development',side_effect=develop):
            self.studio.start_development(2026)
            self.assertTrue(entered.wait(2))
            with self.assertRaises(ValueError):self.studio.start(42,'keys')
            with self.assertRaises(ValueError):self.studio.start_development(77)
            release.set()
            for _ in range(100):
                if not self.studio.busy:break
                time.sleep(.01)
            self.assertFalse(self.studio.busy)
        with self.assertRaises(ValueError):self.studio.start_development(True)
        self.assertFalse(self.studio.busy)


if __name__=='__main__':unittest.main(verbosity=2)
