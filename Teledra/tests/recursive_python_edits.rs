//! Off-device proof through the real action parser, exact edit utility and worker.

#[path = "../src/runtime/music_edits.rs"]
mod music_edits;

use serde_json::{Value, json};
use std::{fs, path::Path, process::Command, time::SystemTime};
use teledra_court::{CourtAction, parse_response};

const AUDIT_WORKER: &str = r#"
import collections, hashlib, json, pathlib, sys
import numpy as np
sys.path.insert(0, sys.argv[1])
import python_music_worker as worker
original_namespace = worker._namespace
captured = {}
def capture_namespace():
    namespace = original_namespace()
    captured['namespace'] = namespace
    return namespace
worker._namespace = capture_namespace
directory = pathlib.Path(sys.argv[2])
status = worker.main(['--code', str(directory/'code.py'), '--output', str(directory/'audio.wav'), '--report', str(directory/'report.json')])
if status:
    raise SystemExit(status)
namespace = captured['namespace']
events = namespace['TELEDRA_EVENTS']
layers = namespace['TELEDRA_LAYERS']
summary = {
    'event_count': len(events),
    'events_by_track': dict(collections.Counter(event['track'] for event in events)),
    'layers': {name: {'digest': hashlib.sha256(array.tobytes()).hexdigest(), 'rms': float(np.sqrt(np.mean(array*array)))} for name, array in layers.items()},
    'sections': {name: {'rms': float(np.sqrt(np.mean(array*array))), 'samples': len(array)} for name, array in namespace['TELEDRA_SECTIONS'].items()},
    'events': events,
}
(directory/'audit.json').write_text(json.dumps(summary, indent=2), encoding='utf-8')
"#;

fn parse_and_edit(base: &str, replacements: &[(&str, &str)]) -> (String, String) {
    let payload = json!({
        "base": music_edits::python_source_identity(base),
        "intent": "Refine the existing answering voice while preserving every other layer",
        "edits": replacements.iter().map(|(old,new)| json!({"old": old, "new": new})).collect::<Vec<_>>()
    }).to_string();
    let parsed = parse_response(&format!("[PYTHON_MUSIC_EDIT: {payload}]"));
    assert!(parsed.visible.is_empty());
    let [CourtAction::CreativeArtifact { kind, content }] = parsed.actions.as_slice() else {
        panic!("exact edit must produce one executable action")
    };
    assert_eq!(kind, "python_music_edit");
    (
        music_edits::apply_python_edit(base, content).unwrap(),
        payload,
    )
}

fn render(workspace: &Path, directory: &Path, source: &str) -> (Vec<u8>, Value, Value) {
    fs::create_dir_all(directory).unwrap();
    fs::write(directory.join("code.py"), source).unwrap();
    let output = Command::new(workspace.join(".venv/Scripts/python.exe"))
        .args(["-B", "-c", AUDIT_WORKER])
        .arg(workspace.join("Music"))
        .arg(directory)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value =
        serde_json::from_slice(&fs::read(directory.join("report.json")).unwrap()).unwrap();
    let audit: Value =
        serde_json::from_slice(&fs::read(directory.join("audit.json")).unwrap()).unwrap();
    let wav = fs::read(directory.join("audio.wav")).unwrap();
    // Python's standard wave writer emits this canonical PCM header.
    assert_eq!(&wav[..4], b"RIFF");
    assert_eq!(&wav[8..16], b"WAVEfmt ");
    assert_eq!(&wav[36..40], b"data");
    assert_eq!(u16::from_le_bytes(wav[20..22].try_into().unwrap()), 1);
    assert_eq!(u16::from_le_bytes(wav[22..24].try_into().unwrap()), 2);
    assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 48_000);
    assert_eq!(u16::from_le_bytes(wav[34..36].try_into().unwrap()), 16);
    assert_eq!(wav.len() - 44, 88 * 48_000 * 2 * 2);
    let verified = teledra_music::VerifiedAudio::new(
        wav[44..]
            .chunks_exact(2)
            .map(|sample| f32::from(i16::from_le_bytes([sample[0], sample[1]])) / 32768.0)
            .collect(),
    )
    .unwrap();
    assert_eq!(verified.report().duration_seconds, 88.0);
    assert_eq!(report["ok"], true);
    assert_eq!(report["frames"], 88 * 48_000);
    assert_eq!(report["notes_requested"], 754);
    assert_eq!(report["notes_rendered"], 754);
    assert_eq!(
        audit["event_count"], 682,
        "72 unplaced prepared kick buffers are intentional V1 behavior"
    );
    assert_eq!(audit["layers"].as_object().unwrap().len(), 8);
    assert_eq!(audit["sections"].as_object().unwrap().len(), 5);
    for layer in audit["layers"].as_object().unwrap().values() {
        assert!(layer["rms"].as_f64().unwrap() > 0.0);
    }
    for section in audit["sections"].as_object().unwrap().values() {
        assert!(section["rms"].as_f64().unwrap() > 0.01);
    }
    (wav[44..].to_vec(), report, audit)
}

