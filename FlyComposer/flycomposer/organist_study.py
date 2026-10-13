"""Fresh local Organist studies: plan, author, inspect facts, optionally revise.

All music comes from model responses. Score facts describe authored structure;
they neither grade musical quality nor automatically rewrite or approve notes.
"""
from __future__ import annotations

from collections import Counter
import copy
import json
import os
from pathlib import Path
from uuid import uuid4

from .organist import (OrganistError, _catalog, _endpoint, _integer, _json, _keys,
                       _number, _sha, _stream_request, _write)
from .organist_continuation import (CHUNK_NOTES, MIN_ADVANCE_SECONDS, _chunk_schema,
                                   _expand_notes, _flat, _hash, _last_end)
from .render import SR, assess_performance

PROTOCOL = "organist_planned_study_v2"
PLAN_TOKENS = 1400
AUTHOR_TOKENS = 4096
REVIEW_TOKENS = 4096
MAX_SILENCE_SECONDS = 2
MAX_TRANSITION_SECONDS = 2


def _text(value, name, limit):
    if not isinstance(value, str) or not value.strip() or len(value) > limit:
        raise ValueError(f"{name} must contain 1..{limit} characters")
    return value


def _frame_intervals(events):
    # Match the renderer exactly; never change the original authored timings.
    return [(int(event["start_seconds"] * SR),
             int(event["start_seconds"] * SR) + int(event["duration_seconds"] * SR), event["layer"])
            for event in events]


def _silent_intervals(intervals, start_frame, end_frame):
    cursor = start_frame
    gaps = []
    for start, end in sorted(intervals):
        if end <= start_frame or start >= end_frame:
            continue
        start, end = max(start_frame, start), min(end_frame, end)
        if start > cursor:
            gaps.append((cursor, start))
        cursor = max(cursor, end)
    if cursor < end_frame:
        gaps.append((cursor, end_frame))
    return gaps


def _gap_facts(gaps, bpm):
    return [{"start_beat": start / SR * bpm / 60, "end_beat": end / SR * bpm / 60,
             "seconds": (end - start) / SR} for start, end in gaps]


def _require_continuity(events, bpm, start_beat, end_beat, label):
    intervals = [(start, end) for start, end, _ in _frame_intervals(events)]
    gaps = _silent_intervals(intervals, int(start_beat * 60 / bpm * SR), int(end_beat * 60 / bpm * SR))
    violations = [(start, end) for start, end in gaps if end - start > MAX_SILENCE_SECONDS * SR]
    if violations:
        details = "; ".join(f"beats {gap['start_beat']:g}..{gap['end_beat']:g} ({gap['seconds']:g}s)"
                            for gap in _gap_facts(violations, bpm))
        raise ValueError(f"Declared fluidity contract failed in {label}: every silence longer than {MAX_SILENCE_SECONDS}s is listed here: {details}. Include leading, internal and trailing silence in this absolute window. Author the corrections yourself; no notes have been filled, shifted or stretched")


