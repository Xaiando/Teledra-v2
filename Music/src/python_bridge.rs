//! Persistent, device-free access to the native voice catalog from Python.
//!
//! Each request is one JSON line. A successful note response is one JSON header
//! line followed immediately by exactly `bytes` raw little-endian f32 mono
//! samples. Catalog and error responses have no binary payload. Output is
//! always 48 kHz; clients must resample rather than relabel these frames.
//!
//! `duration` is the TOTAL output length, including release, like V1's
//! `synth_note`. The native envelope stages are shortened proportionally when
//! necessary to fit that articulation. This is direct voice synthesis: no grid,
//! transposition, inferred chords, note thinning, mixing, effects, or playback.

use std::io::{self, BufRead, BufReader, BufWriter, Write};

use serde::Deserialize;

use crate::{Instrument, SAMPLE_RATE, synth::Voice};

const MAX_REQUEST_BYTES: usize = 4_096;
const MIN_DURATION: f64 = 0.001;
const MAX_DURATION: f64 = 30.0;
const MIN_FREQUENCY: f32 = 8.0;
const MAX_FREQUENCY: f32 = 20_000.0;

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Catalog,
    Note {
        instrument: Instrument,
        frequency: f32,
        duration: f64,
        #[serde(default = "default_velocity")]
        velocity: f32,
        #[serde(default)]
        seed: u32,
        #[serde(default)]
        glide_from_frequency: Option<f32>,
        #[serde(default)]
        glide_seconds: Option<f32>,
    },
}

fn default_velocity() -> f32 {
    1.0
}

/// Serve native note samples until stdin closes. This never opens an audio
/// device, reads a score, or writes files. Diagnostics belong on stderr in the
/// caller; stdout is exclusively the framed protocol described above.
pub fn run_python_note_server() -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    serve(BufReader::new(stdin.lock()), BufWriter::new(stdout.lock()))
}

fn serve(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    while let Some(line) = read_request(&mut input)? {
        let request = line.and_then(|bytes| {
            serde_json::from_slice::<Request>(&bytes)
                .map_err(|error| format!("invalid request: {error}"))
        });
        match request {
            Ok(Request::Catalog) => write_json(
                &mut output,
                &serde_json::json!({
                    "ok": true, "kind": "catalog", "protocol_version": 1,
                    "sample_rate": SAMPLE_RATE, "channels": 1,
                    "instruments": Instrument::ALL.iter().map(|instrument| instrument.info()).collect::<Vec<_>>(),
                    "limits": {"request_bytes": MAX_REQUEST_BYTES, "duration_seconds": [MIN_DURATION, MAX_DURATION],
                        "frequency_hz": [MIN_FREQUENCY, MAX_FREQUENCY]},
                    "duration_convention": "total_output_including_release",
                }),
            )?,
            Ok(request @ Request::Note { .. }) => match render_note(&request) {
                Ok(samples) => {
                    write_json(
                        &mut output,
                        &serde_json::json!({
                            "ok": true, "kind": "note", "protocol_version": 1,
                            "sample_rate": SAMPLE_RATE, "channels": 1,
                            "format": "f32le", "frames": samples.len(), "bytes": samples.len() * 4,
                        }),
                    )?;
                    for sample in samples {
                        output.write_all(&sample.to_le_bytes())?;
                    }
                }
                Err(error) => write_error(&mut output, &error)?,
            },
            Err(error) => write_error(&mut output, &error)?,
        }
        output.flush()?;
    }
    Ok(())
}

/// Drain oversize input to the next newline without allocating its full size,
/// so one bad request cannot corrupt framing for the next valid request.
fn read_request(input: &mut impl BufRead) -> io::Result<Option<Result<Vec<u8>, String>>> {
    let mut line = Vec::new();
    let mut oversized = false;
    loop {
        let buffer = input.fill_buf()?;
        if buffer.is_empty() {
            if line.is_empty() && !oversized {
                return Ok(None);
            }
            break;
        }
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let used = newline.map_or(buffer.len(), |index| index + 1);
        if !oversized {
            if line.len() + used > MAX_REQUEST_BYTES {
                oversized = true;
                line.clear();
            } else {
                line.extend_from_slice(&buffer[..used]);
            }
        }
        input.consume(used);
        if newline.is_some() {
            break;
        }
    }
    Ok(Some(if oversized {
        Err(format!("request exceeds {MAX_REQUEST_BYTES} bytes"))
    } else {
        Ok(line)
    }))
}

