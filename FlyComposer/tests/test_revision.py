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
import urllib.error
import urllib.request
import numpy as np

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT))
from flycomposer.composition import assess_score
from flycomposer.development import write_development
from flycomposer.experiment import Studio,atomic_json
from flycomposer.server import make_server


class RevisionIntegrationTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.root=Path(self.tmp.name)
        (self.root/'data').mkdir()
        for name in ('circuit.npz','circuit.json','instruments.json'):
            shutil.copy2(ROOT/'data'/name,self.root/'data'/name)
        self.studio=Studio(self.root)
        self.source_id='a'*16
        self.score=write_development(self.studio.circuit,2026,direction='sparks')
        self.folder=self.root/'library'/self.source_id;self.folder.mkdir(parents=True)
        atomic_json(self.folder/'score.json',self.score)
        atomic_json(self.folder/'audit.json',{'rms':.055})
        (self.folder/'audio.wav').write_bytes(b'Original audio must stay untouched')
        self.source={'id':self.source_id,'title':'Original Crossing Sparks','kind':'development',
                     'direction':'sparks','direction_name':'Crossing Sparks','condition':'connectome',
                     'arrangement_version':'development_v1','ensemble':'luminous','voices':self.score['voices'],
                     'seed':2026,'duration':self.score['duration_seconds'],'verdict':None,'note':'',
                     'comparison_id':'source-session'}
        self.studio.library={'version':1,'tracks':[copy.deepcopy(self.source)],'comparisons':[
            {'id':'source-session','kind':'development','revealed':True,'track_ids':[self.source_id],
             'feedback':{'preferred_track_id':None,'note':'They all have potential. C is a little fragmented.'}}]}
        atomic_json(self.studio.index,self.studio.library)

    def tearDown(self):self.tmp.cleanup()

    @staticmethod
    def rendered(score,progress):
        facts=assess_score(score)
        audit={**facts,'notes_mixed':facts['notes_scheduled'],'duration_seconds':score['duration_seconds'],
               'raw_rms':.1,'raw_peak':.5,'peak':.275,'rms':.055,'pcm_sha256':score['event_sha256']}
        return np.ones((10,2)),{},audit

    def test_targeted_revision_preserves_source_and_concurrent_feedback_without_duplicates(self):
        captured=[]
        def render(score,progress):
            captured.append(score)
            self.studio.feedback({'track_id':self.source_id,'verdict':'clear','note':'Still considering this original'})
            return self.rendered(score,progress)
        original_files={p.name:p.read_bytes() for p in self.folder.iterdir()}
        old_comparison=copy.deepcopy(self.studio.library['comparisons'][0])
        with patch('flycomposer.experiment.render_score',side_effect=render),patch('flycomposer.experiment.save_render',side_effect=lambda d,s,a,st,audit,t:audit) as save:
            batch=self.studio.revise(self.source_id)
        self.assertEqual(len(captured),1)
        revised=captured[0]
        for key in ('seed','voices','condition','direction','bpm','meter'):
            self.assertEqual(revised[key],self.score[key])
        self.assertNotEqual(revised['event_sha256'],self.score['event_sha256'])
        self.assertEqual(save.call_args.args[-1],.055)
        self.assertEqual(revised['revision_context']['source_feedback'],old_comparison['feedback'])
        reloaded=Studio(self.root)
        self.assertEqual(reloaded.library['comparisons'][0],old_comparison)
        self.assertEqual(reloaded.library['tracks'][0]['note'],'Still considering this original')
        self.assertIsNone(reloaded.library['tracks'][0]['verdict'])
        self.assertIsNone(reloaded.library['tracks'][1]['verdict'])
        self.assertEqual(reloaded.library['tracks'][1]['note'],'')
        self.assertEqual({p.name:p.read_bytes() for p in self.folder.iterdir()},original_files)
        self.assertEqual(len(reloaded.state()['library']),2)
        self.assertEqual(batch['track_ids'][0],self.source_id)
        self.assertEqual([t['display_name'] for t in reloaded.state()['comparisons'][0]['tracks']],
                         ['Original Crossing Sparks','Revised Crossing Sparks'])

    def test_failure_invalid_sources_and_headroom_do_not_publish(self):
        before=copy.deepcopy(self.studio.library)
        for track_id,revision in [('missing','connected_phrases_v1'),(self.source_id,'unknown')]:
            with self.assertRaises(ValueError):self.studio.start_revision(track_id,revision)
            self.assertFalse(self.studio.busy)
        with patch('flycomposer.experiment.render_score',side_effect=ValueError('incomplete note')):
            with self.assertRaisesRegex(ValueError,'incomplete note'):self.studio.revise(self.source_id)
        def too_loud(score,progress):
            audio,stems,audit=self.rendered(score,progress);audit['raw_peak']=10
            return audio,stems,audit
        with patch('flycomposer.experiment.render_score',side_effect=too_loud):
            with self.assertRaisesRegex(ValueError,'headroom'):self.studio.revise(self.source_id)
        self.assertFalse(self.studio.busy)
        self.assertEqual(self.studio.library,before)
        self.assertEqual(Studio(self.root).library,before)

    def test_stale_or_edited_source_is_refused_before_audio_rendering(self):
        altered=copy.deepcopy(self.score);altered['events'][0]['midi']+=1
        atomic_json(self.folder/'score.json',altered)
        with patch('flycomposer.experiment.render_score') as render:
            with self.assertRaisesRegex(ValueError,'exact saved original'):self.studio.revise(self.source_id)
            render.assert_not_called()
        self.assertFalse(self.studio.busy)
        self.assertEqual(len(self.studio.library['tracks']),1)

    def test_listener_choice_carries_the_revision_into_future_development(self):
        captured=[]
        def render(score,progress):captured.append(score);return self.rendered(score,progress)
        with patch('flycomposer.experiment.render_score',side_effect=render),patch('flycomposer.experiment.save_render',side_effect=lambda d,s,a,st,audit,t:audit):
            batch=self.studio.revise(self.source_id)
            self.assertEqual(self.studio.state()['preferences']['preferred_revisions'],{})
            self.studio.comparison_action({'comparison_id':batch['id'],'preferred_track_id':batch['track_ids'][1]})
            self.studio.develop(2027)
        future={s['direction']:s for s in captured[1:]}
        self.assertEqual(future['sparks'].get('revision'),'connected_phrases_v1')
        self.assertIsNone(future['nocturne'].get('revision'))
        self.assertIsNone(future['tide'].get('revision'))
        self.assertTrue(all(t['verdict'] is None for t in self.studio.library['tracks']))
        saved=next(t for t in self.studio.library['tracks'] if t['seed']==2027 and t['direction']=='sparks')
        self.assertEqual(saved['revision'],'connected_phrases_v1')
        self.studio.feedback({'track_id':saved['id'],'verdict':'reject'})
        self.assertEqual(self.studio.state()['preferences']['preferred_revisions'],{})

    def test_revision_http_uses_the_shared_worker_and_releases_after_failure(self):
        entered=threading.Event();release=threading.Event()
        def blocked(*args,**kwargs):entered.set();release.wait(5);raise ValueError('deliberate render failure')
        server=make_server(self.studio,0);thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
        url=f'http://127.0.0.1:{server.server_port}/api/revise'
        try:
            request=urllib.request.Request(url,data=b'{"track_id":"unknown"}',headers={'Content-Type':'application/json'})
            with self.assertRaises(urllib.error.HTTPError) as error:urllib.request.urlopen(request)
            self.assertEqual(error.exception.code,400)
            with patch('flycomposer.development.write_development',side_effect=blocked):
                request=urllib.request.Request(url,data=json.dumps({'track_id':self.source_id}).encode(),headers={'Content-Type':'application/json'})
                with urllib.request.urlopen(request) as response:self.assertEqual(response.status,202)
                self.assertTrue(entered.wait(2))
                for operation in (lambda:self.studio.start(42,'keys'),lambda:self.studio.start_development(42),lambda:self.studio.start_revision(self.source_id)):
                    with self.assertRaises(ValueError):operation()
                release.set()
                for _ in range(100):
                    if not self.studio.busy:break
                    time.sleep(.01)
                self.assertFalse(self.studio.busy)
                self.assertEqual(self.studio.error,'deliberate render failure')
        finally:
            release.set();server.shutdown();server.server_close();thread.join()


if __name__=='__main__':unittest.main(verbosity=2)
