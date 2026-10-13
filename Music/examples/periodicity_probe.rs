//! Measure the public audio verifier on controlled clean polyphonic fixtures.
//! No device is opened and no production score or playback is modified.
use teledra_music::{SAMPLE_RATE, verify_audio};

fn fixture(notes: &[f64], noise: f64) -> Vec<f32> {
    let frames = SAMPLE_RATE as usize * 8;
    let mut state = 0x1234_5678_u64;
    let mut samples = Vec::with_capacity(frames * 2);
    for frame in 0..frames {
        let seconds = frame as f64 / f64::from(SAMPLE_RATE);
        let envelope = (seconds / 0.05).min(1.0)
            * ((frames - 1 - frame) as f64 / (f64::from(SAMPLE_RATE) * 0.05)).min(1.0);
        let pitched = notes
            .iter()
            .map(|midi| {
                let frequency = 440.0 * 2.0_f64.powf((midi - 69.0) / 12.0);
                (std::f64::consts::TAU * frequency * seconds).sin()
            })
            .sum::<f64>()
            / notes.len().max(1) as f64;
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let white = (state >> 32) as u32 as f64 / f64::from(u32::MAX) * 2.0 - 1.0;
        let sample = (0.6 * envelope * ((1.0 - noise) * pitched + noise * white)) as f32;
        samples.extend_from_slice(&[sample, sample]);
    }
    samples
}

fn main() {
    for (label, notes, noise) in [
        ("single A4", vec![69.0], 0.0),
        ("C major triad", vec![48.0, 52.0, 55.0], 0.0),
        ("C major seventh", vec![48.0, 52.0, 55.0, 59.0], 0.0),
        ("D minor ninth", vec![50.0, 53.0, 57.0, 60.0, 64.0], 0.0),
        (
            "D dorian 6 voices",
            vec![38.0, 53.0, 57.0, 60.0, 64.0, 71.0],
            0.0,
        ),
        (
            "spread C Lydian 7 voices",
            vec![36.0, 52.0, 55.0, 59.0, 62.0, 66.0, 69.0],
            0.0,
        ),
        (
            "C major seventh with 20% noise",
            vec![48.0, 52.0, 55.0, 59.0],
            0.2,
        ),
        ("white noise", vec![], 1.0),
    ] {
        match verify_audio(&fixture(&notes, noise)) {
            Ok(report) => println!(
                "{label}: periodicity={:.4}, rms={:.4}, peak={:.4}",
                report.periodicity, report.rms, report.peak
            ),
            Err(error) => println!("{label}: verifier rejected: {error}"),
        }
    }
}
