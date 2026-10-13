//! Runs real playback leveling on mono 48 kHz little-endian f32 PCM, silently.
use std::io::{Error, ErrorKind};
use teledra_tts::{Voice, VoiceName};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "usage: level_probe input.f32 output.f32 voice",
        )
        .into());
    }
    let voice: VoiceName = args[2].parse()?;
    let bytes = std::fs::read(&args[0])?;
    if bytes.is_empty() || bytes.len() % 4 != 0 {
        return Err(Error::new(ErrorKind::InvalidData, "expected nonempty f32 PCM").into());
    }
    let mut samples: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|word| f32::from_le_bytes(word.try_into().unwrap()))
        .collect();
    if samples.iter().any(|sample| !sample.is_finite()) {
        return Err(Error::new(ErrorKind::InvalidData, "PCM contains nonfinite samples").into());
    }
    Voice::new(voice).level(&mut samples);
    assert!(samples.iter().all(|sample| sample.abs() <= Voice::CEILING));
    let output: Vec<u8> = samples
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect();
    std::fs::write(&args[1], output)?;
    println!("Leveled {} samples for {voice}; no playback", samples.len());
    Ok(())
}
