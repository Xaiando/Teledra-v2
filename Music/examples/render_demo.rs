//! Renders the default court score to a WAV, so the synthesiser can be heard
//! and measured without starting the full court.
//!
//! A score path and a variation count may follow, to hear what a live lineage
//! sounds like after the runtime has evolved it rather than only the ideal case.
//!
//! ```text
//! cargo run --release -p teledra-music --example render_demo -- court.wav
//! cargo run --release -p teledra-music --example render_demo -- live.wav current_score.json 20
//! ```

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let output: PathBuf = args
        .next()
        .unwrap_or_else(|| "court_score.wav".to_owned())
        .into();

    let mut score = match args.next() {
        Some(path) => serde_json::from_str(&std::fs::read_to_string(path)?)?,
        None => teledra_music::default_score(),
    };
    let variations: usize = args
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    for _ in 0..variations {
        score = score.evolved()?;
    }

    let audio = score.render()?;
    let report = teledra_music::verify_audio(&audio)?;
    teledra_music::write_wav(&output, &audio)?;

    println!("title            {}", score.title);
    println!("bpm/bars         {} / {}", score.bpm, score.bars);
    println!(
        "voicing          {}",
        score
            .tracks
            .iter()
            .map(|track| track.instrument().label())
            .collect::<Vec<_>>()
            .join(" ")
    );
    println!("duration         {:.1}s", report.duration_seconds);
    println!("peak             {:.3}", report.peak);
    println!("rms              {:.4}", report.rms);
    println!(
        "dynamic range    {:.2}x across eighths",
        report.dynamic_range
    );
    println!("seam delta       {:.3}", report.seam_delta);
    println!("wrote            {}", output.display());
    Ok(())
}
