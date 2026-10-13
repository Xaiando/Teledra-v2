"""Instrument the real hidden Tk workstation for Rust operational checks."""
import json
import os
from pathlib import Path
import time
import tkinter as tk

from daw import OrganistStudio


def main(state_path):
    state_path = Path(state_path)

    class ProbeStudio(OrganistStudio):
        def apply_command(self, command):
            if command.get("action") == "probe_close":
                self.close()
                return
            if command.get("action") == "probe_crash":
                os._exit(2)
            super().apply_command(command)
            if self.closing:
                return
            self.received = getattr(self, "received", 0) + 1
            temporary = state_path.with_suffix(".tmp")
            snapshot = json.dumps({
                "pid": os.getpid(), "transport": self.transport,
                "cadence": self.cadence_seconds, "score": self.score,
                "position": self.position_seconds, "commands": self.received,
            })
            # A polling reader or scanner can briefly hold a Windows handle
            # that blocks replacement. Keep the atomic snapshot, but do not
            # let that transient probe-only failure stop Tk's command loop.
            for attempt in range(50):
                try:
                    temporary.write_text(snapshot, encoding="utf-8")
                    temporary.replace(state_path)
                    break
                except PermissionError:
                    if attempt == 49:
                        raise
                    time.sleep(0.01)

    root = tk.Tk()
    root.withdraw()
    ProbeStudio(root)
    root.mainloop()
