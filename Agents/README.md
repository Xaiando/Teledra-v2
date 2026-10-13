# Agents

This folder owns court-member identities and their prompt profiles. It does not
call Qwen, store conversations, execute tools, or write runtime state.

Edit one file under `profiles/` to refine one member of the court without
loading the implementation details of the rest of Teledra.

Profiles specify distinct office responsibilities, complete action examples,
and honest no-action outcomes. The Court supplies recent shared conversation;
the runtime adds durable recall, execution reports and the Organist's accepted
score plus feedback. Delegations preserve backstage visibility and stop after
three hops. Night Desk prose without an executable step receives one correction.

`Teledra/tests/agent_contracts.rs` checks the concrete profile examples against
the real parsers. Its opt-in `live_offices` test exercises five offices against
the installed Qwen model without executing their actions.
