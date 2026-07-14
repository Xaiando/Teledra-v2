import json
import os

journal = r'd:\Teledra\kraken_beta\journal\20260714.jsonl'
if not os.path.exists(journal):
    print('Journal not found')
else:
    with open(journal, 'r', encoding='utf-8') as f:
        lines = f.readlines()
        
    for line in lines[-2:]:
        try:
            data = json.loads(line)
            print(json.dumps(data, indent=2))
        except Exception as e:
            print(f"Error parsing line: {e}")
