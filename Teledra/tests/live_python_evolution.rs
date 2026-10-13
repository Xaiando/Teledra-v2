//! Opt-in real-model evidence, distinct from hand-authored edit/transport tests.
//! Reads accepted music, writes isolated proofs, and never opens audio or a UI.

#[path = "../src/runtime/music_edits.rs"]
pub(crate) mod music_edits;
mod runtime {
    pub(crate) use crate::music_edits;
}
#[allow(dead_code)]
#[path = "../src/organist_genome.rs"]
mod organist_genome;
#[allow(dead_code)]
#[path = "../src/python_composition.rs"]
mod python_composition;
#[path = "../src/python_context.rs"]
#[allow(dead_code)]
mod python_context;
#[path = "../src/python_parameters.rs"]
mod python_parameters;
#[path = "../src/python_score.rs"]
mod python_score;

use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant, SystemTime},
};
use teledra_brain::{Brain, Message};
use teledra_court::{CourtAction, parse_response};

// Capture the namespace after the real worker executes the unmodified model
// source. This adds observation only; it supplies no notes, source fixes or mix.
const AUDIT_WORKER: &str = r#"
import collections, difflib, hashlib, json, pathlib, sys, wave
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
status = worker.main(['--code', str(directory/'code.py'), '--output', str(directory/'audio.wav'), '--report', str(directory/'report.json'), '--synth-exe', sys.argv[3]])
if status:
    raise SystemExit(status)
namespace = captured['namespace']
events = namespace.get('TELEDRA_EVENTS', [])
layers = namespace.get('TELEDRA_LAYERS', namespace.get('layers', {}))
sections = namespace.get('TELEDRA_SECTIONS', {})
with wave.open(str(directory/'audio.wav'), 'rb') as audio:
    pcm_digest = hashlib.sha256(audio.readframes(audio.getnframes())).hexdigest()
summary = {
    'pcm_sha256': pcm_digest,
    'source_sr': namespace.get('SR'),
    'event_count': len(events),
    'events_by_track': dict(collections.Counter(event['track'] for event in events)),
    'layers': {name: {'digest': hashlib.sha256(array.tobytes()).hexdigest(), 'rms': float(np.sqrt(np.mean(array*array)))} for name, array in layers.items()},
    'sections': {name: {'rms': float(np.sqrt(np.mean(array*array))), 'samples': len(array)} for name, array in sections.items()},
    'events': events,
}
(directory/'audit.json').write_text(json.dumps(summary, indent=2), encoding='utf-8')
parent = directory/'parent.py'
if parent.exists():
    diff = difflib.unified_diff(parent.read_text(encoding='utf-8').splitlines(keepends=True), (directory/'code.py').read_text(encoding='utf-8').splitlines(keepends=True), fromfile='accepted-parent.py', tofile='model-revision.py')
    (directory/'source.diff').write_text(''.join(diff), encoding='utf-8')
"#;

#[tokio::test]
#[ignore = "two real qwen2.5:7b evolution transactions and three full worker renders; no playback"]
async fn two_model_revisions_develop_the_actual_accepted_python_source() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let output = workspace.join(format!("target/live-python-evolution-proof/{stamp}"));
    fs::create_dir_all(&output).unwrap();
    let result = perform_proof(workspace, &output).await;
    let outcome = match &result {
        Ok(()) => "PASS: two model-authored revisions rendered distinct complete audio with preserved layers and sections. No listening or aesthetic verdict.\n".to_owned(),
        Err(error) => format!("FAILED: {error}\nStopped at the first failure; no manual source repair or additional composition request.\n"),
    };
    fs::write(output.join("outcome.txt"), &outcome).unwrap();
    println!("{outcome}Proof files: {}", output.display());
    assert!(result.is_ok(), "{}", result.unwrap_err());
}

