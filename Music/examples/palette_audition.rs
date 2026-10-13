//! Render a repeatable, separated audition of the seven additional voices.
//!
//! cargo run -p teledra-music --example palette_audition -- target/music-quality

use std::fmt::Write as _;
use std::path::PathBuf;

use teledra_music::{
    CompositionPlan, Instrument, Mode, SAMPLE_RATE, Texture, VerifiedAudio, default_score,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory: PathBuf = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "target/music-quality".to_owned())
        .into();
    std::fs::create_dir_all(&directory)?;
    let palette = [
        Instrument::Violin,
        Instrument::Clarinet,
        Instrument::Oboe,
        Instrument::Horn,
        Instrument::Trumpet,
        Instrument::Guitar,
        Instrument::Vibraphone,
    ];
    let mut samples = Vec::new();
    let mut report = String::from(
        "# Instrument palette audition\n\n\
         Seven synthesized instruments play the same C4–E4–G4 phrase at 120 BPM. \
         Each four-second slot has three equal-velocity notes, followed by a \
         quarter-second gap between instruments. The same native arrangement, \
         effects, and mastering path is used for each solo.\n\n\
         | Instrument | Start | End | Solo peak | Solo RMS |\n\
         |---|---:|---:|---:|---:|\n",
    );
    for (index, instrument) in palette.iter().copied().enumerate() {
        let mut score = default_score();
        score.title = format!("{} palette audition", instrument.label());
        score.bpm = 120;
        score.bars = 2;
        score.tonic = 0;
        score.mode = Mode::Major;
        score.progression.clear();
        score.swing = Some(0.0);
        score.sidechain = Some(0.0);
        score.seed = Some(42);
        score.composition = Some(CompositionPlan {
            texture: Texture::Chamber,
            development: 0.0,
            phrase_bars: 2,
            ..Default::default()
        });
        score.tracks.truncate(1);
        let track = &mut score.tracks[0];
        track.name = instrument.label().to_owned();
        track.instrument = Some(instrument);
        track.octave = 4;
        track.gain = 0.5;
        track.pan = Some(0.0);
        track.gate = Some(1.0);
        track.steps = vec![None; 32];
        for (step, degree) in [(0, 0), (10, 2), (20, 4)] {
            track.steps[step] = Some(degree);
        }
        track.velocities = Some(vec![0.7; 32]);
        track.durations = Some(vec![1.9; 32]);
        let solo = VerifiedAudio::new(score.render()?)?;
        let start = samples.len() as f32 / (SAMPLE_RATE * 2) as f32;
        writeln!(
            report,
            "| {} | {start:.2}s | {:.2}s | {:.3} | {:.4} |",
            instrument.label(),
            start + solo.report().duration_seconds,
            solo.report().peak,
            solo.report().rms,
        )?;
        // Score audio loops its effects. Fade the audition boundaries so
        // separate instruments enter and exit quietly beside the silent gaps.
        let mut slot = solo.samples().to_vec();
        let frames = slot.len() / 2;
        let fade = (SAMPLE_RATE as f32 * 0.025) as usize;
        for frame in 0..fade {
            let gain = frame as f32 / fade as f32;
            for channel in 0..2 {
                slot[frame * 2 + channel] *= gain;
                slot[(frames - 1 - frame) * 2 + channel] *= gain;
            }
        }
        samples.extend(slot);
        if index + 1 < palette.len() {
            samples.resize(samples.len() + SAMPLE_RATE as usize / 4 * 2, 0.0);
        }
    }
    let audition = VerifiedAudio::new(samples)?;
    let audio_path = directory.join("palette-audition.wav");
    audition.write_wav(&audio_path)?;
    writeln!(
        report,
        "\nVerified complete audition: {:.2}s, 48 kHz stereo, peak {:.3}, RMS {:.4}, \
         {}/{} active analysis sections.\n",
        audition.report().duration_seconds,
        audition.report().peak,
        audition.report().rms,
        audition.report().active_sections,
        audition.report().total_sections,
    )?;
    report.push_str(
        "Reproduce with `cargo run -p teledra-music --example palette_audition -- \
         target/music-quality`. This example writes files without starting playback.\n",
    );
    let report_path = directory.join("palette-audition.md");
    std::fs::write(&report_path, report)?;
    println!("{}", audio_path.display());
    println!("{}", report_path.display());
    Ok(())
}
