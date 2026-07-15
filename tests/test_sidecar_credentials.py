"""The credential-handling contract for sidecars that read secrets.

A command line is readable by any process listing for the whole life of the
child, and it lands in crash diagnostics and parent-process telemetry. These
tests pin the sidecars that receive a secret or personal text to stdin, and pin
them against ever echoing what they received.
"""

from __future__ import annotations

import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent


def source_of(name: str) -> str:
    return (ROOT / name).read_text(encoding="utf-8")


class RestreamTokenHandlingTests(unittest.TestCase):
    def setUp(self) -> None:
        self.source = source_of("restream_listener.py")

    def test_token_is_read_from_stdin_not_the_command_line(self) -> None:
        self.assertIn("sys.stdin.readline()", self.source)
        self.assertNotIn(
            "sys.argv[1]",
            self.source,
            "the Restream token must never arrive as an argument",
        )

    def test_no_part_of_the_token_is_ever_echoed(self) -> None:
        # A six-character prefix is still credential material, and it was
        # previously written to stderr and to the system activity log.
        self.assertNotIn("token[:6]", self.source)
        # Interpolating the value, as opposed to naming it in a message.
        leaks = ("{token", "+ token", "% token", ".format(token", "str(token")
        for number, line in enumerate(self.source.splitlines(), start=1):
            if "print(" not in line:
                continue
            for leak in leaks:
                self.assertNotIn(
                    leak,
                    line,
                    f"line {number} prints the token value: {line.strip()}",
                )


class MemoryQueryHandlingTests(unittest.TestCase):
    def test_query_is_read_from_stdin(self) -> None:
        source = source_of("retrieve_memory.py")
        self.assertIn("sys.stdin.readline()", source)
        self.assertNotIn(
            "sys.argv[1]",
            source,
            "vault queries can be personal and must not sit in a process listing",
        )


class DreamConfigTests(unittest.TestCase):
    def test_dream_follows_the_courts_selected_config(self) -> None:
        source = source_of("dream.py")
        self.assertIn(
            'os.environ.get("TELEDRA_CONFIG"',
            source,
            "the dream cycle must consolidate with the model the court is running",
        )


if __name__ == "__main__":
    unittest.main()
