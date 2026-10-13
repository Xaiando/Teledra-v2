# Memory

Owns durable cross-session transcript events, bounded relevance recall, and
dream consolidation. Records are append-only JSONL under the configured V2
data directory; role-local interactive histories remain small and are owned by
`Court`.

This crate does not call a model, touch the TUI, or write inside the source
tree.

Recall ranks actual answers and genuine operator input. Recognized autonomous
scheduling prompts, including quoted prior openings in repetition guidance,
are excluded from matching and from returned `User` recollections so repeated
internal instructions cannot become false evidence for the next conversation.
