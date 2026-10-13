//! A compact musical recipe; PyMusic, not the language model, owns array placement.
use serde_json::{Value, json};

pub(crate) const CONTRACT: &str = "PYMUSIC SONG RECIPE: Return the supplied JSON song, not Python source. You choose the actual theme and answering notes, harmonic progression, timbres, rhythm and form. Each theme/answer is an eight-beat phrase: 4-10 sorted non-overlapping events {beat, degree, length, velocity}. Degree 0 is tonic; 7 is tonic one octave up. Leave audible rests, vary durations/onset spacing, use leaps and returns rather than a repeated ascending/descending scale. The renderer relates strong melodic landings to the active chord and gives bass/harmony their own register and rhythm. progression is four scale degrees 0..6, one chord per bar. Supply four distinct sections of 4 or 8 bars: a restrained arrival, theme, contrasting answer/development, and recognizable return. At least one section uses theme and one answer or variation. Section melody 'none' leaves space for accompanying layers. Energy controls dynamics and entrances. pulse soft/syncopated/flowing changes accompaniment timing. For restored PyMusic timbre use classic:sine, classic:triangle, classic:pluck, classic:bell, classic:pad, classic:keys, classic:bass; each of the 72 instrument IDs is also available. Choose complementary voices for lead, answer, harmony, bass, texture. No raw NumPy, device calls, scales masquerading as themes, or metadata-only changes. The runtime compiles this recipe into editable Python, renders the whole arrangement, checks every scheduled placement and archives a verified track. Verification is not a claim the listener likes it.";

pub(crate) const PROPOSAL_CONTRACT: &str = concat!(
    "PYMUSIC MUSICAL PROPOSAL: Return the supplied JSON object, not Python source or a full note-event array. ",
    "For each theme and answer choose {rhythm, degrees, velocities}. rhythm is one named eight-beat pattern below. ",
    "degrees contains exactly six integer scale degrees 0..13; 0 is tonic and 7 is tonic an octave above. ",
    "velocities contains exactly six values selected from 0.35,0.5,0.65,0.8,1. You author all six pitches and dynamics in order. ",
    "Choose a recognizable contour with leaps, returns and at least three distinct pitches; a run of five ascending or descending scale steps is rejected. ",
    "The answer must change pitch or rhythm, not only loudness. Choose its rhythm independently. ",
    "Timing is supplied by the selected pattern; do not calculate or return beat/length fields. ",
    "Available rhythms (six onset beats followed by the corresponding six note lengths in beats): ",
    "question: starts [0,1.5,2.5,4,5.5,7], lengths [1,0.5,0.5,1,0.75,0.5]; ",
    "answer: starts [0.5,2,3,4.5,6,7], lengths [0.75,0.5,1,0.75,0.5,0.5]; ",
    "syncopated: starts [0,1,2.5,3.5,5,6.5], lengths [0.5,0.75,0.5,1,0.5,1]; ",
    "arch: starts [0.5,1.5,3,4.5,5.5,7], lengths [0.5,1,0.75,0.5,1,0.5]; ",
    "held_return: starts [0,2,3,4.5,6,7], lengths [1.5,0.5,0.75,1,0.5,0.5]; ",
    "displaced: starts [0.5,1.5,2.5,4,5,6.5], lengths [0.5,0.75,0.5,0.5,1,1]. ",
    "All patterns include rests and finish within the phrase. All other song fields keep their supplied schema: ",
    "choose title, tonic/mode, BPM, four progression roots (one chord per bar), complementary lead/answer/harmony/bass/texture voices, ",
    "four sections of four or eight bars with a deliberate dynamic arc, and accompaniment pulse. Include a theme section and an answer or variation section. ",
    "Use exact native catalog or classic voice IDs from the schema. The runtime expands your choices to explicit events, ",
    "retains their pitches and velocities, then validates and renders the whole piece. Technical verification is separate from listener approval."
);

struct PhraseRhythm {
    name: &'static str,
    beats: [f64; 6],
    lengths: [f64; 6],
}

