from pathlib import Path
import re, sys

html = Path(r'D:\Teledra\kraken\workspace\games\duchmans_mine\index.html').read_text(encoding='utf-8')
low = html.lower()

ids = re.findall(r"id=[\"']([^\"']+)[\"']", html, re.I)
checks = [
    ('doctype+canvas', '<!doctype html' in low and '<canvas' in low),
    ('requestAnimationFrame', 'requestanimationframe' in low),
    ('gameloop', 'gameloop' in low),
    ('tabindex', 'tabindex' in low),
    ('canvas.width', bool(re.search(r'canvas\.width\s*=', html) or re.search(r'<canvas[^>]+width=', html, re.I))),
    ('no-dup-ids', len(ids) == len(set(ids))),
    ('no-showstartscreen', not re.search(r'playbtn[\s\S]{0,300}showstartscreen\(', low)),
    ('no-https', 'http://' not in low and 'https://' not in low),
    ('size>20000', len(low) > 20000),
    ('beast', '__kraken_beast__' in low),
    ('gameplay-terms', any(k in low for k in ['panning','mining','bounty','hunger','energy','gold','map','poker','fish'])),
    ('audio', 'audiocontext' in low or 'playmusic' in low or 'webaudio' in low),
    ('survival-terms', any(k in low for k in ['eat','sleep','fatigue','inventory','shop'])),
]
print(f"File size: {len(html):,} bytes ({len(html)//1024}KB)")
print()
all_pass = True
for name, result in checks:
    mark = 'PASS' if result else 'FAIL'
    if not result:
        all_pass = False
    print(f"  [{mark}] {name}")

if not all_pass:
    print("\nDuplicate IDs:", [i for i in ids if ids.count(i) > 1][:10])
    sys.exit(1)
else:
    print("\nAll checks PASS")
