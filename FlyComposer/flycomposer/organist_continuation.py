"""Bounded local Organist continuation, preserving the accepted source notes.

Each model response authors flat notes on the recording's absolute beat clock.
No offsets, clip expansion or timing translation occur. A failure returns the last valid prefix with an
explicit partial status; it never fills the remaining time with generated music.
"""
from __future__ import annotations

import copy
import json
import os
from pathlib import Path
from uuid import uuid4

from .organist import (_catalog, _endpoint, _integer, _json, _keys, _number,
                       _sha, _stream_request, _write)
from .render import assess_performance

MAX_CHUNKS = 16
MAX_REQUESTS = 24
WINDOW_SECONDS = 20
MIN_ADVANCE_SECONDS = 4
COMPLETION_TOLERANCE_SECONDS = 2
MAX_NOTES = 6000
CHUNK_NOTES = 64
CHUNK_OUTPUT_TOKENS = 4096
PROTOCOL = "organist_absolute_notes_v2"


def _hash(value):
    return _sha(json.dumps(value, sort_keys=True, allow_nan=False).encode())


def _last_end(events):
    return max(event["start_seconds"] + event["duration_seconds"] for event in events)


def _activity(events):
    """Measured density, not a musical quality judgment or a correction."""
    parts = {}
    for part in sorted({event["layer"] for event in events}):
        notes = [event for event in events if event["layer"] == part]
        timeline = []
        for event in notes:
            timeline.extend([(event["start_seconds"], 1), (event["start_seconds"] + event["duration_seconds"], -1)])
        active = peak = 0
        for _, change in sorted(timeline):
            active += change
            peak = max(active, peak)
        parts[part] = {"notes": len(notes), "max_simultaneous_notes": peak,
                       "midi_range": [min(n["midi"] for n in notes), max(n["midi"] for n in notes)],
                       "velocity_range": [min(n["velocity"] for n in notes), max(n["velocity"] for n in notes)],
                       "duration_seconds_range": [min(n["duration_seconds"] for n in notes), max(n["duration_seconds"] for n in notes)]}
    return {"notes": len(events), "musical_end_seconds": _last_end(events), "parts": parts}


def _flat(event):
    return {"part": event["layer"], "midi": event["midi"], "beat": event["start_beat"],
            "length": event["length_beats"], "velocity": event["velocity"]}


def _beat_activity(events, bpm):
    facts = _activity(events)
    facts["musical_end_beat"] = facts.pop("musical_end_seconds") * bpm / 60
    for part in facts["parts"].values():
        part["length_beats_range"] = [duration * bpm / 60 for duration in part.pop("duration_seconds_range")]
    return facts


def _chunk_schema(voices, bpm, first_beat, last_beat):
    return {"type": "object", "additionalProperties": False, "required": ["title", "notes"],
            "properties": {"title": {"type": "string", "minLength": 1, "maxLength": 160},
                           "notes": {"type": "array", "minItems": 1, "maxItems": CHUNK_NOTES,
                                     "items": {"type": "object", "additionalProperties": False,
                                               "required": ["part", "midi", "beat", "length", "velocity"],
                                               "properties": {
                                                   "part": {"type": "string", "enum": list(voices)},
                                                   "midi": {"type": "integer", "minimum": 0, "maximum": 127},
                                                   "beat": {"type": "number", "minimum": first_beat,
                                                            "maximum": last_beat - .025 * bpm / 60},
                                                   "length": {"type": "number", "minimum": .025 * bpm / 60,
                                                              "maximum": last_beat - first_beat},
                                                   "velocity": {"type": "number", "minimum": .001, "maximum": 1},
                                               }}}}}


