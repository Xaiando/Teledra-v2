//! Replays a recorded court transcript through the parser.
//!
//! The parser's tests use written examples; this runs it against what the
//! model actually said on stream, which is where the discarded work was found.

use std::collections::BTreeMap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: replay <transcript.jsonl>")?;
    let raw = std::fs::read_to_string(path)?;

    let mut turns = 0_usize;
    let mut with_actions = 0_usize;
    // Replies carrying a bare JSON object are not actions here, but the runtime
    // salvages them when it asked for an artifact. Counting them separately
    // keeps this from reporting a silent court that is in fact working.
    let mut salvageable = 0_usize;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();

    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let value: serde_json::Value = serde_json::from_str(line)?;
        let Some(reply) = value.get("assistant").and_then(|v| v.as_str()) else {
            continue;
        };
        turns += 1;
        let parsed = teledra_court::parse_response(reply);
        if !parsed.actions.is_empty() {
            with_actions += 1;
        } else if reply.trim_start().starts_with(['{', '[']) || reply.contains("```json") {
            salvageable += 1;
        }
        for action in &parsed.actions {
            let name = format!("{action:?}");
            let name = name.split(['(', ' ', '{']).next().unwrap_or("?").to_owned();
            *counts.entry(name).or_default() += 1;
        }
    }

    println!(
        "{turns} turns, {with_actions} with a parsed action ({:.0}%), \
         {salvageable} more carrying a bare JSON artifact the runtime salvages",
        100.0 * with_actions as f64 / turns.max(1) as f64
    );
    for (name, count) in counts {
        println!("  {name:<20} {count}");
    }
    Ok(())
}
