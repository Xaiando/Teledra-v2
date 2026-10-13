"""Run the configured local Organist and expand only its authored sequencer plan.

There is no musical recipe, automatic accompaniment, generated pitch correction,
or executable model code in this adapter. Rejected authoring receives at most one
factual repair request. The original profile, requests and streamed replies are
retained independently of whether a recording can be published.
"""
from __future__ import annotations

import hashlib
import ipaddress
import json
import math
import os
from pathlib import Path
import threading
import time
from urllib.parse import urlsplit
from urllib.request import ProxyHandler, Request, build_opener
from uuid import uuid4


ROLES = ("lead", "answer", "harmony", "bass", "texture")
CLASSIC = ("sine", "triangle", "pluck", "bell", "pad", "keys", "bass")
MAX_EVENTS = 6000
MAX_CLIPS = 12
MAX_CLIP_NOTES = 16
MAX_PLACEMENTS = 12
MAX_OUTPUT_TOKENS = 8192
MAX_RESPONSE_BYTES = 2_000_000
MAX_STREAM_BYTES = 8_000_000
MAX_INFERENCE_SECONDS = 600


class OrganistError(RuntimeError):
    """A failed recorded session with inspectable on-disk inference evidence."""
    def __init__(self, message, output_dir):
        self.output_dir = Path(output_dir)
        super().__init__(f"{message}. Evidence: {self.output_dir}")


def _number(value, name, low, high):
    if type(value) not in (int, float) or not math.isfinite(value) or not low <= value <= high:
        raise ValueError(f"{name} must be a finite number in {low:g}..{high:g}")
    return value


def _integer(value, name, low, high):
    if type(value) is not int or not low <= value <= high:
        raise ValueError(f"{name} must be an integer in {low}..{high}")
    return value


def _keys(value, expected, name):
    if not isinstance(value, dict) or set(value) != set(expected):
        raise ValueError(f"{name} must contain exactly: {', '.join(expected)}")


def _list(value, name, low, high):
    if not isinstance(value, list) or not low <= len(value) <= high:
        raise ValueError(f"{name} must be an array with {low}..{high} entries")
    return value


def _json(text):
    def reject_constant(value):
        raise ValueError(f"JSON contains nonfinite number {value}")
    def reject_duplicates(items):
        result = {}
        for key, value in items:
            if key in result:
                raise ValueError(f"JSON contains duplicate key {key}")
            result[key] = value
        return result
    return json.loads(text, parse_constant=reject_constant, object_pairs_hook=reject_duplicates)


def _sha(value):
    return hashlib.sha256(value).hexdigest()


def _write(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2, allow_nan=False), encoding="utf-8")


def _catalog(root):
    catalog = _json((root / "data/instruments.json").read_text(encoding="utf-8-sig"))
    if not isinstance(catalog, list) or not catalog:
        raise ValueError("The packaged instrument catalog is missing or invalid")
    result = []
    ids = set()
    for voice in catalog:
        if not isinstance(voice, dict) or not isinstance(voice.get("id"), str) or voice["id"] in ids:
            raise ValueError("The instrument catalog contains invalid or duplicate IDs")
        ids.add(voice["id"])
        result.append({key: voice[key] for key in ("id", "name", "range", "attack_spacing", "monophonic") if key in voice})
    for name in CLASSIC:
        result.append({"id": f"classic:{name}", "name": f"Classic {name}", "range": [0, 127]})
    return result


def _schema(allowed):
    number = {"type": "number"}
    return {
        "type": "object", "additionalProperties": False,
        "required": ["title", "bpm", "voices", "clips"],
        "properties": {
            "title": {"type": "string", "minLength": 1, "maxLength": 160},
            "bpm": {"type": "number", "minimum": 40, "maximum": 220},
            "voices": {"type": "object", "minProperties": 1, "maxProperties": 5,
                       "additionalProperties": False,
                       "properties": {role: {"type": "string", "enum": allowed} for role in ROLES}},
            "clips": {"type": "array", "minItems": 1, "maxItems": MAX_CLIPS, "items": {
                "type": "object", "additionalProperties": False,
                "required": ["part", "events", "placements"],
                "properties": {
                    "part": {"type": "string", "enum": list(ROLES)},
                    "events": {"type": "array", "minItems": 1, "maxItems": MAX_CLIP_NOTES,
                               "items": {"type": "array", "minItems": 4, "maxItems": 4, "items": number}},
                    "placements": {"type": "array", "minItems": 1, "maxItems": MAX_PLACEMENTS,
                                   "items": {"type": "array", "minItems": 2, "maxItems": 2, "items": number}},
                },
            }},
        },
    }


