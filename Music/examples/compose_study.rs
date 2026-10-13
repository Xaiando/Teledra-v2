//! Write and render a complete native phrase study without starting playback.
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let input = args
        .next()
        .ok_or("pass an existing score and an output directory")?;
    let output = PathBuf::from(args.next().ok_or("pass an output directory")?);
    let reference = teledra_music::Score::from_json(&std::fs::read_to_string(input)?)?;
    let seed = reference
        .seed
        .unwrap_or(0)
        .wrapping_add(reference.evolution);
    let score = if args.next().as_deref() == Some("--layered") {
        teledra_music::compose_layered_study(&reference, seed)?
    } else {
        teledra_music::compose_chamber_study(&reference, seed)?
    };
    std::fs::create_dir_all(&output)?;
    let audio = teledra_music::VerifiedAudio::new(score.render()?)?;
    std::fs::write(
        output.join("score.json"),
        serde_json::to_vec_pretty(&score)?,
    )?;
    audio.write_wav(&output.join("study.wav"))?;
    std::fs::write(
        output.join("score-checks.json"),
        serde_json::to_vec_pretty(&score.composition_report())?,
    )?;
    std::fs::write(
        output.join("audio-checks.json"),
        serde_json::to_vec_pretty(audio.report())?,
    )?;
    std::fs::write(output.join("arrangement.txt"), score.arrangement_report())?;
    println!(
        "{}: {} bars, {} BPM, {:.1}s, peak {:.3}, RMS {:.4}",
        score.title,
        score.bars,
        score.bpm,
        audio.report().duration_seconds,
        audio.report().peak,
        audio.report().rms
    );
    println!("Score concerns: {:?}", score.orchestration_concerns());
    println!("Saved {}", output.display());
    Ok(())
}
