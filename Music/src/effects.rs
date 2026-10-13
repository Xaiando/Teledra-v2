//! The mix bus: delay, reverb, sidechain ducking, saturation, and limiting.
//!
//! Dry synthesis sounds like a synthesiser in an anechoic room. Space is not
//! decoration — it is what tells the ear that separate parts share one place,
//! and it is most of the distance between a court score and a demo tone.

use crate::SAMPLE_RATE;

const SR: f32 = SAMPLE_RATE as f32;

/// A tempo-synced stereo delay that alternates channels.
///
/// Feedback crosses between the two lines, so each repeat lands opposite the
/// last. That is what opens the stereo field without moving the dry signal.
pub struct PingPongDelay {
    left: Vec<f32>,
    right: Vec<f32>,
    position: usize,
    feedback: f32,
}

impl PingPongDelay {
    /// A delay of one dotted eighth, the interval that stays out of the way of
    /// a four-four pulse instead of doubling it.
    pub fn dotted_eighth(bpm: u16, feedback: f32) -> Self {
        let seconds = 60.0 / f32::from(bpm) * 0.75;
        let frames = (seconds * SR) as usize;
        let frames = frames.clamp(1, SAMPLE_RATE as usize * 4);
        Self {
            left: vec![0.0; frames],
            right: vec![0.0; frames],
            position: 0,
            feedback: feedback.clamp(0.0, 0.85),
        }
    }

    pub fn process(&mut self, input: (f32, f32)) -> (f32, f32) {
        let length = self.left.len();
        let read = self.position % length;
        let (delayed_left, delayed_right) = (self.left[read], self.right[read]);
        self.left[read] = input.0 + delayed_right * self.feedback;
        self.right[read] = input.1 + delayed_left * self.feedback;
        self.position = self.position.wrapping_add(1);
        (delayed_left, delayed_right)
    }
}

/// A Schroeder reverb: parallel combs for density, series allpasses to diffuse.
///
/// The two channels use slightly different comb lengths. Identical channels
/// would produce a reverb that sits in the centre, which defeats the point.
pub struct Reverb {
    combs: Vec<Comb>,
    allpasses: Vec<Allpass>,
    damping: f32,
}

struct Comb {
    buffer: Vec<f32>,
    position: usize,
    feedback: f32,
    filtered: f32,
    channel: usize,
}

struct Allpass {
    buffer: Vec<f32>,
    position: usize,
    channel: usize,
}

impl Reverb {
    pub fn hall(size: f32) -> Self {
        // Mutually prime lengths, so repeats do not line up into a ringing
        // resonance at one pitch.
        let comb_frames = [1_557, 1_617, 1_491, 1_422, 1_277, 1_356, 1_188, 1_116];
        let allpass_frames = [225, 556, 441, 341];
        let scale = size.clamp(0.5, 2.0);
        let mut combs = Vec::new();
        for channel in 0..2 {
            for (index, base) in comb_frames.iter().enumerate() {
                // A few frames of offset per channel decorrelates the two
                // sides, which is what produces width rather than volume.
                let frames = ((*base + channel * 23) as f32 * scale) as usize;
                combs.push(Comb {
                    buffer: vec![0.0; frames.max(1)],
                    position: 0,
                    feedback: 0.82 - index as f32 * 0.012,
                    filtered: 0.0,
                    channel,
                });
            }
        }
        let mut allpasses = Vec::new();
        for channel in 0..2 {
            for base in allpass_frames {
                let frames = ((base + channel * 11) as f32 * scale) as usize;
                allpasses.push(Allpass {
                    buffer: vec![0.0; frames.max(1)],
                    position: 0,
                    channel,
                });
            }
        }
        Self {
            combs,
            allpasses,
            damping: 0.28,
        }
    }

