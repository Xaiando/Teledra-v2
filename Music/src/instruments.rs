//! The instrument catalog used by synthesis, arrangement, prompts and Studio.
//! Ranges describe the renderer's working registers, not performer limits.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentRole {
    Bass,
    Harmony,
    Melody,
    Colour,
    Percussion,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct InstrumentInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub group: &'static str,
    pub role: InstrumentRole,
    pub range: [u8; 2],
    pub attack_spacing: f32,
    pub gate: f32,
    pub mix_level: f32,
    pub pan: f32,
    pub monophonic: bool,
    pub description: &'static str,
}

macro_rules! catalog {
    ($($(#[$attribute:meta])* $variant:ident => ($id:literal, $name:literal, $group:literal, $role:ident,
        [$low:literal, $high:literal], $spacing:literal, $gate:literal,
        $mix:literal, $pan:literal, $mono:literal, $description:literal)),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
        pub enum Instrument {
            $($(#[$attribute])* #[serde(rename = $id)] $variant),+
        }

        impl Instrument {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub const fn info(self) -> &'static InstrumentInfo {
                match self {
                    $(Self::$variant => &InstrumentInfo {
                        id: $id, name: $name, group: $group, role: InstrumentRole::$role,
                        range: [$low, $high], attack_spacing: $spacing, gate: $gate,
                        mix_level: $mix, pan: $pan, monophonic: $mono,
                        description: $description,
                    }),+
                }
            }
        }
    };
}

catalog! {
    SubBass => ("sub_bass", "Sub Bass", "Electronic", Bass, [24,48], 0.5, 0.9, 0.60, 0.0, true, "Pure low sine foundation; sparse roots with room for the kick."),
    Bass => ("bass", "Bass", "Electronic", Bass, [28,55], 0.0, 0.72, 0.58, 0.0, true, "Rounded synth bass; articulated pulse and root motion."),
    Pad => ("pad", "Pad", "Electronic", Harmony, [45,79], 1.0, 1.7, 0.42, 0.0, false, "Slow layered synth chords; allow sustained entries to bloom."),
    #[default]
    Keys => ("keys", "Keys", "Keyboards", Harmony, [48,84], 0.0, 0.95, 0.50, 0.18, false, "Soft keyboard chord voicings; rhythmic harmonic support."),
    Pluck => ("pluck", "Pluck", "Electronic", Colour, [55,91], 0.0, 0.5, 0.56, 0.30, false, "Short synth pluck; crisp patterns and answering figures."),
    Lead => ("lead", "Lead", "Electronic", Melody, [60,91], 0.0, 0.85, 0.66, -0.10, true, "Focused synth melody; use a clear contour and rests."),
    Bell => ("bell", "Bell", "Mallets", Colour, [72,100], 0.0, 1.2, 0.52, 0.22, false, "Bright ringing bell; sparse high punctuation."),
    Arp => ("arp", "Arp", "Electronic", Melody, [67,98], 0.0, 0.5, 0.38, -0.28, false, "Light articulated arpeggio voice; use broken harmonic figures."),
    Organ => ("organ", "Organ", "Keyboards", Harmony, [36,84], 1.0, 1.0, 0.44, 0.0, false, "Sustained additive organ chords; smooth common-tone changes."),
    Piano => ("piano", "Piano", "Keyboards", Colour, [33,91], 0.0, 1.6, 0.54, -0.20, false, "Acoustic piano-inspired hammer attack and decay; melody or broken chords."),
    Flute => ("flute", "Flute", "Woodwinds", Melody, [72,98], 0.5, 1.1, 0.50, 0.36, true, "Airy high flute; breathing phrases with connected notes."),
    Reed => ("reed", "Reed", "Woodwinds", Melody, [50,84], 0.5, 1.0, 0.48, -0.34, true, "Warm reed colour; lyrical middle-register lines."),
    Cello => ("cello", "Cello", "Strings", Bass, [36,69], 1.0, 1.9, 0.56, 0.26, true, "Bowed low strings; spacious foundation or a singing low line."),
    Harp => ("harp", "Harp", "Plucked strings", Colour, [36,91], 0.0, 1.5, 0.50, -0.30, false, "Ringing harp plucks; broken chords and delicate answers."),
    Violin => ("violin", "Violin", "Strings", Melody, [55,96], 1.0, 1.25, 0.46, -0.36, true, "Bowed upper string voice; sustained lyricism with gentle vibrato."),
    Clarinet => ("clarinet", "Clarinet", "Woodwinds", Melody, [50,89], 0.5, 1.0, 0.48, 0.20, true, "Hollow odd-harmonic woodwind; warm legato melody."),
    Oboe => ("oboe", "Oboe", "Woodwinds", Melody, [58,91], 0.5, 1.0, 0.40, -0.20, true, "Focused nasal double-reed colour; exposed melodic answers."),
    Horn => ("horn", "Horn", "Brass", Melody, [41,77], 1.0, 1.0, 0.48, 0.30, true, "Mellow rounded horn; broad low-to-middle phrases."),
    Trumpet => ("trumpet", "Trumpet", "Brass", Melody, [54,86], 0.5, 0.9, 0.40, -0.26, true, "Bright brass attack; clear calls with breathing space."),
    Guitar => ("guitar", "Guitar", "Plucked strings", Colour, [40,88], 0.0, 1.25, 0.54, -0.32, false, "Nylon guitar-inspired pluck; warm fingerpicked answers."),
    Vibraphone => ("vibraphone", "Vibraphone", "Mallets", Colour, [53,89], 0.0, 1.5, 0.48, 0.32, false, "Mellow metallic bars with tremolo; spacious ringing harmony."),
    Kick => ("kick", "Kick", "Percussion", Percussion, [30,42], 0.0, 0.3, 0.88, 0.0, false, "Low unpitched drum pulse; degree zero is a hit."),
    Snare => ("snare", "Snare", "Percussion", Percussion, [56,68], 0.0, 0.3, 0.70, 0.0, false, "Noise-and-body backbeat; use accents and ghost hits."),
    Hat => ("hat", "Hat", "Percussion", Percussion, [78,90], 0.0, 0.16, 0.34, 0.34, false, "Tight high-hat ticks; subdivided pulse with rests."),
    Perc => ("perc", "Perc", "Percussion", Percussion, [50,80], 0.0, 0.3, 0.50, -0.42, false, "Short auxiliary percussion; offbeat accents."),
    ElectricPiano => ("electric_piano", "Electric Piano", "Keyboards", Colour, [36,91], 0.0, 1.5, 0.52, -0.18, false, "Bell-like tine attack and warm decay; flowing chord fragments."),
    HonkyTonkPiano => ("honky_tonk_piano", "Honky Tonk Piano", "Keyboards", Colour, [36,88], 0.0, 1.3, 0.48, -0.16, false, "Bright lightly detuned upright colour; bouncy rhythmic phrases."),
    Harpsichord => ("harpsichord", "Harpsichord", "Keyboards", Colour, [41,89], 0.0, 0.8, 0.42, 0.18, false, "Bright plucked keyboard; crisp contrapuntal lines."),
    Clavinet => ("clavinet", "Clavinet", "Keyboards", Colour, [36,84], 0.0, 0.55, 0.46, -0.22, false, "Dry percussive electric keyboard; syncopated short notes."),
    Celesta => ("celesta", "Celesta", "Keyboards", Colour, [60,96], 0.0, 1.25, 0.44, 0.22, false, "Soft struck-key chimes; delicate high-register detail."),
    Accordion => ("accordion", "Accordion", "Keyboards", Harmony, [48,84], 0.5, 1.1, 0.42, 0.0, false, "Reedy sustained keyboard chords; phrased bellows-like swells."),
    Harmonica => ("harmonica", "Harmonica", "Woodwinds", Melody, [55,89], 0.5, 0.9, 0.42, -0.15, true, "Bright free-reed melody; breath-shaped short phrases."),
    Viola => ("viola", "Viola", "Strings", Melody, [48,84], 1.0, 1.35, 0.48, -0.22, true, "Dark middle-register bowed string; inner melodic line."),
    DoubleBass => ("double_bass", "Double Bass", "Strings", Bass, [28,60], 1.0, 1.3, 0.58, 0.08, true, "Woody bowed low string; spacious grounded bass movement."),
    StringEnsemble => ("string_ensemble", "String Ensemble", "Strings", Harmony, [43,88], 1.0, 1.65, 0.36, -0.08, false, "Layered bowed-string chords; slow ensemble swells."),
    PizzicatoStrings => ("pizzicato_strings", "Pizzicato Strings", "Strings", Colour, [43,88], 0.0, 0.5, 0.50, -0.24, false, "Short plucked orchestral strings; nimble rhythmic support."),
    TremoloStrings => ("tremolo_strings", "Tremolo Strings", "Strings", Harmony, [48,91], 1.0, 1.4, 0.34, 0.12, false, "Rapidly pulsing bowed texture; held suspenseful harmony."),
    SteelGuitar => ("steel_guitar", "Steel Guitar", "Plucked strings", Colour, [40,88], 0.0, 1.4, 0.49, -0.28, false, "Steel-string acoustic shimmer; open fingerpicked figures."),
    ElectricGuitar => ("electric_guitar", "Electric Guitar", "Plucked strings", Colour, [40,88], 0.0, 1.0, 0.44, -0.30, false, "Clean electric-string attack; melodic riffs and offbeat answers."),
    JazzGuitar => ("jazz_guitar", "Jazz Guitar", "Plucked strings", Colour, [40,84], 0.0, 1.2, 0.52, -0.25, false, "Rounded dark guitar pluck; warm restrained accompaniment."),
    MutedGuitar => ("muted_guitar", "Muted Guitar", "Plucked strings", Colour, [40,84], 0.0, 0.35, 0.54, 0.26, false, "Damped string ticks; sparse syncopated rhythmic figures."),
    Banjo => ("banjo", "Banjo", "Plucked strings", Colour, [48,88], 0.0, 0.65, 0.40, -0.25, false, "Bright membrane-backed twang; rolling broken chords."),
    Mandolin => ("mandolin", "Mandolin", "Plucked strings", Colour, [55,96], 0.0, 0.75, 0.42, 0.25, false, "Paired-string sparkle; quick repeated notes and answers."),
    Ukulele => ("ukulele", "Ukulele", "Plucked strings", Colour, [60,88], 0.0, 0.65, 0.52, -0.22, false, "Small warm string pluck; light compact chord fragments."),
    Sitar => ("sitar", "Sitar", "Plucked strings", Colour, [48,88], 0.0, 1.5, 0.38, 0.22, false, "Buzzing long pluck with sympathetic colour; spacious melodic turns."),
    Koto => ("koto", "Koto", "Plucked strings", Colour, [45,89], 0.0, 1.2, 0.46, -0.24, false, "Crisp zither-like attack; isolated plucks and broken figures."),
    Oud => ("oud", "Oud", "Plucked strings", Colour, [36,81], 0.0, 0.95, 0.52, 0.20, false, "Dark fretless-lute colour; warm low and middle melodic answers."),
    Piccolo => ("piccolo", "Piccolo", "Woodwinds", Melody, [74,103], 0.5, 0.85, 0.34, 0.32, true, "Small bright flute colour; brief upper-register flourishes."),
    Recorder => ("recorder", "Recorder", "Woodwinds", Melody, [60,96], 0.5, 0.95, 0.46, -0.20, true, "Clear whistle-like woodwind; simple articulated melody."),
    PanFlute => ("pan_flute", "Pan Flute", "Woodwinds", Melody, [60,91], 0.5, 0.9, 0.46, 0.25, true, "Breathy pipe attacks; separated airy melodic gestures."),
    Shakuhachi => ("shakuhachi", "Shakuhachi", "Woodwinds", Melody, [50,86], 0.5, 1.2, 0.44, -0.25, true, "Bamboo-flute-inspired breath and low harmonics; spacious lines."),
    Bassoon => ("bassoon", "Bassoon", "Woodwinds", Bass, [34,77], 0.5, 1.0, 0.48, 0.18, true, "Woody low double reed; articulated bass and lyrical low answers."),
    EnglishHorn => ("english_horn", "English Horn", "Woodwinds", Melody, [52,84], 0.5, 1.15, 0.42, -0.18, true, "Mellow dark double reed; sustained middle-register lament."),
    SopranoSax => ("soprano_sax", "Soprano Sax", "Woodwinds", Melody, [56,89], 0.5, 1.0, 0.40, 0.25, true, "Bright singing reed; expressive upper-middle melody."),
    TenorSax => ("tenor_sax", "Tenor Sax", "Woodwinds", Melody, [44,80], 0.5, 1.1, 0.46, -0.25, true, "Warm broad reed with a buzzy edge; middle-register phrases."),
    Trombone => ("trombone", "Trombone", "Brass", Melody, [40,77], 0.5, 1.05, 0.46, -0.20, true, "Rounded low brass with a firm attack; broad answering phrases."),
    Tuba => ("tuba", "Tuba", "Brass", Bass, [28,65], 1.0, 1.1, 0.52, 0.0, true, "Dark low brass foundation; generous spacing between notes."),
    Flugelhorn => ("flugelhorn", "Flugelhorn", "Brass", Melody, [52,84], 0.5, 1.15, 0.44, 0.20, true, "Soft trumpet-family brass; warm rounded legato calls."),
    BrassEnsemble => ("brass_ensemble", "Brass Ensemble", "Brass", Harmony, [41,84], 1.0, 0.95, 0.32, 0.08, false, "Layered brass chord colour; broad harmonic statements."),
    Marimba => ("marimba", "Marimba", "Mallets", Colour, [45,89], 0.0, 0.8, 0.54, -0.24, false, "Warm wooden bars; rounded rhythmic melody and broken chords."),
    Xylophone => ("xylophone", "Xylophone", "Mallets", Colour, [60,96], 0.0, 0.45, 0.40, 0.24, false, "Hard short wooden mallet attack; bright rhythmic accents."),
    Glockenspiel => ("glockenspiel", "Glockenspiel", "Mallets", Colour, [72,103], 0.0, 1.2, 0.36, -0.20, false, "High metal-bar sparkle; sparse ringing punctuation."),
    TubularBells => ("tubular_bells", "Tubular Bells", "Mallets", Colour, [48,84], 0.0, 1.8, 0.36, 0.15, false, "Deep resonant chimes; widely spaced tonal strikes."),
    Dulcimer => ("dulcimer", "Dulcimer", "Mallets", Colour, [48,91], 0.0, 1.0, 0.44, -0.18, false, "Hammered-string shimmer; rippling melodic figures."),
    Theremin => ("theremin", "Theremin", "Electronic", Melody, [48,96], 0.5, 1.15, 0.42, 0.05, true, "Singing electronic voice with pitch glide and vibrato; connected legato notes."),
    Choir => ("choir", "Choir", "Electronic", Harmony, [48,84], 1.0, 1.6, 0.34, 0.0, false, "Vowel-coloured synthesized voices; slow sustained choral harmony."),
    SynthBrass => ("synth_brass", "Synth Brass", "Electronic", Harmony, [43,84], 0.5, 0.8, 0.38, 0.12, false, "Bright filtered brass-like synth chords; rhythmic swells."),
    WarmPad => ("warm_pad", "Warm Pad", "Electronic", Harmony, [43,84], 1.0, 1.9, 0.40, -0.10, false, "Dark soft sustained pad; quiet harmonic space under a melody."),
    Timpani => ("timpani", "Timpani", "Mallets", Colour, [36,60], 0.5, 0.8, 0.55, 0.0, false, "Tuned low kettle-drum-inspired strikes; write tonic or chord-tone degrees."),
    Shaker => ("shaker", "Shaker", "Percussion", Percussion, [72,90], 0.0, 0.18, 0.28, -0.35, false, "Soft high noise shakes; alternate accents and rests."),
    Conga => ("conga", "Conga", "Percussion", Percussion, [48,72], 0.0, 0.32, 0.50, -0.28, false, "Rounded unpitched hand-drum hit; syncopated interlocking accents."),
    Cowbell => ("cowbell", "Cowbell", "Percussion", Percussion, [60,84], 0.0, 0.35, 0.30, 0.28, false, "Dry metallic unpitched strike; sparse rhythmic punctuation."),
}

impl Instrument {
    pub const fn id(self) -> &'static str {
        self.info().id
    }
    pub const fn label(self) -> &'static str {
        self.info().name
    }
    pub const fn group(self) -> &'static str {
        self.info().group
    }
    pub const fn role(self) -> InstrumentRole {
        self.info().role
    }
    pub const fn natural_range(self) -> (u8, u8) {
        (self.info().range[0], self.info().range[1])
    }
    pub const fn attack_spacing(self) -> f32 {
        self.info().attack_spacing
    }
    pub const fn gate(self) -> f32 {
        self.info().gate
    }
    pub const fn mix_level(self) -> f32 {
        self.info().mix_level
    }
    pub const fn default_pan(self) -> f32 {
        self.info().pan
    }
    pub const fn is_percussive(self) -> bool {
        matches!(self.role(), InstrumentRole::Percussion)
    }
    pub const fn follows_harmony(self) -> bool {
        !self.is_percussive()
    }
    pub const fn is_chordal(self) -> bool {
        matches!(self.role(), InstrumentRole::Harmony)
    }
    /// Keyboard lines can outline harmony with broken chords without the
    /// arranger expanding their individual notes into full chord voicings.
    pub const fn can_accompany_harmony(self) -> bool {
        self.is_chordal()
            || matches!(
                self,
                Self::Piano
                    | Self::ElectricPiano
                    | Self::HonkyTonkPiano
                    | Self::Harpsichord
                    | Self::Clavinet
                    | Self::Celesta
            )
    }
    pub const fn is_monophonic(self) -> bool {
        self.info().monophonic
    }

    /// Compatible performance roles, in a stable order for native rotation.
    pub const fn family(self) -> &'static [Self] {
        use Instrument::*;
        match self.role() {
            InstrumentRole::Bass => &[Bass, Cello, SubBass, DoubleBass, Bassoon, Tuba],
            InstrumentRole::Harmony => &[
                Keys,
                Organ,
                Pad,
                Accordion,
                StringEnsemble,
                TremoloStrings,
                BrassEnsemble,
                Choir,
                SynthBrass,
                WarmPad,
            ],
            InstrumentRole::Melody => &[
                Lead,
                Flute,
                Reed,
                Arp,
                Violin,
                Clarinet,
                Oboe,
                Horn,
                Trumpet,
                Harmonica,
                Viola,
                Piccolo,
                Recorder,
                PanFlute,
                Shakuhachi,
                EnglishHorn,
                SopranoSax,
                TenorSax,
                Trombone,
                Flugelhorn,
                Theremin,
            ],
            InstrumentRole::Colour => &[
                Pluck,
                Harp,
                Piano,
                Bell,
                Guitar,
                Vibraphone,
                ElectricPiano,
                HonkyTonkPiano,
                Harpsichord,
                Clavinet,
                Celesta,
                PizzicatoStrings,
                SteelGuitar,
                ElectricGuitar,
                JazzGuitar,
                MutedGuitar,
                Banjo,
                Mandolin,
                Ukulele,
                Sitar,
                Koto,
                Oud,
                Marimba,
                Xylophone,
                Glockenspiel,
                TubularBells,
                Dulcimer,
                Timpani,
            ],
            InstrumentRole::Percussion => &[],
        }
    }

    /// Prefer the longest named instrument over a generic role in a track name.
    /// For example, Electric Piano must not collapse to Piano, nor Cowbell to Bell.
    pub fn from_name(name: &str) -> Self {
        let normalized = name.to_ascii_lowercase().replace(['_', '-'], " ");
        let mut best: Option<(usize, Self)> = None;
        for &voice in Self::ALL {
            let label = voice.label().to_ascii_lowercase();
            if contains_phrase(&normalized, &label)
                && best.is_none_or(|(length, _)| label.len() > length)
            {
                best = Some((label.len(), voice));
            }
        }
        for (alias, voice) in [
            ("grand piano", Self::Piano),
            ("acoustic piano", Self::Piano),
            ("upright piano", Self::Piano),
            ("rhodes", Self::ElectricPiano),
            ("wurlitzer", Self::ElectricPiano),
            ("honkytonk", Self::HonkyTonkPiano),
            ("vibes", Self::Vibraphone),
            ("french horn", Self::Horn),
            ("cor anglais", Self::EnglishHorn),
            ("doublebass", Self::DoubleBass),
            ("contrabass", Self::DoubleBass),
            ("pizzicato", Self::PizzicatoStrings),
            ("soprano saxophone", Self::SopranoSax),
            ("tenor saxophone", Self::TenorSax),
            ("saxophone", Self::TenorSax),
            ("panpipes", Self::PanFlute),
            ("panflute", Self::PanFlute),
            ("nylon guitar", Self::Guitar),
            ("steel string guitar", Self::SteelGuitar),
            ("muted electric guitar", Self::MutedGuitar),
            ("pherronome", Self::Theremin),
            ("theremin", Self::Theremin),
            ("kettle drum", Self::Timpani),
            ("keyboard", Self::Piano),
            ("lyre", Self::Harp),
            ("whistle", Self::Recorder),
        ] {
            if contains_phrase(&normalized, alias)
                && best.is_none_or(|(length, _)| alias.len() > length)
            {
                best = Some((alias.len(), voice));
            }
        }
        if let Some((_, voice)) = best {
            return voice;
        }
        for (alias, voice) in [
            ("sub", Self::SubBass),
            ("clap", Self::Snare),
            ("cymbal", Self::Hat),
            ("hi hat", Self::Hat),
            ("drum", Self::Perc),
            ("tom", Self::Perc),
            ("string", Self::Pad),
            ("strings", Self::StringEnsemble),
            ("harmony", Self::Pad),
            ("chord", Self::Keys),
            ("key", Self::Keys),
            ("glass", Self::Bell),
            ("chime", Self::Bell),
            ("motif", Self::Lead),
            ("melody", Self::Lead),
        ] {
            if contains_phrase(&normalized, alias) {
                return voice;
            }
        }
        Self::Keys
    }
}