fn write_json(output: &mut impl Write, value: &serde_json::Value) -> io::Result<()> {
    serde_json::to_writer(&mut *output, value).map_err(io::Error::other)?;
    output.write_all(b"\n")
}

fn write_error(output: &mut impl Write, error: &str) -> io::Result<()> {
    write_json(output, &serde_json::json!({"ok": false, "error": error}))
}

fn render_note(request: &Request) -> Result<Vec<f32>, String> {
    let Request::Note {
        instrument,
        frequency,
        duration,
        velocity,
        seed,
        glide_from_frequency,
        glide_seconds,
    } = request
    else {
        return Err("expected a note request".to_owned());
    };
    if !frequency.is_finite() || !(MIN_FREQUENCY..=MAX_FREQUENCY).contains(frequency) {
        return Err(format!(
            "frequency must be {MIN_FREQUENCY}..{MAX_FREQUENCY} Hz"
        ));
    }
    if !duration.is_finite() || !(MIN_DURATION..=MAX_DURATION).contains(duration) {
        return Err(format!(
            "duration must be {MIN_DURATION}..{MAX_DURATION} total seconds"
        ));
    }
    if !velocity.is_finite() || !(0.0..=1.0).contains(velocity) {
        return Err("velocity must be 0..1".to_owned());
    }
    if glide_from_frequency
        .is_some_and(|from| !from.is_finite() || !(MIN_FREQUENCY..=MAX_FREQUENCY).contains(&from))
    {
        return Err(format!(
            "glide_from_frequency must be {MIN_FREQUENCY}..{MAX_FREQUENCY} Hz"
        ));
    }
    if glide_seconds.is_some_and(|seconds| {
        !seconds.is_finite() || !(0.0..=MAX_DURATION as f32).contains(&seconds)
    }) {
        return Err(format!("glide_seconds must be 0..{MAX_DURATION} seconds"));
    }
    let frames = (duration * f64::from(SAMPLE_RATE)) as usize;
    if *velocity == 0.0 {
        return Ok(vec![0.0; frames]);
    }
    let total_seconds = frames as f32 / SAMPLE_RATE as f32;
    let mut patch = instrument.patch();
    let envelope_seconds = patch.amp.attack + patch.amp.decay + patch.amp.release;
    let time_scale = (total_seconds / envelope_seconds.max(1e-6)).min(1.0);
    patch.amp.attack *= time_scale;
    patch.amp.decay *= time_scale;
    patch.amp.release *= time_scale;
    patch.filter.attack *= time_scale;
    patch.filter.decay *= time_scale;
    patch.filter.release *= time_scale;
    let gate_seconds = (total_seconds - patch.amp.release).max(1.0 / SAMPLE_RATE as f32);
    let mut voice = Voice::new(patch, *frequency, *velocity, gate_seconds, *seed);
    if let Some(from) = glide_from_frequency {
        voice = voice.with_glide_from(*from, glide_seconds.unwrap_or(patch.portamento_seconds));
    }
    let mut samples: Vec<_> = (0..frames).map(|_| voice.next(1.0)).collect();
    if samples.iter().any(|sample| !sample.is_finite()) {
        return Err("native note synthesis produced non-finite audio".to_owned());
    }
    // Exact total-duration arrays have no extra release sample after their
    // endpoint. Roundoff must not leave that boundary above zero.
    if let Some(last) = samples.last_mut() {
        *last = 0.0;
    }
    Ok(samples)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read};

    fn note(instrument: Instrument, frequency: f32, duration: f64) -> Request {
        Request::Note {
            instrument,
            frequency,
            duration,
            velocity: 0.7,
            seed: 47,
            glide_from_frequency: None,
            glide_seconds: None,
        }
    }

    #[test]
    fn complete_catalog_is_audible_deterministic_and_exact_length() {
        for &instrument in Instrument::ALL {
            for midi in [instrument.info().range[0], instrument.info().range[1]] {
                let frequency = 440.0 * 2.0_f32.powf((f32::from(midi) - 69.0) / 12.0);
                for duration in [0.04, 0.4] {
                    let request = note(instrument, frequency, duration);
                    let samples = render_note(&request).unwrap();
                    assert_eq!(samples.len(), (duration * f64::from(SAMPLE_RATE)) as usize);
                    assert_eq!(samples, render_note(&request).unwrap());
                    assert_eq!(samples.last(), Some(&0.0));
                    let rms = (samples
                        .iter()
                        .map(|sample| f64::from(*sample).powi(2))
                        .sum::<f64>()
                        / samples.len() as f64)
                        .sqrt();
                    assert!(
                        rms > 0.00001,
                        "{instrument:?} MIDI{midi} duration{duration} is effectively silent: {rms}"
                    );
                }
            }
        }
    }

    #[test]
    fn longer_notes_use_the_existing_native_patch_exactly() {
        for instrument in [
            Instrument::Piano,
            Instrument::Clarinet,
            Instrument::Organ,
            Instrument::Theremin,
        ] {
            let samples = render_note(&note(instrument, 440.0, 5.0)).unwrap();
            let patch = instrument.patch();
            let mut voice = Voice::new(patch, 440.0, 0.7, 5.0 - patch.amp.release, 47);
            for sample in samples.iter().take(samples.len() - 1) {
                assert_eq!(
                    *sample,
                    voice.next(1.0),
                    "{instrument:?} bypassed the native recipe"
                );
            }
        }
    }

    #[test]
    fn persistent_protocol_keeps_binary_frames_and_errors_separate() {
        let input = concat!(
            "{\"kind\":\"catalog\"}\n",
            "{\"kind\":\"note\",\"instrument\":\"theremin\",\"frequency\":440,\"duration\":0.05}\n",
            "{\"kind\":\"note\",\"instrument\":\"not_a_preset\",\"frequency\":440,\"duration\":0.05}\n",
            "{\"kind\":\"note\",\"instrument\":\"piano\",\"frequency\":440,\"duration\":0.025,\"velocity\":0}\n"
        );
        let mut output = Vec::new();
        serve(Cursor::new(input), &mut output).unwrap();
        let mut reader = Cursor::new(output);
        fn header(reader: &mut Cursor<Vec<u8>>) -> serde_json::Value {
            let mut text = String::new();
            reader.read_line(&mut text).unwrap();
            serde_json::from_str(&text).unwrap()
        }
        let catalog = header(&mut reader);
        assert_eq!(
            catalog["instruments"].as_array().unwrap().len(),
            Instrument::ALL.len()
        );
        let note_header = header(&mut reader);
        assert_eq!(note_header["frames"], 2_400);
        assert_eq!(note_header["bytes"], 9_600);
        let mut bytes = vec![0; 9_600];
        reader.read_exact(&mut bytes).unwrap();
        assert!(
            bytes
                .chunks_exact(4)
                .any(|frame| f32::from_le_bytes(frame.try_into().unwrap()).abs() > 0.0001)
        );
        assert_eq!(header(&mut reader)["ok"], false);
        assert_eq!(header(&mut reader)["frames"], 1_200);
        let mut silence = vec![1; 4_800];
        reader.read_exact(&mut silence).unwrap();
        assert!(silence.iter().all(|byte| *byte == 0));
        assert_eq!(reader.position(), reader.get_ref().len() as u64);
    }

    #[test]
    fn invalid_and_oversized_requests_do_not_poison_the_next_note() {
        let mut input = vec![b'x'; MAX_REQUEST_BYTES * 3];
        input.extend_from_slice(b"\n{\"kind\":\"note\",\"instrument\":\"piano\",\"frequency\":440,\"duration\":1000000}\n{\"kind\":\"catalog\"}\n");
        let mut output = Vec::new();
        serve(
            BufReader::with_capacity(16, Cursor::new(input)),
            &mut output,
        )
        .unwrap();
        let lines: Vec<serde_json::Value> = output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0]["ok"], false);
        assert_eq!(lines[1]["ok"], false);
        assert_eq!(lines[2]["kind"], "catalog");
    }
}
