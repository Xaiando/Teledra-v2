//! Instrument-inspired synthesis recipes. These are deliberately synthesized,
//! not recordings: each voice specifies its overtone balance and articulation.

use super::{Adsr, Instrument, Patch, Waveform};

const STRUCK: Patch = Patch {
    waveform: Waveform::Sine,
    amp: Adsr::new(0.002, 1.3, 0.0, 0.24),
    filter: Adsr::new(0.002, 0.30, 0.12, 0.22),
    partial_decay_seconds: 0.45,
    cutoff_hz: 2_100.0,
    env_octaves: 1.0,
    resonance: 0.7,
    drive: 0.8,
    width: 0.40,
    reverb: 0.20,
    delay: 0.05,
    ..Patch::DEFAULT
};

const PLUCKED: Patch = Patch {
    noise: 0.035,
    noise_decay: 0.007,
    cutoff_hz: 1_500.0,
    env_octaves: 1.4,
    partial_decay_seconds: 0.22,
    reverb: 0.16,
    ..STRUCK
};

const WIND: Patch = Patch {
    waveform: Waveform::Sine,
    amp: Adsr::new(0.045, 0.18, 0.82, 0.14),
    filter: Adsr::new(0.055, 0.25, 0.65, 0.14),
    noise: 0.025,
    noise_decay: 0.25,
    cutoff_hz: 1_600.0,
    env_octaves: 0.6,
    resonance: 0.7,
    drive: 0.75,
    width: 0.30,
    reverb: 0.22,
    delay: 0.04,
    ..Patch::DEFAULT
};

const BOWED: Patch = Patch {
    amp: Adsr::new(0.12, 0.26, 0.80, 0.24),
    filter: Adsr::new(0.15, 0.32, 0.70, 0.24),
    noise: 0.01,
    noise_decay: 0.18,
    vibrato_cents: 7.0,
    vibrato_hz: 5.0,
    vibrato_delay: 0.35,
    reverb: 0.25,
    ..WIND
};

const BRASS: Patch = Patch {
    amp: Adsr::new(0.060, 0.20, 0.78, 0.16),
    filter: Adsr::new(0.080, 0.28, 0.58, 0.16),
    noise: 0.018,
    noise_decay: 0.055,
    cutoff_hz: 1_100.0,
    env_octaves: 1.0,
    reverb: 0.20,
    ..WIND
};

/// Relative strengths of the second through seventh harmonics. A strong
/// fundamental remains underneath so orchestral colours preserve their pitch.
const fn harmonics(levels: [f32; 6]) -> [(f32, f32); 6] {
    [
        (2.0, levels[0]),
        (3.0, levels[1]),
        (4.0, levels[2]),
        (5.0, levels[3]),
        (6.0, levels[4]),
        (7.0, levels[5]),
    ]
}

