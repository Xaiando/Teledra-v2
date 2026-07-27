//! The wire format between the court and its voice worker.
//!
//! A synthesiser streams audio as it is produced, so the court can begin
//! playing before the last word exists. The framing is deliberately dull:
//!
//! ```text
//! i32  sample rate
//! i32  sample count, then that many f32 samples   (repeated)
//! i32  0                                          (end of utterance)
//! ```
//!
//! All integers and samples are little endian. A zero count is the terminator
//! and never a frame, so an empty utterance is expressible and a truncated
//! stream is always detectable rather than being mistaken for silence.

use std::io::Read;

use thiserror::Error;

/// Bounds that make a malformed or hostile stream fail fast instead of
/// allocating without limit.
pub const MIN_SAMPLE_RATE: i32 = 8_000;
pub const MAX_SAMPLE_RATE: i32 = 192_000;
pub const MAX_FRAME_SECONDS: i32 = 90;
pub const MAX_UTTERANCE_SECONDS: i32 = 600;

#[derive(Debug, Error)]
pub enum PcmError {
    #[error("the voice ended before the sample-rate header ({read} of 4 bytes)")]
    TruncatedHeader { read: usize },
    #[error("the voice ended mid-stream after {samples} samples")]
    TruncatedStream { samples: usize },
    #[error("sample rate {0} is outside the supported range")]
    UnsupportedSampleRate(i32),
    #[error("a frame declared {0} samples, which is not a valid count")]
    InvalidFrameLength(i32),
    #[error("a frame of {samples} samples exceeds the {limit} sample ceiling")]
    FrameTooLong { samples: i32, limit: i32 },
    #[error("the utterance reached {samples} samples, beyond the {limit} ceiling")]
    UtteranceTooLong { samples: usize, limit: usize },
    #[error("a sample was not a finite number")]
    NonFiniteSample,
    #[error("could not read from the voice: {0}")]
    Io(#[from] std::io::Error),
}

/// A decoded utterance: the audio and the rate it was rendered at.
#[derive(Clone, Debug, PartialEq)]
pub struct Audio {
    pub sample_rate: i32,
    pub samples: Vec<f32>,
}

impl Audio {
    pub fn duration_seconds(&self) -> f64 {
        if self.sample_rate <= 0 {
            return 0.0;
        }
        self.samples.len() as f64 / f64::from(self.sample_rate)
    }