const PHRASE_RHYTHMS: [PhraseRhythm; 6] = [
    PhraseRhythm {
        name: "question",
        beats: [0., 1.5, 2.5, 4., 5.5, 7.],
        lengths: [1., 0.5, 0.5, 1., 0.75, 0.5],
    },
    PhraseRhythm {
        name: "answer",
        beats: [0.5, 2., 3., 4.5, 6., 7.],
        lengths: [0.75, 0.5, 1., 0.75, 0.5, 0.5],
    },
    PhraseRhythm {
        name: "syncopated",
        beats: [0., 1., 2.5, 3.5, 5., 6.5],
        lengths: [0.5, 0.75, 0.5, 1., 0.5, 1.],
    },
    PhraseRhythm {
        name: "arch",
        beats: [0.5, 1.5, 3., 4.5, 5.5, 7.],
        lengths: [0.5, 1., 0.75, 0.5, 1., 0.5],
    },
    PhraseRhythm {
        name: "held_return",
        beats: [0., 2., 3., 4.5, 6., 7.],
        lengths: [1.5, 0.5, 0.75, 1., 0.5, 0.5],
    },
    PhraseRhythm {
        name: "displaced",
        beats: [0.5, 1.5, 2.5, 4., 5., 6.5],
        lengths: [0.5, 0.75, 0.5, 0.5, 1., 1.],
    },
];

/// The model selects musical timing from complete phrases; saved songs retain
/// their existing explicit event representation and its full validation rules.
pub(crate) fn proposal_schema() -> Value {
    let mut proposal = schema();
    // Classic recipes have explicit performance roles. Avoid a blanket pad
    // selection flattening every line into the same slow envelope.
    for (role, allowed) in [
        (
            "lead",
            &[
                "classic:keys",
                "classic:pluck",
                "classic:bell",
                "classic:triangle",
                "classic:sine",
            ][..],
        ),
        (
            "answer",
            &[
                "classic:keys",
                "classic:pluck",
                "classic:bell",
                "classic:triangle",
                "classic:sine",
            ][..],
        ),
        (
            "bass",
            &["classic:bass", "classic:sine", "classic:triangle"][..],
        ),
        (
            "harmony",
            &[
                "classic:pad",
                "classic:keys",
                "classic:sine",
                "classic:triangle",
            ][..],
        ),
    ] {
        proposal["properties"]["voices"]["properties"][role]["enum"]
            .as_array_mut()
            .unwrap()
            .retain(|value| {
                value
                    .as_str()
                    .is_some_and(|id| !id.starts_with("classic:") || allowed.contains(&id))
            });
    }
    let degree = proposal["properties"]["theme"]["items"]["properties"]["degree"].clone();
    let velocity = proposal["properties"]["theme"]["items"]["properties"]["velocity"].clone();
    let rhythm_names: Vec<_> = PHRASE_RHYTHMS.iter().map(|rhythm| rhythm.name).collect();
    let phrase = json!({"type":"object","additionalProperties":false,
        "required":["rhythm","degrees","velocities"],"properties":{
            "rhythm":{"type":"string","enum":rhythm_names},
            "degrees":{"type":"array","minItems":6,"maxItems":6,"items":degree},
            "velocities":{"type":"array","minItems":6,"maxItems":6,"items":velocity}}});
    proposal["properties"]["theme"] = phrase.clone();
    proposal["properties"]["answer"] = phrase;
    proposal
}

pub(crate) fn expand_proposal(mut proposal: Value) -> Result<Value, String> {
    validate_schema(&proposal, &proposal_schema(), "proposal")?;
    for name in ["theme", "answer"] {
        let phrase = &proposal[name];
        let rhythm = PHRASE_RHYTHMS
            .iter()
            .find(|rhythm| phrase["rhythm"] == rhythm.name)
            .ok_or_else(|| format!("{name}: select an available phrase rhythm"))?;
        let events: Vec<_> = (0..6)
            .map(|index| {
                json!({
                    "beat":rhythm.beats[index], "length":rhythm.lengths[index],
                    "degree":phrase["degrees"][index], "velocity":phrase["velocities"][index]
                })
            })
            .collect();
        proposal[name] = Value::Array(events);
    }
    validate(&proposal)?;
    Ok(proposal)
}

/// Only exact supported rhythms are reversible. Do not quantize an authored
/// phrase to make it fit the proposal interface and silently change its timing.
pub(crate) fn proposal_from_song(song: &Value) -> Option<Value> {
    validate(song).ok()?;
    let mut proposal = song.clone();
    for name in ["theme", "answer"] {
        let events = song[name].as_array()?;
        if events.len() != 6 {
            return None;
        }
        let rhythm = PHRASE_RHYTHMS.iter().find(|rhythm| {
            events.iter().enumerate().all(|(index, event)| {
                event["beat"].as_f64() == Some(rhythm.beats[index])
                    && event["length"].as_f64() == Some(rhythm.lengths[index])
            })
        })?;
        proposal[name] = json!({"rhythm":rhythm.name,
            "degrees":events.iter().map(|event| event["degree"].clone()).collect::<Vec<_>>(),
            "velocities":events.iter().map(|event| event["velocity"].clone()).collect::<Vec<_>>()});
    }
    Some(proposal)
}