def score_facts(events, bpm, catalog, *, start_beat=0, end_beat=None):
    """Factual articulation, register and simultaneous activity; no taste score."""
    by_voice = {voice["id"]: voice for voice in catalog}
    by_part = {}
    timeline = {}
    for event in events:
        part = event["layer"]
        by_part.setdefault(part, []).append(event)
    intervals = _frame_intervals(events)
    first_frame = int(start_beat * 60 / bpm * SR)
    last_frame = max(first_frame, max(end for _, end, _ in intervals)) if end_beat is None else int(end_beat * 60 / bpm * SR)
    if last_frame < first_frame:
        raise ValueError("Analysis window must end at or after its start")
    scoped_intervals = [(max(first_frame, start), min(last_frame, end), part)
                        for start, end, part in intervals if end > first_frame and start < last_frame]
    for start_frame, end_frame, part in scoped_intervals:
        timeline.setdefault(start_frame, []).append((part, 1))
        timeline.setdefault(end_frame, []).append((part, -1))
    active = Counter()
    occupied = Counter()
    maximum_by_part = Counter()
    total_active = cross_part_overlap = 0
    max_polyphony = 0
    previous = first_frame
    for time in sorted(timeline):
        duration = time - previous
        sounding = [part for part, count in active.items() if count > 0]
        if sounding:
            total_active += duration
        if len(sounding) > 1:
            cross_part_overlap += duration
        for part in sounding:
            occupied[part] += duration
        for part, delta in timeline[time]:
            active[part] += delta
        max_polyphony = max(max_polyphony, sum(active.values()))
        for part, count in active.items():
            maximum_by_part[part] = max(maximum_by_part[part], count)
        previous = time

    def distribution(values):
        counts = Counter(round(value, 5) for value in values)
        return {"distinct_values": len(counts), "most_common": [{"value": value, "notes": count}
                                                               for value, count in counts.most_common(5)]}
    parts = {}
    for part, notes in by_part.items():
        ordered = sorted(notes, key=lambda event: event["start_seconds"])
        span = by_voice.get(notes[0]["voice"], {}).get("range")
        jumps = [abs(b["midi"] - a["midi"]) for a, b in zip(ordered, ordered[1:])
                 if b["start_seconds"] > a["start_seconds"]]
        parts[part] = {"voice": notes[0]["voice"], "notes": len(notes),
                       "midi_range": [min(e["midi"] for e in notes), max(e["midi"] for e in notes)],
                       "catalog_recommended_midi_range": span,
                       "notes_outside_recommended_range": sum(not span[0] <= e["midi"] <= span[1] for e in notes) if span else None,
                       "length_beats": distribution(e["length_beats"] for e in notes),
                       "velocity": distribution(e["velocity"] for e in notes),
                       "adjacent_event_jumps_at_least_octave": sum(jump >= 12 for jump in jumps),
                       "adjacent_event_jump_caveat": "Event-order comparison, not an inferred melodic voice through chords",
                       "occupied_beats": occupied[part] / SR * bpm / 60,
                       "max_simultaneous_notes": maximum_by_part[part]}
    gaps = _silent_intervals([(start, end) for start, end, _ in scoped_intervals], first_frame, last_frame)
    silence_frames = sum(end - start for start, end in gaps)
    window_frames = max(0, last_frame - first_frame)
    max_gap_frames = max((end - start for start, end in gaps), default=0)
    return {"notes": len(events), "musical_end_beat": _last_end(events) * bpm / 60,
            "global_max_polyphony": max_polyphony, "occupied_beats": total_active / SR * bpm / 60,
            "cross_part_overlap_beats": cross_part_overlap / SR * bpm / 60,
            "cross_part_overlap_fraction_of_occupied_time": cross_part_overlap / total_active if total_active else 0,
            "silence_intervals": _gap_facts(gaps, bpm), "global_max_gap_seconds": max_gap_frames / SR,
            "global_max_gap_beats": max_gap_frames / SR * bpm / 60,
            "occupied_fraction": 1 - silence_frames / window_frames if window_frames else 0,
            "analysis_window_beats": [first_frame / SR * bpm / 60, last_frame / SR * bpm / 60],
            "analysis_sample_rate": SR,
            "parts": parts, "interpretation": "Measured score structure and recommended instrument ranges; no listening or aesthetic grade"}


def _global_end_beat(plan, events, bpm):
    return max(max(passage["end_beat"] for passage in plan["passages"]), _last_end(events) * bpm / 60)


def _plan_schema(voices, end_beat):
    short = {"type": "string", "minLength": 1, "maxLength": 200}
    return {"type": "object", "additionalProperties": False,
            "required": ["title", "idea", "part_intentions", "passages"],
            "properties": {
                "title": {"type": "string", "minLength": 1, "maxLength": 160},
                "idea": {"type": "string", "minLength": 1, "maxLength": 400},
                "part_intentions": {"type": "object", "additionalProperties": False, "required": list(voices),
                                    "properties": {part: short for part in voices}},
                "passages": {"type": "array", "minItems": 2, "maxItems": 4, "items": {
                    "type": "object", "additionalProperties": False,
                    "required": ["name", "start_beat", "end_beat", "intent"],
                    "properties": {"name": {"type": "string", "minLength": 1, "maxLength": 80},
                                   "start_beat": {"type": "number", "minimum": 0, "maximum": end_beat},
                                   "end_beat": {"type": "number", "minimum": 0, "maximum": end_beat},
                                   "intent": short}}}}}