#[test]
#[ignore = "renders three full V1 compositions using the local Python runtime, without an audio device"]
fn two_recursive_edits_preserve_v1_layers_and_render_distinct_complete_audio() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let output = workspace.join(format!("target/recursive-python-proof/{stamp}"));
    let original = fs::read_to_string(workspace.join("Music/v1_music_example.py")).unwrap();
    let (first, first_edit) = parse_and_edit(
        &original,
        &[(
            "answer_start = cursor + step * 0.5",
            "answer_start = cursor + step * 0.75",
        )],
    );
    let (second, second_edit) = parse_and_edit(
        &first,
        &[
            ("answer_note, step * 1.4", "answer_note, step * 1.15"),
            (
                "\"counterline\", \"motion\", answer_start, step * 1.4",
                "\"counterline\", \"motion\", answer_start, step * 1.15",
            ),
        ],
    );
    assert!(second.contains("answer_start = cursor + step * 0.75"));
    assert!(
        music_edits::apply_python_edit(&original, &second_edit)
            .unwrap_err()
            .contains("stale")
    );
    let sources = [&original, &first, &second];
    let mut renders = Vec::new();
    let mut summaries = Vec::new();
    for (index, source) in sources.into_iter().enumerate() {
        let directory = output.join(format!("revision-{index}"));
        let (pcm, report, audit) = render(workspace, &directory, source);
        summaries.push(json!({"revision": index, "directory": directory,
            "source_identity": music_edits::python_source_identity(source),
            "parent_source_identity": index.checked_sub(1).map(|previous| music_edits::python_source_identity(sources[previous])),
            "report": report, "events_by_track": audit["events_by_track"], "layers": audit["layers"]}));
        renders.push((pcm, audit));
    }
    for pair in renders.windows(2) {
        assert_ne!(
            pair[0].0, pair[1].0,
            "each focused revision must actually change complete PCM"
        );
        for (name, layer) in pair[0].1["layers"].as_object().unwrap() {
            if name == "counterline" {
                assert_ne!(layer["digest"], pair[1].1["layers"][name]["digest"]);
            } else {
                assert_eq!(
                    layer["digest"], pair[1].1["layers"][name]["digest"],
                    "unmodified {name} must retain exact audio"
                );
            }
        }
        assert_eq!(pair[0].1["events_by_track"], pair[1].1["events_by_track"]);
    }
    fs::write(output.join("proof.json"), serde_json::to_vec_pretty(&json!({
        "scope": "Off-device parser -> accepted-source edit -> complete worker render -> VerifiedAudio. No aesthetic verdict or device playback.",
        "edits": [serde_json::from_str::<Value>(&first_edit).unwrap(),serde_json::from_str::<Value>(&second_edit).unwrap()],
        "revisions": summaries
    })).unwrap()).unwrap();
    println!(
        "Recursive V1 proof: {}",
        output.join("proof.json").display()
    );
}