def expand_authoring(authoring, allowed, seed=2026, seconds=75):
    """Validate and expand exact authored notes; never repair or embellish them."""
    _integer(seed, "seed", 0, 2**31 - 1)
    _number(seconds, "recording seconds", 20, 180)
    _keys(authoring, ("title", "bpm", "voices", "clips"), "performance")
    title = authoring["title"]
    if not isinstance(title, str) or not title.strip() or len(title) > 160 or any(ord(c) < 32 for c in title):
        raise ValueError("title must be 1..160 printable characters")
    bpm = _number(authoring["bpm"], "bpm", 40, 220)
    beat_seconds = 60 / bpm
    musical_seconds = seconds - 1
    max_beat = musical_seconds / beat_seconds
    voices = authoring["voices"]
    if not isinstance(voices, dict) or not 1 <= len(voices) <= len(ROLES) or not set(voices) <= set(ROLES):
        raise ValueError("voices must assign 1..5 of lead, answer, harmony, bass, texture")
    if any(not isinstance(voice, str) or voice not in allowed for voice in voices.values()):
        raise ValueError("Every voice must be an exact supplied catalog ID")
    events = []
    for ci, clip in enumerate(_list(authoring["clips"], "clips", 1, MAX_CLIPS)):
        _keys(clip, ("part", "events", "placements"), f"clip {ci}")
        part = clip["part"]
        if not isinstance(part, str) or part not in voices:
            raise ValueError(f"clip {ci} part must have an instrument in voices")
        local = []
        for ni, raw in enumerate(_list(clip["events"], f"clip {ci} events", 1, MAX_CLIP_NOTES)):
            _list(raw, f"clip {ci} event {ni}", 4, 4)
            pitch = _integer(raw[0], f"clip {ci} event {ni} MIDI", 0, 127)
            beat = _number(raw[1], f"clip {ci} event {ni} relative beat", 0, max_beat)
            length = _number(raw[2], f"clip {ci} event {ni} length", .025 / beat_seconds, max_beat)
            velocity = _number(raw[3], f"clip {ci} event {ni} velocity", .001, 1)
            if not voices[part].startswith("classic:") and length * beat_seconds > 30:
                raise ValueError(f"clip {ci} event {ni} exceeds the native instrument's 30-second note limit")
            local.append((pitch, beat, length, velocity))
        for pi, raw in enumerate(_list(clip["placements"], f"clip {ci} placements", 1, MAX_PLACEMENTS)):
            _list(raw, f"clip {ci} placement {pi}", 2, 2)
            start = _number(raw[0], f"clip {ci} placement {pi} start beat", 0, max_beat)
            transpose = _integer(raw[1], f"clip {ci} placement {pi} transpose", -127, 127)
            if len(events) + len(local) > MAX_EVENTS:
                raise ValueError(f"Expanded recording exceeds {MAX_EVENTS} notes")
            for ni, (pitch, beat, length, velocity) in enumerate(local):
                final_pitch = _integer(pitch + transpose, f"clip {ci} placement {pi} transposed MIDI", 0, 127)
                onset = start + beat
                if (onset + length) * beat_seconds > musical_seconds + 1e-8:
                    raise ValueError(f"clip {ci} placement {pi} event {ni} ends at {(onset + length) * beat_seconds:.3f}s; every note must end by {musical_seconds:g}s ({max_beat:g} beats at {bpm:g} BPM)")
                events.append({"id": len(events), "layer": part, "voice": voices[part], "midi": final_pitch,
                               "start_beat": onset, "length_beats": length, "velocity": velocity,
                               "start_seconds": onset * beat_seconds, "duration_seconds": length * beat_seconds,
                               "section": 0, "source_clip": ci, "source_placement": pi, "source_event": ni})
    active = {event["layer"] for event in events}
    if active != set(voices):
        raise ValueError("Every assigned voice must have at least one placed note; omit unused roles")
    first = min(event["start_seconds"] for event in events)
    last = max(event["start_seconds"] + event["duration_seconds"] for event in events)
    if first > 5:
        raise ValueError(f"The recording must begin by 5s; its first authored note begins at {first:.3f}s")
    recording_seconds = max(10, min(seconds, last + 1))
    # Stable timeline ordering changes no note, timing, overlap, velocity or voice.
    events.sort(key=lambda event: event["start_seconds"])
    score = {
        "kind": "performance", "performer": "organist", "condition": "organist", "title": title,
        "seed": seed, "bpm": bpm, "voices": dict(voices), "events": events,
        "duration_seconds": recording_seconds, "capture_limit_seconds": seconds,
        "musical_end_seconds": last, "release_seconds": recording_seconds - last,
        "sections": [{"id": 0, "name": "Recorded performance", "start_seconds": 0, "end_seconds": recording_seconds}],
        "arrangement_version": "organist_authored_clips_v1", "authoring": authoring,
        "event_sha256": _sha(json.dumps(events, sort_keys=True, allow_nan=False).encode()),
        "composition_rules": "Only the Organist's explicit clip notes, placements and transpositions are expanded. No generated accompaniment, fixed song form or pitch corrections.",
    }
    return score