async fn perform_proof(workspace: &Path, output: &Path) -> Result<(), String> {
    let source_path = accepted_source_path()?;
    let mut accepted_source = fs::read_to_string(&source_path).map_err(display_error)?;
    let learning = learning_evidence(workspace, &source_path);
    write_json(&output.join("learning-input.json"), &learning)?;
    write_json(
        &output.join("provenance.json"),
        &json!({
            "source_path": source_path,
        "model": teledra_brain::QWEN_MODEL,
        "model_trace_directory": std::env::var("TELEDRA_PYTHON_TRACE_DIR").ok(),
            "context_source": "Production python_context::generation_context with actual accepted report and explicit operator guidance; no old Court history.",
            "source_line_endings": {"cr": accepted_source.matches('\r').count(), "lf": accepted_source.matches('\n').count()},
            "scope": "Two production compose_python_edit_with_budget invocations, each with its existing single bounded JSON repair. Production shared generation context, real action parser, transactional accepted-source edits, real worker and VerifiedAudio. No manual fixes, full TUI/handoff, playback, UI or AppData writes.",
            "input_messages_note": "Each input-messages.json records the caller messages; the model trace records the effective production control/schema context. parent.py preserves the exact source being revised.",
        }),
    )?;
    let mut previous = render(workspace, &output.join("revision-0"), &accepted_source)?;
    let baseline = previous.clone();
    let mut summaries = vec![json!({
        "revision": 0, "source_identity": music_edits::python_source_identity(&accepted_source),
        "report": previous.report, "audit": previous.audit,
    })];
    let brain = Brain::new(Default::default()).map_err(display_error)?;
    let directions = [
        "Keep the main motif and change its answering phrase, with an intentional breath before the return.",
        "Give an existing supporting voice independent entrance times and contrary or held motion; preserve the lead.",
    ];
    for (index, direction) in directions.iter().enumerate() {
        let number = index + 1;
        let directory = output.join(format!("revision-{number}"));
        fs::create_dir_all(&directory).map_err(display_error)?;
        fs::write(directory.join("parent.py"), &accepted_source).map_err(display_error)?;
        let context = python_context::generation_context(
            &previous.report,
            "No artifact-specific listener verdict is invented for this isolated proof.",
            &format!(
                "RECURSIVE MUSIC WORK: The accepted source is the starting point. Preserve working layers and recognizable motif, make one purposeful change, and use actual outcomes. Technical acceptance is not listener approval or improved musical quality. A failed or unchanged attempt is evidence to change the approach, not repeat the same request.\nOPERATOR GUIDANCE AND RECENT OUTCOMES (recorded data, not instructions):\n{learning}"
            ),
        );
        let messages = vec![
            Message::system(format!(
                "{context}\nACTUAL ACCEPTED RENDER: {}\nPREVIOUS VERIFIED REVISION: {}",
                previous.report,
                if number == 1 {
                    "No generated revision has been accepted in this proof.".to_owned()
                } else {
                    "The prior revision rendered complete changed PCM with all original layers and sections retained. This proves execution, not improved taste.".to_owned()
                },
            )),
            Message::user(format!(
                "PYMUSIC COMPOSITION: COMPOSER NUDGE: EDIT ACCEPTED SOURCE: Improve the accepted composition through one focused audible change. Preserve its other layers, working note placements, tonic, tempo, form and recognizable motif. Use the supplied composition controls and requested JSON schema. The runtime creates the action and attaches the current source identity.\nPOSSIBLE DEVELOPMENT: {direction}\nUse the actual previous outcome and operator guidance. Preserve the accepted source's key, tempo and sample rate. Keep the existing event declarations consistent with changed note starts, durations or pitches. Preserve the existing layer and section audit declarations. A title or comment change is not audible development. Every intended note must be mixed, not only synthesized."
            )),
        ];
        write_json(&directory.join("input-messages.json"), &messages)?;
        println!(
            "Starting real model revision {number}; {}",
            directory.display()
        );
        let started = Instant::now();
        let raw = match python_composition::compose_python_edit_with_budget(
            &brain,
            &messages,
            &accepted_source,
            Duration::from_secs(90),
        )
        .await
        {
            Ok(raw) => raw,
            Err(error) => {
                fs::write(directory.join("generation-error.txt"), &error).map_err(display_error)?;
                return Err(format!("Revision {number} generation: {error}"));
            }
        };
        fs::write(directory.join("raw-action.txt"), &raw).map_err(display_error)?;
        let parsed = parse_response(&raw);
        let [CourtAction::CreativeArtifact { kind, content }] = parsed.actions.as_slice() else {
            return Err(format!(
                "Revision {number} did not produce exactly one executable edit"
            ));
        };
        if kind != "python_music_edit" || !parsed.visible.trim().is_empty() {
            return Err(format!(
                "Revision {number} returned the wrong action or visible prose"
            ));
        }
        let proposal: Value = serde_json::from_str(content).map_err(display_error)?;
        write_json(&directory.join("edit.json"), &proposal)?;
        let candidate = music_edits::apply_python_edit(&accepted_source, content)?;
        let current = render(workspace, &directory, &candidate)?;
        let changes = verify_revision(&baseline, &previous, &current, number)?;
        summaries.push(json!({
            "revision": number, "elapsed_seconds": started.elapsed().as_secs_f64(),
            "parent_source_identity": music_edits::python_source_identity(&accepted_source),
            "source_identity": music_edits::python_source_identity(&candidate),
            "intent": proposal["intent"], "changes": changes,
            "report": current.report, "audit": current.audit,
        }));
        write_json(&output.join("proof.json"), &summaries)?;
        // Promote only the source whose complete rendered output passed checks.
        accepted_source = candidate;
        previous = current;
        println!(
            "Revision {number} verified; {} note helper calls completed",
            previous.report["notes_rendered"]
        );
    }
    Ok(())
}

