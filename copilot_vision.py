"""Co-pilot vision sensor: grab the screen and ask the local moondream vision
model (via Ollama) for a short, stream-commentable description of what's on it.

Usage:
    python copilot_vision.py            # full primary screen
    python copilot_vision.py "custom prompt"

Prints a single JSON line: {"ok": bool, "description": str, "error": str}.
The screen is downscaled before sending so the request stays fast and small.
"""

from __future__ import annotations

import base64
import io
import json
import re
import sys
import urllib.error
import urllib.request

OLLAMA_URL = "http://localhost:11434/api/generate"
VISION_MODEL = "moondream"
MAX_WIDTH = 1024
DEFAULT_PROMPT = (
    "Describe what is happening on this screen in one or two short sentences. "
    "Focus on the game or app, the main action or characters, and any clearly "
    "visible text, score, or health."
)


def grab_screen_b64() -> str:
    from PIL import ImageGrab

    img = ImageGrab.grab()
    if img.mode != "RGB":
        img = img.convert("RGB")
    if img.width > MAX_WIDTH:
        ratio = MAX_WIDTH / float(img.width)
        img = img.resize((MAX_WIDTH, max(1, int(img.height * ratio))))
    buf = io.BytesIO()
    img.save(buf, format="JPEG", quality=70)
    return base64.b64encode(buf.getvalue()).decode("ascii")


def _ask_ollama(prompt: str, image_b64: str, temperature: float) -> str:
    body = json.dumps(
        {
            "model": VISION_MODEL,
            "prompt": prompt,
            "images": [image_b64],
            "stream": False,
            "options": {"num_predict": 90, "temperature": temperature},
        }
    ).encode("utf-8")
    req = urllib.request.Request(
        OLLAMA_URL, data=body, headers={"Content-Type": "application/json"}, method="POST"
    )
    with urllib.request.urlopen(req, timeout=120) as resp:
        data = json.loads(resp.read().decode("utf-8", errors="replace"))
    return (data.get("response") or "").strip().replace("\n", " ")


def describe(prompt: str) -> dict:
    try:
        image_b64 = grab_screen_b64()
    except Exception as exc:
        return {"ok": False, "description": "", "error": f"screen grab failed: {exc}"}

    last_err = "no usable vision response"
    fallback = ""
    # moondream often returns an empty or garbled reply, and the failure is
    # deterministic at low temperature, so escalate temperature across retries
    # and keep the first response that reads like a real sentence.
    for temperature in (0.0, 0.5, 0.8, 1.0, 1.1):
        try:
            desc = _ask_ollama(prompt, image_b64, temperature)
        except urllib.error.HTTPError as exc:
            detail = exc.read().decode("utf-8", errors="replace") if exc.fp else str(exc)
            return {"ok": False, "description": "", "error": f"ollama HTTP {exc.code}: {detail[:200]}"}
        except Exception as exc:
            last_err = f"ollama request failed: {exc}"
            continue
        if _looks_usable(desc):
            return {"ok": True, "description": desc, "error": ""}
        if len(desc) > len(fallback):
            fallback = desc
    if len(fallback) >= 20:
        return {"ok": True, "description": fallback, "error": ""}
    return {"ok": False, "description": "", "error": last_err}


def _looks_usable(desc: str) -> bool:
    """Reject moondream's garbage/garbled replies (token soup, watermark reads)."""
    words = [w for w in re.findall(r"[A-Za-z]+", desc) if len(w) >= 3]
    if len(words) < 5:
        return False
    letters = sum(1 for c in desc if c.isalpha() or c.isspace())
    if letters / max(1, len(desc)) < 0.7:
        return False
    return True


def main() -> int:
    prompt = sys.argv[1].strip() if len(sys.argv) > 1 and sys.argv[1].strip() else DEFAULT_PROMPT
    print(json.dumps(describe(prompt), ensure_ascii=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
