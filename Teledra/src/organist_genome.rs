//! A drawn genome for each of the Organist's pieces, as Buzzart has.
//!
//! Buzzart stopped sounding the same once every tune drew its own grammar:
//! key, pace, groove and ensemble chosen per piece, kept apart from what played
//! recently. The Organist gets the same treatment in his own terms. A genome
//! fixes the frame of a round's piece — tonic, mode, tempo, pulse, progression
//! and a coherent ensemble — while he still writes the theme, the answer, the
//! form and the title. When his composition fails, the genome also carries a
//! complete melody of its own, so his round can still bring a new piece.
//!
//! The tables live in `Music/organist_style.json`, shared with the trainer
//! (`Music/organist_train.py`). Training learns a weight for each choice from
//! thousands of rendered and critic-scored pieces; a trained copy at
//! `<data>/music/organist_style.json` is read on every draw, so a new style
//! takes effect without restarting the court. Unlisted choices weigh 1.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::{Value, json};

/// Fields a round's genome fixes; the Organist authors everything else.
pub(crate) const FIXED: [&str; 6] = ["tonic", "mode", "bpm", "pulse", "progression", "voices"];
/// The prompt line that carries the genome to the composition request.
pub(crate) const MARKER: &str = "ROUND GENOME: ";

const VELOCITIES: [f64; 5] = [0.35, 0.5, 0.65, 0.8, 1.0];
const BUNDLED_STYLE: &str = include_str!("../../Music/organist_style.json");
static TRAINED_STYLE: OnceLock<PathBuf> = OnceLock::new();

/// Where a trained style may be found; set once when the court starts.
pub(crate) fn use_trained_style(path: PathBuf) {
    let _ = TRAINED_STYLE.set(path);
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Ensemble {
    pub(crate) name: String,
    pub(crate) lead: Vec<String>,
    pub(crate) answer: Vec<String>,
    pub(crate) harmony: Vec<String>,
    pub(crate) bass: Vec<String>,
    pub(crate) texture: Vec<String>,
}

impl Ensemble {
    fn role(&self, role: &str) -> &[String] {
        match role {
            "lead" => &self.lead,
            "answer" => &self.answer,
            "harmony" => &self.harmony,
            "bass" => &self.bass,
            _ => &self.texture,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
struct Section {
    name: String,
    melody: String,
    energy: f64,
}

/// The Organist's genome tables and learned choice weights.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Style {
    tonics: Vec<String>,
    modes: Vec<String>,
    tempi: Vec<u32>,
    pulses: Vec<String>,
    rhythms: Vec<String>,
    progressions: HashMap<String, Vec<[u8; 4]>>,
    pub(crate) ensembles: Vec<Ensemble>,
    forms: Vec<Vec<Section>>,
    steps: Vec<i64>,
    adjectives: Vec<String>,
    nouns: Vec<String>,
    #[serde(default)]
    weights: HashMap<String, HashMap<String, f64>>,
}

impl Style {
    pub(crate) fn bundled() -> Self {
        serde_json::from_str(BUNDLED_STYLE).expect("the bundled Organist style parses")
    }

    /// A style file, refused unless every table is usable.
    pub(crate) fn load(path: &std::path::Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let style: Self =
            serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))?;
        if style.is_sound() {
            Ok(style)
        } else {
            Err(format!("{}: a table is empty or malformed", path.display()))
        }
    }

    /// The trained style when one is present and sound, else the bundled one.
    pub(crate) fn current() -> Self {
        TRAINED_STYLE
            .get()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str::<Self>(&text).ok())
            .filter(Self::is_sound)
            .unwrap_or_else(Self::bundled)
    }

    fn is_sound(&self) -> bool {
        !self.tonics.is_empty()
            && !self.modes.is_empty()
            && !self.tempi.is_empty()
            && !self.pulses.is_empty()
            && !self.rhythms.is_empty()
            && !self.steps.is_empty()
            && !self.adjectives.is_empty()
            && !self.nouns.is_empty()
            && !self.ensembles.is_empty()
            && self.ensembles.iter().all(|ensemble| {
                ["lead", "answer", "harmony", "bass", "texture"]
                    .iter()
                    .all(|role| !ensemble.role(role).is_empty())
            })
            && !self.forms.is_empty()
            && self.forms.iter().all(|form| form.len() == 4)
            && self
                .modes
                .iter()
                .all(|mode| self.progressions.get(mode).is_some_and(|p| !p.is_empty()))
    }

    /// A choice's learned weight; unknown or unusable weights count as 1.
    fn weight(&self, dimension: &str, key: &str) -> f64 {
        self.weights
            .get(dimension)
            .and_then(|choices| choices.get(key))
            .copied()
            .filter(|weight| weight.is_finite() && *weight > 0.0)
            .unwrap_or(1.0)
    }
}