def _expand_notes(authoring, voices, bpm, first_beat, last_beat, old_end, remaining_notes):
    _keys(authoring, ("title", "notes"), "continuation")
    title = authoring["title"]
    if not isinstance(title, str) or not title.strip() or len(title) > 160 or any(ord(c) < 32 for c in title):
        raise ValueError("title must be 1..160 printable characters")
    notes = authoring["notes"]
    if not isinstance(notes, list) or not 1 <= len(notes) <= CHUNK_NOTES:
        raise ValueError(f"Each response needs 1..{CHUNK_NOTES} explicit notes")
    if len(notes) > remaining_notes:
        raise ValueError(f"The recording has room for {remaining_notes} more explicit notes; this response contains {len(notes)}")
    result, outside = [], []
    for index, note in enumerate(notes):
        _keys(note, ("part", "midi", "beat", "length", "velocity"), f"note {index}")
        part = note["part"]
        if not isinstance(part, str) or part not in voices:
            raise ValueError(f"note {index} part must be one of {', '.join(voices)}")
        midi = _integer(note["midi"], f"note {index} MIDI", 0, 127)
        beat = _number(note["beat"], f"note {index} absolute beat", 0, 1000000)
        length = _number(note["length"], f"note {index} length beats", .025 * bpm / 60, 600 * bpm / 60)
        velocity = _number(note["velocity"], f"note {index} velocity", .001, 1)
        duration = length * 60 / bpm
        if not voices[part].startswith("classic:") and duration > 30:
            raise ValueError(f"note {index} exceeds the native instrument's 30-second note limit")
        if beat < first_beat - 1e-8 or beat + length > last_beat + 1e-8:
            outside.append(f"n{index}:start={beat:g},end={beat + length:g}")
        result.append({"id": index, "layer": part, "voice": voices[part], "midi": midi,
                       "start_beat": beat, "length_beats": length, "velocity": velocity,
                       "start_seconds": beat * 60 / bpm, "duration_seconds": duration,
                       "section": 0, "source_event": index, "source_protocol": PROTOCOL})
    if outside:
        raise ValueError("All absolute-window violations (zero-based note indices; all values are absolute beats): "
                         + "; ".join(outside) + f". Every note start must be >= {first_beat:g} and every note END (beat+length) <= {last_beat:g}. Correct every listed note; no offsets are added and no notes are clipped or shifted")
    advance = _last_end(result) - old_end
    minimum = min(MIN_ADVANCE_SECONDS, max(0, last_beat * 60 / bpm - old_end))
    if advance < minimum - 1e-8:
        raise ValueError(f"This response extends the accepted timeline by only {advance:.3f}s. Author new notes ending at or after absolute beat {(old_end + minimum) * bpm / 60:g}, and at or before {last_beat:g}, to continue forward")
    return result