def _endpoint():
    base = os.environ.get("TELEDRA_OLLAMA_URL", "http://localhost:11434").rstrip("/")
    parsed = urlsplit(base)
    host = parsed.hostname
    try:
        loopback = host == "localhost" or ipaddress.ip_address(host or "").is_loopback
    except ValueError:
        loopback = False
    if parsed.scheme not in ("http", "https") or not loopback or parsed.username or parsed.password or parsed.query or parsed.fragment or parsed.path not in ("", "/"):
        raise ValueError("TELEDRA_OLLAMA_URL must be a local loopback Ollama base URL")
    return base + "/api/chat"


def _stream_request(endpoint, payload, output_dir, attempt, progress):
    _write(output_dir / f"request-{attempt}.json", payload)
    request = Request(endpoint, data=json.dumps(payload, allow_nan=False).encode(), headers={"Content-Type": "application/json"}, method="POST")
    started = time.monotonic()
    status = {"chunks": 0, "characters": 0}
    stop = threading.Event()
    def heartbeat():
        while not stop.wait(2):
            elapsed = int(time.monotonic() - started)
            progress(f"Organist authoring · {elapsed}s · {status['characters']} characters / {status['chunks']} streamed chunks" + (" · factual repair" if attempt else ""))
    heartbeat_thread = threading.Thread(target=heartbeat, name="organist-progress", daemon=True)
    heartbeat_thread.start()
    fragments, final, raw_bytes = [], None, 0
    try:
        progress("Organist loading the configured local model" if not attempt else "Organist correcting the reported authoring error")
        with (output_dir / f"response-{attempt}.ndjson").open("wb") as raw_file:
            # A local performance must not send its persona through an inherited proxy.
            with build_opener(ProxyHandler({})).open(request, timeout=120) as response:
                while True:
                    if time.monotonic() - started > MAX_INFERENCE_SECONDS:
                        raise ValueError("Organist inference exceeded the 600-second recording preparation limit")
                    line = response.readline(MAX_RESPONSE_BYTES + 1)
                    if not line:
                        break
                    raw_bytes += len(line)
                    if len(line) > MAX_RESPONSE_BYTES or raw_bytes > MAX_STREAM_BYTES:
                        raise ValueError("Organist response exceeded the bounded stream size")
                    raw_file.write(line)
                    raw_file.flush()
                    packet = _json(line.decode("utf-8"))
                    if not isinstance(packet, dict):
                        raise ValueError("Ollama returned an invalid stream packet")
                    if packet.get("error"):
                        raise RuntimeError(f"Ollama: {packet['error']}")
                    message = packet.get("message", {})
                    fragment = message.get("content", "") if isinstance(message, dict) else None
                    if not isinstance(fragment, str):
                        raise ValueError("Ollama returned non-text authoring")
                    fragments.append(fragment)
                    status["chunks"] += 1
                    status["characters"] += len(fragment)
                    if status["characters"] > MAX_RESPONSE_BYTES:
                        raise ValueError("Organist authoring exceeded its bounded response size")
                    if packet.get("done") is True:
                        final = packet
                        break
        content = "".join(fragments)
        (output_dir / f"authoring-{attempt}.txt").write_text(content, encoding="utf-8")
        if final is None:
            raise ValueError("Ollama stream ended before completion; incomplete authoring was not accepted")
        if final.get("done_reason") == "length":
            raise ValueError("Ollama reached its output-token limit; incomplete authoring was not accepted")
        progress(f"Organist authored {status['characters']} characters in {int(time.monotonic() - started)}s")
        return content, {key: final[key] for key in ("model", "created_at", "done_reason", "total_duration", "load_duration", "prompt_eval_count", "prompt_eval_duration", "eval_count", "eval_duration") if key in final}
    finally:
        stop.set()
        heartbeat_thread.join(timeout=3)