pub(super) fn patch(instrument: Instrument) -> Patch {
    use Instrument::*;
    match instrument {
        // Keyboards: tine strike, paired strings, plectrum, tangent, and metal
        // plates have different spectra and very different decay behaviour.
        ElectricPiano => Patch {
            partials: [
                (2.0, 0.17),
                (3.0, 0.08),
                (7.0, 0.12),
                (14.0, 0.025),
                (0.0, 0.0),
                (0.0, 0.0),
            ],
            amp: Adsr::new(0.003, 2.1, 0.0, 0.32),
            partial_decay_seconds: 0.24,
            cutoff_hz: 2_600.0,
            tremolo_depth: 0.10,
            tremolo_hz: 4.1,
            width: 0.50,
            ..STRUCK
        },
        HonkyTonkPiano => Patch {
            waveform: Waveform::Triangle,
            voices: 2,
            detune_cents: 5.0,
            partials: harmonics([0.38, 0.23, 0.14, 0.08, 0.04, 0.02]),
            amp: Adsr::new(0.002, 1.35, 0.0, 0.25),
            partial_decay_seconds: 0.38,
            noise: 0.020,
            noise_decay: 0.004,
            cutoff_hz: 1_800.0,
            width: 0.52,
            ..STRUCK
        },
        Harpsichord => Patch {
            partials: harmonics([0.65, 0.42, 0.28, 0.18, 0.13, 0.08]),
            amp: Adsr::new(0.001, 0.90, 0.0, 0.12),
            filter: Adsr::new(0.001, 0.32, 0.10, 0.10),
            partial_decay_seconds: 0.32,
            cutoff_hz: 2_400.0,
            noise: 0.055,
            noise_decay: 0.004,
            ..PLUCKED
        },
        Clavinet => Patch {
            partials: harmonics([0.13, 0.58, 0.10, 0.29, 0.04, 0.11]),
            amp: Adsr::new(0.001, 0.58, 0.0, 0.08),
            filter: Adsr::new(0.001, 0.12, 0.05, 0.08),
            partial_decay_seconds: 0.18,
            cutoff_hz: 1_500.0,
            resonance: 0.95,
            drive: 1.1,
            ..PLUCKED
        },
        Celesta => Patch {
            partials: [
                (2.0, 0.08),
                (4.0, 0.29),
                (6.0, 0.045),
                (10.0, 0.018),
                (0.0, 0.0),
                (0.0, 0.0),
            ],
            amp: Adsr::new(0.003, 1.9, 0.0, 0.38),
            partial_decay_seconds: 0.55,
            cutoff_hz: 3_800.0,
            reverb: 0.25,
            width: 0.58,
            ..STRUCK
        },
        Accordion => Patch {
            voices: 2,
            detune_cents: 3.5,
            partials: harmonics([0.49, 0.31, 0.17, 0.12, 0.08, 0.05]),
            amp: Adsr::new(0.045, 0.10, 0.93, 0.11),
            cutoff_hz: 2_100.0,
            env_octaves: 0.4,
            noise: 0.012,
            width: 0.43,
            ..WIND
        },
        Harmonica => Patch {
            partials: harmonics([0.35, 0.51, 0.19, 0.27, 0.11, 0.13]),
            amp: Adsr::new(0.035, 0.16, 0.80, 0.09),
            cutoff_hz: 2_350.0,
            noise: 0.040,
            noise_decay: 0.40,
            tremolo_depth: 0.06,
            tremolo_hz: 4.8,
            vibrato_cents: 6.0,
            vibrato_hz: 4.8,
            vibrato_delay: 0.35,
            ..WIND
        },

        Viola => Patch {
            partials: harmonics([0.62, 0.43, 0.23, 0.14, 0.075, 0.035]),
            amp: Adsr::new(0.10, 0.25, 0.79, 0.23),
            cutoff_hz: 1_200.0,
            env_octaves: 0.85,
            vibrato_hz: 5.2,
            ..BOWED
        },
        DoubleBass => Patch {
            partials: harmonics([0.72, 0.38, 0.15, 0.08, 0.04, 0.02]),
            amp: Adsr::new(0.14, 0.30, 0.77, 0.26),
            cutoff_hz: 480.0,
            env_octaves: 0.9,
            vibrato_cents: 3.5,
            vibrato_hz: 4.3,
            width: 0.17,
            reverb: 0.16,
            ..BOWED
        },
        StringEnsemble => Patch {
            voices: 3,
            detune_cents: 3.0,
            partials: harmonics([0.51, 0.33, 0.22, 0.15, 0.08, 0.06]),
            amp: Adsr::new(0.24, 0.45, 0.81, 0.40),
            filter: Adsr::new(0.30, 0.40, 0.75, 0.40),
            cutoff_hz: 1_350.0,
            vibrato_cents: 3.0,
            vibrato_hz: 4.7,
            width: 0.83,
            reverb: 0.28,
            ..BOWED
        },
        PizzicatoStrings => Patch {
            partials: harmonics([0.52, 0.27, 0.14, 0.06, 0.025, 0.01]),
            amp: Adsr::new(0.004, 0.55, 0.0, 0.13),
            partial_decay_seconds: 0.16,
            cutoff_hz: 1_000.0,
            noise: 0.020,
            width: 0.53,
            ..PLUCKED
        },
        TremoloStrings => Patch {
            partials: harmonics([0.56, 0.38, 0.25, 0.15, 0.09, 0.055]),
            amp: Adsr::new(0.09, 0.18, 0.86, 0.20),
            cutoff_hz: 1_700.0,
            tremolo_depth: 0.36,
            tremolo_hz: 9.0,
            vibrato_cents: 3.0,
            width: 0.64,
            ..BOWED
        },

        SteelGuitar => Patch {
            partials: harmonics([0.54, 0.37, 0.24, 0.17, 0.10, 0.07]),
            amp: Adsr::new(0.002, 1.65, 0.0, 0.29),
            partial_decay_seconds: 0.48,
            cutoff_hz: 2_000.0,
            noise: 0.055,
            noise_decay: 0.006,
            ..PLUCKED
        },
        ElectricGuitar => Patch {
            partials: harmonics([0.30, 0.38, 0.14, 0.21, 0.08, 0.10]),
            amp: Adsr::new(0.004, 1.9, 0.0, 0.21),
            partial_decay_seconds: 0.65,
            cutoff_hz: 2_100.0,
            drive: 1.25,
            noise: 0.020,
            delay: 0.10,
            ..PLUCKED
        },
        JazzGuitar => Patch {
            partials: harmonics([0.46, 0.17, 0.075, 0.035, 0.015, 0.005]),
            amp: Adsr::new(0.007, 1.10, 0.0, 0.15),
            partial_decay_seconds: 0.31,
            cutoff_hz: 850.0,
            env_octaves: 1.0,
            noise: 0.014,
            drive: 0.78,
            ..PLUCKED
        },
        MutedGuitar => Patch {
            partials: harmonics([0.63, 0.33, 0.14, 0.08, 0.03, 0.01]),
            amp: Adsr::new(0.002, 0.22, 0.0, 0.045),
            filter: Adsr::new(0.001, 0.06, 0.0, 0.04),
            partial_decay_seconds: 0.055,
            cutoff_hz: 1_200.0,
            noise: 0.075,
            noise_decay: 0.012,
            reverb: 0.08,
            ..PLUCKED
        },
        Banjo => Patch {
            partials: harmonics([0.76, 0.45, 0.26, 0.17, 0.10, 0.07]),
            amp: Adsr::new(0.001, 0.60, 0.0, 0.095),
            partial_decay_seconds: 0.16,
            cutoff_hz: 2_700.0,
            noise: 0.075,
            noise_decay: 0.008,
            resonance: 0.85,
            reverb: 0.12,
            ..PLUCKED
        },
        Mandolin => Patch {
            voices: 2,
            detune_cents: 2.5,
            partials: harmonics([0.47, 0.40, 0.25, 0.16, 0.11, 0.065]),
            amp: Adsr::new(0.002, 0.74, 0.0, 0.11),
            partial_decay_seconds: 0.21,
            cutoff_hz: 2_350.0,
            noise: 0.05,
            width: 0.48,
            ..PLUCKED
        },
        Ukulele => Patch {
            partials: harmonics([0.34, 0.17, 0.065, 0.03, 0.012, 0.005]),
            amp: Adsr::new(0.003, 0.66, 0.0, 0.10),
            partial_decay_seconds: 0.19,
            cutoff_hz: 1_350.0,
            env_octaves: 1.0,
            noise: 0.025,
            reverb: 0.12,
            ..PLUCKED
        },
        Sitar => Patch {
            partials: harmonics([0.57, 0.53, 0.36, 0.26, 0.18, 0.13]),
            amp: Adsr::new(0.002, 1.8, 0.0, 0.40),
            filter: Adsr::new(0.002, 0.58, 0.18, 0.28),
            partial_decay_seconds: 0.70,
            cutoff_hz: 2_600.0,
            resonance: 1.0,
            noise: 0.065,
            noise_decay: 0.055,
            reverb: 0.25,
            delay: 0.12,
            ..PLUCKED
        },
        Koto => Patch {
            partials: harmonics([0.61, 0.29, 0.19, 0.12, 0.06, 0.035]),
            amp: Adsr::new(0.001, 1.45, 0.0, 0.32),
            partial_decay_seconds: 0.31,
            cutoff_hz: 2_100.0,
            noise: 0.070,
            noise_decay: 0.011,
            reverb: 0.21,
            ..PLUCKED
        },
        Oud => Patch {
            partials: harmonics([0.60, 0.31, 0.12, 0.065, 0.025, 0.015]),
            amp: Adsr::new(0.003, 0.98, 0.0, 0.17),
            partial_decay_seconds: 0.25,
            cutoff_hz: 1_050.0,
            env_octaves: 1.3,
            noise: 0.045,
            noise_decay: 0.014,
            resonance: 0.78,
            ..PLUCKED
        },

        Piccolo => Patch {
            partials: harmonics([0.19, 0.075, 0.035, 0.01, 0.0, 0.0]),
            amp: Adsr::new(0.028, 0.13, 0.83, 0.10),
            cutoff_hz: 4_000.0,
            noise: 0.035,
            vibrato_cents: 5.0,
            vibrato_hz: 5.6,
            vibrato_delay: 0.30,
            ..WIND
        },
        Recorder => Patch {
            partials: harmonics([0.045, 0.10, 0.015, 0.024, 0.0, 0.005]),
            amp: Adsr::new(0.035, 0.15, 0.87, 0.09),
            cutoff_hz: 2_650.0,
            noise: 0.027,
            noise_decay: 0.18,
            ..WIND
        },
        PanFlute => Patch {
            partials: harmonics([0.22, 0.09, 0.028, 0.012, 0.0, 0.0]),
            amp: Adsr::new(0.080, 0.22, 0.74, 0.17),
            cutoff_hz: 2_300.0,
            noise: 0.14,
            noise_decay: 0.16,
            vibrato_cents: 7.0,
            vibrato_hz: 4.7,
            vibrato_delay: 0.35,
            reverb: 0.29,
            ..WIND
        },
        Shakuhachi => Patch {
            partials: harmonics([0.31, 0.15, 0.065, 0.025, 0.01, 0.0]),
            amp: Adsr::new(0.105, 0.30, 0.70, 0.22),
            filter: Adsr::new(0.10, 0.40, 0.50, 0.21),
            cutoff_hz: 1_450.0,
            noise: 0.19,
            noise_decay: 0.40,
            vibrato_cents: 8.0,
            vibrato_hz: 4.3,
            vibrato_delay: 0.40,
            reverb: 0.31,
            ..WIND
        },
        Bassoon => Patch {
            partials: harmonics([0.42, 0.56, 0.27, 0.22, 0.10, 0.06]),
            amp: Adsr::new(0.065, 0.20, 0.81, 0.16),
            cutoff_hz: 1_050.0,
            env_octaves: 0.8,
            noise: 0.02,
            vibrato_cents: 3.0,
            vibrato_hz: 4.8,
            vibrato_delay: 0.40,
            ..WIND
        },
        EnglishHorn => Patch {
            partials: harmonics([0.64, 0.42, 0.22, 0.13, 0.065, 0.03]),
            amp: Adsr::new(0.065, 0.18, 0.80, 0.16),
            cutoff_hz: 1_350.0,
            env_octaves: 0.70,
            vibrato_cents: 4.0,
            vibrato_hz: 4.9,
            vibrato_delay: 0.37,
            ..WIND
        },
        SopranoSax => Patch {
            partials: harmonics([0.59, 0.44, 0.29, 0.20, 0.12, 0.08]),
            amp: Adsr::new(0.030, 0.19, 0.81, 0.12),
            cutoff_hz: 2_100.0,
            env_octaves: 0.80,
            noise: 0.045,
            noise_decay: 0.22,
            vibrato_cents: 6.0,
            vibrato_hz: 5.4,
            vibrato_delay: 0.34,
            ..WIND
        },
        TenorSax => Patch {
            partials: harmonics([0.71, 0.47, 0.24, 0.16, 0.08, 0.045]),
            amp: Adsr::new(0.045, 0.22, 0.79, 0.16),
            cutoff_hz: 1_200.0,
            env_octaves: 0.95,
            noise: 0.055,
            noise_decay: 0.28,
            vibrato_cents: 5.0,
            vibrato_hz: 5.0,
            vibrato_delay: 0.36,
            drive: 0.87,
            ..WIND
        },

        Trombone => Patch {
            partials: harmonics([0.69, 0.46, 0.27, 0.17, 0.09, 0.05]),
            amp: Adsr::new(0.065, 0.21, 0.81, 0.17),
            cutoff_hz: 1_050.0,
            env_octaves: 1.10,
            ..BRASS
        },
        Tuba => Patch {
            partials: harmonics([0.63, 0.31, 0.13, 0.055, 0.025, 0.01]),
            amp: Adsr::new(0.11, 0.25, 0.83, 0.22),
            filter: Adsr::new(0.13, 0.30, 0.55, 0.21),
            cutoff_hz: 520.0,
            env_octaves: 0.85,
            noise: 0.010,
            width: 0.17,
            ..BRASS
        },
        Flugelhorn => Patch {
            partials: harmonics([0.53, 0.23, 0.09, 0.04, 0.017, 0.007]),
            amp: Adsr::new(0.075, 0.22, 0.80, 0.18),
            cutoff_hz: 950.0,
            env_octaves: 0.9,
            noise: 0.015,
            vibrato_cents: 3.0,
            vibrato_hz: 5.0,
            vibrato_delay: 0.4,
            ..BRASS
        },
        BrassEnsemble => Patch {
            voices: 3,
            detune_cents: 2.5,
            partials: harmonics([0.62, 0.41, 0.28, 0.19, 0.10, 0.07]),
            amp: Adsr::new(0.085, 0.23, 0.83, 0.23),
            cutoff_hz: 1_500.0,
            env_octaves: 0.95,
            width: 0.76,
            reverb: 0.26,
            ..BRASS
        },

        // Tuned bars and bells keep a dominant fundamental. Their higher
        // modes decay independently, preventing a lingering dissonant clang.
        Marimba => Patch {
            partials: [
                (4.0, 0.21),
                (10.0, 0.045),
                (18.0, 0.008),
                (0.0, 0.0),
                (0.0, 0.0),
                (0.0, 0.0),
            ],
            amp: Adsr::new(0.004, 0.85, 0.0, 0.18),
            partial_decay_seconds: 0.12,
            cutoff_hz: 2_100.0,
            noise: 0.028,
            noise_decay: 0.005,
            ..STRUCK
        },
        Xylophone => Patch {
            partials: [
                (3.0, 0.32),
                (6.0, 0.12),
                (10.0, 0.045),
                (14.0, 0.014),
                (0.0, 0.0),
                (0.0, 0.0),
            ],
            amp: Adsr::new(0.001, 0.48, 0.0, 0.10),
            partial_decay_seconds: 0.13,
            cutoff_hz: 4_100.0,
            noise: 0.048,
            noise_decay: 0.003,
            ..STRUCK
        },
        Glockenspiel => Patch {
            partials: [
                (2.76, 0.025),
                (4.0, 0.26),
                (7.0, 0.065),
                (11.0, 0.018),
                (0.0, 0.0),
                (0.0, 0.0),
            ],
            amp: Adsr::new(0.002, 2.60, 0.0, 0.55),
            partial_decay_seconds: 0.7,
            cutoff_hz: 5_800.0,
            env_octaves: 0.6,
            reverb: 0.27,
            width: 0.62,
            ..STRUCK
        },
        TubularBells => Patch {
            partials: [
                (2.0, 0.43),
                (3.0, 0.26),
                (4.0, 0.13),
                (5.43, 0.028),
                (7.0, 0.08),
                (9.0, 0.028),
            ],
            amp: Adsr::new(0.003, 3.1, 0.0, 0.65),
            partial_decay_seconds: 0.85,
            cutoff_hz: 3_000.0,
            reverb: 0.30,
            width: 0.62,
            ..STRUCK
        },
        Dulcimer => Patch {
            voices: 2,
            detune_cents: 2.0,
            partials: harmonics([0.44, 0.32, 0.21, 0.13, 0.08, 0.045]),
            amp: Adsr::new(0.001, 1.50, 0.0, 0.30),
            partial_decay_seconds: 0.39,
            cutoff_hz: 2_450.0,
            noise: 0.045,
            noise_decay: 0.005,
            width: 0.58,
            ..STRUCK
        },

        Theremin => Patch {
            waveform: Waveform::Sine,
            partials: harmonics([0.13, 0.035, 0.008, 0.0, 0.0, 0.0]),
            amp: Adsr::new(0.085, 0.18, 0.92, 0.21),
            filter: Adsr::new(0.10, 0.25, 0.85, 0.20),
            cutoff_hz: 2_300.0,
            env_octaves: 0.35,
            resonance: 0.7,
            vibrato_cents: 9.0,
            vibrato_hz: 5.2,
            vibrato_delay: 0.25,
            portamento_seconds: 0.12,
            drive: 0.75,
            width: 0.25,
            reverb: 0.27,
            delay: 0.10,
            ..Patch::DEFAULT
        },
        Choir => Patch {
            waveform: Waveform::Sine,
            voices: 3,
            detune_cents: 2.0,
            partials: harmonics([0.32, 0.19, 0.36, 0.24, 0.09, 0.12]),
            amp: Adsr::new(0.30, 0.40, 0.78, 0.45),
            filter: Adsr::new(0.28, 0.42, 0.70, 0.40),
            cutoff_hz: 1_650.0,
            env_octaves: 0.55,
            vibrato_cents: 3.0,
            vibrato_hz: 4.6,
            vibrato_delay: 0.50,
            noise: 0.02,
            noise_decay: 0.6,
            width: 0.83,
            reverb: 0.31,
            ..WIND
        },
        SynthBrass => Patch {
            waveform: Waveform::Saw,
            voices: 2,
            detune_cents: 3.0,
            partials: harmonics([0.20, 0.08, 0.0, 0.0, 0.0, 0.0]),
            amp: Adsr::new(0.028, 0.25, 0.68, 0.17),
            filter: Adsr::new(0.065, 0.30, 0.35, 0.16),
            cutoff_hz: 800.0,
            env_octaves: 1.8,
            resonance: 0.95,
            drive: 1.1,
            width: 0.55,
            noise: 0.0,
            ..BRASS
        },
        WarmPad => Patch {
            waveform: Waveform::Sine,
            voices: 3,
            detune_cents: 3.0,
            partials: harmonics([0.29, 0.16, 0.07, 0.035, 0.015, 0.0]),
            amp: Adsr::new(0.55, 0.80, 0.74, 0.65),
            filter: Adsr::new(0.65, 1.0, 0.60, 0.55),
            cutoff_hz: 1_200.0,
            env_octaves: 0.65,
            resonance: 0.7,
            drive: 0.8,
            width: 0.90,
            reverb: 0.27,
            delay: 0.12,
            ..Patch::DEFAULT
        },

        Timpani => Patch {
            partials: [
                (1.5, 0.055),
                (2.0, 0.25),
                (2.5, 0.035),
                (3.0, 0.09),
                (4.0, 0.035),
                (0.0, 0.0),
            ],
            amp: Adsr::new(0.003, 1.65, 0.0, 0.30),
            partial_decay_seconds: 0.11,
            filter: Adsr::new(0.001, 0.18, 0.08, 0.22),
            cutoff_hz: 800.0,
            env_octaves: 1.3,
            noise: 0.16,
            noise_decay: 0.018,
            width: 0.25,
            reverb: 0.26,
            ..STRUCK
        },
        Shaker => Patch {
            oscillator_level: 0.0,
            noise: 1.0,
            noise_decay: 0.070,
            amp: Adsr::new(0.009, 0.14, 0.0, 0.04),
            filter: Adsr::new(0.008, 0.10, 0.0, 0.035),
            cutoff_hz: 9_000.0,
            env_octaves: 0.35,
            high_pass_hz: 3_500.0,
            drive: 1.0,
            width: 0.65,
            reverb: 0.10,
            delay: 0.03,
            ..STRUCK
        },
        Conga => Patch {
            partials: [
                (1.48, 0.19),
                (2.0, 0.13),
                (2.64, 0.05),
                (0.0, 0.0),
                (0.0, 0.0),
                (0.0, 0.0),
            ],
            amp: Adsr::new(0.001, 0.40, 0.0, 0.075),
            partial_decay_seconds: 0.07,
            noise: 0.18,
            noise_decay: 0.012,
            pitch_octaves: 0.15,
            pitch_time: 0.018,
            cutoff_hz: 1_600.0,
            env_octaves: 0.85,
            width: 0.35,
            reverb: 0.14,
            ..STRUCK
        },
        Cowbell => Patch {
            partials: [
                (1.48, 0.56),
                (2.12, 0.22),
                (2.9, 0.13),
                (4.1, 0.07),
                (0.0, 0.0),
                (0.0, 0.0),
            ],
            amp: Adsr::new(0.001, 0.34, 0.0, 0.085),
            partial_decay_seconds: 0.23,
            cutoff_hz: 4_200.0,
            env_octaves: 0.55,
            high_pass_hz: 250.0,
            drive: 1.2,
            width: 0.35,
            reverb: 0.12,
            ..STRUCK
        },
        _ => unreachable!("existing instrument patches live in synth.rs"),
    }
}