    pub fn process(&mut self, input: (f32, f32)) -> (f32, f32) {
        let source = [input.0, input.1];
        let mut wet = [0.0_f32; 2];
        for comb in &mut self.combs {
            let read = comb.position % comb.buffer.len();
            let delayed = comb.buffer[read];
            // One-pole lowpass inside the feedback path: high frequencies die
            // away first, the way they do in any real room.
            comb.filtered = delayed * (1.0 - self.damping) + comb.filtered * self.damping;
            comb.buffer[read] = source[comb.channel] + comb.filtered * comb.feedback;
            comb.position = comb.position.wrapping_add(1);
            wet[comb.channel] += delayed;
        }
        for value in &mut wet {
            *value /= 8.0;
        }
        for allpass in &mut self.allpasses {
            let read = allpass.position % allpass.buffer.len();
            let delayed = allpass.buffer[read];
            let input = wet[allpass.channel];
            allpass.buffer[read] = input + delayed * 0.5;
            wet[allpass.channel] = delayed - input;
            allpass.position = allpass.position.wrapping_add(1);
        }
        (wet[0], wet[1])
    }
}

/// A modulated short delay, which thickens a single voice into a section.
pub struct Chorus {
    buffer: Vec<f32>,
    position: usize,
    phase: f32,
}

impl Chorus {
    pub fn new() -> Self {
        Self {
            buffer: vec![0.0; (0.04 * SR) as usize],
            position: 0,
            phase: 0.0,
        }
    }

    /// Returns the two detuned copies that become the left and right sides.
    pub fn process(&mut self, input: f32) -> (f32, f32) {
        let length = self.buffer.len();
        self.buffer[self.position % length] = input;
        self.phase = (self.phase + 0.6 / SR).fract();

        let mut voices = [0.0_f32; 2];
        for (index, voice) in voices.iter_mut().enumerate() {
            let offset = self.phase + index as f32 * 0.5;
            let depth = 0.010 + 0.006 * (offset * std::f32::consts::TAU).sin();
            let frames = depth * SR;
            let read = self.position as f32 - frames;
            *voice = self.read_interpolated(read);
        }
        self.position = self.position.wrapping_add(1);
        (voices[0], voices[1])
    }

    /// Linear interpolation, so the modulated read position does not step and
    /// produce zipper noise as it sweeps.
    fn read_interpolated(&self, position: f32) -> f32 {
        let length = self.buffer.len() as f32;
        let wrapped = position.rem_euclid(length);
        let index = wrapped.floor() as usize % self.buffer.len();
        let next = (index + 1) % self.buffer.len();
        let fraction = wrapped.fract();
        self.buffer[index] * (1.0 - fraction) + self.buffer[next] * fraction
    }
}

impl Default for Chorus {
    fn default() -> Self {
        Self::new()
    }
}

/// A ducking envelope triggered by the kick.
///
/// Pulling everything else down for the length of each kick is the single
/// clearest source of forward motion in electronic music: the mix breathes in
/// time rather than sitting still. It also buys headroom exactly where the low
/// end would otherwise collide.
pub fn sidechain_envelope(frames: usize, onsets: &[usize], bpm: u16, depth: f32) -> Vec<f32> {
    let mut envelope = vec![1.0_f32; frames];
    if onsets.is_empty() || depth <= 0.0 {
        return envelope;
    }
    // Recover over roughly half a beat, which keeps the pump locked to tempo
    // at any speed instead of sounding fast at 70 and slow at 160.
    let recovery = (60.0 / f32::from(bpm) * 0.5 * SR).max(1.0);
    let depth = depth.clamp(0.0, 0.85);
    for onset in onsets {
        let end = (*onset + recovery as usize).min(frames);
        for (offset, level) in envelope[*onset..end].iter_mut().enumerate() {
            let progress = offset as f32 / recovery;
            let shaped = progress * progress * (3.0 - 2.0 * progress);
            *level = level.min(1.0 - depth * (1.0 - shaped));
        }
    }
    envelope
}

/// Remove subsonic rumble and ultrasonic content without burying upper parts.
///
/// Patch filters provide each instrument's tone. The former fixed tilt and
/// phase-subtracted presence band reduced 2–6kHz by 12–21dB on every score,
/// masking percussion and melodic detail even in an already balanced mix.
pub fn master_tone(samples: &mut [f32]) {
    use crate::synth::{Pass, Svf};

    let mut rumble = [Svf::default(), Svf::default()];
    let mut ultrasonic = [Svf::default(), Svf::default()];
    // Warm the filters on the loop's tail so their startup transient does not
    // create a discontinuity at the wrap. No gain boost or broad spectral tilt.
    let warmup = (SR * 0.25) as usize;
    if samples.len() / 2 > warmup {
        let tail = &samples[samples.len() - warmup * 2..];
        for frame in tail.chunks_exact(2) {
            for (channel, sample) in frame.iter().enumerate() {
                let cleaned = rumble[channel].process(*sample, 30.0, 0.707, Pass::High);
                ultrasonic[channel].process(cleaned, 18_000.0, 0.707, Pass::Low);
            }
        }
    }
    for frame in samples.chunks_exact_mut(2) {
        for (channel, sample) in frame.iter_mut().enumerate() {
            let cleaned = rumble[channel].process(*sample, 30.0, 0.707, Pass::High);
            *sample = ultrasonic[channel].process(cleaned, 18_000.0, 0.707, Pass::Low);
        }
    }
}
/// Soft saturation. Gentle enough to add harmonics without audible distortion.
pub fn saturate(sample: f32, drive: f32) -> f32 {
    (sample * drive).tanh() / drive.max(1e-4).tanh()
}

