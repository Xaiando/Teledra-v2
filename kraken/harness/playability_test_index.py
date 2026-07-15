from pathlib import Path
import re

html = Path(__file__).with_name("index.html").read_text(encoding="utf-8")
low = html.lower()
assert "<canvas" in low
assert "requestanimationframe" in low
assert "gameloop" in low
assert "tabindex" in low
assert re.search(r"canvas\.width\s*=", html) or re.search(
    r"<canvas[^>]+width=", html, re.I
)
ids = re.findall(r'id=["\']([^"\']+)["\']', html, re.I)
assert len(ids) == len(set(ids)), "duplicate HTML ids"
assert not re.search(r"playbtn[\s\S]{0,300}showstartscreen\(", low)
assert "http://" not in low and "https://" not in low
assert len(low) > 6000