/// The weight-table key for a progression, e.g. "0-5-3-4".
pub(crate) fn progression_key(progression: &[u8]) -> String {
    progression
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join("-")
}

/// SplitMix64: small, deterministic, and enough for musical choices.
struct Draw(u64);

impl Draw {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1_u64 << 53) as f64
    }
    /// An index into `items`, in proportion to each item's weight.
    fn weighted<T>(&mut self, items: &[T], weight: impl Fn(&T) -> f64) -> usize {
        let weights: Vec<f64> = items.iter().map(&weight).collect();
        let total: f64 = weights.iter().sum();
        let mut target = self.unit() * total;
        for (index, w) in weights.iter().enumerate() {
            if target < *w {
                return index;
            }
            target -= w;
        }
        items.len() - 1
    }
    fn pick<'a, T: ToString>(&mut self, style: &Style, dimension: &str, items: &'a [T]) -> &'a T {
        let index = self.weighted(items, |item| style.weight(dimension, &item.to_string()));
        &items[index]
    }
}

/// One phrase: six degrees with leaps and returns, ending where the phrase
/// should land (open for a theme, home for an answer).
fn phrase(draw: &mut Draw, style: &Style, closing: bool) -> Vec<i64> {
    let (start_dim, end_dim, ends): (&str, &str, &[i64]) = if closing {
        ("answer_start", "answer_end", &[0, 7])
    } else {
        ("theme_start", "theme_end", &[2, 4, 7])
    };
    for _ in 0..400 {
        let mut degrees = vec![*draw.pick(style, start_dim, &[0_i64, 2, 4, 7])];
        for _ in 0..4 {
            let last = *degrees.last().unwrap();
            let step = *draw.pick(style, "step", &style.steps);
            degrees.push((last + step).clamp(0, 11));
        }
        degrees.push(*draw.pick(style, end_dim, ends));
        let distinct: std::collections::BTreeSet<_> = degrees.iter().collect();
        let scalar = degrees.windows(5).any(|run| {
            let steps: Vec<_> = run.windows(2).map(|pair| pair[1] - pair[0]).collect();
            steps.iter().all(|step| *step == 1) || steps.iter().all(|step| *step == -1)
        });
        let leaps = degrees
            .windows(2)
            .filter(|pair| (pair[1] - pair[0]).abs() >= 2)
            .count();
        if distinct.len() >= 4 && !scalar && leaps >= 2 {
            return degrees;
        }
    }
    // A learned step table too narrow to leap still yields a valid phrase.
    if closing {
        vec![4, 7, 5, 2, 4, 0]
    } else {
        vec![0, 4, 2, 5, 7, 4]
    }
}

fn velocities(draw: &mut Draw) -> Vec<f64> {
    // A shaped phrase: rise to a peak note, then ease off.
    let peak = 1 + draw.below(4);
    (0..6)
        .map(|index| {
            let distance = (index as i64 - peak as i64).unsigned_abs() as usize;
            VELOCITIES[4_usize.saturating_sub(distance).max(1) - usize::from(draw.below(3) == 0)]
        })
        .collect()
}

