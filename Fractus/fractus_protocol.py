"""Atomic, versioned file protocol between Teledra and Fractus."""

from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
import math
import os
from pathlib import Path
import re
import time
from typing import Any, Mapping

from fractus_dsl import parse_live_code
from fractus_model import Scene, SceneError, scene_from_dict
from fractus_registry import legacy_scene, validate_scene


BASE_DIR = Path(__file__).resolve().parent
COMMAND_PATH = BASE_DIR / "fractus_command.json"
STATUS_PATH = BASE_DIR / "fractus_status.json"
STATUS_DIR = BASE_DIR / "status"
OUTPUT_DIR = BASE_DIR / "output"
COMMAND_ID = re.compile(r"^[A-Za-z0-9_.:-]{1,128}$")
MAX_COMMAND_BYTES = 256_000
TERMINAL_STATES = {
    "completed",
    "rejected",
    "cancelled",
    "superseded",
    "closing",
    "pong",
}


class ProtocolError(SceneError):
    pass


@dataclass(frozen=True)
class CommandEnvelope:
    schema_version: int
    command_id: str
    sequence: int
    action: str
    source: str
    scene: Scene | None
    persist: bool = False
    output_path: Path | None = None


def _strict_int(
    value: Any,
    field_name: str,
    *,
    minimum: int | None = None,
    maximum: int | None = None,
) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise ProtocolError(f"{field_name} must be an integer")
    if minimum is not None and value < minimum:
        raise ProtocolError(f"{field_name} must be at least {minimum}")
    if maximum is not None and value > maximum:
        raise ProtocolError(f"{field_name} must be at most {maximum}")
    return value


