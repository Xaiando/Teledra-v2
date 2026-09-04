# Memory

Owns durable cross-session transcript events, bounded relevance recall, and
dream consolidation. Records are append-only JSONL under the configured V2
data directory; role-local interactive histories remain small and are owned by
`Court`.

This crate does not call a model, touch the TUI, or write inside the source
tree.
