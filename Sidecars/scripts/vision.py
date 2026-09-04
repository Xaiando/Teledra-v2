"""Capture a bounded desktop frame and extract optional OCR context."""

import hashlib
import base64
import io
import json
import os
import shutil
import sys
import tempfile
import urllib.error
import urllib.request


def looks_usable(text):
    words = [word for word in text.replace("\n", " ").split() if sum(ch.isalpha() for ch in word) >= 3]
    letters = sum(1 for character in text if character.isalpha() or character.isspace())
    return len(words) >= 5 and letters / max(1, len(text)) >= 0.65


def describe_with_ollama(image):
    if os.environ.get("TELEDRA_DISABLE_VISION_MODEL"):
        return "", "vision model disabled"
    model = os.environ.get("TELEDRA_VISION_MODEL", "moondream").strip()
    endpoint = os.environ.get("TELEDRA_OLLAMA_URL", "http://localhost:11434").rstrip("/")
    prompt = os.environ.get(
        "TELEDRA_VISION_PROMPT",
        "Describe this game or app screen in one or two short factual sentences. Focus on the main action, characters, visible text, score, warnings, and health. Do not guess what is outside the image.",
    )
    buffer = io.BytesIO()
    image.convert("RGB").save(buffer, format="JPEG", quality=70)
    encoded = base64.b64encode(buffer.getvalue()).decode("ascii")
    last_error = "no usable vision response"
    fallback = ""
    for temperature in (0.1, 0.7, 1.0):
        body = json.dumps({
            "model": model,
            "prompt": prompt,
            "images": [encoded],
            "stream": False,
            "options": {"num_predict": 100, "temperature": temperature},
        }).encode("utf-8")
        request = urllib.request.Request(
            endpoint + "/api/generate",
            data=body,
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        try:
            with urllib.request.urlopen(request, timeout=90) as response:
                payload = json.loads(response.read(512 * 1024).decode("utf-8", errors="replace"))
            description = " ".join(str(payload.get("response", "")).split())[:4000]
            if looks_usable(description):
                return description, ""
            if len(description) > len(fallback):
                fallback = description
        except urllib.error.HTTPError as error:
            detail = error.read(1000).decode("utf-8", errors="replace") if error.fp else str(error)
            return "", f"vision model HTTP {error.code}: {detail}"
        except Exception as error:
            last_error = f"vision model unavailable: {error}"
    return (fallback, "") if len(fallback) >= 20 else ("", last_error)


try:
    from PIL import ImageGrab

    image = ImageGrab.grab(all_screens=True)
    image.thumbnail((1280, 720))
    target = os.environ.get(
        "TELEDRA_VISION_OUTPUT",
        os.path.join(tempfile.gettempdir(), "teledra-vision.png"),
    )
    image.save(target)
    with open(target, "rb") as handle:
        digest = hashlib.sha256(handle.read()).hexdigest()
    state = os.path.join(tempfile.gettempdir(), "teledra-v2-vision.sha256")
    previous = ""
    try:
        with open(state, "r", encoding="ascii") as handle:
            previous = handle.read().strip()
    except OSError:
        pass
    changed = digest != previous
    with open(state, "w", encoding="ascii") as handle:
        handle.write(digest)

    ocr = ""
    description = ""
    vision_error = ""
    if changed:
        description, vision_error = describe_with_ollama(image)
        try:
            import pytesseract

            configured = os.environ.get("TELEDRA_TESSERACT")
            discovered = shutil.which("tesseract")
            default_windows = r"C:\Program Files\Tesseract-OCR\tesseract.exe"
            if configured:
                pytesseract.pytesseract.tesseract_cmd = configured
            elif discovered:
                pytesseract.pytesseract.tesseract_cmd = discovered
            elif os.path.isfile(default_windows):
                pytesseract.pytesseract.tesseract_cmd = default_windows
            ocr = " ".join(pytesseract.image_to_string(image).split())[:6000]
        except Exception:
            pass
    response = {
        "ok": True,
        "capability": "vision",
        "message": f"Desktop captured at {target}",
        "data": {
            "path": target,
            "width": image.width,
            "height": image.height,
            "changed": changed,
            "ocr": ocr,
            "description": description,
            "vision_error": vision_error,
        },
    }
except Exception as error:
    response = {
        "ok": False,
        "capability": "vision",
        "message": str(error),
        "data": {},
    }

print(json.dumps(response, ensure_ascii=False))
sys.exit(0 if response["ok"] else 2)