def _strict_float(value: Any, field_name: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ProtocolError(f"{field_name} must be numeric")
    number = float(value)
    if not math.isfinite(number):
        raise ProtocolError(f"{field_name} must be finite")
    return number


def command_file_stem(command_id: str) -> str:
    """Return a Windows-safe, non-reserved filename stem for a command id."""

    cleaned = re.sub(r"[^A-Za-z0-9_.-]", "_", command_id).strip(". ")
    digest = hashlib.sha256(command_id.encode("utf-8")).hexdigest()[:16]
    return f"cmd_{(cleaned[:72] or 'unknown')}_{digest}"


def _object_no_duplicates(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ProtocolError(f"duplicate JSON field '{key}'")
        result[key] = value
    return result


def _decode_json(raw: bytes) -> Mapping[str, Any]:
    if len(raw) > MAX_COMMAND_BYTES:
        raise ProtocolError("command exceeds the 256 KB protocol budget")
    try:
        value = json.loads(raw.decode("utf-8"), object_pairs_hook=_object_no_duplicates)
    except UnicodeDecodeError as exc:
        raise ProtocolError("command is not valid UTF-8") from exc
    except json.JSONDecodeError as exc:
        raise ProtocolError(f"command is not valid JSON: {exc.msg}") from exc
    if not isinstance(value, dict):
        raise ProtocolError("command root must be a JSON object")
    return value


def _safe_output_path(value: Any) -> Path | None:
    if value is None or value == "":
        return None
    if not isinstance(value, str):
        raise ProtocolError("output.path must be text")
    requested = Path(value)
    if not requested.is_absolute():
        requested = OUTPUT_DIR / requested
    resolved = requested.resolve()
    output_root = OUTPUT_DIR.resolve()
    try:
        resolved.relative_to(output_root)
    except ValueError as exc:
        raise ProtocolError("agent-requested output must stay inside Fractus/output") from exc
    if resolved.suffix.lower() not in {".png", ".jpg", ".jpeg", ".webp"}:
        raise ProtocolError("agent-requested output must be PNG, JPEG, or WEBP")
    return resolved


def _parse_v2(data: Mapping[str, Any], raw_hash: str) -> CommandEnvelope:
    allowed = {
        "schema_version",
        "command_id",
        "sequence",
        "action",
        "source",
        "script",
        "scene",
        "output",
    }
    unknown = sorted(set(data) - allowed)
    if unknown:
        raise ProtocolError(f"unsupported v2 command field(s): {', '.join(unknown)}")
    command_id_value = data.get("command_id", f"fractus-{raw_hash[:16]}")
    if not isinstance(command_id_value, str):
        raise ProtocolError("command_id must be text")
    command_id = command_id_value
    if not COMMAND_ID.fullmatch(command_id):
        raise ProtocolError("command_id contains unsupported characters or is too long")
    sequence = _strict_int(
        data.get("sequence", 0),
        "sequence",
        minimum=0,
        maximum=(1 << 63) - 1,
    )
    action = str(data.get("action", "apply")).strip().lower()
    if action not in {"apply", "close", "ping"}:
        raise ProtocolError("action must be apply, close, or ping")
    source = str(data.get("source", "external")).strip()[:80] or "external"
    scene: Scene | None = None
    if action == "apply":
        has_script = "script" in data
        has_scene = "scene" in data
        if has_script == has_scene:
            raise ProtocolError("apply requires exactly one of script or scene")
        if has_script:
            if not isinstance(data["script"], str):
                raise ProtocolError("script must be text")
            scene = parse_live_code(data["script"])
        else:
            if not isinstance(data["scene"], dict):
                raise ProtocolError("scene must be a JSON object")
            scene = validate_scene(scene_from_dict(data["scene"]))
    output = data.get("output", {})
    if output is None:
        output = {}
    if not isinstance(output, dict):
        raise ProtocolError("output must be a JSON object")
    unknown_output = sorted(set(output) - {"persist", "path"})
    if unknown_output:
        raise ProtocolError(
            f"unsupported output field(s): {', '.join(unknown_output)}"
        )
    persist_value = output.get("persist", False)
    if not isinstance(persist_value, bool):
        raise ProtocolError("output.persist must be true or false")
    persist = persist_value
    output_path = _safe_output_path(output.get("path"))
    if output_path is not None:
        persist = True
    return CommandEnvelope(2, command_id, sequence, action, source, scene, persist, output_path)


def _parse_v1(data: Mapping[str, Any], raw_hash: str) -> CommandEnvelope:
    allowed = {"schema_version", "type", "iterations", "palette", "c_real", "c_imag", "seed", "width", "height"}
    unknown = sorted(set(data) - allowed)
    if unknown:
        raise ProtocolError(f"unsupported legacy command field(s): {', '.join(unknown)}")
    family = data.get("type", "mandala")
    palette = data.get("palette", "purple_haze")
    if not isinstance(family, str) or not isinstance(palette, str):
        raise ProtocolError("legacy type and palette must be text")
    scene = legacy_scene(
        family=family,
        iterations=_strict_int(data.get("iterations", 180), "iterations", minimum=20, maximum=800),
        palette=palette,
        c_real=_strict_float(data.get("c_real", -0.7), "c_real"),
        c_imag=_strict_float(data.get("c_imag", 0.27015), "c_imag"),
        seed=_strict_int(data.get("seed", 1), "seed", minimum=-(1 << 63), maximum=(1 << 63) - 1),
        width=_strict_int(data.get("width", 600), "width", minimum=64, maximum=2048),
        height=_strict_int(data.get("height", 600), "height", minimum=64, maximum=2048),
    )
    return CommandEnvelope(
        1,
        f"legacy-{raw_hash[:16]}",
        0,
        "apply",
        "legacy",
        scene,
    )


def parse_command_bytes(raw: bytes) -> CommandEnvelope:
    data = _decode_json(raw)
    raw_hash = hashlib.sha256(raw).hexdigest()
    version = _strict_int(
        data.get("schema_version", 1), "schema_version", minimum=1, maximum=2
    )
    if version == 1:
        try:
            return _parse_v1(data, raw_hash)
        except ProtocolError:
            raise
        except (SceneError, TypeError, ValueError, KeyError, AttributeError) as exc:
            raise ProtocolError(f"invalid legacy scene: {exc}") from exc
    if version == 2:
        try:
            return _parse_v2(data, raw_hash)
        except ProtocolError:
            raise
        except (SceneError, TypeError, ValueError, KeyError, AttributeError) as exc:
            raise ProtocolError(f"invalid v2 scene: {exc}") from exc
    raise ProtocolError(f"unsupported command schema_version {version}")


def command_token(path: Path = COMMAND_PATH) -> str | None:
    try:
        stat = path.stat()
        if stat.st_size > MAX_COMMAND_BYTES:
            return f"oversize:{stat.st_mtime_ns}:{stat.st_size}"
        raw = path.read_bytes()
    except OSError:
        return None
    digest = hashlib.sha256(raw).hexdigest()[:20]
    return f"{stat.st_mtime_ns}:{stat.st_size}:{digest}"


def command_has_terminal_status(
    command_id: str,
    current_path: Path = STATUS_PATH,
    archive_dir: Path = STATUS_DIR,
) -> bool:
    """Prevent a completed persisted mailbox command from replaying at startup."""

    candidates = [
        current_path,
        archive_dir / f"{command_file_stem(command_id)}.json",
    ]
    for path in candidates:
        try:
            stat = path.stat()
            if stat.st_size > 64_000:
                continue
            payload = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, UnicodeError, json.JSONDecodeError, TypeError):
            continue
        if (
            isinstance(payload, dict)
            and payload.get("command_id") == command_id
            and payload.get("state") in TERMINAL_STATES
        ):
            return True
    return False


def read_changed_command(
    last_token: str | None,
    path: Path = COMMAND_PATH,
) -> tuple[str | None, CommandEnvelope | None]:
    token = command_token(path)
    if token is None or token == last_token:
        return token, None
    if token.startswith("oversize:"):
        raise ProtocolError(
            f"command exceeds the {MAX_COMMAND_BYTES:,}-byte protocol budget"
        )
    try:
        raw = path.read_bytes()
    except OSError as exc:
        raise ProtocolError(f"could not read command: {exc}") from exc
    return token, parse_command_bytes(raw)


def atomic_write_json(path: Path, payload: Mapping[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.{time.time_ns()}.tmp")
    try:
        temporary.write_text(
            json.dumps(payload, indent=2, sort_keys=True, allow_nan=False) + "\n",
            encoding="utf-8",
        )
        os.replace(temporary, path)
    except Exception:
        try:
            temporary.unlink(missing_ok=True)
        except OSError:
            pass
        raise


def write_command_atomic(payload: Mapping[str, Any], path: Path = COMMAND_PATH) -> None:
    # Parse before publishing so this helper can never emit an invalid command.
    raw = json.dumps(payload, sort_keys=True, allow_nan=False).encode("utf-8")
    parse_command_bytes(raw)
    atomic_write_json(path, payload)


def make_status(
    command_id: str,
    state: str,
    *,
    sequence: int = 0,
    source: str = "external",
    detail: str = "",
    result: Any = None,
    output_path: Path | None = None,
) -> dict[str, Any]:
    if state not in {"accepted", "rendering", "completed", "rejected", "cancelled", "superseded", "closing", "pong"}:
        raise ProtocolError(f"unsupported status state '{state}'")
    payload: dict[str, Any] = {
        "schema_version": 2,
        "command_id": command_id,
        "sequence": int(sequence),
        "source": source,
        "state": state,
        "updated_at_unix_ms": int(time.time() * 1000),
    }
    if detail:
        payload["detail"] = detail[:2_000]
    if result is not None:
        payload.update(
            {
                "recipe_hash": result.recipe_hash,
                "render_hash": result.render_hash,
                "duration_ms": result.duration_ms,
                "frame_index": result.frame_index,
                "width": result.image.width,
                "height": result.image.height,
                "metrics": result.metrics,
            }
        )
    if output_path is not None:
        payload["output_path"] = str(output_path.resolve())
    return payload


def write_status(payload: Mapping[str, Any], current_path: Path = STATUS_PATH) -> None:
    command_id = str(payload.get("command_id", "unknown"))
    if not COMMAND_ID.fullmatch(command_id):
        command_id = "invalid-command"
    atomic_write_json(current_path, payload)
    archive_dir = STATUS_DIR if current_path.resolve() == STATUS_PATH.resolve() else current_path.parent / "status"
    atomic_write_json(archive_dir / f"{command_file_stem(command_id)}.json", payload)
