//! Renders one line in a court voice through the real warm worker to a WAV file, silently.
use std::process::Command;
use teledra_tts::{
    Voice, VoiceName,
    worker::{Synthesizer, WarmWorker},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err("usage: say_line voice \"text\" output.wav".into());
    }
    let voice: VoiceName = args[0].parse()?;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let mut command = Command::new(root.join(".venv/Scripts/python.exe"));
    command
        .arg(root.join("TTS/worker/voice_worker.py"))
        .env("TELEDRA_QUEEN_VOICE", "clean");
    let mut worker = WarmWorker::start(command)?;
    let mut audio = worker.synthesize(voice, &args[1])?;
    worker.shutdown();
    Voice::new(voice).level(&mut audio.samples);

    let data: Vec<u8> = audio
        .samples
        .iter()
        .flat_map(|sample| ((sample.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())
        .collect();
    let rate = audio.sample_rate;
    let mut wav = Vec::with_capacity(44 + data.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
    wav.extend_from_slice(&data);
    if let Some(parent) = std::path::Path::new(&args[2]).parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&args[2], wav)?;
    println!("{voice}: {:.2}s at {rate} Hz -> {}", audio.duration_seconds(), args[2]);
    Ok(())
}
