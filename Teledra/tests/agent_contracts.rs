use teledra_agents::AgentId;
use teledra_court::{Court, CourtAction, parse_response};

#[test]
fn the_compact_organist_contract_uses_executable_note_events_and_grid() {
    let profile = AgentId::Organist.profile();
    assert!(profile.contains("[COURT_SCORE: {JSON}]"));
    assert!(profile.contains("PRESERVE") && profile.contains("REPLACE"));
    assert!(profile.contains("different onset rhythm"));
    assert!(profile.contains("4/min(event_count, steps_per_bar)"));
    assert!(
        !profile
            .lines()
            .any(|line| line.starts_with("[COURT_SCORE: {\"title\"")),
        "the profile should teach notation without priming a complete piece"
    );

    // Exercise the actual inline syntax, not a separately maintained example.
    let examples: Vec<serde_json::Value> = profile
        .match_indices("{\"degree\":")
        .map(|(start, _)| {
            let tail = &profile[start..];
            serde_json::from_str(&tail[..=tail.find('}').unwrap()]).unwrap()
        })
        .collect();
    assert_eq!(
        examples.len(),
        2,
        "one sounding event and one rest are documented"
    );
    let mut value = serde_json::to_value(teledra_music::default_score()).unwrap();
    value["bars"] = 8.into();
    value["bpm"] = 96.into();
    value["composition"] = serde_json::json!({
        "form": "arc", "phrase_bars": 2, "chord_bars": 2,
        "steps_per_bar": 4, "development": 0, "texture": "chamber", "harmony": "diatonic"
    });
    let mut track = value["tracks"][0].clone();
    let fields = track.as_object_mut().unwrap();
    for field in ["steps", "velocities", "durations"] {
        fields.remove(field);
    }
    track["notes"] = (0..32).map(|index| examples[index % 2].clone()).collect();
    value["tracks"] = serde_json::json!([track]);
    let parsed = parse_response(&format!("[COURT_SCORE: {value}]"));
    let [CourtAction::CourtScore(raw)] = parsed.actions.as_slice() else {
        panic!("missing score")
    };
    let score = teledra_music::Score::from_json(raw)
        .unwrap()
        .mastered()
        .unwrap();
    assert_eq!(score.tracks[0].steps.len(), 32);
    assert_eq!(score.tracks[0].steps[..2], [Some(2), None]);
    assert!((score.tracks[0].velocities.as_ref().unwrap()[0] - 0.65).abs() < 0.0001);
    assert_eq!(score.tracks[0].durations.as_ref().unwrap()[0], 0.75);
    assert_eq!(score.grid_divisions(&score.tracks[0]), 4);
    assert_eq!(score.duration_seconds(), 20.0);
    assert!(parsed.visible.is_empty());
}

#[test]
fn the_artist_and_alchemist_examples_reach_their_real_parsers() {
    let example = AgentId::Artist
        .profile()
        .lines()
        .find(|line| line.starts_with("[FRACTUS_LIVE: version"))
        .unwrap();
    let parsed = parse_response(example);
    let [CourtAction::Fractus { payload, live }] = parsed.actions.as_slice() else {
        panic!("missing scene")
    };
    assert!(*live);
    // Parsed exactly as the runtime receives it. Replacing `;` here once hid
    // that the runtime rejected every payload written in this documented form.
    teledra_fractus::Scene::parse(payload).unwrap();
    let profile = AgentId::Alchemist.profile();
    let start = profile.find("[WORKSHOP_TOOL: filename:").unwrap();
    let parsed = parse_response(&profile[start..]);
    let CourtAction::WorkshopTool(raw) = &parsed.actions[0] else {
        panic!("missing experiment")
    };
    teledra_workshop::Draft::parse(raw).unwrap();
}

/// Exercises the installed model without executing the generated actions.
/// cargo test -p teledra --test agent_contracts live_offices -- --ignored --nocapture
#[tokio::test]
#[ignore = "requires local Ollama and qwen2.5:7b"]
async fn live_offices() {
    let brain = teledra_brain::Brain::new(Default::default()).unwrap();
    brain.check().await.unwrap();
    let court = Court::default();
    let cases = [
        (
            AgentId::Organist,
            "Compose one original gentle court piece at 96 BPM in D dorian, 48 bars, with five tracks. Emit one complete COURT_SCORE tag. No prose instructions or abbreviated fields.",
        ),
        (
            AgentId::Artist,
            "Create one moving violet mandala with seed 731 using Fractus. Emit the complete supported action; do not claim rendering already succeeded.",
        ),
        (
            AgentId::Scribe,
            "Append this performed lore to notes/court.md: The court compared music and geometry. Use one complete SCRIBE_APPEND tag, preserving the statement as lore.",
        ),
        (
            AgentId::Archivist,
            "Find the official Rust release announcement using https://blog.rust-lang.org/ . No source has been retrieved yet. Emit the focused research action; do not invent release facts.",
        ),
        (
            AgentId::Queen,
            "Ask the Organist to compose a quiet 48-bar court piece at 96 BPM. Summon that minister with the executable delegation and one short spoken sentence.",
        ),
    ];
    let mut results = Vec::new();
    for (agent, user) in cases {
        let request = court.prepare_turn_for(agent, user);
        let raw = if agent == AgentId::Organist {
            let schema = serde_json::from_str(teledra_music::SCORE_SCHEMA).unwrap();
            format!(
                "[COURT_SCORE: {}]",
                brain
                    .complete_structured(&request.messages, 0.35, &schema)
                    .await
                    .unwrap()
            )
        } else {
            brain
                .complete_reviewed(&request.messages, 0.35, agent.slug())
                .await
                .unwrap()
        };
        let parsed = parse_response(&raw);
        let valid = parsed.actions.iter().any(|action| match (agent, action) {
            (AgentId::Organist, CourtAction::CourtScore(raw)) => {
                teledra_music::Score::from_json(raw)
                    .and_then(|score| score.mastered())
                    .is_ok()
            }
            (AgentId::Artist, CourtAction::Fractus { payload, live }) => {
                *live && teledra_fractus::Scene::parse(payload).is_ok()
            }
            (AgentId::Scribe, CourtAction::ScribeAppend { path, content }) => {
                path == "notes/court.md" && content.contains("music and geometry")
            }
            (AgentId::Archivist, CourtAction::Research(query)) => {
                query.contains("blog.rust-lang.org")
            }
            (AgentId::Queen, CourtAction::Delegate { role, .. }) => {
                role.eq_ignore_ascii_case("organist")
            }
            _ => false,
        });
        println!("{} valid={valid}: {raw}", agent.slug());
        results.push((agent, valid));
    }
    assert!(results.iter().all(|(_, valid)| *valid), "{results:?}");
}