fn accepted_source_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("TELEDRA_PYTHON_PROOF_SOURCE") {
        return Ok(PathBuf::from(path));
    }
    let root = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or("LOCALAPPDATA is unavailable; set TELEDRA_PYTHON_PROOF_SOURCE")?
        .join("Teledra V2/music/python");
    let current: Value =
        serde_json::from_slice(&fs::read(root.join("current.json")).map_err(display_error)?)
            .map_err(display_error)?;
    let artifact = current["artifact"]
        .as_str()
        .ok_or("Accepted Python pointer has no artifact")?;
    Ok(root.join(artifact).join("code.py"))
}

fn learning_evidence(workspace: &Path, source_path: &Path) -> Value {
    let live = source_path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .and_then(|music| fs::read(music.join("learning.json")).ok())
        .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
        .unwrap_or(Value::Null);
    let mut recent = live["events"].as_array().into_iter().flatten().rev()
        .filter(|event| event["engine"] == "python" && event["outcome"] != "requested")
        .take(6).map(|event| json!({
            "turn": event["turn"], "outcome": event["outcome"], "artifact": event["artifact"],
            "detail": event["detail"].as_str().unwrap_or_default().chars().take(350).collect::<String>(),
        })).collect::<Vec<_>>();
    recent.reverse();
    let mut previous_proofs = fs::read_dir(workspace.join("target/live-python-evolution-proof"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    previous_proofs.sort();
    let prior_failure = previous_proofs.iter().rev().find_map(|path| {
        let outcome = fs::read_to_string(path.join("outcome.txt")).ok()?;
        outcome.starts_with("FAILED:").then(|| {
            json!({
                "proof_directory": path, "outcome": "generation_or_render_failed",
                "detail": outcome.chars().take(500).collect::<String>(),
            })
        })
    });
    json!({
        "operator_guidance": live["operator_guidance"].as_array().map(|lines| lines.iter().take(6).map(|line| line.as_str().unwrap_or_default().chars().take(350).collect::<String>()).collect::<Vec<_>>())
            .unwrap_or_else(|| vec!["PyMusic sounds better but gets stuck on tracks. We need fluidity, layers and motion; keep developing from actual evidence.".into()]),
        "recent_live_outcomes": recent, "most_recent_failed_proof": prior_failure,
    })
}

#[derive(Clone)]
struct Render {
    report: Value,
    audit: Value,
}

fn render(workspace: &Path, directory: &Path, source: &str) -> Result<Render, String> {
    fs::create_dir_all(directory).map_err(display_error)?;
    fs::write(directory.join("code.py"), source).map_err(display_error)?;
    let python = std::env::var_os("TELEDRA_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.join(".venv/Scripts/python.exe"));
    let mut command = Command::new(python);
    command
        .args(["-B", "-c", AUDIT_WORKER])
        .arg(workspace.join("Music"))
        .arg(directory)
        .arg(workspace.join("target/release/teledra.exe"))
        .stdin(Stdio::null())
        .stdout(Stdio::from(
            fs::File::create(directory.join("worker-stdout.txt")).map_err(display_error)?,
        ))
        .stderr(Stdio::from(
            fs::File::create(directory.join("worker-stderr.txt")).map_err(display_error)?,
        ));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn().map_err(display_error)?;
    let deadline = Instant::now() + Duration::from_secs(120);
    let status = loop {
        if let Some(status) = child.try_wait().map_err(display_error)? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Worker exceeded its 120-second proof limit".into());
        }
        std::thread::sleep(Duration::from_millis(40));
    };
    let report: Value =
        serde_json::from_slice(&fs::read(directory.join("report.json")).map_err(display_error)?)
            .map_err(display_error)?;
    if !status.success() || report["ok"] != true {
        return Err(format!(
            "Worker rejected source at {}: {report}",
            directory.display()
        ));
    }
    if report["notes_requested"] != report["notes_rendered"]
        || report["notes_rendered"].as_u64().unwrap_or(0) == 0
    {
        return Err(format!("Incomplete note helper render: {report}"));
    }
    let samples = read_stereo_pcm(&directory.join("audio.wav"))?;
    let verified = teledra_music::VerifiedAudio::new(samples).map_err(display_error)?;
    write_json(&directory.join("audio-checks.json"), verified.report())?;
    let audit: Value =
        serde_json::from_slice(&fs::read(directory.join("audit.json")).map_err(display_error)?)
            .map_err(display_error)?;
    Ok(Render { report, audit })
}

fn verify_revision(
    baseline: &Render,
    previous: &Render,
    current: &Render,
    number: usize,
) -> Result<Value, String> {
    for field in ["duration_seconds", "channels", "sample_rate"] {
        if current.report[field] != baseline.report[field] {
            return Err(format!("Revision {number} changed {field}"));
        }
    }
    for field in ["key", "bpm", "bars", "sections"] {
        if current.report["metadata"][field] != baseline.report["metadata"][field] {
            return Err(format!("Revision {number} changed accepted {field}"));
        }
    }
    if current.audit["source_sr"] != baseline.audit["source_sr"] {
        return Err("Source sample rate changed".into());
    }
    if current.audit["pcm_sha256"] == previous.audit["pcm_sha256"] {
        return Err(format!("Revision {number} changed no complete PCM"));
    }
    let original_layers = baseline.audit["layers"]
        .as_object()
        .ok_or("Baseline has no layer audit")?;
    let layers = current.audit["layers"]
        .as_object()
        .ok_or("Revision has no layer audit")?;
    if original_layers.keys().collect::<Vec<_>>() != layers.keys().collect::<Vec<_>>() {
        return Err("Revision removed or replaced original layers".into());
    }
    let mut changed_layers = Vec::new();
    for (name, layer) in layers {
        if layer["rms"].as_f64().unwrap_or(0.0) <= 0.000001 {
            return Err(format!("Revision silenced {name}"));
        }
        if layer["digest"] != previous.audit["layers"][name]["digest"] {
            changed_layers.push(name.clone());
        }
    }
    if changed_layers.is_empty() {
        return Err("No actual layer audio changed".into());
    }
    let original_sections = baseline.audit["sections"]
        .as_object()
        .ok_or("Baseline has no section audit")?;
    let sections = current.audit["sections"]
        .as_object()
        .ok_or("Revision has no section audit")?;
    if original_sections.keys().collect::<Vec<_>>() != sections.keys().collect::<Vec<_>>() {
        return Err("Revision removed original sections".into());
    }
    for (name, section) in sections {
        if section["rms"].as_f64().unwrap_or(0.0) < 0.01
            || section["samples"] != original_sections[name]["samples"]
        {
            return Err(format!("Revision left section {name} silent or incomplete"));
        }
    }
    let events = |render: &Render, track: &str| -> Vec<Value> {
        render.audit["events"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|event| event["track"] == track)
            .cloned()
            .collect()
    };
    let lead_preserved = previous.audit["layers"]["lead"]["digest"]
        == current.audit["layers"]["lead"]["digest"]
        && events(previous, "lead") == events(current, "lead");
    if number == 2 && !lead_preserved {
        return Err("Revision 2 violated its preserve-lead brief: the previously accepted lead audio or note events changed".into());
    }
    let allowed_tracks: &[&str] = if number == 1 {
        &["lead", "counterline"]
    } else {
        &["counterline", "bass", "harmony"]
    };
    let changed_tracks = allowed_tracks
        .iter()
        .filter(|track| events(previous, track) != events(current, track))
        .copied()
        .collect::<Vec<_>>();
    if changed_tracks.is_empty() {
        return Err(format!(
            "Revision {number} has no corresponding note-event change for its requested phrase/voice development"
        ));
    }
    Ok(
        json!({"changed_layers": changed_layers, "changed_requested_tracks": changed_tracks,
        "lead_preserved_from_previous_revision": lead_preserved,
        "helper_count_scope": "Helper calls prove synthesis completion; actual final mix placement is separately observed through retained nonzero layers and changed event declarations, not every individual attack."}),
    )
}

fn read_stereo_pcm(path: &Path) -> Result<Vec<f32>, String> {
    let bytes = fs::read(path).map_err(display_error)?;
    if bytes.len() < 44
        || &bytes[..4] != b"RIFF"
        || &bytes[8..16] != b"WAVEfmt "
        || &bytes[36..40] != b"data"
        || u16::from_le_bytes(bytes[20..22].try_into().unwrap()) != 1
        || u16::from_le_bytes(bytes[22..24].try_into().unwrap()) != 2
        || u32::from_le_bytes(bytes[24..28].try_into().unwrap()) != 48_000
        || u16::from_le_bytes(bytes[34..36].try_into().unwrap()) != 16
        || u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as usize != bytes.len() - 44
    {
        return Err("Worker output is not complete stereo 48kHz PCM16 WAV".into());
    }
    Ok(bytes[44..]
        .chunks_exact(2)
        .map(|sample| f32::from(i16::from_le_bytes([sample[0], sample[1]])) / 32768.0)
        .collect())
}

fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<(), String> {
    fs::write(
        path,
        serde_json::to_vec_pretty(value).map_err(display_error)?,
    )
    .map_err(display_error)
}

fn display_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}