pub(crate) fn schema() -> Value {
    let mut voices = teledra_music::Instrument::ALL
        .iter()
        .map(|v| v.info().id.to_owned())
        .collect::<Vec<_>>();
    voices.extend(
        ["sine", "triangle", "pluck", "bell", "pad", "keys", "bass"]
            .map(|s| format!("classic:{s}")),
    );
    let phrase = json!({"type":"array","minItems":4,"maxItems":10,"items":{
        "type":"object","additionalProperties":false,"required":["beat","degree","length","velocity"],
        "properties":{"beat":{"type":"number","enum":[0,0.5,1,1.5,2,2.5,3,3.5,4,4.5,5,5.5,6,6.5,7,7.5]},"degree":{"type":"integer","enum":[0,1,2,3,4,5,6,7,8,9,10,11,12,13]},
            "length":{"type":"number","enum":[0.25,0.5,0.75,1,1.5,2]},"velocity":{"type":"number","enum":[0.35,0.5,0.65,0.8,1]}}}});
    let voice = json!({"type":"string","enum":voices});
    let pitched = |bass: bool| {
        json!({"type":"string","enum":voices.iter().filter(|id| {
        id.starts_with("classic:") || teledra_music::Instrument::ALL.iter().any(|v| v.info().id == id.as_str() && !v.is_percussive() && (!bass || v.info().range[0] <= 48))
    }).collect::<Vec<_>>()})
    };
    json!({"type":"object","additionalProperties":false,
    "required":["title","tonic","mode","bpm","progression","voices","theme","answer","sections","pulse"],
    "properties":{
        "title":{"type":"string","minLength":1,"maxLength":100},
        "tonic":{"type":"string","enum":["C","Db","D","Eb","E","F","Gb","G","Ab","A","Bb","B"]},
            "mode":{"type":"string","enum":["major","minor","dorian"]},"bpm":{"type":"number","enum":[72,80,84,88,92,96,100,108,112,120,128]},
            "progression":{"type":"array","minItems":4,"maxItems":4,"items":{"type":"integer","enum":[0,1,2,3,4,5,6]}},
        "voices":{"type":"object","additionalProperties":false,"required":["lead","answer","harmony","bass","texture"],
            "properties":{"lead":pitched(false),"answer":pitched(false),"harmony":pitched(false),"bass":pitched(true),"texture":voice}},
        "theme":phrase,"answer":phrase,
        "sections":{"type":"array","minItems":4,"maxItems":4,"items":{"type":"object","additionalProperties":false,
            "required":["name","bars","melody","energy"],"properties":{
                "name":{"type":"string","minLength":1,"maxLength":50},"bars":{"type":"integer","enum":[4,8]},
                    "melody":{"type":"string","enum":["none","theme","answer","variation"]},"energy":{"type":"number","enum":[0.35,0.45,0.55,0.65,0.75,0.85,1]}}}},
        "pulse":{"type":"string","enum":["soft","syncopated","flowing"]}
    }})
}

