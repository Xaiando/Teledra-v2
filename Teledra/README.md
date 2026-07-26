# Teledra application

This folder is the native application shell and composition root. It wires the
Court to the Qwen Brain and owns operator-facing commands. Domain behavior
belongs in its subsystem, not in `main.rs`.