/// A feed-forward limiter with a short lookahead.
///
/// Clamping samples, which is what the previous engine did, is clipping: it
/// generates broadband distortion on every peak. A limiter reduces gain before
/// the peak arrives, so loud passages stay clean.
pub fn limit(samples: &mut [f32], ceiling: f32) {
    let lookahead = (0.005 * SR) as usize;
    let release = (0.12 * SR) as usize;
    let frames = samples.len() / 2;
    if frames <= lookahead {
        return;
    }

    let mut gain = 1.0_f32;
    let release_step = 1.0 / release.max(1) as f32;
    let mut reductions = vec![1.0_f32; frames];

    for (frame, reduction) in reductions.iter_mut().enumerate() {
        let ahead = (frame + lookahead).min(frames - 1);
        let peak = samples[ahead * 2].abs().max(samples[ahead * 2 + 1].abs());
        let needed = if peak > ceiling { ceiling / peak } else { 1.0 };
        gain = if needed < gain {
            needed
        } else {
            (gain + release_step).min(needed.max(gain))
        };
        *reduction = gain.min(1.0);
    }

    // Smooth the gain curve so the reduction itself does not become audible as
    // a click at each attack.
    let window = (0.002 * SR) as usize;
    for frame in 0..frames {
        let start = frame.saturating_sub(window);
        let smallest = reductions[start..=frame]
            .iter()
            .copied()
            .fold(f32::INFINITY, f32::min);
        samples[frame * 2] *= smallest;
        samples[frame * 2 + 1] *= smallest;
    }
}

/// Attenuates an overloaded render without turning a quiet performance up.
///
/// Peak normalization erased the score's dynamics: lowering every track gain
/// made the final mix just as loud, and a sparse bell duet was boosted to the
/// same peak as a full ensemble. The master is a ceiling, not a loudness target.
pub fn attenuate_to_peak(samples: &mut [f32], target: f32) {
    let peak = samples
        .iter()
        .map(|sample| sample.abs())
        .fold(0.0_f32, f32::max);
    if peak <= target || peak <= 1e-6 {
        return;
    }
    let gain = target / peak;
    for sample in samples {
        *sample *= gain;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone_gain_db(frequency: f32) -> f32 {
        let mut samples: Vec<_> = (0..SAMPLE_RATE)
            .flat_map(|frame| {
                let value = 0.1 * (std::f32::consts::TAU * frequency * frame as f32 / SR).sin();
                [value, value]
            })
            .collect();
        let rms = |samples: &[f32]| {
            (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32)
                .sqrt()
        };
        let skip = SAMPLE_RATE as usize / 2;
        let before = rms(&samples[skip..]);
        master_tone(&mut samples);
        assert!(samples.iter().all(|sample| sample.is_finite()));
        20.0 * (rms(&samples[skip..]) / before).log10()
    }

    #[test]
    fn master_preserves_musical_layers_without_the_old_presence_notch() {
        for frequency in [100.0, 440.0, 1_000.0, 2_000.0, 3_500.0, 6_000.0, 10_000.0] {
            let change = tone_gain_db(frequency);
            assert!(
                (-0.75..0.15).contains(&change),
                "{frequency} Hz changed by {change:.2} dB"
            );
        }
    }

    #[test]
    fn master_still_removes_rumble_and_ultrasonic_energy() {
        assert!(tone_gain_db(10.0) < -15.0);
        assert!(tone_gain_db(22_000.0) < -12.0);
    }
}
