//! Emits the actual per-grid-position trigger trace as JSON, including rests,
//! authored mutes, chord voice counts, and legacy arranger omissions.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let score: teledra_music::Score = match std::env::args().nth(1) {
        Some(path) => serde_json::from_str(&std::fs::read_to_string(path)?)?,
        None => teledra_music::default_score(),
    };
    score.validate()?;
    println!(
        "{}",
        serde_json::to_string_pretty(&score.note_trigger_audit())?
    );
    Ok(())
}