fn contains_phrase(name: &str, phrase: &str) -> bool {
    name.match_indices(phrase).any(|(start, matched)| {
        let end = start + matched.len();
        (start == 0 || !name.as_bytes()[start - 1].is_ascii_alphanumeric())
            && (end == name.len() || !name.as_bytes()[end].is_ascii_alphanumeric())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn catalog_entries_are_unique_bounded_and_have_compatible_families() {
        assert!(Instrument::ALL.len() >= 70);
        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        for &voice in Instrument::ALL {
            let info = voice.info();
            assert!(ids.insert(info.id));
            assert!(names.insert(info.name));
            assert_eq!(Instrument::from_name(info.name), voice);
            assert_eq!(Instrument::from_name(info.id), voice);
            assert!(info.range[0] < info.range[1] && info.range[1] <= 108);
            assert!((0.0..=1.0).contains(&info.mix_level));
            assert!((-1.0..=1.0).contains(&info.pan));
            assert!(!info.description.is_empty());
            if !voice.is_percussive() {
                assert!(voice.family().contains(&voice), "{voice:?} unreachable");
                let mut unique = HashSet::new();
                for member in voice.family() {
                    assert_eq!(member.role(), voice.role());
                    assert!(unique.insert(member));
                }
            }
        }
    }

    #[test]
    fn specific_instrument_names_survive_generic_role_words() {
        for (name, voice) in [
            ("Electric Piano Melody", Instrument::ElectricPiano),
            ("Piano Melody", Instrument::Piano),
            ("Cello Harmony", Instrument::Cello),
            ("Low English Horn", Instrument::EnglishHorn),
            ("Muted Electric Guitar", Instrument::MutedGuitar),
            ("Bass Clarinet", Instrument::Clarinet),
            ("Cowbell Rhythm", Instrument::Cowbell),
            ("Harpsichord Answer", Instrument::Harpsichord),
            ("Honky-Tonk Piano", Instrument::HonkyTonkPiano),
            ("pherronome", Instrument::Theremin),
        ] {
            assert_eq!(Instrument::from_name(name), voice, "{name}");
        }
    }
}