def generate_continuation(root, source_score, seconds=120, progress=lambda value: None):
    root = Path(root).resolve()
    _number(seconds, "continuation capture limit", 20, 180)
    if source_score.get("kind") != "performance" or source_score.get("performer") != "organist":
        raise ValueError("An accepted Organist performance is required for continuation")
    assess_performance(source_score)
    seed = _integer(source_score.get("seed"), "source seed", 0, 2**31 - 1)
    bpm = _number(source_score.get("bpm"), "source BPM", 40, 220)
    source = copy.deepcopy(source_score)
    source_events = source["events"]
    if _last_end(source_events) > seconds - 1:
        raise ValueError("The source already exceeds this continuation capture limit")
    voices = source["voices"]
    endpoint = _endpoint()
    model = os.environ.get("TELEDRA_MODEL", "qwen2.5:7b")
    if not model.strip() or len(model) > 200:
        raise ValueError("TELEDRA_MODEL must name the configured local model")
    profile_path = root / "profiles/organist.md"
    if not profile_path.is_file():
        profile_path = root.parent / "Agents/profiles/organist.md"
    profile_bytes = profile_path.read_bytes()
    catalog = _catalog(root)
    selected_catalog = [voice for voice in catalog if voice["id"] in voices.values()]
    session_id = uuid4().hex
    output_dir = root / "reports" / f"organist-continuation-{session_id}"
    output_dir.mkdir(parents=True, exist_ok=False)
    (output_dir / "source-profile.md").write_bytes(profile_bytes)
    _write(output_dir / "source-score.json", source)
    _write(output_dir / "catalog.json", selected_catalog)
    provenance = {"provider": "local_ollama", "performer": "organist", "model": model,
                  "returned_models": [], "endpoint": endpoint, "profile_sha256": _sha(profile_bytes),
                  "source_profile": str(profile_path), "source_score_sha256": _hash(source),
                  "source_event_sha256": source["event_sha256"], "source_seed": seed,
                  "capture_limit_seconds": seconds, "target_seconds": seconds, "output_dir": str(output_dir),
                  "adapter": PROTOCOL, "protocol": "flat notes on one absolute recording beat clock",
                  "max_requests": MAX_REQUESTS,
                  "scope": "Standalone Organist persona using the configured local model and direct sequencer interface. Source notes are preserved; no live Court memory or production recipe is used. Shared mastering and authored overlap can change prefix PCM.",
                  "mixer_policy": source.get("inference_provenance", {}).get("mixer_policy", "Shared PyMusic fixed layer gain/pan and per-note RMS normalization; model velocity is an input to this renderer."),
                  "chunks": []}
    combined = copy.deepcopy(source_events)
    accepted = []
    calls = 0
    stop_reason = "request_limit"
    original_examples = {part: [_flat(event) for event in sorted(source_events, key=lambda note: note["start_beat"])
                                if event["layer"] == part][:2] for part in voices}
    prior_chunks = copy.deepcopy(source.get("continuation_authoring", []))
    if not isinstance(prior_chunks, list):
        prior_chunks = []

    def checkpoint(reason):
        last = _last_end(combined)
        recorded = max(10, min(seconds, last + 1))
        reached = recorded >= seconds - COMPLETION_TOLERANCE_SECONDS
        summary = {"status": "complete" if reached else "partial", "target_seconds": seconds,
                   "actual_seconds": recorded, "target_reached": reached,
                   "completion_tolerance_seconds": COMPLETION_TOLERANCE_SECONDS,
                   "new_notes": len(combined) - len(source_events), "accepted_chunks": len(accepted),
                   "new_chunks": len(accepted),
                   "requests": calls, "stop_reason": reason, "output_dir": str(output_dir),
                   "source_event_sha256": source["event_sha256"], "preserved_source_notes": len(source_events)}
        provenance.update(recorded_seconds=recorded, continuation=summary)
        result = copy.deepcopy(source)
        result.update(kind="performance", performer="organist", condition="organist",
                      title=source.get("title", "Organist") + " · continued", events=copy.deepcopy(combined),
                      duration_seconds=recorded, capture_limit_seconds=seconds, musical_end_seconds=last,
                      release_seconds=recorded - last, event_sha256=_hash(combined),
                      sections=[{"id": 0, "name": "Recorded performance", "start_seconds": 0, "end_seconds": recorded}],
                      arrangement_version=PROTOCOL, continuation=summary,
                      continuation_authoring=copy.deepcopy(prior_chunks + accepted),
                      source_inference_provenance=copy.deepcopy(source.get("inference_provenance")),
                      inference_provenance=copy.deepcopy(provenance))
        assess_performance(result)
        _write(output_dir / "provenance.json", provenance)
        _write(output_dir / "accepted-prefix.json", result)
        return result

    checkpoint("awaiting_model")
    for chunk_index in range(MAX_CHUNKS):
        old_end = _last_end(combined)
        if old_end + 1 >= seconds - COMPLETION_TOLERANCE_SECONDS:
            stop_reason = "target_reached"
            break
        if calls >= MAX_REQUESTS:
            stop_reason = "request_limit"
            break
        anchor = max(0, old_end - 1)
        window = min(WINDOW_SECONDS, seconds - 1 - anchor)
        first_beat = anchor * bpm / 60
        last_beat = (anchor + window) * bpm / 60
        tail = sorted(combined, key=lambda note: note["start_seconds"])[-48:]
        chunk_history = prior_chunks + accepted
        previous_notes = []
        if chunk_history:
            previous = chunk_history[-1]
            previous_notes = [_flat(event) for event in combined[-previous["notes"]:]]
        context = {"source_title": source.get("title"), "source_motif_examples": original_examples,
                   "bpm": bpm, "fixed_instruments": voices,
                   "clock": "Every beat is absolute on the complete recording timeline; length is a duration in beats",
                   "source_activity": _beat_activity(source_events, bpm), "accepted_activity": _beat_activity(combined, bpm),
                   "recent_notes": [_flat(event) for event in tail],
                   "previous_chunks": [{"chunk": item["chunk"], "session_id": item.get("session_id", "earlier_session"),
                                        "title": item["authoring"]["title"],
                                        "end_beat": item["end_seconds"] * bpm / 60,
                                        "notes": item["notes"]} for item in chunk_history],
                   "prior_session_chunks": len(prior_chunks),
                   "previous_chunk_performed_notes": previous_notes,
                   "playhead_beat": old_end * bpm / 60,
                   "next_note_beat_min": first_beat, "next_note_end_beat_max": last_beat,
                   "progress_end_beat_min": (old_end + min(MIN_ADVANCE_SECONDS, max(0, anchor + window - old_end))) * bpm / 60,
                   "recording_target_end_beat": (seconds - 1) * bpm / 60,
                   "notes_remaining": MAX_NOTES - len(combined)}
        chunk_dir = output_dir / f"chunk-{chunk_index:02d}"
        chunk_dir.mkdir()
        _write(chunk_dir / "context.json", context)
        log = {"chunk": chunk_index, "protocol": PROTOCOL, "first_beat": first_beat, "last_beat": last_beat,
               "window_seconds": window,
               "previous_end_seconds": old_end, "context_sha256": _hash(context), "attempts": []}
        provenance["chunks"].append(log)
        system = profile_bytes.decode("utf-8-sig") + f"""

CURRENT STANDALONE CONTINUATION CONTRACT (supersedes older score recipes, raw Python, native formats, clips and playback markers above):
Develop your accepted performance by authoring the next notes on ONE ABSOLUTE BEAT CLOCK. Existing notes are immutable. Every response note is played once at its exact authored beat and pitch. You own every variation, rhythm, dynamic and overlap. No accompaniment, pitch correction, thinning, scale filter, transposition, time shift, duration stretching or whole-piece looping is applied. The target is roughly {seconds:g} seconds across several bounded requests.
The source tempo stays {bpm:g} BPM and parts keep these instruments: {json.dumps(voices, separators=(",", ":"))}. Any subset of the source parts can play in this response. You choose how motifs develop and parts hand off. Density facts are measurements, not a quality verdict.
Return only compact JSON {{"title":string,"notes":[{{"part":part,"midi":integer,"beat":number,"length":number,"velocity":number}},...]}}. Maximum {CHUNK_NOTES} notes and {CHUNK_OUTPUT_TOKENS} output tokens. Do not return BPM, voices, clips, placements or offsets. No musical notes are generated from descriptions or instructions to repeat.
Every note's beat is an ABSOLUTE recording beat, including history and new response notes. No offset will be added. Every new beat must be at least {first_beat:g}; every new note END, beat + length, must be at most {last_beat:g}. The previous musical end is absolute beat {old_end * bpm / 60:g}. To make progress, at least one new note must end at or after absolute beat {context['progress_end_beat_min']:g}. Never restart at beat zero. You may overlap the previous ending inside the allowed absolute window. Do not insert a fixed pause between replies.
Aim for roughly 8–19 seconds of forward musical development in this response, or the shorter remaining window. Toward the final target, choose your own ending. The target ending is absolute beat {(seconds - 1) * bpm / 60:g}. This requests duration and continuity; all pitches, rhythms and layers remain your decisions.
MIDI is integer 0..127, velocity is 0.001..1, length is a duration in quarter-note beats (minimum {.025 * bpm / 60:g} beats). Native notes may last at most 30 seconds. All numbers finite. Seconds are derived directly as beat * 60 / BPM; this calculation adds no offset. Return only new notes in the allowed absolute beat window, not copies of old history notes.
Available source instrument capabilities: {json.dumps(selected_catalog, separators=(",", ":"))}
"""
        messages = [{"role": "system", "content": system},
                    {"role": "user", "content": "Continue your performance with fluidity, layers and motion. Compose only the next window, with your own development of the earlier material. Exact accepted context follows:\n" + json.dumps(context, separators=(",", ":"))}]
        appended = False
        for attempt in range(2):
            if calls >= MAX_REQUESTS:
                stop_reason = "request_limit"
                break
            calls += 1
            payload = {"model": model, "messages": messages, "stream": True,
                       "format": _chunk_schema(voices, bpm, first_beat, last_beat), "keep_alive": "5m",
                       "options": {"seed": (seed + chunk_index) % 2**31, "temperature": .7,
                                   "num_ctx": 16384, "num_predict": CHUNK_OUTPUT_TOKENS}}
            progress(f"Organist continuation {chunk_index + 1} · accepted {old_end + 1:.1f}s / target {seconds:g}s")
            try:
                content, facts = _stream_request(endpoint, payload, chunk_dir, attempt,
                    lambda message: progress(f"Continuation {chunk_index + 1}: {message}"))
            except Exception as error:
                log["attempts"].append({"attempt": attempt, "status": "inference_failed", "error": str(error)})
                stop_reason = "inference_failed: " + str(error)
                break
            returned_model = facts.get("model")
            if returned_model and returned_model not in provenance["returned_models"]:
                provenance["returned_models"].append(returned_model)
            try:
                authored = _json(content)
                fresh = _expand_notes(authored, voices, bpm, first_beat, last_beat, old_end, MAX_NOTES - len(combined))
            except (ValueError, TypeError) as error:
                log["attempts"].append({"attempt": attempt, "status": "invalid_authoring", "error": str(error), "inference": facts})
                stop_reason = "invalid_authoring: " + str(error)
                if attempt == 0 and calls < MAX_REQUESTS:
                    messages += [{"role": "assistant", "content": content},
                                 {"role": "user", "content": "Factual sequencer validation error: " + str(error) + ". Return one complete corrected absolute-note JSON. Correct only the execution or forward-progress problem using your own musical decisions; do not change the accepted source notes."}]
                    continue
                break
            for event in fresh:
                event["id"] = f"{session_id}:{chunk_index}:{event['id']}"
                event["source_chunk"] = chunk_index
            combined.extend(fresh)
            accepted.append({"chunk": chunk_index, "session_id": session_id, "protocol": PROTOCOL,
                             "first_allowed_beat": first_beat, "end_seconds": _last_end(fresh),
                             "notes": len(fresh), "authoring": authored, "authoring_sha256": _sha(content.encode())})
            log.update(status="accepted", end_seconds=_last_end(fresh), new_notes=len(fresh), activity=_activity(fresh))
            log["attempts"].append({"attempt": attempt, "status": "accepted_authoring", "inference": facts})
            checkpoint("continuing")
            appended = True
            break
        _write(chunk_dir / "chunk-log.json", log)
        if not appended:
            break
    else:
        stop_reason = "chunk_limit"
    result = checkpoint(stop_reason)
    if result["continuation"]["target_reached"]:
        result["continuation"]["stop_reason"] = "target_reached"
        result["inference_provenance"]["continuation"]["stop_reason"] = "target_reached"
        _write(output_dir / "provenance.json", result["inference_provenance"])
        _write(output_dir / "accepted-prefix.json", result)
    _write(output_dir / "score.json", result)
    progress(f"Organist continuation {result['continuation']['status']} · {result['duration_seconds']:.1f}s recorded · {result['continuation']['new_notes']} new notes")
    return result