/// A complete proposal in the song schema, drawn from `seed`.
fn draw_one(seed: u64, style: &Style) -> Value {
    let mut draw = Draw(seed);
    let mode = draw.pick(style, "mode", &style.modes).clone();
    let bpm = *draw.pick(style, "tempo", &style.tempi);
    let ensemble_index = draw.weighted(&style.ensembles, |e| style.weight("ensemble", &e.name));
    let ensemble = &style.ensembles[ensemble_index];
    let form_indices: Vec<usize> = (0..style.forms.len()).collect();
    let form_index = draw.weighted(&form_indices, |i| style.weight("form", &i.to_string()));
    let form = &style.forms[form_index];
    // Enough bars for a library piece of at least 30 seconds at this tempo.
    let needed = (f64::from(bpm) * 30.0 / 240.0).ceil() as u64;
    let mut bars: Vec<u64> = (0..4).map(|_| [4_u64, 8][draw.below(2)]).collect();
    let mut index = 0;
    while bars.iter().sum::<u64>() < needed {
        bars[index % 4] = 8;
        index += 1;
    }
    let progressions = &style.progressions[&mode];
    let progression_index = draw.weighted(progressions, |p| {
        style.weight(&format!("progression:{mode}"), &progression_key(p))
    });
    let mut voice = |role: &str| {
        draw.pick(
            style,
            &format!("voice:{}:{role}", ensemble.name),
            ensemble.role(role),
        )
        .clone()
    };
    let voices = json!({
        "lead": voice("lead"), "answer": voice("answer"), "harmony": voice("harmony"),
        "bass": voice("bass"), "texture": voice("texture"),
    });
    let theme_rhythm = draw.pick(style, "rhythm:theme", &style.rhythms).clone();
    let answer_rhythm = draw.pick(style, "rhythm:answer", &style.rhythms).clone();
    json!({
        "title": format!("{} {}", style.adjectives[draw.below(style.adjectives.len())], style.nouns[draw.below(style.nouns.len())]),
        "tonic": draw.pick(style, "tonic", &style.tonics),
        "mode": mode,
        "bpm": bpm,
        "progression": progressions[progression_index],
        "voices": voices,
        "theme": {"rhythm": theme_rhythm, "degrees": phrase(&mut draw, style, false), "velocities": velocities(&mut draw)},
        "answer": {"rhythm": answer_rhythm, "degrees": phrase(&mut draw, style, true), "velocities": velocities(&mut draw)},
        "sections": form.iter().zip(&bars).map(|(section, bars)| {
            json!({"name": section.name, "bars": bars, "melody": section.melody, "energy": section.energy})
        }).collect::<Vec<_>>(),
        "pulse": draw.pick(style, "pulse", &style.pulses),
        // Carried alongside for the brief and training; stripped before validation.
        "ensemble": ensemble.name,
        "form_index": form_index,
    })
}

/// How far apart two pieces' frames are: 0 (same) to 7 (nothing shared).
pub(crate) fn distance(a: &Value, b: &Value) -> usize {
    let bpm = |value: &Value| value["bpm"].as_f64().unwrap_or(0.0);
    let differs = [
        a["tonic"] != b["tonic"],
        a["mode"] != b["mode"],
        (bpm(a) - bpm(b)).abs() >= 12.0,
        a["progression"] != b["progression"],
        a["pulse"] != b["pulse"],
        a["voices"]["lead"] != b["voices"]["lead"],
        a["voices"]["harmony"] != b["voices"]["harmony"],
    ];
    differs.iter().filter(|differs| **differs).count()
}

/// Draws a genome clearly apart from every recent piece, and valid when
/// expanded, so it can always be played as a piece in its own right.
pub(crate) fn draw(seed: u64, recent: &[Value]) -> Value {
    draw_with(seed, recent, &Style::current())
        .or_else(|| draw_with(seed, recent, &Style::bundled()))
        .expect("the bundled genome tables always yield a valid proposal")
}

