# Parity gate

The replacement launcher is `D:\Teledra V2\Teledra.bat`. The installed desktop
shortcut `C:\Users\Kaged\Desktop\Teledra V2.lnk` points to that launcher and
does not alter the V1 shortcut or tree.

## Implemented root-court surface

This is the replacement gate for the sovereign court launched from the
repository root. Standalone Kraken and FrankenSwarm code surfaces are also
migrated; generated experiments remain data rather than maintained
capabilities, and external/heavy acceptance tracks are explicit in
`PARITY_GAPS.md`.

| Original capability | V2 owner |
| --- | --- |
| Qwen court, shared Teledra channel, delegated roles, autonomy, modes | `Agents`, `Court`, `Teledra/Brain`, `Teledra` |
| Original wide TUI composition and two input fields | `Teledra/src/ui.rs`, `Teledra/src/app.rs` |
| Simultaneous vertical OBS TUI via `/Vertical` | `Teledra/src/session.rs`, `Teledra/src/ui.rs` |
| Warm multi-voice TTS and named output device | `TTS` |
| Court Synth score, startup playback, hot-reload Organist DAW, keep/reject evidence | `Music`, `Teledra/src/runtime/daw.rs` |
| Static/living art and phase-animated Fractus Studio | `Fractus`, `Teledra/src/runtime/fractus.rs` |
| Cross-session memory, recall, dream consolidation | `Memory` |
| Web research and YouTube transcript ingestion | `Research`, `Sidecars` |
| Restream chat, microphone, desktop/OCR, somatic signals | `Sidecars` |
| Diplomacy approval, treasury feeds, approved MCP servers | `Teledra/src/runtime`, `Sidecars` |
| Durable missions, court-panel composition, standalone merge laboratory/taskforce, workspace verification, residents | `Swarm`, `Frankenswarm`, `Kraken`, `Tower` |
| Original off-air stream test harness commands | `Teledra/src/app.rs`, `Teledra/src/runtime/commands.rs` |

Every optional root-court capability reports unavailable state without taking
down the core court. Strict startup is available when a fully provisioned
stream setup is desired.

## Automated gate

From `D:\Teledra V2`:

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p teledra -- --check-environment
```

The current machine must have the Visual Studio C++ Build Tools workload; the
MSBuild-only installation does not provide `link.exe` or the MSVC CRT import
libraries required by Rust's Windows target.

## Live gate

In one session:

1. Start V2 from its own launcher and confirm the portrait, all panels, chat,
   and YouTube field render correctly.
2. Address Teledra and verify direct speech reaches the Queen while delegated
   court replies remain visible in the shared neural channel.
3. Confirm the right reference voice, selected OBS audio device, and continuous
   back-to-back playback.
4. Type `/Vertical`; capture the dedicated 54x48 portrait console in a 9:16
   OBS scene while continuing to operate the wide court.
5. Confirm Organist Studio and music start automatically. Run `/Art` and
   `/FractusLive`; verify the single Fractus Studio window hot-reloads the PNG
   and animated GIF, and that both artifacts have manifests.
6. Exercise configured stream chat, microphone, vision, research, YouTube,
   outreach approval, treasury, and MCP integrations.
7. Run the off-air `/Test` commands, a durable `/Mission`, `/Verify`, and
   `/Tower`.
8. Exit both views and verify no Python, MCP, music, or voice child remains.

Only then repoint the original shortcut. Keep `D:\Teledra` dormant for one
full stream as a rollback before considering any cleanup.
