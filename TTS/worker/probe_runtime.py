"""Silent PCM fixture for device interruption/restart checks; no model required."""
from array import array
import os
from pathlib import Path
import struct
import sys
import time

root = Path(os.environ["TELEDRA_PROBE_DIR"])
assert Path.cwd() == root
assert os.environ.get("TELEDRA_PROBE_MARKER") == "preserved"
with (root / "starts.log").open("a", encoding="utf-8") as log:
    log.write(f"{os.getpid()}\n")
print("READY", file=sys.stderr, flush=True)
for line in sys.stdin:
    _, text = line.rstrip("\n").split("\t", 1)
    (root / f"{text}.started").write_text("started", encoding="utf-8")
    if text == "crash":
        os._exit(2)
    if text == "corrupt":
        sys.stdout.buffer.write(struct.pack("<ii", 48_000, -1))
        sys.stdout.buffer.flush()
        continue
    if text == "empty":
        sys.stdout.buffer.write(struct.pack("<ii", 48_000, 0))
        sys.stdout.buffer.flush()
        continue
    if text == "slow":
        time.sleep(0.5)
    audio = array("f", [0.0]) * (48_000 * 20)
    output = sys.stdout.buffer
    output.write(struct.pack("<ii", 48_000, len(audio)))
    output.write(audio.tobytes())
    output.write(struct.pack("<i", 0))
    output.flush()
