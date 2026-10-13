//! Numeric development controls for the recognized, working V1 arrangement.
//! This adapter changes existing expressions; it never supplies musical values.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::runtime::music_edits::{apply_python_edit, python_source_identity};

const MAX_BYTES: usize = 128 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum Control {
    AnswerOffset,
    AnswerGate,
    LeadGate,
    PhraseBreathSteps,
    AnswerPeriod,
    AnswerPhase,
    BassGate,
}

impl Control {
    fn limits(self) -> (f64, f64, bool) {
        match self {
            Self::AnswerOffset => (0.0, 2.0, false),
            Self::AnswerGate => (0.35, 2.5, false),
            Self::LeadGate => (0.35, 1.2, false),
            Self::PhraseBreathSteps => (1.0, 6.0, true),
            Self::AnswerPeriod => (2.0, 16.0, true),
            Self::AnswerPhase => (0.0, 15.0, true),
            Self::BassGate => (0.35, 1.0, false),
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::AnswerOffset => {
                "Answer entrance offset in local melody steps after its selected onset; later values create more separation from the lead."
            }
            Self::AnswerGate => {
                "Answer note length in melody steps, including release; updates both synthesis and its recorded duration."
            }
            Self::LeadGate => {
                "Lead note length in melody steps, including release; lower values leave more space between notes. Updates synthesis and recorded duration."
            }
            Self::PhraseBreathSteps => {
                "Whole silent melody steps after each motif before the next phrase; greater values leave a longer breath."
            }
            Self::AnswerPeriod => {
                "Answer once per this many melody steps in the existing answering sections; larger values make answers sparser. Must exceed the final answer_phase."
            }
            Self::AnswerPhase => {
                "Which step within answer_period triggers the answer, counting from zero; shifts its relation to the lead. Must be smaller than the final answer_period."
            }
            Self::BassGate => {
                "Bass note length in beats, including release; lower values separate the bass pulses. Updates synthesis and recorded duration."
            }
        }
    }
}

#[derive(Clone, Debug)]
struct Binding {
    line: usize,
    start: usize,
    end: usize,
    value: f64,
}

#[derive(Clone, Debug)]
struct BoundControl {
    current: f64,
    bindings: Vec<Binding>,
}

#[derive(Clone, Debug)]
pub(crate) struct DevelopmentControls {
    identity: String,
    controls: BTreeMap<Control, BoundControl>,
}

/// Match one complete numeric expression at a known source location. Byte
/// offsets include the original indentation; unrelated text remains untouched.
fn bind_number(
    lines: &[&str],
    prefix: &str,
    suffix: &str,
    tail_allowed: bool,
    previous: Option<&str>,
) -> Option<Binding> {
    let mut found = None;
    for (index, line) in lines.iter().enumerate() {
        let body = line.trim_start_matches([' ', '\t']);
        let Some(rest) = body.strip_prefix(prefix) else {
            continue;
        };
        if previous.is_some_and(|expected| index == 0 || lines[index - 1].trim() != expected) {
            continue;
        }
        let number = if suffix.is_empty() {
            rest
        } else if tail_allowed {
            rest.split_once(suffix)?.0
        } else {
            rest.strip_suffix(suffix)?
        };
        let number = number.trim();
        let value: f64 = number.parse().ok()?;
        if !value.is_finite() || found.is_some() {
            return None;
        }
        let start = line.len() - body.len() + prefix.len() + rest.len() - rest.trim_start().len();
        found = Some(Binding {
            line: index + 1,
            start,
            end: start + number.len(),
            value,
        });
    }
    found
}

fn bind_answer_condition(lines: &[&str]) -> Option<(Binding, Binding)> {
    let prefix = "if section_idx in (2, 4) and note_idx % ";
    let mut found = None;
    for (index, line) in lines.iter().enumerate() {
        let body = line.trim_start_matches([' ', '\t']);
        let Some(rest) = body.strip_prefix(prefix) else {
            continue;
        };
        let (period, phase) = rest.strip_suffix(':')?.split_once(" == ")?;
        let period_value = period.parse::<u8>().ok()?;
        let phase_value = phase.parse::<u8>().ok()?;
        if period_value == 0 || phase_value >= period_value || found.is_some() {
            return None;
        }
        let start = line.len() - body.len() + prefix.len();
        let phase_start = start + period.len() + " == ".len();
        found = Some((
            Binding {
                line: index + 1,
                start,
                end: start + period.len(),
                value: period_value.into(),
            },
            Binding {
                line: index + 1,
                start: phase_start,
                end: phase_start + phase.len(),
                value: phase_value.into(),
            },
        ));
    }
    found
}

