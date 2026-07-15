"""Signed research query cache — reject tampered workspace cache entries."""

from __future__ import annotations

import hashlib
import hmac
import json
import os
import secrets


KEY_NAME = "research_cache.key"


def _key_path(root: str) -> str:
    hub = os.path.join(root, "hub")
    os.makedirs(hub, exist_ok=True)
    return os.path.join(hub, KEY_NAME)


def _load_key(root: str) -> bytes:
    path = _key_path(root)
    if os.path.exists(path):
        with open(path, "rb") as fh:
            data = fh.read()
            if len(data) >= 16:
                return data
    key = secrets.token_bytes(32)
    with open(path, "wb") as fh:
        fh.write(key)
    return key


def _canonical(links: list, engine: str, timestamp: float) -> str:
    body = {"engine": engine, "links": links, "timestamp": timestamp}
    return json.dumps(body, sort_keys=True, separators=(",", ":"))


def sign_entry(root: str, links: list[str], engine: str, timestamp: float) -> dict:
    entry = {"engine": engine, "links": links, "timestamp": timestamp}
    sig = hmac.new(
        _load_key(root),
        _canonical(links, engine, timestamp).encode("utf-8"),
        hashlib.sha256,
    ).hexdigest()
    entry["sig"] = sig
    return entry


def verify_entry(root: str, entry: dict) -> bool:
    sig = entry.get("sig")
    if not sig or not isinstance(sig, str):
        return False
    try:
        links = entry["links"]
        engine = entry["engine"]
        timestamp = entry["timestamp"]
    except (KeyError, TypeError):
        return False
    if not isinstance(links, list) or not links:
        return False
    for link in links:
        if not isinstance(link, str) or not link.startswith(("http://", "https://")):
            return False
    if not isinstance(engine, str) or engine.startswith("cache:"):
        return False
    expected = hmac.new(
        _load_key(root),
        _canonical(links, engine, timestamp).encode("utf-8"),
        hashlib.sha256,
    ).hexdigest()
    return hmac.compare_digest(sig, expected)