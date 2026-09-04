//! Renders consecutive verified native variations and prints machine evidence.

use std::path::{Path, PathBuf};

use teledra_music::{Score, default_score, verify_audio, write_wav};

fn digest(bytes: impl IntoIterator<Item = u8>) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn difference(before: &[f32], after: &[f32]) -> (f32, f32) {
    let length = before.len().min(after.len());
    let (mut squared, mut dot, mut before_energy, mut after_energy) = (0.0, 0.0, 0.0, 0.0);
    for (before, after) in before[..length].iter().zip(&after[..length]) {
        let delta = before - after;
        squared += delta * delta;
        dot += before * after;
        before_energy += before * before;
        after_energy += after * after;
    }
    let difference_rms = (squared / length.max(1) as f32).sqrt();
    let correlation = dot / (before_energy * after_energy).sqrt().max(f32::EPSILON);
    (difference_rms, correlation)
}

fn load(path: Option<&Path>) -> Result<Score, Box<dyn std::error::Error>> {
    let Some(path) = path else {
        return Ok(default_score());
    };
    let raw = std::fs::read_to_string(path)?;
    Ok(Score::from_json(&raw)?.mastered()?)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    let score_path = arguments.first().map(PathBuf::from);
    let output = arguments
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("evolution-proof"));
    let count = arguments
        .get(2)
        .and_then(|value| value.to_str())
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(3)
        .clamp(1, 12);
    std::fs::create_dir_all(&output)?;

    let mut score = load(score_path.as_deref())?;
    let mut previous_audio = score.render()?;
    for _ in 0..count {
        score = score.evolved()?;
        let audio = score.render()?;
        let report = verify_audio(&audio)?;
        let craft = score.composition_report();
        let score_json = serde_json::to_vec_pretty(&score)?;
        let score_digest = digest(score_json.iter().copied());
        let audio_digest = digest(
            audio
                .iter()
                .flat_map(|sample| sample.to_bits().to_le_bytes()),
        );
        let (difference_rms, correlation) = difference(&previous_audio, &audio);
        let stem = format!("variation-{:06}-{score_digest}", score.evolution);
        let score_output = output.join(format!("{stem}.json"));
        let audio_output = output.join(format!("{stem}.wav"));
        std::fs::write(&score_output, score_json)?;
        write_wav(&audio_output, &audio)?;
        println!(
            "{}",
            serde_json::json!({
                "evolution": score.evolution,
                "title": score.title,
                "bpm": score.bpm,
                "tonic": score.tonic,
                "mode": score.mode,
                "movement": craft.movement,
                "craft_quality": craft.quality,
                "craft_grade": craft.grade,
                "score_digest": score_digest,
                "audio_digest": audio_digest,
                "duration_seconds": report.duration_seconds,
                "rms": report.rms,
                "peak": report.peak,
                "dynamic_range": report.dynamic_range,
                "periodicity": report.periodicity,
                "difference_rms": difference_rms,
                "correlation_to_previous": correlation,
                "score_path": score_output,
                "audio_path": audio_output,
            })
        );
        previous_audio = audio;
    }
    Ok(())
}