// Validate against the same bounded schema before any Python worker starts.
fn validate_schema(value: &Value, rule: &Value, path: &str) -> Result<(), String> {
    let valid = match rule["type"].as_str().unwrap_or("") {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.is_i64(),
        "number" => value.as_f64().is_some_and(f64::is_finite),
        _ => false,
    };
    if !valid {
        return Err(format!("{path}: expected {}", rule["type"]));
    }
    if let Some(options) = rule["enum"].as_array()
        && !options.iter().any(|option| {
            option == value
                || option
                    .as_f64()
                    .zip(value.as_f64())
                    .is_some_and(|(a, b)| (a - b).abs() < 0.000000001)
        })
    {
        return Err(format!(
            "{path}: {value} is invalid; select one of {options:?}"
        ));
    }
    if let Some(object) = value.as_object() {
        for key in rule["required"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if !object.contains_key(key) {
                return Err(format!("{path}: missing {key}"));
            }
        }
        for (key, v) in object {
            let Some(child) = rule["properties"].get(key) else {
                return Err(format!("{path}: unknown field {key}"));
            };
            validate_schema(v, child, &format!("{path}.{key}"))?;
        }
    }
    if let Some(items) = value.as_array() {
        if items.len() < rule["minItems"].as_u64().unwrap_or(0) as usize
            || items.len() > rule["maxItems"].as_u64().unwrap_or(u64::MAX) as usize
        {
            return Err(format!("{path}: wrong item count"));
        }
        for item in items {
            validate_schema(item, &rule["items"], path)?;
        }
    }
    if let Some(s) = value.as_str()
        && (s.trim().is_empty()
            || s.chars().count() > rule["maxLength"].as_u64().unwrap_or(u64::MAX) as usize)
    {
        return Err(format!("{path}: text is empty or too long"));
    }
    if let Some(n) = value.as_f64()
        && (n < rule["minimum"].as_f64().unwrap_or(f64::MIN)
            || n > rule["maximum"].as_f64().unwrap_or(f64::MAX))
    {
        return Err(format!("{path}: value outside musical bounds"));
    }
    Ok(())
}

pub(crate) fn validate(song: &Value) -> Result<(), String> {
    validate_schema(song, &schema(), "song")?;
    let mut errors = Vec::<String>::new();
    let phrase_identity = |name: &str| {
        song[name]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                (
                    e["beat"].as_f64().unwrap(),
                    e["degree"].as_i64().unwrap(),
                    e["length"].as_f64().unwrap(),
                )
            })
            .collect::<Vec<_>>()
    };
    if phrase_identity("theme") == phrase_identity("answer") {
        errors.push("The answer must develop pitch or rhythm, not just velocity".into());
    }
    for name in ["theme", "answer"] {
        let notes = song[name].as_array().unwrap();
        let mut end = 0.0;
        let mut sounding = 0.0;
        let mut gap: f64 = 0.0;
        let mut pitches = std::collections::BTreeSet::new();
        for note in notes {
            let beat = note["beat"].as_f64().unwrap();
            let length = note["length"].as_f64().unwrap();
            if beat + 0.0001 < end || beat + length > 7.75 {
                errors.push(format!(
                    "{name}: events must be sorted, non-overlapping and finish by beat 7.75"
                ));
            }
            if end > 0.0 {
                gap = gap.max(beat - end);
            }
            end = beat + length;
            sounding += length;
            pitches.insert(note["degree"].as_i64().unwrap());
        }
        if gap < 0.2 {
            errors.push(format!(
                "{name}: leave an internal breath of at least 0.2 beats"
            ));
        }
        let varied_lengths = notes.iter().any(|n| {
            (n["length"].as_f64().unwrap() - notes[0]["length"].as_f64().unwrap()).abs() > 0.000001
        });
        let onset_intervals = notes
            .windows(2)
            .map(|p| p[1]["beat"].as_f64().unwrap() - p[0]["beat"].as_f64().unwrap())
            .collect::<Vec<_>>();
        if !varied_lengths
            && onset_intervals
                .iter()
                .all(|n| (n - onset_intervals[0]).abs() < 0.001)
        {
            errors.push(format!(
                "{name}: vary note lengths or onset spacing to shape the phrase"
            ));
        }
        if sounding > 6.8 || pitches.len() < 3 {
            errors.push(format!(
                "{name}: leave at least 1.2 beats of breathing space and use a developed pitch contour"
            ));
        }
        let scalar = notes.windows(notes.len().min(5)).any(|run| {
            let steps = run
                .windows(2)
                .map(|pair| {
                    pair[1]["degree"].as_i64().unwrap() - pair[0]["degree"].as_i64().unwrap()
                })
                .collect::<Vec<_>>();
            steps.iter().all(|s| *s == 1) || steps.iter().all(|s| *s == -1)
        });
        if scalar {
            errors.push(format!(
                "{name}: five consecutive scale steps are an exercise; write a phrase with leaps, returns and rests"
            ));
        }
    }
    let sections = song["sections"].as_array().unwrap();
    let bars: u64 = sections.iter().map(|s| s["bars"].as_u64().unwrap()).sum();
    if bars as f64 * 240.0 / song["bpm"].as_f64().unwrap() < 30.0 {
        errors.push("A complete library track must span at least 30 seconds".into());
    }
    if !sections.iter().any(|s| s["melody"] == "theme")
        || !sections
            .iter()
            .any(|s| s["melody"] == "answer" || s["melody"] == "variation")
    {
        errors.push("The form needs a theme and a contrasting answer/development".into());
    }
    if sections
        .iter()
        .all(|s| s["energy"] == sections[0]["energy"])
    {
        errors.push("Give the sections a deliberate dynamic arc".into());
    }
    if song["progression"]
        .as_array()
        .unwrap()
        .iter()
        .all(|d| d == &song["progression"][0])
    {
        errors
            .push("Use harmonic motion and a prepared return, not a single repeated chord".into());
    }
    if errors.is_empty() {
        Ok(())
    } else {
        errors.sort();
        errors.dedup();
        Err(errors.join("; "))
    }
}

