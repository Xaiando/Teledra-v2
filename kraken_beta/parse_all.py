import json
import os

journal = r'd:\Teledra\kraken_beta\journal\20260714.jsonl'
with open(journal, 'r', encoding='utf-8') as f:
    lines = f.readlines()
    
print("--- CONTROL RUN RESULTS ---")
for line in lines[-10:]:
    try:
        data = json.loads(line)
        if 'ms' in data:
            print(f"Job: {data.get('job')} | Verdict: {data.get('verdict')} | Time: {data.get('ms')/1000:.1f}s")
            if 'reasons' in data:
                print(f"  Reasons: {data.get('reasons')}")
    except Exception as e:
        pass
