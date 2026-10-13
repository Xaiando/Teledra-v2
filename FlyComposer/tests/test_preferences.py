import copy
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT))
from flycomposer.composition import ENSEMBLES
from flycomposer.preferences import summarize_preferences


def batch(library, batch_id, ensemble, verdicts, voices=None):
    ids=[]
    for i,verdict in enumerate(verdicts):
        track_id=f'{batch_id}-{i}';ids.append(track_id)
        library['tracks'].append({'id':track_id,'ensemble':ensemble,'voices':dict(voices or ENSEMBLES[ensemble]),'verdict':verdict,'note':'Keep this source text unchanged'})
    library['comparisons'].append({'id':batch_id,'track_ids':ids,'feedback':{}})
    return ids


class PreferenceTests(unittest.TestCase):
    def test_correlated_samples_count_as_one_session_and_do_not_mutate_evidence(self):
        library={'tracks':[],'comparisons':[]}
        batch(library,'keys','keys',['reject']*3)
        liked=batch(library,'luminous','luminous',['keep']*3)
        batch(library,'unknown','chamber',[None]*3)
        original=copy.deepcopy(library)
        result=summarize_preferences(library)
        self.assertEqual(library,original)
        self.assertEqual(result['preferred_ensemble'],'luminous')
        self.assertEqual(result['supporting_batches'],1)
        self.assertEqual(result['rated_batches'],2)
        self.assertEqual(result['parent_track_ids'],liked)
        self.assertNotIn('preferred_controller',result)

    def test_custom_instruments_are_not_counted_as_preset(self):
        library={'tracks':[],'comparisons':[]}
        custom={**ENSEMBLES['keys'],'lead':'marimba','harmony':'cello'}
        batch(library,'keys','keys',['reject']*3)
        batch(library,'custom','keys',['keep']*3,custom)
        result=summarize_preferences(library)
        self.assertEqual(result['preferred_label'],'Custom palette')
        self.assertEqual(result['preferred_voices'],custom)
        self.assertEqual(len(result['palettes']),2)

    def test_kept_independent_palette_keeps_a_valid_legacy_ensemble(self):
        library={'tracks':[],'comparisons':[]}
        custom={**ENSEMBLES['luminous'],'lead':'clavinet'}
        batch(library,'performance','independent',['keep'],custom)
        result=summarize_preferences(library)
        self.assertEqual(result['preferred_voices'],custom)
        self.assertIn(result['preferred_ensemble'],ENSEMBLES)

    def test_clear_and_conflicting_votes_are_recomputed(self):
        library={'tracks':[],'comparisons':[]}
        batch(library,'a','chamber',['keep',None,None])
        result=summarize_preferences(library)
        self.assertEqual(result['preferred_ensemble'],'chamber')
        library['tracks'][0]['verdict']=None
        cleared=summarize_preferences(library)
        self.assertEqual(cleared['supporting_batches'],0)
        self.assertEqual(cleared['rated_batches'],0)
        self.assertNotEqual(result['evidence_sha256'],cleared['evidence_sha256'])
        library['tracks'][0]['verdict']='keep';library['tracks'][1]['verdict']='reject'
        self.assertEqual(summarize_preferences(library)['supporting_batches'],0)

    def test_development_preference_guides_order_without_inventing_other_rejections(self):
        library={'tracks':[],'comparisons':[]}
        ids=batch(library,'development','luminous',[None]*3)
        for track,direction in zip(library['tracks'],('nocturne','tide','sparks')):track['direction']=direction
        library['comparisons'][0].update(kind='development',feedback={'preferred_track_id':ids[1]})
        result=summarize_preferences(library)
        self.assertEqual(result['direction_ranking'],[{'id':'tide','mean_verdict':1,'rated_batches':1}])
        self.assertEqual(result['rated_batches'],0)
        library['tracks'][1]['verdict']='keep'
        self.assertEqual(summarize_preferences(library)['direction_ranking'][0]['rated_batches'],1)

    def test_feedback_hash_identifies_individual_parents_not_only_counts(self):
        library={'tracks':[],'comparisons':[]}
        batch(library,'a','luminous',['keep',None,None])
        first=summarize_preferences(library)
        library['tracks'][0]['verdict']=None;library['tracks'][1]['verdict']='keep'
        second=summarize_preferences(library)
        self.assertEqual(first['evidence'],second['evidence'])
        self.assertNotEqual(first['parent_track_ids'],second['parent_track_ids'])
        self.assertNotEqual(first['evidence_sha256'],second['evidence_sha256'])

    def test_referencing_a_parent_does_not_duplicate_votes_or_infer_new_verdicts(self):
        library={'tracks':[],'comparisons':[]}
        ids=batch(library,'development','luminous',['keep',None,None])
        for track in library['tracks']:track['direction']='sparks'
        library['comparisons'][0]['kind']='development'
        before=summarize_preferences(library)
        library['tracks'].append({**library['tracks'][0],'id':'revision-track','verdict':None,'note':''})
        library['comparisons'].append({'id':'revision','kind':'revision','track_ids':[ids[0],'revision-track'],
                                       'feedback':{'note':'They both have potential','preferred_track_id':None}})
        after=summarize_preferences(library)
        self.assertEqual(after['parent_track_ids'],before['parent_track_ids'])
        self.assertEqual(after['evidence'],before['evidence'])
        self.assertEqual(after['direction_ranking'],before['direction_ranking'])
        library['comparisons'][-1]['feedback']['preferred_track_id']='revision-track'
        self.assertEqual(summarize_preferences(library)['direction_ranking'][0]['rated_batches'],2)
        self.assertEqual(library['tracks'][-1]['verdict'],None)

    def test_revision_adoption_requires_explicit_choice_or_keep_and_is_reversible(self):
        library={'tracks':[],'comparisons':[]}
        ids=batch(library,'development','luminous',[None]*3)
        source=library['tracks'][2];source['direction']='sparks'
        revised={**source,'id':'connected','revision':'connected_phrases_v1'}
        library['tracks'].append(revised)
        revision={'id':'revision','kind':'revision','track_ids':[ids[2],'connected'],
                  'feedback':{'note':'They all have potential'}}
        library['comparisons'].append(revision)
        self.assertEqual(summarize_preferences(library)['preferred_revisions'],{})
        revision['feedback']['preferred_track_id']='connected'
        self.assertEqual(summarize_preferences(library)['preferred_revisions'],{'sparks':'connected_phrases_v1'})
        self.assertIsNone(revised['verdict'])
        revision['feedback']['preferred_track_id']=ids[2]
        self.assertEqual(summarize_preferences(library)['preferred_revisions'],{})
        revision['feedback']['preferred_track_id']=None;revised['verdict']='keep'
        self.assertEqual(summarize_preferences(library)['preferred_revisions'],{'sparks':'connected_phrases_v1'})
        revised['verdict']=None
        self.assertEqual(summarize_preferences(library)['preferred_revisions'],{})

    def test_later_generated_revision_feedback_can_withdraw_an_adopted_rule(self):
        library={'tracks':[],'comparisons':[]}
        ids=batch(library,'source','luminous',[None])
        original=library['tracks'][0];original['direction']='sparks'
        library['tracks'].append({**original,'id':'revision','revision':'connected_phrases_v1'})
        library['comparisons'].append({'id':'pair','kind':'revision','track_ids':[ids[0],'revision'],
                                       'feedback':{'preferred_track_id':'revision'}})
        future_ids=batch(library,'future','luminous',[None]*3)
        library['tracks'][-1].update(direction='sparks',revision='connected_phrases_v1')
        library['comparisons'][-1].update(kind='development',feedback={'preferred_track_id':future_ids[0]})
        # Choosing a different direction does not reject the unchosen study.
        self.assertEqual(summarize_preferences(library)['preferred_revisions'],{'sparks':'connected_phrases_v1'})
        library['tracks'][-1]['verdict']='reject'
        self.assertEqual(summarize_preferences(library)['preferred_revisions'],{})
        library['tracks'][-1]['verdict']=None
        self.assertEqual(summarize_preferences(library)['preferred_revisions'],{'sparks':'connected_phrases_v1'})


if __name__=='__main__':unittest.main(verbosity=2)
