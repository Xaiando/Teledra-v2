# Teledra application

This folder is the native TUI and composition root. It wires the Court to the
Qwen Brain while keeping responsibilities split:

- `main.rs`: startup and the asynchronous event loop.
- `app.rs`: testable application state and court interactions.
- `ui.rs`: rendering only.
- `terminal.rs`: raw-mode and alternate-screen lifecycle.

Domain behavior belongs in its subsystem, not in the application shell.

Public conversation keeps a bounded subject anchor across 24 accepted spoken
replies, then carries the latest thought forward. Operator speech establishes
the current focus; private work, failed inference, and withheld duplicates do
not advance that window. `/Lock <topic>` starts a 60-minute podcast plan;
`/Podcast 90m <topic>` accepts a 60–240 minute plan. Only the operator can
release the lock with `/Unlock`, which restores the earlier mode. `/Mode normal`
pauses the episode clock and bare `/Podcast` resumes. `/Podcast status` shows
elapsed active time and chapter; the episode continues beyond its target.
`podcast.rs` owns the timed twelve-part outline, bounded episode notes, and
speech generation checks. In-memory sessions last for the current app run.

YouTube video links in chat route to caption ingestion, as do `/YouTube` and
the dedicated input. Retrieved video evidence stays in bounded conversation
context, and failed ingestion is routed into an honest spoken response.

Live audience chat from Restream, Social Stream Ninja (including YouTube and
Twitch), and the stream-chat polling endpoint queues a direct spoken reply from
Teledra. The Orator is available for explicitly delegated presentations;
ordinary viewer conversation does not depend on an Orator handoff. Audience
memory and private conduct bookkeeping remain attached to each viewer.

Speakers share the last 12 public contributions, including the end of long
replies so questions survive truncation. Their turn directions vary how they
develop the current thought instead of assigning unrelated subjects. Memory
search uses the subject, and repetition guidance uses public contributions,
keeping scheduler instructions and backstage reports out of those signals.
Durable speech memory is committed when playback completes or when text
fallback makes the answer visible. Interruptions preserve only the displayed
fragment, so an unheard queued answer cannot return through later recall.