/// As [`draw`], from one given style; None if nothing it draws is valid.
pub(crate) fn draw_with(seed: u64, recent: &[Value], style: &Style) -> Option<Value> {
    let mut best: Option<(usize, Value)> = None;
    for attempt in 0..96_u64 {
        let genome = draw_one(
            seed.wrapping_add(attempt.wrapping_mul(0x2545_f491_4f6c_dd1d)),
            style,
        );
        if crate::python_score::expand_proposal(proposal(&genome)).is_err() {
            continue;
        }
        let apart = recent
            .iter()
            .map(|piece| distance(&genome, piece))
            .min()
            .unwrap_or(7);
        if apart >= 5 {
            return Some(genome);
        }
        if best.as_ref().is_none_or(|(score, _)| apart > *score) {
            best = Some((apart, genome));
        }
    }
    best.map(|(_, genome)| genome)
}

/// The genome as a song proposal (without its descriptive extras).
pub(crate) fn proposal(genome: &Value) -> Value {
    let mut proposal = genome.clone();
    if let Some(object) = proposal.as_object_mut() {
        object.remove("ensemble");
        object.remove("form_index");
    }
    proposal
}

/// Fixes the round's frame onto the Organist's own proposal. His title,
/// theme, answer and form stay his.
pub(crate) fn impose(mut proposal: Value, genome: &Value) -> Value {
    for field in FIXED {
        if !genome[field].is_null() {
            proposal[field] = genome[field].clone();
        }
    }
    // A faster imposed tempo must not leave his form under library length.
    let bpm = proposal["bpm"].as_f64().unwrap_or(120.0);
    if let Some(sections) = proposal["sections"].as_array_mut() {
        let bars: u64 = sections.iter().filter_map(|s| s["bars"].as_u64()).sum();
        if (bars as f64) * 240.0 / bpm < 30.0 {
            for section in sections {
                section["bars"] = json!(8);
            }
        }
    }
    proposal
}

/// The prompt text for a round: what is fixed, and what remains his.
pub(crate) fn brief(genome: &Value) -> String {
    let fixed: serde_json::Map<String, Value> = FIXED
        .iter()
        .map(|field| ((*field).to_owned(), genome[*field].clone()))
        .collect();
    format!(
        "THIS ROUND'S GENOME (drawn so each of your pieces sounds different from the last ones): a {ensemble} ensemble in {tonic} {mode} at {bpm} BPM with a {pulse} pulse and chord roots {progression}; lead {lead}, answer {answer}, harmony {harmony}, bass {bass}, texture {texture}. These frame fields are fixed and the runtime applies them. Everything that makes it yours is still your choice: the title, the theme and answering phrase (rhythm, degrees, dynamics) and the four-section form. Write a melody that suits this ensemble and key; do not reuse an earlier theme.\n{MARKER}{}",
        Value::Object(fixed),
        ensemble = genome["ensemble"].as_str().unwrap_or("chosen"),
        tonic = genome["tonic"].as_str().unwrap_or(""),
        mode = genome["mode"].as_str().unwrap_or(""),
        bpm = genome["bpm"],
        pulse = genome["pulse"].as_str().unwrap_or(""),
        progression = genome["progression"],
        lead = genome["voices"]["lead"].as_str().unwrap_or(""),
        answer = genome["voices"]["answer"].as_str().unwrap_or(""),
        harmony = genome["voices"]["harmony"].as_str().unwrap_or(""),
        bass = genome["voices"]["bass"].as_str().unwrap_or(""),
        texture = genome["voices"]["texture"].as_str().unwrap_or(""),
    )
}

