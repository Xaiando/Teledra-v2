"""Transparent adaptation from explicit listener feedback, not neural training.

Each comparison contributes at most one mean verdict per actual voice palette.
Referencing an existing piece in a revision does not repeat its existing vote.
Missing ratings are missing evidence. Free text stays source data, not commands.
"""
from collections import defaultdict
import hashlib
import json

from .composition import ENSEMBLES, ROLES


def palette_key(voices):
    return hashlib.sha256(json.dumps(voices,sort_keys=True).encode()).hexdigest()


def summarize_preferences(library):
    palettes={};evidence=[];direction_votes=defaultdict(list);revision_votes=defaultdict(list)
    tracks={t['id']:t for t in library['tracks']}
    seen_palette_tracks=set();seen_direction_tracks=set()
    seen_revision_tracks=set()
    for batch in library.get('comparisons',[]):
        grouped=defaultdict(list)
        for track_id in batch['track_ids']:
            if track_id in seen_palette_tracks:continue
            seen_palette_tracks.add(track_id)
            track=tracks.get(track_id)
            if not track or track.get('verdict') not in ('keep','reject'):continue
            voices=track.get('voices',ENSEMBLES.get(track.get('ensemble'),{}))
            if set(voices)!=set(ROLES):continue
            key=palette_key(voices)
            preset=next((name for name,mapping in ENSEMBLES.items() if mapping==voices),None)
            palettes.setdefault(key,{'voices':dict(voices),'preset':preset,
                                     'ensemble':preset or (track.get('ensemble') if track.get('ensemble') in ENSEMBLES else 'luminous'),
                                     'label':preset.title() if preset else 'Custom palette','votes':[],
                                     'supporting_batches':[],'kept_track_ids':[]})
            grouped[key].append(track)
            if track['verdict']=='keep':palettes[key]['kept_track_ids'].append(track_id)
        for key,items in grouped.items():
            vote=sum(1 if t['verdict']=='keep' else -1 for t in items)/len(items)
            palettes[key]['votes'].append(vote);palettes[key]['supporting_batches'].append(batch['id'])
            evidence.append({'comparison_id':batch['id'],'palette_id':key,'rated_tracks':len(items),
                             'keep':sum(t['verdict']=='keep' for t in items),
                             'reject':sum(t['verdict']=='reject' for t in items),'batch_vote':vote})
        if batch.get('kind') in ('development','revision'):
            preferred=batch.get('feedback',{}).get('preferred_track_id')
            # A choice and Keep for the same study are one signal in this batch.
            by_direction=defaultdict(list)
            for track_id in batch['track_ids']:
                if track_id in seen_direction_tracks:continue
                track=tracks.get(track_id,{})
                if not track.get('direction'):continue
                verdict=track.get('verdict')
                if verdict in ('keep','reject'):value=1 if verdict=='keep' else -1
                elif track_id==preferred:value=1
                else:continue
                seen_direction_tracks.add(track_id)
                by_direction[track['direction']].append(value)
            for direction,values in by_direction.items():direction_votes[direction].append(sum(values)/len(values))
        if batch.get('kind') in ('revision','development'):
            preferred=batch.get('feedback',{}).get('preferred_track_id')
            for track_id in batch['track_ids']:
                track=tracks.get(track_id,{})
                if not track.get('revision') or track_id in seen_revision_tracks:continue
                if preferred==track_id:value=1
                elif batch.get('kind')=='revision' and preferred in batch['track_ids']:value=-1
                elif track.get('verdict') in ('keep','reject'):value=1 if track['verdict']=='keep' else -1
                else:continue
                seen_revision_tracks.add(track_id)
                revision_votes[(track['direction'],track['revision'])].append(value)
    summaries=[]
    for key,palette in palettes.items():
        summaries.append({**palette,'id':key,'rated_batches':len(palette['votes']),
                          'mean_verdict':sum(palette['votes'])/len(palette['votes'])})
    positive=[p for p in summaries if p['mean_verdict']>0 and p['kept_track_ids']]
    winner=max(positive,key=lambda p:(p['mean_verdict'],p['rated_batches'],p['preset']=='luminous',p['id'])) if positive else None
    fallback={'ensemble':'luminous','label':'Luminous','voices':dict(ENSEMBLES['luminous']),
              'kept_track_ids':[],'supporting_batches':[],'rated_batches':0}
    chosen=winner or fallback
    decisions=[{'track_id':t['id'],'verdict':t.get('verdict'),'voices':t.get('voices'),
                'note':t.get('note','')} for t in library['tracks'] if t.get('verdict') in ('keep','reject') or t.get('note')]
    selections=[{'comparison_id':c['id'],'feedback':c['feedback']} for c in library.get('comparisons',[]) if c.get('feedback')]
    revision_evidence=[{'direction':direction,'id':revision,'mean_verdict':sum(values)/len(values),'rated_batches':len(values)}
                       for (direction,revision),values in sorted(revision_votes.items())]
    preferred_revisions={}
    for result in sorted(revision_evidence,key=lambda r:(r['mean_verdict'],r['rated_batches'],r['id'])):
        if result['mean_verdict']>0:preferred_revisions[result['direction']]=result['id']
    fingerprint=hashlib.sha256(json.dumps({'palettes':evidence,'directions':dict(direction_votes),
        'decisions':decisions,'selections':selections,'revisions':revision_evidence},sort_keys=True).encode()).hexdigest()
    return {'version':2,'policy':'explicit_feedback_unique_track_batch_means_v2','evidence_sha256':fingerprint,
            'preferred_ensemble':chosen['ensemble'],'preferred_label':chosen['label'],
            'preferred_voices':chosen['voices'],'parent_track_ids':chosen['kept_track_ids'],
            'supporting_batch_ids':chosen['supporting_batches'],
            'supporting_batches':chosen['rated_batches'],'rated_batches':len({e['comparison_id'] for e in evidence}),
            'basis':'saved listener verdicts' if winner else 'Luminous starting palette; no positive saved palette verdict',
            'palettes':summaries,'evidence':evidence,
            'preferred_revisions':preferred_revisions,'revision_evidence':revision_evidence,
            'direction_ranking':[{'id':key,'mean_verdict':sum(values)/len(values),'rated_batches':len(values)}
                                 for key,values in sorted(direction_votes.items(),key=lambda item:(-sum(item[1])/len(item[1]),item[0]))],
            'scope':'Palette, development-order and explicitly preferred revision adaptation. No trained musical critic or inferred wiring winner; notes and unrated pieces remain unchanged.'}
