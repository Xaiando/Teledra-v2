//! Opt-in real Organist -> Python source -> worker -> verified PCM proof.
//! It never starts playback, opens a window, or edits accepted user music.
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use teledra_agents::AgentId;
use teledra_court::{Court, CourtAction, parse_response};

#[tokio::test]
#[ignore = "requires installed Ollama, Python and release synth; writes isolated proof files without playback"]
async fn live_organist_python_piece_reaches_the_real_worker_and_audio_verifier() {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    let output = workspace.join("target/python-music-proof/model-study-guided");
    std::fs::create_dir_all(&output).unwrap();
    let result = perform_proof(&workspace, &output).await;
    let outcome = match &result {
        Ok(summary) => format!("PASS: {summary}\n"),
        Err(error) => {
            format!("FAILED: {error}\nNo fallback or additional composition request was made.\n")
        }
    };
    std::fs::write(output.join("outcome.txt"), &outcome).unwrap();
    println!("{outcome}Proof files: {}", output.display());
    assert!(result.is_ok(), "{}", result.unwrap_err());
}

async fn perform_proof(workspace: &Path, output: &Path) -> Result<String, String> {
    let catalog = teledra_music::Instrument::ALL
        .iter()
        .map(|voice| voice.info())
        .collect::<Vec<_>>();
    // Production dispatch supplies both this shared guide and the editable
    // source. Use its bundled initial source without reading user music.
    let context = format!(
        "{}\nPYMUSIC COMPOSITION CONTEXT AT DISPATCH: Return [PYTHON_MUSIC: complete Python source]. Use import numpy as np and from teledra_synth import *. This is restored V1 freeform NumPy composition: timed overlapping arrays, automation and effects. synth_note('C4', duration, wave_type='sine', volume=0.2, sr=48000) provides classic synthesis. instrument_note('C4', duration, instrument='piano', volume=0.2, sr=48000) provides every native instrument below including theremin. Numeric note means Hz, never MIDI. duration is total note length in seconds. Use sr=48000 consistently; mix_waves(track, note, start_time, sr=48000) extends and adds each note without stealing voices. Compose all layers first, then call play_sound(full_track, loop=True, sr=48000) exactly once. That call captures a complete WAV for the court's one player; never use loops to play sound, file/network/process/device APIs, or external libraries. Keep total audio 4-600 seconds. Declare TELEDRA_SCORE={{'title':'...'}} for its name. Lead phrases need rests, answers and development; harmony and bass need independent timing and harmonic arrivals. Preserve tonic 0 and 120 BPM as comparison anchors.\nFULL 72-INSTRUMENT CATALOG: {}\nACCEPTED PYTHON REPORT: null\nCURRENT PYTHON SOURCE (editable draft, possibly unplayed):\n{}\nPYTHON OPERATOR VERDICTS (only evidence for the named artifacts, never invent why a verdict was given):\nNo verdicts in this isolated verification fixture.\nThis isolated verification run does not replace accepted music.",
        include_str!("../../Music/PYTHON_AUTHORING.md"),
        serde_json::to_string(&catalog).map_err(|error| error.to_string())?,
        include_str!("../../Music/pymusic_example.py"),
    );
    let brief = "PYMUSIC COMPOSITION: Compose one original 16-second layered chamber miniature at 120 BPM in C major, using Python and exactly three complementary voices: piano for a recognizable short motif, double_bass for a quieter harmonic foundation, and marimba for a rhythmically different answer. Include an actual pause in the piano line while another part continues, a contrasting phrase, and a recognizable return with a settled ending. Keep all notes in comfortable separate registers and align structural melody notes with the bass harmony. Give the layers distinct entry times; do not double one scale in all parts. Use NumPy, instrument_note(note_name, duration_seconds, instrument=exact_id, volume=level, sr=48000), and mix_waves(full_track, note_audio, start_time=seconds, sr=48000). Start with a sufficiently long zero array; mix_waves returns the updated array, so keep its result. Use at most 64 instrument_note calls and keep the completed audio between 4 and 20 seconds. Optional TELEDRA_SCORE metadata should identify title, key, bpm and bars. Submit exactly once with play_sound(full_track, sr=48000, loop=True). Use only the documented teledra_synth helpers and plain NumPy; do not import file, GUI, network, audio-device or process libraries. Return exactly one complete [PYTHON_MUSIC: code] tag and no other action or prose. Native COURT_SCORE and its atomic grid do not apply to this Python request.";
    let prepared =
        Court::default().prepare_turn_for_with_context(AgentId::Organist, brief, &context);
    std::fs::write(output.join("brief.txt"), brief).map_err(|error| error.to_string())?;
    std::fs::write(
        output.join("messages.json"),
        serde_json::to_vec_pretty(&prepared.messages).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let brain = teledra_brain::Brain::new(Default::default()).map_err(|error| error.to_string())?;
    // Exactly one production invocation; its existing bounded review is part
    // of the same path used for Python Organist turns in spawn_turn.
    let raw = brain
        .complete_reviewed(&prepared.messages, 0.45, AgentId::Organist.slug())
        .await
        .map_err(|error| format!("Organist request failed: {error}"))?;
    std::fs::write(output.join("raw-output.txt"), &raw).map_err(|error| error.to_string())?;
    let parsed = parse_response(&raw);
    let [CourtAction::CreativeArtifact { kind, content }] = parsed.actions.as_slice() else {
        return Err(
            "Organist did not return exactly one Python music artifact; raw output retained".into(),
        );
    };
    if kind != "python_music" || !parsed.visible.trim().is_empty() {
        return Err("Expected only a complete PYTHON_MUSIC tag; raw output retained".into());
    }
    let content = content.trim();
    let code = content
        .strip_prefix("```python")
        .or_else(|| content.strip_prefix("```"))
        .map_or(content, |body| {
            body.strip_suffix("```").unwrap_or(body).trim()
        });
    std::fs::write(output.join("code.py"), code).map_err(|error| error.to_string())?;

    let python = std::env::var_os("TELEDRA_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.join(".venv/Scripts/python.exe"));
    let synth = workspace.join("target/release/teledra.exe");
    if !python.is_file() || !synth.is_file() {
        return Err("The configured Python or release synth executable is missing".into());
    }
    let mut command = Command::new(python);
    command
        .arg("-B")
        .arg(workspace.join("Music/python_music_worker.py"))
        .arg("--code")
        .arg(output.join("code.py"))
        .arg("--output")
        .arg(output.join("study.wav"))
        .arg("--report")
        .arg(output.join("worker-report.json"))
        .arg("--synth-exe")
        .arg(synth)
        .current_dir(workspace.join("Music"))
        .stdin(Stdio::null())
        .stdout(Stdio::from(
            std::fs::File::create(output.join("worker-stdout.txt"))
                .map_err(|error| error.to_string())?,
        ))
        .stderr(Stdio::from(
            std::fs::File::create(output.join("worker-stderr.txt"))
                .map_err(|error| error.to_string())?,
        ));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("Worker launch failed: {error}"))?;
    let deadline = Instant::now() + Duration::from_secs(120);
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Python worker exceeded the 120-second proof limit".into());
        }
        std::thread::sleep(Duration::from_millis(40));
    };
    let report: serde_json::Value = serde_json::from_slice(
        &std::fs::read(output.join("worker-report.json"))
            .map_err(|error| format!("Worker produced no report: {error}"))?,
    )
    .map_err(|error| error.to_string())?;
    if !status.success() || report["ok"] != true {
        return Err(format!(
            "Python worker rejected the returned source: {report}"
        ));
    }
    if report["notes_requested"] != report["notes_rendered"]
        || report["notes_rendered"].as_u64().unwrap_or(0) < 12
    {
        return Err(format!(
            "The requested layered note material was incomplete: {report}"
        ));
    }
    let samples = read_stereo_pcm(&output.join("study.wav"))?;
    let audio = teledra_music::VerifiedAudio::new(samples).map_err(|error| error.to_string())?;
    std::fs::write(
        output.join("audio-checks.json"),
        serde_json::to_vec_pretty(audio.report()).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    if !(4.0..=20.0).contains(&audio.report().duration_seconds) {
        return Err(format!(
            "The returned piece exceeded the requested proof length: {}s",
            audio.report().duration_seconds
        ));
    }
    Ok(format!(
        "{}; {:.2}s; {} note helper calls completed; peak {:.3}; RMS {:.4}. Audio was rendered and verified, not played.",
        report["title"].as_str().unwrap_or("Python composition"),
        audio.report().duration_seconds,
        report["notes_rendered"],
        audio.report().peak,
        audio.report().rms
    ))
}

