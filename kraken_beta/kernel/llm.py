"""Kraken LLM access — thin Ollama client, stdlib only.

All Kraken model calls go through here so timeouts, retries, and journaling
stay in one place. No streaming; Kraken is a silent batch worker.
"""

from __future__ import annotations

import json
import urllib.error
import urllib.request

OLLAMA = "http://localhost:11434"

QWEN = "qwen2.5:7b"
ORNITH = "hf.co/deepreinforce-ai/Ornith-1.0-9B-GGUF:Q4_K_M"
MOONDREAM = "moondream"

DEFAULT_TIMEOUT = 300  # raised; code_forge overrides to 600 for rich HTML games (qwen2.5 primary + Ornith coupled) on full-file polish


class LLMError(RuntimeError):
    pass


def _post(path: str, payload: dict, timeout: int) -> dict:
    body = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(
        OLLAMA + path, data=body, headers={"Content-Type": "application/json"}
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return json.loads(resp.read().decode("utf-8"))
    except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as exc:
        raise LLMError(f"ollama {path} failed: {exc}") from exc


def generate(prompt: str, model: str = QWEN, system: str | None = None,
             timeout: int = DEFAULT_TIMEOUT, retries: int = 1,
             options: dict | None = None) -> str:
    """One-shot completion. Returns the response text, raises LLMError."""
    # Ollama's default num_ctx (4096) silently truncates multi-source prompts;
    # 16k fits comfortably in 16GB VRAM alongside a 7B/9B Q4 model.
    merged = {"num_ctx": 16384}
    merged.update(options or {})
    payload = {"model": model, "prompt": prompt, "stream": False,
               "options": merged}
    if system:
        payload["system"] = system
    last: Exception | None = None
    for _ in range(retries + 1):
        try:
            return _post("/api/generate", payload, timeout).get("response", "")
        except LLMError as exc:
            last = exc
    raise LLMError(f"generate failed after {retries + 1} tries: {last}")


def generate_json(prompt: str, model: str = QWEN, system: str | None = None,
                  timeout: int = DEFAULT_TIMEOUT) -> dict | list:
    """Completion that must parse as JSON; one repair attempt via re-prompt."""
    text = generate(prompt, model=model, system=system, timeout=timeout)
    try:
        return _extract_json(text)
    except ValueError:
        repair = generate(
            "Convert this to strictly valid JSON, no commentary:\n" + text[:4000],
            model=model, timeout=timeout,
        )
        return _extract_json(repair)  # raises ValueError to the caller if hopeless


def _extract_json(text: str):
    text = text.strip()
    if text.startswith("```"):
        text = text.strip("`")
        if text.startswith("json"):
            text = text[4:]
    start = min((i for i in (text.find("{"), text.find("[")) if i >= 0), default=-1)
    if start < 0:
        raise ValueError("no JSON found")
    decoder = json.JSONDecoder()
    obj, _ = decoder.raw_decode(text[start:])
    return obj


def available() -> bool:
    try:
        req = urllib.request.Request(OLLAMA + "/api/tags")
        with urllib.request.urlopen(req, timeout=5):
            return True
    except Exception:
        return False


def ensure_models():
    """Ensure required models (qwen2.5 primary + Ornith) are present in Ollama.
    Pulls if missing. Idempotent. Addresses possible regression to 'pure ollama'
    without qwen2.5:7b (as suspected in antigravity/Gemini changes).
    Call early in kraken startup and code_forge.
    """
    try:
        tags_resp = _post("/api/tags", {}, timeout=10)
        names = [m.get("name", "") for m in tags_resp.get("models", [])]
        for mname in (QWEN, ORNITH):
            if mname not in names:
                # Pull (can take time; long timeout)
                _post("/api/pull", {"name": mname, "stream": False}, timeout=600)
    except Exception:
        # Non-fatal; generate will fail later if truly missing
        pass
