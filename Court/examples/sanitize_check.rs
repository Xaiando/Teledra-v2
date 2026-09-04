//! Runs real replies through the parser and reports what the audience would hear.
//!
//! Machine payloads leaking into the public feed is a recurring failure, and
//! reasoning about the stripper is not the same as running it. Point this at a
//! saved reply, or at the live transcript to measure every turn at once.
//!
//! ```text
//! cargo run --release -p teledra-court --example sanitize_check -- reply.txt
//! cargo run --release -p teledra-court --example sanitize_check -- court_transcript.jsonl
//! ```

/// The shapes actually observed reaching the throne room.
fn leaks(text: &str) -> bool {
    text.contains("root_degree")
        || text.contains("\"tracks\"")
        || text.contains("\"waveform\"")
        || text.contains("\"steps\"")
        || text.contains("**Waveform:**")
        || text.contains("### ")
}

fn flat(text: &str, limit: usize) -> String {
    text.chars()
        .take(limit)
        .map(|character| if character == '\n' { ' ' } else { character })
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: sanitize_check <reply.txt | transcript.jsonl>")?;

    if !path.ends_with(".jsonl") {
        let raw = std::fs::read_to_string(path)?;
        let parsed = teledra_court::parse_response(&raw);
        println!("raw {} -> visible {}", raw.len(), parsed.visible.len());
        println!("leaked  {}", leaks(&parsed.visible));
        println!("--- visible ---\n{}", parsed.visible);
        return Ok(());
    }

    let transcript = std::fs::read_to_string(&path)?;
    let (mut turns, mut before, mut after, mut emptied) = (0, 0, 0, 0);
    let mut removed = 0_usize;
    // Turns that never leaked, ordered by how much was cut: the safety question
    // is not "did the payloads go" but "did the speech survive".
    let mut innocent: Vec<(usize, String, String)> = Vec::new();

    for line in transcript.lines() {
        let Ok(record) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(raw) = record.get("assistant").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if raw.trim().is_empty() {
            continue;
        }
        turns += 1;
        let leaked_before = leaks(raw);
        if leaked_before {
            before += 1;
        }
        let parsed = teledra_court::parse_response(raw);
        if leaks(&parsed.visible) {
            after += 1;
        }
        // A turn reduced to nothing is not a win by itself: the runtime sends
        // those backstage as "acted without speaking", so count them apart.
        if parsed.visible.trim().is_empty() {
            emptied += 1;
        }
        let cut = raw.len().saturating_sub(parsed.visible.len());
        removed += cut;
        if !leaked_before && cut > 0 {
            innocent.push((cut, raw.to_owned(), parsed.visible));
        }
    }

    println!("turns examined     {turns}");
    println!("leaking before     {before}");
    println!("leaking after      {after}");
    println!("emptied entirely   {emptied}");
    println!("characters removed {removed}");

    innocent.sort_by_key(|(cut, _, _)| std::cmp::Reverse(*cut));
    println!("\ndeepest cuts on turns that never leaked:");
    for (cut, raw, visible) in innocent.iter().take(3) {
        println!("  -{cut} chars ({} -> {})", raw.len(), visible.len());
        println!("    raw:     {}", flat(raw, 140));
        println!("    visible: {}", flat(visible, 140));
    }
    Ok(())
}
