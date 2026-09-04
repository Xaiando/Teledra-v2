//! How many voices the engine actually sounds at once.
//!
//! Hardware and software synths allocate a fixed voice pool and steal from it.
//! This engine allocates without limit, so the question is what the ceiling
//! actually reaches in practice.

fn main() {
    let score = teledra_music::default_score();
    let report = score.voice_load_report();
    println!("{report}");
}