/// The fixed frame carried by a prompt built with [`brief`], if any.
pub(crate) fn from_prompt(prompt: &str) -> Option<Value> {
    let line = prompt.lines().find_map(|line| line.strip_prefix(MARKER))?;
    let frame: Value = serde_json::from_str(line.trim()).ok()?;
    FIXED
        .iter()
        .all(|field| !frame[*field].is_null())
        .then_some(frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learned_weights_steer_the_draw_and_a_broken_style_falls_back() {
        let mut style = Style::bundled();
        style.weights.insert(
            "mode".into(),
            [
                ("major".to_owned(), 50.0),
                ("minor".to_owned(), 1.0),
                ("dorian".to_owned(), 1.0),
            ]
            .into_iter()
            .collect(),
        );
        let major = (0..200)
            .filter(|seed| draw_with(*seed, &[], &style).unwrap()["mode"] == "major")
            .count();
        assert!(major > 160, "{major} of 200 drew major");
        let unweighted = (0..200)
            .filter(|seed| draw_with(*seed, &[], &Style::bundled()).unwrap()["mode"] == "major")
            .count();
        assert!(
            unweighted < 110,
            "{unweighted} of 200 drew major without weights"
        );
        let broken = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(broken.path(), r#"{"tonics":[]}"#).unwrap();
        assert!(Style::load(broken.path()).is_err());
    }

    #[test]
    fn every_drawn_genome_expands_into_a_valid_song() {
        for seed in 0..300 {
            let genome = draw(seed, &[]);
            let song = crate::python_score::expand_proposal(proposal(&genome)).unwrap();
            crate::python_score::source(&song).unwrap();
        }
    }

    #[test]
    fn every_ensemble_voice_is_accepted_in_its_role() {
        let schema = crate::python_score::proposal_schema();
        for ensemble in &Style::bundled().ensembles {
            for (role, voices) in [
                ("lead", &ensemble.lead),
                ("answer", &ensemble.answer),
                ("harmony", &ensemble.harmony),
                ("bass", &ensemble.bass),
                ("texture", &ensemble.texture),
            ] {
                let allowed = schema["properties"]["voices"]["properties"][role]["enum"]
                    .as_array()
                    .unwrap();
                for voice in voices {
                    assert!(
                        allowed.contains(&json!(voice)),
                        "{} {role} {voice}",
                        ensemble.name
                    );
                }
            }
        }
    }

    #[test]
    fn genomes_range_widely_and_stay_apart_from_recent_pieces() {
        let mut recent: Vec<Value> = Vec::new();
        for seed in 0..40 {
            let genome = draw(seed * 7919, &recent[recent.len().saturating_sub(6)..]);
            for piece in &recent[recent.len().saturating_sub(6)..] {
                assert!(distance(&genome, piece) >= 5, "seed {seed} too close");
            }
            recent.push(genome);
        }
        let distinct = |field: &str| {
            recent
                .iter()
                .map(|genome| genome[field].to_string())
                .collect::<std::collections::BTreeSet<_>>()
                .len()
        };
        assert!(distinct("tonic") >= 10);
        assert_eq!(distinct("mode"), 3);
        assert!(distinct("bpm") >= 8);
        assert!(distinct("ensemble") >= 7);
        assert!(distinct("title") >= 30);
    }

    #[test]
    fn the_frame_travels_in_the_prompt_and_overrides_only_fixed_fields() {
        let genome = draw(11, &[]);
        let prompt = format!("PYMUSIC COMPOSITION: new piece\n{}\nmore", brief(&genome));
        let frame = from_prompt(&prompt).unwrap();
        let mut his = proposal(&draw(12, &[]));
        his["title"] = json!("His own title");
        let merged = impose(his.clone(), &frame);
        for field in FIXED {
            assert_eq!(merged[field], genome[field], "{field}");
        }
        for field in ["title", "theme", "answer", "sections"] {
            assert_eq!(merged[field], his[field], "{field}");
        }
        assert!(from_prompt("no genome here").is_none());
    }
}
