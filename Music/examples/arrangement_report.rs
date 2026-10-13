//! Prints what the arranger actually scheduled, so the audio can be checked
//! against the plan rather than against an assumption.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let score = match std::env::args().nth(1) {
        // Point it at the live score to see what the court is actually playing.
        Some(path) => serde_json::from_str(&std::fs::read_to_string(path)?)?,
        None => teledra_music::default_score(),
    };
    let report = score.arrangement_report();
    println!("{report}");
    Ok(())
}
