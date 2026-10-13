import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


class LifetimeTests(unittest.TestCase):
    def test_duplicate_does_not_kill_owner_and_crash_releases_lock(self):
        module = str(Path(__file__).resolve().parent)
        code = "import sys; from pathlib import Path; sys.path.insert(0, sys.argv[1]); from studio_lifetime import claim_window_lock; claim_window_lock(Path(sys.argv[2])); print('OWNER', flush=True); sys.stdin.readline()"
        with tempfile.TemporaryDirectory(prefix="teledra-studio-lock-") as directory:
            command = [sys.executable, "-c", code, module, str(Path(directory) / "studio.lock")]
            flags = subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0
            owner = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, creationflags=flags)
            try:
                self.assertEqual(owner.stdout.readline().strip(), b"OWNER")
                duplicate = subprocess.run(command, input=b"\n", capture_output=True, timeout=5, creationflags=flags)
                self.assertEqual(duplicate.returncode, 0)
                self.assertEqual(duplicate.stdout, b"")
                self.assertIsNone(owner.poll())
                owner.kill()
                owner.wait(timeout=5)
                replacement = subprocess.run(command, input=b"\n", capture_output=True, timeout=5, creationflags=flags)
                self.assertEqual(replacement.returncode, 0)
                self.assertEqual(replacement.stdout.strip(), b"OWNER")
            finally:
                if owner.poll() is None:
                    owner.kill()
                    owner.wait(timeout=5)
                owner.stdin.close()
                owner.stdout.close()