def generate_performance(root, seed=2026, seconds=75, progress=lambda value: None):
    """Compose through the actual local Organist; save evidence and return its score."""
    root = Path(root).resolve()
    _integer(seed, "seed", 0, 2**31 - 1)
    _number(seconds, "recording seconds", 20, 180)
    endpoint = _endpoint()
    model = os.environ.get("TELEDRA_MODEL", "qwen2.5:7b")
    if not model.strip() or len(model) > 200:
        raise ValueError("TELEDRA_MODEL must name the configured local model")
    profile_path = root / "profiles/organist.md"
    if not profile_path.is_file():
        profile_path = root.parent / "Agents/profiles/organist.md"
    profile_bytes = profile_path.read_bytes()
    profile = profile_bytes.decode("utf-8-sig")
    catalog = _catalog(root)
    allowed = [item["id"] for item in catalog]
    output_dir = root / "reports" / ("organist-" + uuid4().hex)
    output_dir.mkdir(parents=True, exist_ok=False)
    (output_dir / "source-profile.md").write_bytes(profile_bytes)
    _write(output_dir / "catalog.json", catalog)
    provenance = {"performer": "organist", "provider": "local_ollama", "model": model, "endpoint": endpoint,
                  "seed": seed, "requested_seconds": seconds, "capture_limit_seconds": seconds,
                  "source_profile": str(profile_path),
                  "profile_sha256": _sha(profile_bytes), "catalog_sha256": _sha(json.dumps(catalog, sort_keys=True).encode()),
                  "output_dir": str(output_dir), "adapter": "organist_authored_clips_v1", "attempts": [],
                  "response_limits": {"clips": MAX_CLIPS, "notes_per_clip": MAX_CLIP_NOTES, "placements_per_clip": MAX_PLACEMENTS, "output_tokens": MAX_OUTPUT_TOKENS},
                  "scope": "Standalone Organist persona with the configured local model and direct sequencer interface; does not share live Court memory or the production recipe.",
                  "mixer_policy": "Shared PyMusic synthesis controls fixed layer gain/pan and per-note RMS normalization. The Organist's exact velocity is a render input, not a claim of final acoustic loudness."}
    _write(output_dir / "provenance.json", provenance)
    system = profile + "\n\nCURRENT STANDALONE RECORDING CONTRACT (supersedes native score, PyMusic recipe, raw Python, playback and edit-marker formats above):\n" + f"""
You are independently composing one new recorded performance. Return only the supplied JSON sequencer format. You own every instrument choice, pitch, rhythm, harmony, phrase, repetition, variation, entrance, exit and ending. No composition or accompaniment is generated after you answer. The adapter only places exactly the notes you author and renders them with PyMusic. Do not copy a Fly Composer song recipe; none is supplied. Do not emit Python, tool calls, commentary, markdown or playback requests.

Write {{"title":string,"bpm":number,"voices":{{part:instrumentID}},"clips":[{{"part":part,"events":[[midi,relativeBeat,lengthBeats,velocity],...],"placements":[[startBeat,transposeSemitones],...]}}]}}.
Parts are lead, answer, harmony, bass, texture. Choose any 1–5 parts and any catalog instrument for each; omit unused parts. Part labels route the mix, not a required musical role. Use 1–{MAX_CLIPS} clips. Each clip has 1–{MAX_CLIP_NOTES} note events and 1–{MAX_PLACEMENTS} placements. A clip's events may overlap and chords use simultaneous events. Placements add startBeat to every event's relativeBeat and add the integer transposeSemitones to each MIDI pitch. You may author different clips for entirely different material. Nothing automatically repeats or develops. Total expanded notes must not exceed {MAX_EVENTS}.
RESPONSE CAPACITY: The complete JSON must fit within {MAX_OUTPUT_TOKENS} output tokens. Use compact JSON without indentation or unnecessary whitespace, and short decimal numbers. Budget all selected parts and the ending before emitting your response. When you decide a phrase should repeat, author its notes once and list your chosen placements; do not flatten copies into hundreds of note rows. For your own changes of notes or rhythm, author a separate short clip for the same part. These are serialization limits, not a prescribed form, tune, harmony or repetition pattern. Finish the full voices and clips objects and closing JSON braces in this single response.
Quarter-note BPM is 40–220. MIDI and transpose are integers; final MIDI is 0–127. Beat starts are nonnegative; lengths are positive and at least 0.025 seconds at your tempo. Each native catalog instrument note may last at most 30 seconds; classic: voices may hold longer within the recording. Velocity is >0 through 1. Times are beats, never array indices; onset spacing is exactly as authored. Length includes each patch's release. Catalog ranges and attack spacing describe the available instruments.
The capture limit is UP TO {seconds:g} seconds, not a required performance length. Choose when your music ends naturally; an earlier ending is accepted without extending or repeating your material. All placed note ends must be at or before {seconds - 1:g} seconds. The recording ends one second after your last note, with a minimum recorded duration of 10 seconds and this capture limit as the maximum. Convert seconds to beats using BPM/60. At your chosen BPM the final beat limit is ({seconds - 1:g} * BPM / 60). Begin by 5 seconds. Choose your own form and cadence. There is no required key, progression, meter, motif, musical duration or minimum instrument count.
Available homemade instruments ({len(catalog)} total, including the seven classic PyMusic timbres):
""" + json.dumps(catalog, ensure_ascii=False, separators=(",", ":"))
    messages = [
        {"role": "system", "content": system},
        {"role": "user", "content": f"Compose and record your own musical performance as the Organist, within a capture limit of {seconds:g} seconds. I want to hear your independent decisions, including where the piece ends; there is no need to fill the whole limit. Fluidity, layers and motion matter to me; music should develop instead of getting stuck on a repeated scale. Choose the instruments and composition yourself. The listener will decide its quality after hearing the recording. Return the complete authored clip JSON."},
    ]
    last_error = None
    for attempt in range(2):
        payload = {"model": model, "messages": messages, "stream": True, "format": _schema(allowed),
                   "keep_alive": "5m", "options": {"seed": seed, "temperature": .7, "num_ctx": 16384, "num_predict": MAX_OUTPUT_TOKENS}}
        try:
            content, facts = _stream_request(endpoint, payload, output_dir, attempt, progress)
        except Exception as error:
            provenance["attempts"].append({"attempt": attempt, "status": "inference_failed", "error": str(error)})
            _write(output_dir / "provenance.json", provenance)
            raise OrganistError(f"Organist inference failed: {error}", output_dir) from error
        try:
            authoring = _json(content)
            score = expand_authoring(authoring, allowed, seed, seconds)
        except (ValueError, TypeError) as error:
            last_error = str(error)
            provenance["attempts"].append({"attempt": attempt, "status": "invalid_authoring", "error": last_error, "inference": facts})
            _write(output_dir / "provenance.json", provenance)
            if attempt == 0:
                messages = messages + [{"role": "assistant", "content": content},
                    {"role": "user", "content": "The sequencer could not execute your authoring. Factual validation error: " + last_error + ". Return the complete corrected JSON once. Correct the execution problem using your own musical decisions; no replacement music or stylistic prescription is being supplied."}]
                progress("Organist authoring needs one factual correction: " + last_error)
                continue
            break
        provenance["attempts"].append({"attempt": attempt, "status": "accepted_authoring", "inference": facts})
        provenance["returned_model"] = facts.get("model")
        provenance["recorded_seconds"] = score["duration_seconds"]
        provenance["authoring_sha256"] = _sha(content.encode())
        score["inference_provenance"] = provenance
        _write(output_dir / "provenance.json", provenance)
        _write(output_dir / "score.json", score)
        progress(f"Organist authored {len(score['events'])} explicit notes; ready to record")
        return score
    raise OrganistError(f"Organist authoring remained invalid after one factual repair: {last_error}. No substitute piece was generated", output_dir)