    pub fn is_silent(&self) -> bool {
        self.samples.is_empty()
    }
}

/// Reads one complete utterance, frame by frame, from a streaming source.
///
/// The reader is consumed up to and including the end marker, leaving the
/// stream positioned for the next utterance — which is what allows one warm
/// worker to serve many requests over a single pipe.
pub fn read_utterance<R: Read>(reader: &mut R) -> Result<Audio, PcmError> {
    let sample_rate = read_i32_exact(reader).map_err(|error| match error {
        ShortRead::Eof { read } => PcmError::TruncatedHeader { read },
        ShortRead::Io(io) => PcmError::Io(io),
    })?;
    if !(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&sample_rate) {
        return Err(PcmError::UnsupportedSampleRate(sample_rate));
    }

    let frame_limit = sample_rate.saturating_mul(MAX_FRAME_SECONDS);
    let utterance_limit = (sample_rate as i64 * i64::from(MAX_UTTERANCE_SECONDS)) as usize;
    let mut samples: Vec<f32> = Vec::new();

    loop {
        let count = read_i32_exact(reader).map_err(|error| match error {
            ShortRead::Eof { .. } => PcmError::TruncatedStream {
                samples: samples.len(),
            },
            ShortRead::Io(io) => PcmError::Io(io),
        })?;

        if count == 0 {
            return Ok(Audio {
                sample_rate,
                samples,
            });
        }
        if count < 0 {
            return Err(PcmError::InvalidFrameLength(count));
        }
        if count > frame_limit {
            return Err(PcmError::FrameTooLong {
                samples: count,
                limit: frame_limit,
            });
        }
        if samples.len() + count as usize > utterance_limit {
            return Err(PcmError::UtteranceTooLong {
                samples: samples.len() + count as usize,
                limit: utterance_limit,
            });
        }

        let mut frame = vec![0u8; count as usize * 4];
        read_filled(reader, &mut frame).map_err(|error| match error {
            ShortRead::Eof { .. } => PcmError::TruncatedStream {
                samples: samples.len(),
            },
            ShortRead::Io(io) => PcmError::Io(io),
        })?;
        for chunk in frame.chunks_exact(4) {
            let sample = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            if !sample.is_finite() {
                // A NaN reaching the audio device is a click at best.
                return Err(PcmError::NonFiniteSample);
            }
            samples.push(sample);
        }
    }
}

enum ShortRead {
    Eof { read: usize },
    Io(std::io::Error),
}

fn read_i32_exact<R: Read>(reader: &mut R) -> Result<i32, ShortRead> {
    let mut bytes = [0u8; 4];
    read_filled(reader, &mut bytes)?;
    Ok(i32::from_le_bytes(bytes))
}

/// Reads until the buffer is full, distinguishing a clean end of stream from a
/// truncated one.
fn read_filled<R: Read>(reader: &mut R, buffer: &mut [u8]) -> Result<(), ShortRead> {
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..]) {
            Ok(0) => return Err(ShortRead::Eof { read: filled }),
            Ok(count) => filled += count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(ShortRead::Io(error)),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(sample_rate: i32, frames: &[&[f32]]) -> Vec<u8> {
        let mut bytes = sample_rate.to_le_bytes().to_vec();
        for frame in frames {
            bytes.extend((frame.len() as i32).to_le_bytes());
            for sample in *frame {
                bytes.extend(sample.to_le_bytes());
            }
        }
        bytes.extend(0i32.to_le_bytes());
        bytes
    }

    #[test]
    fn frames_are_joined_into_one_utterance() {
        let encoded = encode(48_000, &[&[0.0, 0.5], &[-0.5, 1.0]]);
        let audio = read_utterance(&mut encoded.as_slice()).expect("a well-formed utterance");
        assert_eq!(audio.sample_rate, 48_000);
        assert_eq!(audio.samples, vec![0.0, 0.5, -0.5, 1.0]);
    }

    #[test]
    fn an_empty_utterance_is_expressible() {
        let encoded = encode(48_000, &[]);
        let audio = read_utterance(&mut encoded.as_slice()).expect("silence is legal");
        assert!(audio.is_silent());
        assert_eq!(audio.duration_seconds(), 0.0);
    }

    #[test]
    fn one_stream_carries_utterances_back_to_back() {
        // This is what lets a single warm worker serve the whole session.
        let mut stream = encode(48_000, &[&[0.25]]);
        stream.extend(encode(48_000, &[&[0.75, 0.5]]));
        let mut cursor = stream.as_slice();

        let first = read_utterance(&mut cursor).expect("first utterance");
        let second = read_utterance(&mut cursor).expect("second utterance");
        assert_eq!(first.samples, vec![0.25]);
        assert_eq!(second.samples, vec![0.75, 0.5]);
        assert!(cursor.is_empty(), "the stream is fully consumed");
    }

    #[test]
    fn a_worker_that_dies_is_never_mistaken_for_silence() {
        // A crashed synthesiser must surface as an error the court can report,
        // not as a valid empty utterance that plays as a mysterious gap.
        let died_before_header = [0u8; 2];
        assert!(matches!(
            read_utterance(&mut died_before_header.as_slice()),
            Err(PcmError::TruncatedHeader { read: 2 })
        ));

        let mut died_mid_frame = 48_000i32.to_le_bytes().to_vec();
        died_mid_frame.extend(4i32.to_le_bytes());
        died_mid_frame.extend(0.5f32.to_le_bytes());
        assert!(matches!(
            read_utterance(&mut died_mid_frame.as_slice()),
            Err(PcmError::TruncatedStream { .. })
        ));
    }

    #[test]
    fn implausible_framing_is_refused_before_allocating() {
        let mut absurd_frame = 48_000i32.to_le_bytes().to_vec();
        absurd_frame.extend(i32::MAX.to_le_bytes());
        assert!(matches!(
            read_utterance(&mut absurd_frame.as_slice()),
            Err(PcmError::FrameTooLong { .. })
        ));

        let mut negative = 48_000i32.to_le_bytes().to_vec();
        negative.extend((-7i32).to_le_bytes());
        assert!(matches!(
            read_utterance(&mut negative.as_slice()),
            Err(PcmError::InvalidFrameLength(-7))
        ));

        let bad_rate = encode(2, &[&[0.1]]);
        assert!(matches!(
            read_utterance(&mut bad_rate.as_slice()),
            Err(PcmError::UnsupportedSampleRate(2))
        ));
    }

    #[test]
    fn a_non_finite_sample_never_reaches_the_speakers() {
        let encoded = encode(48_000, &[&[0.1, f32::NAN]]);
        assert!(matches!(
            read_utterance(&mut encoded.as_slice()),
            Err(PcmError::NonFiniteSample)
        ));
    }

    #[test]
    fn duration_is_reported_from_the_sample_count() {
        let audio = Audio {
            sample_rate: 48_000,
            samples: vec![0.0; 24_000],
        };
        assert!((audio.duration_seconds() - 0.5).abs() < f64::EPSILON);
    }
}
