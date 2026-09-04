# Swarm

Owns durable missions, bounded task envelopes, dependency-aware work, retry
policy, cancellation, atomic persistence, and evidence-bearing executor
contracts. It can request registered work but does not own the TUI or a model
client and cannot bypass another subsystem's validation.
