//! Verifies the real Rust/Python PCM boundary without playing sound.
use std::process::Command;
use teledra_tts::{
    VoiceName,
    worker::{Synthesizer, WarmWorker},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let python = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join(".venv/Scripts/python.exe"));
    let mut command = Command::new(python);
    command
        .arg(root.join("TTS/worker/voice_worker.py"))
        .env("TELEDRA_QUEEN_VOICE", "clean");
    let mut worker = WarmWorker::start(command)?;
    for text in [
        "The silver flute carries the melody.",
        "The quiet organ leaves room for every phrase.",
    ] {
        let audio = worker.synthesize(VoiceName::Queen, text)?;
        assert_eq!(audio.sample_rate, 48_000);
        assert!(audio.samples.iter().all(|sample| sample.is_finite()));
        assert!(audio.duration_seconds() > 0.5);
        println!(
            "PCM verified: {:.3}s, {} samples",
            audio.duration_seconds(),
            audio.samples.len()
        );
    }
    worker.shutdown();
    Ok(())
}
