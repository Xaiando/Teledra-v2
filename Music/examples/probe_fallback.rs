//! Renders a bare harmony-and-motif score the way the court actually sends one.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let raw = r#"{"title":"Spring Shadows Waltz","transport":{"bpm":92,"bars":16},
        "harmony":{"tonic":"C","mode":"minor","progression":["Cm","Ab","Eb","Gm"]},
        "motif":["C","Eb","G","Bb","G","Eb","C","G"]}"#;
    let score = teledra_music::Score::from_json(raw)?;
    let audio = score.render()?;
    let report = teledra_music::verify_audio(&audio)?;
    teledra_music::write_wav(
        std::path::Path::new(&std::env::args().nth(1).unwrap()),
        &audio,
    )?;
    println!("{} — {} tracks", score.title, score.tracks.len());
    println!(
        "periodicity {:.3}  dynamic range {:.2}x  crest-safe peak {:.3}",
        report.periodicity, report.dynamic_range, report.peak
    );
    Ok(())
}
