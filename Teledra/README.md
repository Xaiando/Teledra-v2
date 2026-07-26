# Teledra application

This folder is the native TUI and composition root. It wires the Court to the
Qwen Brain while keeping responsibilities split:

- `main.rs`: startup and the asynchronous event loop.
- `app.rs`: testable application state and court interactions.
- `ui.rs`: rendering only.
- `terminal.rs`: raw-mode and alternate-screen lifecycle.

Domain behavior belongs in its subsystem, not in the application shell.