def _validate_plan(plan, voices, bpm, seconds):
    _keys(plan, ("title", "idea", "part_intentions", "passages"), "study plan")
    _text(plan["title"], "title", 160)
    _text(plan["idea"], "idea", 400)
    _keys(plan["part_intentions"], voices, "part intentions")
    for part, intent in plan["part_intentions"].items():
        _text(intent, f"{part} intention", 200)
    passages = plan["passages"]
    if not isinstance(passages, list) or not 2 <= len(passages) <= 4:
        raise ValueError("Plan 2..4 passages")
    end_limit = (seconds - 1) * bpm / 60
    previous_start = -1
    for index, passage in enumerate(passages):
        _keys(passage, ("name", "start_beat", "end_beat", "intent"), f"passage {index}")
        _text(passage["name"], "passage name", 80)
        _text(passage["intent"], "passage intention", 200)
        start = _number(passage["start_beat"], "passage start beat", 0, end_limit)
        end = _number(passage["end_beat"], "passage end beat", 0, end_limit)
        if start < previous_start or end - start < .025 * bpm / 60:
            raise ValueError("Passage starts must be ordered, with each end after its own start; overlaps are allowed")
        previous_start = start
    if passages[0]["start_beat"] * 60 / bpm > 5:
        raise ValueError("The first passage must permit an onset by five seconds")
    windows = [(int(p["start_beat"] * 60 / bpm * SR), int(p["end_beat"] * 60 / bpm * SR)) for p in passages]
    gaps = _silent_intervals(windows, 0, max(end for _, end in windows))
    violations = [(start, end) for start, end in gaps if end - start > MAX_SILENCE_SECONDS * SR]
    if violations:
        detail = "; ".join(f"beats {g['start_beat']:g}..{g['end_beat']:g} ({g['seconds']:g}s)" for g in _gap_facts(violations, bpm))
        raise ValueError(f"Plan windows must permit the requested continuous performance. All gaps over {MAX_SILENCE_SECONDS}s: {detail}. The windows may overlap; choose your own connected plan")
    return plan


def _passage_end(passage, bpm, end_limit):
    return min(end_limit, passage['end_beat'] + MAX_TRANSITION_SECONDS * bpm / 60)


def _passage_notes(authoring, passage, voices, bpm, end_limit=None):
    # A fresh passage has no append playhead. Placing this validation origin
    # before its permitted window disables the continuation-only progress rule.
    origin = passage["start_beat"] * 60 / bpm - MIN_ADVANCE_SECONDS
    allowed_end = passage['end_beat'] if end_limit is None else _passage_end(passage, bpm, end_limit)
    notes = _expand_notes(authoring, voices, bpm, passage["start_beat"], allowed_end, origin, CHUNK_NOTES)
    _require_continuity(notes, bpm, passage["start_beat"], passage["end_beat"], passage["name"])
    return notes


def _review_schema(voices, bpm, end_beat, passage_count):
    note = _chunk_schema(voices, bpm, 0, end_beat)["properties"]["notes"]["items"]
    return {"type": "object", "additionalProperties": False, "required": ["critique", "replacements"],
            "properties": {
                "critique": {"type": "string", "minLength": 1, "maxLength": 1200},
                "replacements": {"type": "array", "maxItems": 2, "items": {
                    "type": "object", "additionalProperties": False,
                    "required": ["passage_index", "reason", "notes"],
                    "properties": {"passage_index": {"type": "integer", "minimum": 0, "maximum": passage_count - 1},
                                   "reason": {"type": "string", "minLength": 1, "maxLength": 250},
                                   "notes": {"type": "array", "minItems": 1, "maxItems": 32, "items": note}}}}}}