fn read_stereo_pcm(path: &Path) -> Result<Vec<f32>, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    if bytes.len() < 44 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("Worker output is not a complete WAV".into());
    }
    let mut cursor = 12;
    let mut valid_format = false;
    let mut samples = None;
    while cursor + 8 <= bytes.len() {
        let length = u32::from_le_bytes(bytes[cursor + 4..cursor + 8].try_into().unwrap()) as usize;
        let start = cursor + 8;
        let end = start
            .checked_add(length)
            .filter(|end| *end <= bytes.len())
            .ok_or("Truncated WAV chunk")?;
        match &bytes[cursor..cursor + 4] {
            b"fmt " if length >= 16 => {
                valid_format = u16::from_le_bytes(bytes[start..start + 2].try_into().unwrap()) == 1
                    && u16::from_le_bytes(bytes[start + 2..start + 4].try_into().unwrap()) == 2
                    && u32::from_le_bytes(bytes[start + 4..start + 8].try_into().unwrap())
                        == 48_000
                    && u16::from_le_bytes(bytes[start + 14..start + 16].try_into().unwrap()) == 16;
            }
            b"data" if length.is_multiple_of(4) => {
                samples = Some(
                    bytes[start..end]
                        .chunks_exact(2)
                        .map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32768.0)
                        .collect(),
                );
            }
            _ => {}
        }
        cursor = end + length % 2;
    }
    if !valid_format {
        return Err("Expected the production worker's PCM16 stereo 48kHz format".into());
    }
    samples.ok_or_else(|| "Worker WAV has no complete stereo sample data".into())
}
