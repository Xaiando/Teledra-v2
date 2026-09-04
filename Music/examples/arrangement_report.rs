//! Prints what the arranger actually scheduled, so the audio can be checked
//! against the plan rather than against an assumption.

fn main() {
    let score = teledra_music::default_score();
    let report = score.arrangement_report();
    println!("{report}");
}