pub(crate) fn source(song: &Value) -> Result<String, String> {
    validate(song)?;
    // This schema contains no nulls or booleans, so JSON is also a Python literal.
    Ok(format!(
        "from teledra_synth import render_song, play_sound\nSONG = {}\nplay_sound(render_song(SONG), sr=44100, loop=True)\n",
        serde_json::to_string(song).map_err(|e| e.to_string())?
    ))
}

pub(crate) fn from_source(source: &str) -> Option<Value> {
    let payload = source
        .lines()
        .find_map(|line| line.strip_prefix("SONG = "))?;
    let song = serde_json::from_str(payload).ok()?;
    validate(&song).ok()?;
    Some(song)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn example() -> Value {
        json!({"title":"A remembered turn","tonic":"A","mode":"minor","bpm":96,"progression":[0,3,5,4],
            "voices":{"lead":"classic:keys","answer":"classic:bell","harmony":"classic:pad","bass":"classic:bass","texture":"classic:triangle"},
            "theme":[{"beat":0,"degree":4,"length":1,"velocity":0.8},{"beat":1.5,"degree":7,"length":0.5,"velocity":0.65},{"beat":3,"degree":6,"length":1,"velocity":0.65},{"beat":5,"degree":2,"length":1.5,"velocity":0.8}],
            "answer":[{"beat":0.5,"degree":2,"length":0.5,"velocity":0.65},{"beat":2,"degree":5,"length":1,"velocity":0.5},{"beat":4,"degree":3,"length":0.75,"velocity":0.65},{"beat":6,"degree":0,"length":1.5,"velocity":0.65}],
            "sections":[{"name":"arrival","bars":4,"melody":"none","energy":0.45},{"name":"statement","bars":8,"melody":"theme","energy":0.65},{"name":"answer","bars":8,"melody":"answer","energy":0.85},{"name":"return","bars":4,"melody":"variation","energy":0.55}],"pulse":"syncopated"})
    }
    #[test]
    fn recipe_roundtrips_and_rejects_tiny_scalar_or_unformed_tracks() {
        let mut song = example();
        assert_eq!(from_source(&source(&song).unwrap()), Some(song.clone()));
        song["theme"] = json!([0, 1, 2, 3, 4].map(
            |d| json!({"degree":d,"beat":d,"length":if d==0 {0.75} else {0.5},"velocity":0.65})
        ));
        assert!(validate(&song).unwrap_err().contains("scale steps"));
        song = example();
        song["sections"][2]["melody"] = json!("theme");
        song["sections"][3]["melody"] = json!("theme");
        assert!(validate(&song).unwrap_err().contains("contrasting"));
        song = example();
        song["voices"]["lead"] = json!("invented_voice");
        assert!(validate(&song).is_err());
    }

    pub(crate) fn proposal(theme_rhythm: &str, answer_rhythm: &str) -> Value {
        let mut proposed = example();
        proposed["theme"] = json!({"rhythm":theme_rhythm,"degrees":[4,7,6,2,5,0],
            "velocities":[0.8,0.65,0.5,0.8,0.65,1]});
        proposed["answer"] = json!({"rhythm":answer_rhythm,"degrees":[2,5,3,7,4,0],
            "velocities":[0.65,0.5,0.8,0.65,0.5,0.8]});
        proposed
    }

    #[test]
    fn proposal_schema_preserves_controls_and_uses_roles_for_classic_voices() {
        let full = schema();
        let compact = proposal_schema();
        assert_eq!(compact["required"], full["required"]);
        for (name, property) in full["properties"].as_object().unwrap() {
            if name != "theme" && name != "answer" && name != "voices" {
                assert_eq!(&compact["properties"][name], property, "{name}");
            }
        }
        let selected = &compact["properties"]["voices"]["properties"];
        assert!(
            !selected["lead"]["enum"]
                .as_array()
                .unwrap()
                .contains(&json!("classic:pad"))
        );
        assert!(
            !selected["bass"]["enum"]
                .as_array()
                .unwrap()
                .contains(&json!("classic:pad"))
        );
        assert!(
            selected["harmony"]["enum"]
                .as_array()
                .unwrap()
                .contains(&json!("classic:pad"))
        );
        for voice in teledra_music::Instrument::ALL {
            assert!(
                selected["texture"]["enum"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(voice.info().id))
            );
        }
        for name in ["theme", "answer"] {
            assert_eq!(compact["properties"][name]["type"], "object");
            assert_eq!(
                compact["properties"][name]["properties"]["degrees"]["minItems"],
                6
            );
            assert_eq!(
                compact["properties"][name]["properties"]["velocities"]["maxItems"],
                6
            );
        }
    }

    #[test]
    fn every_rhythm_pair_expands_without_overlap_or_lost_notes_and_roundtrips_exactly() {
        for theme in &PHRASE_RHYTHMS {
            for answer in &PHRASE_RHYTHMS {
                let proposed = proposal(theme.name, answer.name);
                let song = expand_proposal(proposed.clone()).unwrap();
                for (name, rhythm) in [("theme", theme), ("answer", answer)] {
                    let events = song[name].as_array().unwrap();
                    assert_eq!(events.len(), 6);
                    let mut end = 0.0;
                    let mut internal_gap: f64 = 0.0;
                    for (index, event) in events.iter().enumerate() {
                        let start = event["beat"].as_f64().unwrap();
                        let length = event["length"].as_f64().unwrap();
                        assert_eq!(start, rhythm.beats[index]);
                        assert_eq!(length, rhythm.lengths[index]);
                        assert!(start >= end);
                        if index > 0 {
                            internal_gap = internal_gap.max(start - end);
                        }
                        end = start + length;
                        assert!(end <= 7.75);
                        assert_eq!(event["degree"], proposed[name]["degrees"][index]);
                        assert_eq!(event["velocity"], proposed[name]["velocities"][index]);
                    }
                    assert!(internal_gap >= 0.2);
                }
                assert_eq!(proposal_from_song(&song), Some(proposed));
                assert_eq!(from_source(&source(&song).unwrap()), Some(song));
            }
        }
    }

    #[test]
    fn proposal_adapter_keeps_contour_and_contrast_validation() {
        let mut proposed = proposal("question", "answer");
        proposed["theme"]["degrees"] = json!([0, 1, 2, 3, 4, 5]);
        assert!(
            expand_proposal(proposed)
                .unwrap_err()
                .contains("scale steps")
        );
        let mut proposed = proposal("arch", "arch");
        proposed["answer"]["degrees"] = proposed["theme"]["degrees"].clone();
        assert!(
            expand_proposal(proposed)
                .unwrap_err()
                .contains("not just velocity")
        );
        let mut proposed = proposal("held_return", "displaced");
        proposed["theme"]["degrees"] = json!([0, 0, 0, 0, 0, 0]);
        assert!(
            expand_proposal(proposed)
                .unwrap_err()
                .contains("developed pitch contour")
        );
    }

    #[test]
    fn malformed_proposals_are_rejected_before_expansion() {
        for (field, value) in [
            ("degrees", json!([0, 4, 2, 6, 3])),
            ("degrees", json!([0, 4, 2, 6, 3, 14])),
            ("velocities", json!([0.8, 0.5, 0.65, 0.42, 1, 0.8])),
            ("rhythm", json!("invented")),
            ("beat", json!(0.0)),
        ] {
            let mut proposed = proposal("question", "answer");
            proposed["theme"][field] = value;
            assert!(expand_proposal(proposed).is_err(), "{field}");
        }
    }

    #[test]
    fn proposal_inverse_does_not_quantize_existing_custom_phrases() {
        assert!(
            proposal_from_song(&example()).is_none(),
            "four-note source stays authored"
        );
        let mut song = expand_proposal(proposal("question", "answer")).unwrap();
        song["theme"][0]["beat"] = json!(0.5);
        validate(&song).unwrap();
        assert!(
            proposal_from_song(&song).is_none(),
            "custom six-note timing must not be rounded"
        );
    }
}