/// Only expose controls when all known V1 bindings and their matching event
/// declarations exist uniquely. Unsupported free-form music keeps its own path.
pub(crate) fn controls(source: &str) -> Option<DevelopmentControls> {
    if source.len() > MAX_BYTES {
        return None;
    }
    let lines = source.lines().collect::<Vec<_>>();
    for anchor in [
        "TELEDRA_EVENTS = []",
        "counter_motif = list(reversed(lead_motif))",
        "motif = motif_forms[section_idx]",
        "phrase_breath = phrase_slot >= len(motif)",
        "place(\"lead\", lead, cursor)",
        "place(\"counterline\", answer, answer_start)",
        "cursor += step",
        "note_idx += 1",
    ] {
        if lines.iter().filter(|line| line.trim() == anchor).count() != 1 {
            return None;
        }
    }
    let answer_offset = bind_number(&lines, "answer_start = cursor + step * ", "", false, None)?;
    let phrase_breath = bind_number(
        &lines,
        "phrase_slot = note_idx % (len(motif) + ",
        ")",
        false,
        None,
    )?;
    if phrase_breath.value.fract() != 0.0 || phrase_breath.value < 1.0 {
        return None;
    }
    let answer_gate = bind_number(
        &lines,
        "answer_note, step * ",
        ", wave_type=",
        true,
        Some("answer = synth_note("),
    )?;
    let answer_event = bind_number(
        &lines,
        "\"note\", \"counterline\", \"motion\", answer_start, step * ",
        ", ",
        true,
        Some("record_event("),
    )?;
    let lead_gate = bind_number(
        &lines,
        "note, step * ",
        ", wave_type=",
        true,
        Some("lead = synth_note("),
    )?;
    let lead_event = bind_number(
        &lines,
        "\"note\", \"lead\", \"lead\", cursor, step * ",
        ", ",
        true,
        Some("record_event("),
    )?;
    let bass_gate = bind_number(
        &lines,
        "bass_note, BEAT * ",
        ", wave_type=",
        true,
        Some("bass = synth_note("),
    )?;
    let bass_event = bind_number(
        &lines,
        "record_event(\"note\", \"bass\", \"bass\", bass_start, BEAT * ",
        ", ",
        true,
        None,
    )?;
    let (answer_period, answer_phase) = bind_answer_condition(&lines)?;
    let mut values = BTreeMap::new();
    for (control, bindings) in [
        (Control::AnswerOffset, vec![answer_offset]),
        (Control::AnswerGate, vec![answer_gate, answer_event]),
        (Control::LeadGate, vec![lead_gate, lead_event]),
        (Control::PhraseBreathSteps, vec![phrase_breath]),
        (Control::AnswerPeriod, vec![answer_period]),
        (Control::AnswerPhase, vec![answer_phase]),
        (Control::BassGate, vec![bass_gate, bass_event]),
    ] {
        let current = bindings[0].value;
        if bindings.iter().any(|binding| binding.value != current) {
            return None;
        }
        values.insert(control, BoundControl { current, bindings });
    }
    Some(DevelopmentControls {
        identity: python_source_identity(source),
        controls: values,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Proposal {
    intent: String,
    changes: Vec<Change>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Change {
    control: Control,
    value: f64,
}

impl DevelopmentControls {
    /// Explicit preservation language narrows both the offered schema and the
    /// server-side validator; a model cannot override it with an extra control.
    pub(crate) fn for_brief(&self, brief: &str) -> Self {
        let mut restricted = self.clone();
        let lower = brief.to_ascii_lowercase();
        if [
            "preserve the lead",
            "preserve lead",
            "leave the lead unchanged",
            "keep the lead unchanged",
        ]
        .iter()
        .any(|phrase| lower.contains(phrase))
        {
            restricted.controls.remove(&Control::LeadGate);
            restricted.controls.remove(&Control::PhraseBreathSteps);
        }
        restricted
    }

    pub(crate) fn schema(&self) -> Value {
        let variants = self.controls.keys().map(|control| {
            let (minimum, maximum, integer) = control.limits();
            json!({
                "type":"object", "additionalProperties":false, "required":["control","value"],
                "properties":{
                    "control":{"type":"string", "enum":[control]},
                    "value":{"type":if integer {"integer"} else {"number"}, "minimum":minimum, "maximum":maximum}
                }
            })
        }).collect::<Vec<_>>();
        json!({
            "type":"object", "additionalProperties":false, "required":["intent","changes"],
            "properties":{
                "intent":{"type":"string", "minLength":1, "maxLength":800},
                "changes":{"type":"array", "minItems":1, "maxItems":3, "items":{"oneOf":variants}}
            }
        })
    }

    pub(crate) fn context(&self) -> String {
        let choices = self
            .controls
            .iter()
            .map(|(control, binding)| {
                let (minimum, maximum, integer) = control.limits();
                json!({"control":control, "current":binding.current, "minimum":minimum,
                "maximum":maximum, "integer":integer, "musical_effect":control.description()})
            })
            .collect::<Vec<_>>();
        let protection = if self.controls.contains_key(&Control::LeadGate) {
            ""
        } else {
            "\nPROTECTED LEAD: The brief explicitly preserves the lead. Its gate and phrase-breath controls are unavailable; develop the answer or bass without changing lead timing or articulation."
        };
        format!(
            "AVAILABLE MUSICAL DEVELOPMENT CONTROLS, read from this accepted arrangement:\n{}\nChoose one to three different controls and concrete numeric values that serve the musical brief. Do not return an unchanged value. For answer_period and answer_phase, the final phase must be smaller than the final period; change both together when needed. Existing notes, instruments, key, tempo, sections and placement code remain in place; coupled audio/event durations change together. Return no Python or source-line addresses.{}",
            serde_json::to_string_pretty(&choices).unwrap(),
            protection
        )
    }

    pub(crate) fn apply(&self, source: &str, response: &str) -> Result<String, String> {
        if self.identity != python_source_identity(source) {
            return Err("Development controls refer to a stale accepted source".into());
        }
        if response.len() > MAX_BYTES {
            return Err("Development response exceeds 128 KiB".into());
        }
        let proposal: Proposal = serde_json::from_str(response)
            .map_err(|error| format!("Invalid musical control JSON: {error}"))?;
        if proposal.intent.trim().is_empty() || proposal.intent.chars().count() > 800 {
            return Err("Describe the musical intent in 1 to 800 characters".into());
        }
        if !(1..=3).contains(&proposal.changes.len()) {
            return Err("Choose one to three musical controls".into());
        }
        let mut final_values = self
            .controls
            .iter()
            .map(|(control, binding)| (*control, binding.current))
            .collect::<BTreeMap<_, _>>();
        let mut selected = BTreeSet::new();
        let mut replacements: BTreeMap<usize, Vec<(usize, usize, String)>> = BTreeMap::new();
        for change in proposal.changes {
            if !selected.insert(change.control) {
                return Err("Choose each musical control only once".into());
            }
            let binding = self.controls.get(&change.control).ok_or_else(|| {
                format!(
                    "{:?} is unavailable for this brief; use one of the offered musical controls",
                    change.control
                )
            })?;
            let (minimum, maximum, integer) = change.control.limits();
            if !change.value.is_finite()
                || !(minimum..=maximum).contains(&change.value)
                || (integer && change.value.fract() != 0.0)
            {
                return Err(format!(
                    "{:?} requires {} in {minimum}..={maximum}",
                    change.control,
                    if integer { "an integer" } else { "a number" }
                ));
            }
            if change.value == binding.current {
                return Err(format!(
                    "{:?} is already {}; choose a changed musical value",
                    change.control, binding.current
                ));
            }
            final_values.insert(change.control, change.value);
            let literal = change.value.to_string();
            for site in &binding.bindings {
                replacements.entry(site.line).or_default().push((
                    site.start,
                    site.end,
                    literal.clone(),
                ));
            }
        }
        if final_values[&Control::AnswerPhase] >= final_values[&Control::AnswerPeriod] {
            return Err("The final answer_phase must be less than answer_period, or no answer would ever trigger; change both controls together when needed".into());
        }
        let lines = source.lines().collect::<Vec<_>>();
        let mut edits = Vec::new();
        for (line, mut changes) in replacements {
            changes.sort_unstable_by_key(|(start, _, _)| *start);
            if changes.windows(2).any(|pair| pair[0].1 > pair[1].0) {
                return Err("Musical control source bindings overlap".into());
            }
            let mut body = lines[line - 1].to_owned();
            for (start, end, replacement) in changes.into_iter().rev() {
                body.replace_range(start..end, &replacement);
            }
            edits.push(json!({"line":line, "new":body.trim_start_matches([' ', '\t'])}));
        }
        let payload =
            json!({"base":self.identity, "intent":proposal.intent, "edits":edits}).to_string();
        let revised = apply_python_edit(source, &payload)?;
        let next = controls(&revised)
            .ok_or_else(|| "Revised source no longer matches its musical bindings".to_owned())?;
        if final_values
            .iter()
            .any(|(control, value)| next.controls[control].current != *value)
        {
            return Err("Revised source did not retain every chosen musical control".into());
        }
        Ok(format!("[PYTHON_MUSIC_EDIT: {payload}]"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1: &str = include_str!("../../Music/v1_music_example.py");

    fn request(changes: Value) -> String {
        json!({"intent":"Develop the answer while preserving the working ensemble", "changes":changes}).to_string()
    }

    fn apply(source: &str, changes: Value) -> (String, Value) {
        let adapter = controls(source).expect("the actual V1 arrangement should expose controls");
        let tag = adapter.apply(source, &request(changes)).unwrap();
        let parsed = teledra_court::parse_response(&tag);
        let [teledra_court::CourtAction::CreativeArtifact { kind, content }] =
            parsed.actions.as_slice()
        else {
            panic!("one source edit expected")
        };
        assert_eq!(kind, "python_music_edit");
        (
            apply_python_edit(source, content).unwrap(),
            serde_json::from_str(content).unwrap(),
        )
    }

    #[test]
    fn preserving_the_lead_excludes_and_rejects_its_timing_controls() {
        let all = controls(V1).unwrap();
        assert_eq!(
            all.for_brief("Develop the answering rhythm").controls.len(),
            7
        );
        for brief in [
            "Preserve the lead and improve the counterline.",
            "preserve lead timing",
            "Leave the lead unchanged; vary the answer.",
            "KEEP THE LEAD UNCHANGED while the bass becomes lighter.",
        ] {
            let restricted = all.for_brief(brief);
            assert_eq!(restricted.controls.len(), 5);
            assert!(restricted.context().contains("PROTECTED LEAD"));
            let schema = restricted.schema().to_string();
            assert!(!schema.contains("lead_gate"));
            assert!(!schema.contains("phrase_breath_steps"));
            for (control, value) in [("lead_gate", 0.7), ("phrase_breath_steps", 3.0)] {
                let error = restricted
                    .apply(
                        V1,
                        &request(json!([
                            {"control":"answer_offset", "value":0.75},
                            {"control":control, "value":value}
                        ])),
                    )
                    .unwrap_err();
                assert!(error.contains("unavailable"));
            }
            let tag = restricted
                .apply(
                    V1,
                    &request(json!([
                        {"control":"answer_offset", "value":0.75}
                    ])),
                )
                .unwrap();
            let parsed = teledra_court::parse_response(&tag);
            let [teledra_court::CourtAction::CreativeArtifact { content, .. }] =
                parsed.actions.as_slice()
            else {
                panic!("one permitted answer edit expected")
            };
            let revised = apply_python_edit(V1, content).unwrap();
            assert_eq!(
                revised,
                V1.replace(
                    "answer_start = cursor + step * 0.5",
                    "answer_start = cursor + step * 0.75"
                ),
                "an allowed answer change must preserve every lead/phrase byte and all unrelated source"
            );
        }
        assert_eq!(
            all.controls.len(),
            7,
            "restriction must not mutate the default control set"
        );
    }

    #[test]
    fn actual_v1_values_and_coupled_gates_are_detected_without_fixed_defaults() {
        let adapter = controls(V1).unwrap();
        assert_eq!(adapter.controls.len(), 7);
        assert_eq!(adapter.controls[&Control::AnswerOffset].current, 0.5);
        assert_eq!(adapter.controls[&Control::AnswerGate].current, 1.4);
        assert_eq!(adapter.controls[&Control::LeadGate].current, 0.82);
        assert_eq!(adapter.controls[&Control::PhraseBreathSteps].current, 2.0);
        assert_eq!(adapter.controls[&Control::AnswerPeriod].current, 8.0);
        assert_eq!(adapter.controls[&Control::AnswerPhase].current, 2.0);
        assert_eq!(adapter.controls[&Control::BassGate].current, 0.82);
        assert_eq!(
            adapter.schema()["properties"]["changes"]["items"]["oneOf"]
                .as_array()
                .unwrap()
                .len(),
            7
        );
        assert!(adapter.context().contains("Answer note length"));

        let (revised, payload) = apply(
            V1,
            json!([
                {"control":"answer_gate","value":1.1},
                {"control":"lead_gate","value":0.7},
                {"control":"bass_gate","value":0.65}
            ]),
        );
        assert_eq!(payload["edits"].as_array().unwrap().len(), 6);
        let expected = V1
            .replace("answer_note, step * 1.4,", "answer_note, step * 1.1,")
            .replace("answer_start, step * 1.4,", "answer_start, step * 1.1,")
            .replace("note, step * 0.82,", "note, step * 0.7,")
            .replace("cursor, step * 0.82,", "cursor, step * 0.7,")
            .replace("bass_note, BEAT * 0.82,", "bass_note, BEAT * 0.65,")
            .replace("bass_start, BEAT * 0.82,", "bass_start, BEAT * 0.65,");
        assert_eq!(
            revised, expected,
            "all unrelated bytes and duration declarations must remain exact"
        );
        let next = controls(&revised).unwrap();
        assert_eq!(next.controls[&Control::AnswerGate].current, 1.1);
        assert_eq!(next.controls[&Control::LeadGate].current, 0.7);
        assert_eq!(next.controls[&Control::BassGate].current, 0.65);
        assert!(
            adapter
                .apply(
                    &revised,
                    &request(json!([{"control":"answer_offset","value":0.75}]))
                )
                .unwrap_err()
                .contains("stale")
        );
    }

    #[test]
    fn period_and_phase_merge_transactionally_and_recursive_offsets_keep_raw_newlines() {
        let raw = V1.replace("\r\n", "\n").replace('\n', "\r\n");
        let (revised, payload) = apply(
            &raw,
            json!([
                {"control":"answer_period","value":4},
                {"control":"answer_phase","value":1},
                {"control":"phrase_breath_steps","value":3}
            ]),
        );
        assert_eq!(payload["edits"].as_array().unwrap().len(), 2);
        assert_eq!(payload["base"], python_source_identity(&raw));
        assert_eq!(
            revised,
            raw.replace("note_idx % 8 == 2:", "note_idx % 4 == 1:")
                .replace("len(motif) + 2)", "len(motif) + 3)")
        );
        let (again, _) = apply(&revised, json!([{"control":"answer_offset","value":0.75}]));
        assert_eq!(
            again,
            revised.replace(
                "answer_start = cursor + step * 0.5",
                "answer_start = cursor + step * 0.75"
            )
        );
        assert_eq!(
            controls(&again).unwrap().controls[&Control::AnswerOffset].current,
            0.75
        );
    }

    #[test]
    fn invalid_controls_never_produce_a_partial_edit() {
        let adapter = controls(V1).unwrap();
        for changes in [
            json!([]),
            json!([{"control":"missing","value":1}]),
            json!([{"control":"answer_offset","value":0.5}]),
            json!([{"control":"answer_gate","value":50}]),
            json!([{"control":"answer_offset","value":-1}]),
            json!([{"control":"phrase_breath_steps","value":2.5}]),
            json!([{"control":"answer_period","value":2}]),
            json!([{"control":"answer_phase","value":8}]),
            json!([{"control":"answer_gate","value":1.1},{"control":"answer_gate","value":1.2}]),
            json!([{"control":"answer_offset","value":"1; play_sound(x)"}]),
            json!([{"control":"answer_offset","value":0.75,"new":"malicious source"}]),
        ] {
            assert!(adapter.apply(V1, &request(changes)).is_err());
        }
        assert!(
            adapter
                .apply(
                    V1,
                    r#"{"intent":"x","changes":[{"control":"answer_offset","value":NaN}]}"#
                )
                .is_err()
        );
        assert!(
            adapter
                .apply(
                    V1,
                    &json!({"intent":"", "changes":[{"control":"answer_offset","value":0.75}]})
                        .to_string()
                )
                .is_err()
        );
        assert!(adapter.apply(V1, &json!({"intent":"x", "base":"invented", "changes":[{"control":"answer_offset","value":0.75}]}).to_string()).is_err());
    }

    #[test]
    fn unsupported_or_ambiguous_music_does_not_acquire_v1_controls() {
        assert!(controls("answer_start = cursor + step * 0.5\nplay_sound(x)").is_none());
        assert!(
            controls(&V1.replace("answer_start, step * 1.4,", "answer_start, step * 1.3,"))
                .is_none()
        );
        assert!(controls(&format!("{V1}\nanswer_start = cursor + step * 0.5\n")).is_none());
        assert!(
            controls(&V1.replace("answer = synth_note(", "other_voice = synth_note(")).is_none()
        );
        assert!(
            controls(&V1.replace(
                "place(\"counterline\", answer, answer_start)",
                "place(\"counterline\", answer, cursor)"
            ))
            .is_none()
        );
    }
}
