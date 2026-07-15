import argparse
import hashlib
import json
import os
import queue
import subprocess
import sys
import threading
import time
import tkinter as tk
from tkinter import messagebox


ROOT = os.path.abspath(os.path.dirname(__file__))
MUSIC_PATH = os.path.join(ROOT, "music.py")
PYTHON_EXE = os.path.join(ROOT, ".venv", "Scripts", "python.exe")
KNOWLEDGE_DIR = os.path.join(ROOT, "knowledge")
EXPERIMENT_DIR = os.path.join(ROOT, "music_experiments", "editor")
MUSIC_EXPERIMENTS_PATH = os.path.join(KNOWLEDGE_DIR, "music_experiments.jsonl")
ORGANIST_VAULT_PATH = os.path.join(KNOWLEDGE_DIR, "organist_music_vault.md")


class PythonMusicEditor:
    def __init__(self, run_on_start=False):
        self.root = tk.Tk()
        self.root.title("Teledra Python Music Editor")
        self.root.geometry("980x720")
        self.root.configure(bg="#0c0418")
        self.root.protocol("WM_DELETE_WINDOW", self.close)

        self.process = None
        self.log_queue = queue.Queue()
        self.run_on_external_update = True
        self.last_music_mtime = None
        self.last_archived_hash = None

        toolbar = tk.Frame(self.root, bg="#140820")
        toolbar.pack(fill="x")

        self.run_button = tk.Button(toolbar, text="Run", command=self.run_music, bg="#1b0a2a", fg="#00e5ff")
        self.run_button.pack(side="left", padx=6, pady=6)

        self.stop_button = tk.Button(toolbar, text="Stop", command=self.stop_music, bg="#1b0a2a", fg="#ff007f")
        self.stop_button.pack(side="left", padx=6, pady=6)

        self.save_button = tk.Button(toolbar, text="Save", command=self.save_code, bg="#1b0a2a", fg="#dcd0ff")
        self.save_button.pack(side="left", padx=6, pady=6)

        self.status = tk.StringVar(value="Ready")
        tk.Label(toolbar, textvariable=self.status, bg="#140820", fg="#39ff14", font=("Consolas", 10, "bold")).pack(side="left", padx=12)

        self.editor = tk.Text(
            self.root,
            bg="#08020f",
            fg="#dcd0ff",
            insertbackground="#00e5ff",
            selectbackground="#5d3fd3",
            font=("Consolas", 10),
            undo=True,
            wrap="none",
        )
        self.editor.pack(fill="both", expand=True, padx=8, pady=(8, 4))

        log_frame = tk.Frame(self.root, bg="#050108")
        log_frame.pack(fill="x", padx=8, pady=(0, 8))
        log_scroll = tk.Scrollbar(log_frame, bg="#0c0418", troughcolor="#0c0418",
                                   activebackground="#3b1958", relief="flat")
        log_scroll.pack(side="right", fill="y")
        self.log = tk.Text(
            log_frame,
            height=8,
            bg="#050108",
            fg="#39ff14",
            insertbackground="#39ff14",
            selectbackground="#1a3a1a",
            selectforeground="#a0ff80",
            font=("Consolas", 9),
            wrap="word",
            yscrollcommand=log_scroll.set,
        )
        self.log.pack(side="left", fill="both", expand=True)
        log_scroll.config(command=self.log.yview)

        # Track mouse-button state so auto-scroll doesn't interrupt selection.
        self._user_selecting = False
        self.log.bind("<ButtonPress-1>", self._on_log_press)
        self.log.bind("<ButtonRelease-1>", self._on_log_release)

        self.load_code()
        self.remember_music_mtime()
        self.root.after(750, self.poll_external_music_update)
        if run_on_start:
            self.root.after(250, self.run_music)

    def load_code(self):
        try:
            with open(MUSIC_PATH, "r", encoding="utf-8") as f:
                code = f.read()
        except FileNotFoundError:
            code = "import numpy as np\nfrom teledra_synth import *\n\n# Compose here.\n"
        self.editor.delete("1.0", "end")
        self.editor.insert("1.0", code)

    def remember_music_mtime(self):
        try:
            self.last_music_mtime = os.path.getmtime(MUSIC_PATH)
        except OSError:
            self.last_music_mtime = None

    def save_code(self):
        code = self.editor.get("1.0", "end-1c")
        with open(MUSIC_PATH, "w", encoding="utf-8") as f:
            f.write(code.rstrip() + "\n")
        self.remember_music_mtime()
        archived_path = self.archive_experiment(code, "editor_save")
        self.status.set("Saved music.py")
        self.append_log(f"Saved {MUSIC_PATH}")
        if archived_path:
            self.append_log(f"Archived experiment: {archived_path}")

    def archive_experiment(self, code, source):
        code = code.rstrip() + "\n"
        code_hash = hashlib.sha256(code.encode("utf-8", errors="replace")).hexdigest()[:12]
        if code_hash == self.last_archived_hash:
            return None

        os.makedirs(EXPERIMENT_DIR, exist_ok=True)
        os.makedirs(KNOWLEDGE_DIR, exist_ok=True)
        timestamp = int(time.time())
        stamp = time.strftime("%Y%m%d_%H%M%S")
        safe_source = "".join(ch if ch.isalnum() or ch in ("-", "_") else "_" for ch in source)[:48]
        experiment_path = os.path.join(EXPERIMENT_DIR, f"{stamp}_{safe_source}_{code_hash}.py")

        with open(experiment_path, "w", encoding="utf-8") as handle:
            handle.write(code)

        payload = {
            "timestamp": timestamp,
            "source": source,
            "environment": "python_editor",
            "path": experiment_path,
            "code_hash": code_hash,
            "bytes": len(code.encode("utf-8", errors="replace")),
        }
        with open(MUSIC_EXPERIMENTS_PATH, "a", encoding="utf-8") as handle:
            handle.write(json.dumps(payload, ensure_ascii=True) + "\n")
        with open(ORGANIST_VAULT_PATH, "a", encoding="utf-8") as handle:
            handle.write(
                f"- [{timestamp}] Python editor experiment archived ({code_hash}) "
                f"from {source}: {experiment_path}. Mutate it before reusing it.\n"
            )

        self.last_archived_hash = code_hash
        return experiment_path

    def poll_external_music_update(self):
        try:
            mtime = os.path.getmtime(MUSIC_PATH)
        except OSError:
            self.root.after(750, self.poll_external_music_update)
            return

        if self.last_music_mtime is None:
            self.last_music_mtime = mtime
        elif mtime > self.last_music_mtime + 0.0001:
            self.last_music_mtime = mtime
            self.load_code()
            self.append_log("Reloaded external Teledra update from music.py")
            self.status.set("Reloaded music.py")
            if self.run_on_external_update:
                self.root.after(150, self.run_music)

        self.root.after(750, self.poll_external_music_update)

    def _on_log_press(self, event=None):
        self._user_selecting = True

    def _on_log_release(self, event=None):
        self._user_selecting = False

    def append_log(self, text):
        self.log.insert("end", text.rstrip() + "\n")
        # Only auto-scroll to the latest line when the user is NOT selecting
        # text (e.g. preparing a copy-paste). Suppressing during selection
        # prevents the view from jumping away from the highlighted region.
        if not self._user_selecting:
            self.log.see("end")

    def run_music(self):
        self.save_code()
        self.stop_music(update_status=False)

        python_exe = PYTHON_EXE if os.path.exists(PYTHON_EXE) else sys.executable
        try:
            self.process = subprocess.Popen(
                [python_exe, MUSIC_PATH],
                cwd=ROOT,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                bufsize=1,
            )
        except Exception as exc:
            self.status.set("Launch failed")
            messagebox.showerror("Music launch failed", str(exc))
            return

        self.status.set("Running music.py")
        self.append_log("Running music.py...")
        threading.Thread(target=self.read_process_log, daemon=True).start()
        self.root.after(100, self.poll_process)

    def read_process_log(self):
        if not self.process:
            return
        if self.process.stdout:
            for line in self.process.stdout:
                self.log_queue.put(line)

    def poll_process(self):
        while True:
            try:
                self.append_log(self.log_queue.get_nowait())
            except queue.Empty:
                break

        if not self.process:
            return
        code = self.process.poll()
        if code is None:
            self.root.after(150, self.poll_process)
        else:
            self.append_log(f"music.py exited with code {code}")
            self.status.set("Stopped" if code == 0 else f"Exited: {code}")
            self.process = None

    def stop_music(self, update_status=True):
        if self.process and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                self.process.kill()
            self.append_log("Stopped music.py")
        self.process = None
        if update_status:
            self.status.set("Stopped")

    def close(self):
        self.stop_music(update_status=False)
        self.root.destroy()

    def mainloop(self):
        self.root.mainloop()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--run", action="store_true", help="Run music.py after opening the editor.")
    parser.add_argument("--x", type=int, default=None, help="Window x anchor position")
    parser.add_argument("--y", type=int, default=None, help="Window y anchor position")
    parser.add_argument("--geometry", help="Tk geometry string e.g. 900x600+1000+50 to anchor away from Fractus")
    args = parser.parse_args()
    app = PythonMusicEditor(run_on_start=args.run)
    if args.geometry:
        app.root.geometry(args.geometry)
    elif args.x is not None and args.y is not None:
        app.root.geometry(f"900x600+{args.x}+{args.y}")
    app.mainloop()


def launch_court_synth_compat() -> int:
    """Keep old shortcuts useful without reviving the retired code editor.

    Court runtime commands already open Court Synth directly.  This adapter is
    for a person double-clicking the former Python Music Editor or an old
    desktop shortcut that still targets this file.
    """
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--run", action="store_true")
    parser.add_argument("--x", type=int)
    parser.add_argument("--y", type=int)
    parser.add_argument("--geometry")
    args, _unknown = parser.parse_known_args()
    geometry = args.geometry
    if not geometry and args.x is not None and args.y is not None:
        geometry = f"1320x820+{args.x}+{args.y}"
    command = [
        PYTHON_EXE if os.path.exists(PYTHON_EXE) else sys.executable,
        os.path.join(ROOT, "court_synthesizer.py"),
        "open",
        os.path.join(ROOT, "court_synth", "current_score.json"),
    ]
    if geometry:
        command.extend(["--geometry", geometry])
    if args.run:
        command.append("--play")
    return subprocess.call(command, cwd=ROOT)


if __name__ == "__main__":
    raise SystemExit(launch_court_synth_compat())