def generate_study(root, source_score, feedback, seconds=75, progress=lambda value: None):
    root = Path(root).resolve()
    _number(seconds, "study capture limit", 20, 180)
    if source_score.get("kind") != "performance" or source_score.get("performer") != "organist":
        raise ValueError("A recorded Organist source is required")
    assess_performance(source_score)
    source = copy.deepcopy(source_score)
    seed = _integer(source.get("seed"), "source seed", 0, 2**31 - 1)
    bpm = _number(source.get("bpm"), "source BPM", 40, 220)
    voices = copy.deepcopy(source["voices"])
    feedback = copy.deepcopy(feedback)
    # Reject unserializable feedback before making a request, without changing it.
    json.dumps(feedback, allow_nan=False)
    model = os.environ.get("TELEDRA_MODEL", "qwen2.5:7b")
    if not model.strip() or len(model) > 200:
        raise ValueError("TELEDRA_MODEL must name the configured local model")
    endpoint = _endpoint()
    profile_path = root / "profiles/organist-study.md"
    profile_bytes = profile_path.read_bytes()
    catalog = _catalog(root)
    selected_catalog = [voice for voice in catalog if voice["id"] in voices.values()]
    session_id = uuid4().hex
    output_dir = root / "reports" / f"organist-study-{session_id}"
    output_dir.mkdir(parents=True, exist_ok=False)
    (output_dir / "source-profile.md").write_bytes(profile_bytes)
    _write(output_dir / "source-score.json", source)
    _write(output_dir / "operator-feedback.json", feedback)
    criticized_facts = score_facts(source["events"], bpm, catalog)
    _write(output_dir / "criticized-source-facts.json", criticized_facts)
    provenance = {"performer": "organist", "provider": "local_ollama", "model": model,
                  "returned_models": [], "endpoint": endpoint, "source_seed": seed,
                  "source_score_sha256": _hash(source), "source_event_sha256": source["event_sha256"],
                  "profile_sha256": _sha(profile_bytes), "source_profile": str(profile_path),
                  "feedback": feedback, "capture_limit_seconds": seconds, "adapter": PROTOCOL,
                  "continuity_contract": {"max_silence_seconds": MAX_SILENCE_SECONDS,
                                          "internal_transition_overlap_seconds": MAX_TRANSITION_SECONDS,
                                          "scope": "This study's declared passage windows and combined timeline; not a universal musical-quality grade",
                                          "basis": "Engineering interpretation of requested fluidity; the operator did not specify a numeric silence threshold"},
                  "output_dir": str(output_dir), "phases": [],
                  "scope": "New model-authored interpretation of a criticized recording. Same seed, tempo and instruments, with all musical notes eligible for replacement. Factual checks and model self-critique are not listening or listener approval.",
                  "mixer_policy": "Shared PyMusic fixed layer gain/pan and per-note RMS normalization; authored velocity is a renderer input, not a claim of final acoustic loudness."}
    _write(output_dir / "provenance.json", provenance)
    end_limit = (seconds - 1) * bpm / 60
    contract = profile_bytes.decode("utf-8-sig") + f"""

CURRENT STUDIO: source tempo {bpm:g} BPM and source instruments {json.dumps(voices, separators=(",", ":"))} stay fixed for comparison. This is a fresh study, not an append. Do not copy the criticized take's note pattern. One absolute recording beat clock is used throughout; seconds equal beat * 60 / BPM. Every new note must end by absolute beat {end_limit:g}, within an up-to-{seconds:g}-second capture. An earlier musical ending is allowed; silence is not padded to fill the limit. The renderer adds one second of release, with a minimum ten-second file.
All notes are explicit objects {{"part":part,"midi":integer,"beat":absoluteBeat,"length":durationBeats,"velocity":number}}. Array order consumes no time. Parts may share beat starts or overlap, and a held supporting note may continue under foreground notes. Nothing automatically places notes sequentially or generates accompaniment. Chords require your simultaneous notes. MIDI 0..127, velocity 0.001..1, note length at least {.025 * bpm / 60:g} beats. Native notes have a 30-second maximum. Source instrument ranges below are recommended capabilities, not a quality score.
For this requested fluidity study, every declared passage window and the combined study must have no silence longer than {MAX_SILENCE_SECONDS} seconds, including leading, internal and trailing gaps. Plan windows must allow connected music; they may overlap. A supporting held note can carry a foreground rest. You author every correction; the renderer fills no gaps and stretches no notes. This is this study's operational check for requested fluidity, not a universal beauty grade; the operator did not specify the numeric threshold. At {bpm:g} BPM, one quarter-note beat lasts {60 / bpm:g} seconds, and the silence limit is {MAX_SILENCE_SECONDS * bpm / 60:g} beats. Choose lengths with those audible durations in mind.
Use comfortable registers from the catalog as a starting point; departures need your musical intention, not automatic pitch drift toward MIDI extremes. The minimum {.025 * bpm / 60:g}-beat note lasts only 25 milliseconds: a brief ornament, not a default phrase articulation. There is no automatic pitch filter or register correction.
Internal passage transitions may overlap: notes may extend up to {MAX_TRANSITION_SECONDS * bpm / 60:g} beats beyond a passage's planned end, while every note must still end by the complete recording's absolute limit {end_limit:g}. Planned windows describe the intended coverage; they do not cut off your overlapping notes.
Instrument capabilities: {json.dumps(selected_catalog, separators=(",", ":"))}
"""

    def phase(name, prompt, schema, validate, max_tokens):
        directory = output_dir / name
        directory.mkdir()
        messages = [{"role": "system", "content": contract}, {"role": "user", "content": prompt}]
        log = {"phase": name, "attempts": [], "context_sha256": _hash(messages)}
        provenance["phases"].append(log)
        attempts = 3 if name.startswith('passage-') else 2
        for attempt in range(attempts):
            payload = {"model": model, "messages": messages, "stream": True, "format": schema,
                       "keep_alive": "5m", "options": {"seed": seed, "temperature": .7,
                                                       "num_ctx": 16384, "num_predict": max_tokens}}
            try:
                raw, inference = _stream_request(endpoint, payload, directory, attempt,
                    lambda message: progress(f"Organist study {name}: {message}"))
            except Exception as error:
                log["attempts"].append({"attempt": attempt, "status": "inference_failed", "error": str(error)})
                _write(output_dir / "provenance.json", provenance)
                raise OrganistError(f"Study {name} inference failed: {error}", output_dir) from error
            returned = inference.get("model")
            if returned and returned not in provenance["returned_models"]:
                provenance["returned_models"].append(returned)
            try:
                parsed = _json(raw)
                value = validate(parsed)
            except (ValueError, TypeError) as error:
                log["attempts"].append({"attempt": attempt, "status": "invalid_authoring", "error": str(error), "inference": inference})
                _write(output_dir / "provenance.json", provenance)
                if attempt + 1 < attempts:
                    messages += [{"role": "assistant", "content": raw}, {"role": "user", "content": "Factual contract error: " + str(error) + ". Return the complete corrected JSON once. Choose any musical corrections yourself; no replacement notes are supplied."}]
                    continue
                raise OrganistError(f"Study {name} remained invalid after {attempts - 1} factual repairs: {error}", output_dir) from error
            log["attempts"].append({"attempt": attempt, "status": "accepted_authoring", "inference": inference,
                                    "response_sha256": _sha(raw.encode())})
            _write(directory / "accepted.json", parsed)
            _write(output_dir / "provenance.json", provenance)
            return value
        raise AssertionError("bounded phase did not return")

    plan_prompt = "Plan a fresh, more deliberately developed interpretation responding to the operator. The previous artifact was criticized, not approved. These facts describe it; they are not instructions to reproduce its values. Choose 2–4 passages with your own musical purposes and absolute beat windows. Ordered starts are required; windows may overlap so supporting notes can bridge transitions. Do not prescribe a pause at boundaries. State each source part's musical intention. You choose harmony, phrasing, development, dynamics and ending. Return only the plan JSON.\n" + json.dumps({"operator_feedback": feedback, "criticized_source_facts": criticized_facts}, separators=(",", ":"))
    plan = phase("plan", plan_prompt, _plan_schema(voices, end_limit),
                 lambda value: _validate_plan(value, voices, bpm, seconds), PLAN_TOKENS)
    _write(output_dir / "plan.json", plan)
    authored_passages = []

    def make_score(passages):
        events = []
        sections = []
        for index, notes in enumerate(passages):
            for event in notes:
                event = dict(event)
                event.update(id=f"{session_id}:{index}:{event['id']}", source_passage=index, source_protocol=PROTOCOL)
                events.append(event)
            sections.append({"id": index, "name": plan["passages"][index]["name"],
                             "start_seconds": min(n["start_seconds"] for n in notes), "end_seconds": _last_end(notes)})
        last = _last_end(events)
        duration = max(10, min(seconds, last + 1))
        result = {"title": plan["title"], "kind": "performance", "performer": "organist", "condition": "organist",
                  "seed": seed, "bpm": bpm, "voices": copy.deepcopy(voices), "events": events,
                  "duration_seconds": duration, "capture_limit_seconds": seconds, "musical_end_seconds": last,
                  "release_seconds": duration - last, "sections": sections, "event_sha256": _hash(events),
                  "arrangement_version": PROTOCOL, "plan": copy.deepcopy(plan),
                  "source_inference_provenance": copy.deepcopy(source.get("inference_provenance")),
                  "composition_rules": "All notes authored by the local Organist; no fixed tune, accompaniment, timing translation, pitch correction or aesthetic gate."}
        assess_performance(result)
        return result

    for index, passage in enumerate(plan["passages"]):
        previous_notes = [note for notes in authored_passages for note in notes]
        # These are request-size limits, not new musical sections. Divide evenly
        # so a passage just over 16 beats does not leave an unplayable tiny page.
        span = passage["end_beat"] - passage["start_beat"]
        page_count = max(1, int(span / 16))
        if span > page_count * 16:
            page_count += 1
        passage_notes = []
        for page_index in range(page_count):
            window = {**passage, "start_beat": passage["start_beat"] + span * page_index / page_count,
                      "end_beat": passage["end_beat"] if page_index + 1 == page_count else passage["start_beat"] + span * (page_index + 1) / page_count}
            allowed_end = _passage_end(window, bpm, end_limit)
            phase_name = f"passage-{index}" if page_count == 1 else f"passage-{index}-page-{page_index}"
            work_window = {"phase": phase_name, "source_passage": index, "page_index": page_index, "page_count": page_count,
                           "start_beat": window["start_beat"], "end_beat": window["end_beat"],
                           "start_seconds": window["start_beat"] * 60 / bpm, "end_seconds": window["end_beat"] * 60 / bpm,
                           "allowed_note_end_beat": allowed_end,
                           "required_coverage_end_beat": max(window["start_beat"], window["end_beat"] - MAX_SILENCE_SECONDS * bpm / 60)}
            provenance.setdefault("authoring_windows", []).append(copy.deepcopy(work_window))
            _write(output_dir / "provenance.json", provenance)
            accepted_so_far = previous_notes + passage_notes
            context = {"operator_feedback": feedback, "your_plan": plan, "passage_index": index,
                       "passage": passage, "work_window": work_window,
                       "previous_new_study_notes": [_flat(note) for note in sorted(accepted_so_far, key=lambda n: n["start_beat"])[-48:]],
                       "earlier_pages_in_this_passage": [_flat(note) for note in sorted(passage_notes, key=lambda n: n["start_beat"])[-48:]],
                       "clock": "All note beats, including earlier new-study notes, are absolute recording beats"}
            prompt = f"Author work window {page_index + 1}/{page_count} of original passage {index} as compact JSON {{title,notes}} with at most {CHUNK_NOTES} explicit notes. This is one page of your existing musical passage, not a new section or restart. Its absolute window is beats {window['start_beat']:g}..{window['end_beat']:g}, seconds {work_window['start_seconds']:g}..{work_window['end_seconds']:g}: a span of {window['end_beat'] - window['start_beat']:g} quarter-note beats ({work_window['end_seconds'] - work_window['start_seconds']:g} seconds). Every note start must be >= {window['start_beat']:g}, every note end (beat+length) <= {allowed_end:g}. Sound must reach at least absolute beat {work_window['required_coverage_end_beat']:g} to meet the {MAX_SILENCE_SECONDS}s trailing-silence tolerance; also avoid longer leading or internal holes. Internal transitions may overlap the page end by up to {MAX_TRANSITION_SECONDS}s, within the stated allowed end. Choose and write your supporting harmony across the WHOLE work window first, then author foreground and answering phrases on that same absolute clock. You choose all pitches, rhythms, harmony, lengths and velocities. Develop your original passage's musical intention and carry ideas from its earlier pages. Array order consumes no time: independently timed parts may sound simultaneously and held support can continue under changing foreground. Existing new-study notes are context; do not include them again. No notes are automatically added, shifted, filled or stretched. Return only this work window's JSON.\n" + json.dumps(context, separators=(",", ":"))
            def validate_page(value, p=window):
                notes = _passage_notes(value, p, voices, bpm, end_limit)
                candidate = accepted_so_far + notes
                prefix_end = max([p["end_beat"], _last_end(candidate) * bpm / 60]
                                 + [item["end_beat"] for item in plan["passages"][:index]])
                _require_continuity(candidate, bpm, 0, prefix_end, "combined study so far")
                return notes
            notes = phase(phase_name, prompt, _chunk_schema(voices, bpm, window["start_beat"], allowed_end),
                          validate_page, AUTHOR_TOKENS)
            for note in notes:
                note.update(source_page=page_index, source_phase=phase_name)
                if page_count > 1:
                    note["id"] = f"page-{page_index}:{note['id']}"
            passage_notes.extend(notes)
            provenance["authoring_windows"][-1].update(status="accepted", notes=len(notes), event_sha256=_hash(notes))
            _write(output_dir / "provenance.json", provenance)
        authored_passages.append(passage_notes)
        _write(output_dir / "draft-prefix.json", make_score(authored_passages))
    draft = make_score(authored_passages)
    draft_facts = score_facts(draft["events"], bpm, catalog, end_beat=_global_end_beat(plan, draft["events"], bpm))
    passage_facts = [score_facts(notes, bpm, catalog, start_beat=plan["passages"][index]["start_beat"],
                                end_beat=plan["passages"][index]["end_beat"]) for index, notes in enumerate(authored_passages)]
    _write(output_dir / "draft-score.json", draft)
    _write(output_dir / "factual-critique.json", {"whole_study": draft_facts, "passages": passage_facts,
                                                "scope": "Mechanical score observations, not an audio audition or aesthetic grade"})

    def validate_review(review):
        _keys(review, ("critique", "replacements"), "model review")
        _text(review["critique"], "model critique", 1200)
        replacements = review["replacements"]
        if not isinstance(replacements, list) or len(replacements) > 2:
            raise ValueError("Review may replace at most two passages")
        seen, validated = set(), []
        for replacement in replacements:
            _keys(replacement, ("passage_index", "reason", "notes"), "passage replacement")
            index = _integer(replacement["passage_index"], "passage index", 0, len(authored_passages) - 1)
            if index in seen:
                raise ValueError("Each passage may be replaced once in this review")
            seen.add(index)
            _text(replacement["reason"], "replacement reason", 250)
            if not isinstance(replacement["notes"], list) or len(replacement["notes"]) > 32:
                raise ValueError("Each reviewed passage allows at most 32 notes")
            notes = _passage_notes({"title": plan["passages"][index]["name"], "notes": replacement["notes"]}, plan["passages"][index], voices, bpm, end_limit)
            validated.append((index, notes))
        candidate = list(authored_passages)
        for index, notes in validated:
            candidate[index] = notes
        candidate_events = [note for notes in candidate for note in notes]
        _require_continuity(candidate_events, bpm, 0, _global_end_beat(plan, candidate_events, bpm),
                            "joint model-review candidate")
        return review, validated

    review_context = {"operator_feedback": feedback, "your_plan": plan, "factual_critique": draft_facts,
                      "passages": [{"passage_index": index, "facts": passage_facts[index],
                                    "note_columns": ["part", "midi", "absolute_beat", "length_beats", "velocity"],
                                    "notes": [[n["layer"], n["midi"], n["start_beat"], n["length_beats"], n["velocity"]] for n in notes]}
                                   for index, notes in enumerate(authored_passages)]}
    review_prompt = "Inspect your actual authored study against your own plan and the operator's feedback. You have not heard its audio. Use the measured articulation, register, simultaneous parts and occupied-time facts; do not equate a numerical score with beauty. Did the notes realize your intended phrasing and coordinated layers, or merely change part labels along one serial line? This is your one opportunity to revise up to two passages using your own replacement notes. Return {critique,replacements}; each replacement has passage_index, reason and the complete new notes for that passage (at most32 notes each). Cover each replaced passage's planned window, with only the explicitly allowed internal transition overlap; respect the final recording limit. You may return an empty replacement list if you decide none is warranted, and explain that decision. No notes are changed from critique prose alone.\n" + json.dumps(review_context, separators=(",", ":"))
    review_status, revised = "completed", []
    try:
        model_review, revisions = phase("model-review", review_prompt,
            _review_schema(voices, bpm, end_limit, len(authored_passages)), validate_review, REVIEW_TOKENS)
        # Apply validated whole-passage replacements together; an invalid review
        # leaves the complete authored draft available, without partial edits.
        for index, notes in revisions:
            authored_passages[index] = notes
            revised.append(index)
        _write(output_dir / "model-critique.json", {"scope": "Model self-critique of the score, not listening", **model_review})
    except OrganistError as error:
        review_status = "failed_draft_retained"
        _write(output_dir / "model-critique.json", {"scope": "Model review unavailable; complete draft retained", "error": str(error)})
    result = make_score(authored_passages)
    def signature(events):
        return sorted((e["layer"], e["voice"], e["midi"], e["start_beat"], e["length_beats"], e["velocity"]) for e in events)
    if signature(result["events"]) == signature(source["events"]):
        raise OrganistError("The new study repeats the source note ledger; no replacement was published", output_dir)
    final_end_beat = _global_end_beat(plan, result["events"], bpm)
    _require_continuity(result["events"], bpm, 0, final_end_beat, "final combined study")
    final_facts = score_facts(result["events"], bpm, catalog, end_beat=final_end_beat)
    provenance.update(recorded_seconds=result["duration_seconds"], review_status=review_status,
                      revised_passages=revised, score_facts=final_facts)
    result["inference_provenance"] = copy.deepcopy(provenance)
    result["study"] = {"feedback": feedback, "source_score_sha256": _hash(source),
                       "capture_limit_seconds": seconds, "actual_seconds": result["duration_seconds"],
                       "planned_passages": len(plan["passages"]), "review_status": review_status,
                       "revised_passages": revised, "output_dir": str(output_dir),
                       "continuity_contract": copy.deepcopy(provenance["continuity_contract"]),
                       "continuity_evidence": {key: final_facts[key] for key in ("global_max_gap_seconds", "occupied_fraction", "analysis_window_beats", "silence_intervals")},
                       "listener_verdict": "unreviewed"}
    _write(output_dir / "provenance.json", provenance)
    _write(output_dir / "score.json", result)
    progress(f"Organist study score ready: {len(result['events'])} notes, {result['duration_seconds']:.1f}s, model review {review_status}")
    return result
