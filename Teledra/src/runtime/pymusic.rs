//! Python authors audio; the court alone owns playback and window switching.
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use super::music_library::{LibraryVerdict, MusicLibrary};
use super::{RuntimeServices, ToolEffect, ToolReply};
use crate::app::App;
use serde::{Deserialize, Serialize};
use teledra_music::{Player, VerifiedAudio};
use tokio::sync::mpsc;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MusicEngine {
    #[default]
    Native,
    Python,
}
impl MusicEngine {
    pub(super) fn other(self) -> Self {
        match self {
            Self::Native => Self::Python,
            Self::Python => Self::Native,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum PythonAction {
    Run { code: String },
    Save { code: String },
    Play,
    Pause,
    Stop,
    Native,
    Compose,
    LibraryPlay { id: String },
    Alternate,
    Keep,
    Reject,
    Close,
}

#[derive(Debug)]
pub(crate) struct PythonRender {
    pub(super) restore: bool,
    pub(super) code: String,
    pub(super) directory: PathBuf,
    pub(super) report: serde_json::Value,
    pub(super) audio: VerifiedAudio,
}
pub(super) struct ReadyPython {
    pub(super) ticket: u64,
    pub(super) render: PythonRender,
    pub(super) switch_after: f64,
}

pub(super) struct PythonMusic {
    python: PathBuf,
    workspace: PathBuf,
    script: PathBuf,
    synth: PathBuf,
    draft: PathBuf,
    accepted: Option<PathBuf>,
    accepted_code: Option<String>,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    reader: Option<std::thread::JoinHandle<()>>,
    pub(super) session: u64,
    pub(super) code: String,
    pub(super) report: serde_json::Value,
    pub(super) duration: f64,
    pub(super) rendering: Option<u64>,
    pub(super) repair_source: Option<String>,
    cancel: Option<Arc<AtomicBool>>,
    pub(super) ready: Option<ReadyPython>,
    pub(super) library: Option<MusicLibrary>,
    pub(super) active_library_id: Option<String>,
    library_error: Option<String>,
}

impl PythonMusic {
    pub(super) fn new(python: PathBuf, root: &Path) -> Self {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_owned();
        let home = root.join("music/python");
        let accepted = fs::read_to_string(home.join("current.json"))
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .and_then(|v| v["artifact"].as_str().map(str::to_owned))
            .filter(|name| {
                !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            })
            .map(|name| home.join(name));
        let accepted_code = accepted
            .as_ref()
            .and_then(|dir| fs::read_to_string(dir.join("code.py")).ok());
        let code = fs::read_to_string(home.join("draft.py"))
            .ok()
            .or_else(|| accepted_code.clone())
            .or_else(|| fs::read_to_string(workspace.join("Music/v1_music_example.py")).ok())
            .or_else(|| fs::read_to_string(workspace.join("Music/pymusic_example.py")).ok())
            .unwrap_or_default();
        let report = accepted
            .as_ref()
            .and_then(|dir| fs::read(dir.join("report.json")).ok())
            .and_then(|raw| serde_json::from_slice(&raw).ok())
            .unwrap_or(serde_json::Value::Null);
        let (library, library_error) = match MusicLibrary::load(root) {
            Ok(library) => (Some(library), None),
            Err(error) => (None, Some(format!("Music library could not load: {error}"))),
        };
        let active_library_id = accepted
            .as_deref()
            .and_then(|path| library.as_ref()?.id_for_artifact(path));
        Self {
            python,
            script: workspace.join("Music/python_music_editor.py"),
            synth: std::env::current_exe().unwrap_or_default(),
            workspace,
            draft: home.join("draft.py"),
            accepted,
            accepted_code,
            child: None,
            stdin: None,
            reader: None,
            session: 0,
            code,
            report,
            duration: 0.0,
            rendering: None,
            repair_source: None,
            cancel: None,
            ready: None,
            library,
            active_library_id,
            library_error,
        }
    }

    pub(super) fn accepted_source(&self) -> Option<String> {
        self.accepted_code.clone()
    }

    pub(super) fn accepted_artifact(&self) -> Option<&Path> {
        self.accepted.as_deref()
    }
    pub(super) fn show(
        &mut self,
        transport: &str,
        alternating: bool,
        tools: &mpsc::UnboundedSender<ToolReply>,
    ) -> io::Result<()> {
        if self
            .child
            .as_mut()
            .is_some_and(|child| child.try_wait().ok().flatten().is_some())
        {
            self.close();
        }
        if self.child.is_none() {
            if let Ok(code) = fs::read_to_string(&self.draft) {
                self.code = code;
            }
            self.session = self.session.wrapping_add(1);
            let mut command = Command::new(&self.python);
            command
                .arg(&self.script)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            super::daw::hide_console(&mut command);
            let mut child = command.spawn()?;
            self.stdin = child.stdin.take();
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| io::Error::other("PyMusic editor stdout missing"))?;
            let session = self.session;
            let tools = tools.clone();
            let draft = self.draft.clone();
            self.reader = Some(std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if let Ok(action) = serde_json::from_str::<PythonAction>(&line) {
                        // Shutdown blocks the event loop while this owned reader
                        // drains the editor's final Save acknowledgement.
                        if let PythonAction::Save { code } = &action
                            && code.len() <= 128 * 1024
                        {
                            let _ = fs::create_dir_all(draft.parent().unwrap());
                            let _ = fs::write(&draft, code);
                        }
                        let _ = tools.send(ToolReply {
                            source: "PyMusic",
                            result: Ok("PyMusic control".to_owned()),
                            court_prompt: None,
                            effect: Some(ToolEffect::PythonAction { session, action }),
                        });
                    }
                }
            }));
            self.child = Some(child);
        }
        self.send(serde_json::json!({"action":"source","code":self.code,"transport":transport,"alternating":alternating,
            "instrument_catalog":teledra_music::Instrument::ALL.iter().map(|voice|voice.info()).collect::<Vec<_>>() }))?;
        self.send_library()
    }
    fn send_library(&mut self) -> io::Result<()> {
        if let Some(library) = &self.library {
            self.send(library.editor_state(self.active_library_id.as_deref()))?;
        }
        if let Some(error) = &self.library_error {
            self.send(serde_json::json!({"action":"status","message":error}))?;
        }
        Ok(())
    }
    pub(super) fn send(&mut self, value: serde_json::Value) -> io::Result<()> {
        let Some(stdin) = self.stdin.as_mut() else {
            return Ok(());
        };
        serde_json::to_writer(&mut *stdin, &value)?;
        stdin.write_all(b"\n")?;
        stdin.flush()
    }
    pub(super) fn close(&mut self) {
        let _ = self.send(serde_json::json!({"action":"shutdown"}));
        self.stdin = None;
        if let Some(mut child) = self.child.take() {
            let until = Instant::now() + Duration::from_secs(2);
            while child.try_wait().ok().flatten().is_none() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(25));
            }
            if child.try_wait().ok().flatten().is_none() {
                super::daw::kill_process_tree(&mut child);
            }
            let _ = child.wait();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        self.session = self.session.wrapping_add(1);
    }
    pub(super) fn cancel(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::Release);
        }
        self.rendering = None;
        self.repair_source = None;
        self.ready = None;
    }
}
impl Drop for PythonMusic {
    fn drop(&mut self) {
        self.cancel();
        self.close();
    }
}

impl RuntimeServices {
    pub(crate) fn music_snapshot(&self, app: &App) -> crate::session::MusicSnapshot {
        use crate::session::{MusicSnapshot, MusicSnapshotEngine, MusicSnapshotTransport};

        let python = self.music_engine == MusicEngine::Python;
        let ready = self
            .python_music
            .ready
            .as_ref()
            .map(|ready| (ready.ticket, ready.switch_after))
            .or_else(|| {
                self.music_ready
                    .as_ref()
                    .map(|ready| (ready.ticket, ready.switch_after))
            });
        let transport = match &self.music_player {
            None => MusicSnapshotTransport::Stopped,
            Some(player) if player.is_finished() => MusicSnapshotTransport::Finished,
            Some(_) if !app.music_enabled() => MusicSnapshotTransport::Paused,
            Some(_) => MusicSnapshotTransport::Playing,
        };
        let cached_identity = |field: &str| {
            python
                .then(|| self.python_music.report[field].as_str())
                .flatten()
                .filter(|identity| !identity.is_empty())
        };
        MusicSnapshot {
            engine: if python {
                MusicSnapshotEngine::Python
            } else {
                MusicSnapshotEngine::Native
            },
            alternating: self.music_alternating,
            transport,
            muted: app.music_muted(),
            accepted_artifact: python
                .then(|| self.python_music.accepted_artifact())
                .flatten()
                .and_then(Path::file_name)
                .map(|name| name.to_string_lossy().into_owned()),
            source_identity: cached_identity("source_digest")
                .map(|identity| format!("fnv1a64:{identity}")),
            pcm_identity: cached_identity("audio_digest").map(str::to_owned),
            composer_pending: !app.can_queue_organist_composition(),
            pending_turn: app
                .pending()
                .filter(|pending| pending.agent == teledra_agents::AgentId::Organist)
                .map(|pending| pending.id),
            rendering_ticket: self
                .python_music
                .rendering
                .or_else(|| self.music_rendering.then_some(self.music_epoch)),
            ready_ticket: ready.map(|(ticket, _)| ticket),
            switch_after_seconds: ready.map(|(_, boundary)| boundary.ceil() as u64),
            position_seconds: self
                .music_player
                .as_ref()
                .map_or(0, |player| player.position_seconds() as u64),
            duration_seconds: if python && self.python_music.duration == 0.0 {
                self.python_music.report["duration_seconds"]
                    .as_f64()
                    .unwrap_or(0.0)
            } else {
                self.active_music_duration()
            }
            .ceil() as u64,
            next_evolution_seconds: self.next_native_evolution.map(|deadline| {
                deadline
                    .saturating_duration_since(Instant::now())
                    .as_secs_f64()
                    .ceil() as u64
            }),
        }
    }

    pub(crate) fn music_reply_cancelled(&self, id: u64, agent: teledra_agents::AgentId) -> bool {
        agent == teledra_agents::AgentId::Organist
            && self.music_cancelled_turn.is_some_and(|last| id <= last)
    }

    pub(super) fn cancel_music_composition(&mut self, app: &mut App) {
        if let Some(pending) = app.pending()
            && pending.agent == teledra_agents::AgentId::Organist
        {
            self.music_cancelled_turn = Some(pending.id);
        }
        app.cancel_organist_compositions();
    }

    /// The source of the piece now accepted, so a new composition can be told
    /// it reproduced the music already playing instead of being silently dropped.
    pub(crate) fn accepted_python_source(&self) -> Option<String> {
        self.python_music.accepted_source()
    }
    pub(crate) fn python_edit_source(&self, user: &str) -> Option<String> {
        let lower = user.to_ascii_lowercase();
        if !(lower.contains("edit accepted source:") || lower.contains("python_music_edit"))
            && [
                "composer correction:",
                "new piece",
                "new layered piece",
                "new composition",
                "from scratch",
            ]
            .iter()
            .any(|marker| lower.contains(marker))
        {
            return None;
        }
        // Ordinary improvement requests need the current baseline too.
        self.python_music.accepted_source()
    }

    /// Fresh artifact-generation evidence only. The constrained generator adds
    /// the exact accepted source once; court dialogue and old action payloads
    /// must not compete with it for the model's context window.
    pub(crate) fn python_generation_context(&self) -> String {
        let feedback =
            fs::read_to_string(self.root.join("music/python/feedback.jsonl")).unwrap_or_default();
        let context = crate::python_context::generation_context(
            &self.python_music.report,
            &python_feedback_context(&feedback),
            &self.music_learning.context(MusicEngine::Python),
        );
        format!("{context}\n{}", self.music_study_context())
    }

    pub(crate) fn python_song_generation_context(&self) -> String {
        let feedback =
            fs::read_to_string(self.root.join("music/python/feedback.jsonl")).unwrap_or_default();
        let context = crate::python_context::song_generation_context(
            &self.python_music.report,
            &python_feedback_context(&feedback),
            &self.music_learning.context(MusicEngine::Python),
        );
        let library = self
            .python_music
            .library
            .as_ref()
            .map(|l| l.editor_state(self.python_music.active_library_id.as_deref()))
            .unwrap_or_default();
        let songs = library["entries"]
            .as_array()
            .map(|entries| entries.iter().take(8).cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        format!(
            "{context}\nRECENT LIBRARY PIECES (avoid repeating the same theme): {}\n{}",
            serde_json::json!(songs),
            self.music_study_context()
        )
    }

    pub(super) fn python_composition_context(&self) -> String {
        let baseline = self
            .python_music
            .accepted_source()
            .unwrap_or_else(|| self.python_music.code.clone());
        let source_identity = super::music_edits::python_source_identity(&baseline);
        let artifact = self
            .python_music
            .accepted_artifact()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "none yet".to_owned());
        let source_kind = if self.python_music.accepted_code.is_some() {
            "ACCEPTED PYTHON SOURCE (up to 9000 characters; editor drafts are separate)"
        } else {
            "STARTING PYTHON SOURCE (no accepted Python artifact yet)"
        };
        let tonal_anchor = self.python_music.report["metadata"]["key"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("native tonic {}", self.score.tonic));
        let tempo = self.python_music.report["metadata"]["bpm"]
            .as_f64()
            .unwrap_or(f64::from(self.score.bpm));
        let edit_contract = if self.python_music.accepted_code.is_some() {
            format!(
                "For one recursive improvement, return [PYTHON_MUSIC_EDIT: {{\"base\":\"{source_identity}\",\"intent\":\"the specific audible improvement\",\"edits\":[{{\"old\":\"exact existing source text\",\"new\":\"replacement source text\"}}]}}]. Each old text must identify its intended source occurrence exactly. Keep the accepted composition intact outside that change. A complete [PYTHON_MUSIC: source] remains valid for a requested new piece. Merely changing title, metadata or comments is not musical development."
            )
        } else {
            "Return [PYTHON_MUSIC: complete Python source] for the first piece.".to_owned()
        };
        // Id and range per voice. The full records, with every description,
        // and the render report's per-note audit made this context ~92,000
        // characters: several times the model's window, so Ollama kept only
        // its tail and the Organist composed without his contract.
        let catalog = teledra_music::Instrument::ALL
            .iter()
            .map(|voice| {
                let info = voice.info();
                format!("{} (MIDI {}-{})", info.id, info.range[0], info.range[1])
            })
            .collect::<Vec<_>>()
            .join(", ");
        let report = &self.python_music.report;
        let report_summary = serde_json::json!({
            "title": report["title"], "metadata": report["metadata"],
            "duration_seconds": report["duration_seconds"], "peak": report["peak"], "rms": report["rms"],
            "notes_requested": report["notes_requested"], "notes_rendered": report["notes_rendered"],
            "layers": report["composition_audit"]["layer_stats"],
            "notes_scheduled": report["composition_audit"]["notes_scheduled"],
            "notes_mixed": report["composition_audit"]["notes_mixed"],
        });
        let feedback =
            fs::read_to_string(self.root.join("music/python/feedback.jsonl")).unwrap_or_default();
        let feedback = python_feedback_context(&feedback);
        let context = format!(
            "PYMUSIC COMPOSITION CONTEXT AT DISPATCH: {edit_contract} Use import numpy as np and from teledra_synth import *. This is restored V1 freeform NumPy composition: timed overlapping arrays, automation and effects. synth_note('C4', duration, wave_type='sine', volume=0.2, sr=48000) provides classic synthesis. instrument_note('C4', duration, instrument='piano', volume=0.2, sr=48000) provides every native instrument in the catalog below. Numeric note means Hz, never MIDI. duration is total note length in seconds. Preserve the accepted source's sample rate consistently across note creation, sample counts, mixing and play_sound. For a new piece sr=48000 is supported. mix_waves(track, note, start_time, sr=48000) extends and adds each note without stealing voices. Compose all layers first, then call play_sound(full_track, loop=True, sr=48000) exactly once using the source's actual sample rate. That call captures a complete WAV for the court's one player; never use loops to play sound, file/network/process/device APIs, or external libraries. Keep total audio 4-600 seconds. Declare TELEDRA_SCORE={{'title':'...'}} for its name. Lead phrases need rests, answers and development; harmony and bass need independent timing and harmonic arrivals. Preserve {tonal_anchor} and {tempo} BPM as accepted comparison anchors unless the operator requests otherwise.\nSOURCE IDENTITY: {source_identity}\nFULL 72-INSTRUMENT CATALOG: {catalog}\nACCEPTED PYTHON REPORT: {}\n{source_kind}:\n{}\nPYTHON OPERATOR VERDICTS (only evidence for the named artifacts, never invent why a verdict was given):\n{feedback}\n{}",
            report_summary,
            baseline.chars().take(9000).collect::<String>(),
            self.music_feedback_guidance()
        );
        format!(
            "{}\nACCEPTED PYTHON ARTIFACT: {artifact}\n{context}\n{}",
            include_str!("../../../Music/PYTHON_AUTHORING.md"),
            self.music_learning.context(MusicEngine::Python)
        )
    }

    pub(super) fn request_python_repair(&mut self, app: &mut App, source: String, error: &str) {
        if !app.music_enabled() || !app.can_queue_organist_composition() {
            return;
        }
        let baseline = self
            .python_music
            .accepted
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| "first-python-piece".to_owned());
        if !self
            .corrections_requested
            .insert(super::digest_fingerprint(&format!(
                "python-repair:{baseline}"
            )))
        {
            return;
        }
        app.queue_backstage_turn(teledra_agents::AgentId::Organist,format!(
            "PYMUSIC COMPOSITION: COMPOSER CORRECTION: Your Python composition did not pass execution and audio verification. The previous music remains accepted. Correct the actual source once and return one complete [PYTHON_MUSIC: code]. Use the supplied authoring contract. Check imports, argument order, seconds/sample counts, additive mixing positions and all note placements. Do not rename the piece or claim playback.\nACTUAL ERROR:\n{}\nFAILED SOURCE:\n{}",error.chars().take(4000).collect::<String>(),source.chars().take(18000).collect::<String>()));
        app.push_private_event(
            "PyMusic",
            "Requested one correction using the actual render error",
        );
    }
    pub(crate) fn python_composition_requested(&self, user: &str) -> bool {
        if user.contains("NATIVE COMPOSITION") {
            return false;
        }
        user.contains("PYMUSIC COMPOSITION")
            || user.to_ascii_lowercase().contains("python_music")
            || self.music_engine == MusicEngine::Python
    }
    pub(super) fn active_music_duration(&self) -> f64 {
        // A guest set owns the stream until the Organist takes it back.
        if let Some(seconds) = self.guest.playing_seconds {
            seconds
        } else if self.music_engine == MusicEngine::Python && self.python_music.duration > 0.0 {
            self.python_music.duration
        } else {
            f64::from(self.score.duration_seconds())
        }
    }
    pub(super) fn music_transport(&mut self, state: &str) {
        if self.music_engine == MusicEngine::Python {
            let _ = self
                .python_music
                .send(serde_json::json!({"action":"transport","state":state}));
        } else {
            let _ = self.daw.set_transport(state);
        }
    }
    pub(super) fn stop_music_work(&mut self) {
        self.music_epoch = self.music_epoch.wrapping_add(1);
        self.music_pending = None;
        self.music_draft = None;
        self.music_ready = None;
        self.music_deferred = None;
        self.python_music.cancel();
        self.next_native_evolution = None;
    }
    pub(super) fn select_music_engine(
        &mut self,
        app: &mut App,
        tools: &mpsc::UnboundedSender<ToolReply>,
        engine: MusicEngine,
    ) {
        self.cancel_music_composition(app);
        self.stop_music_work();
        if let Some(player) = self.music_player.take() {
            player.stop();
        }
        // Wait for the old window's owned process tree before showing the new one.
        self.daw.stop();
        self.python_music.close();
        self.music_engine = engine;
        app.set_music_enabled(true);
        self.save_music_selection();
        self.daw.reopen();
        self.start_organist(app, tools);
    }
    pub(super) fn save_music_selection(&self) {
        let path = self.root.join("music/playback.json");
        let _ = fs::create_dir_all(path.parent().unwrap());
        let _ = fs::write(
            path,
            serde_json::json!({"engine":self.music_engine,"alternating":self.music_alternating})
                .to_string(),
        );
    }
    pub(super) fn toggle_music_comparison(&mut self, app: &mut App) {
        self.music_alternating = !self.music_alternating;
        self.save_music_selection();
        let _ = self.daw.set_alternating(self.music_alternating);
        let _ = self
            .python_music
            .send(serde_json::json!({"action":"alternating","enabled":self.music_alternating}));
        app.set_notice(if self.music_alternating {
            "New compositions will alternate Native Synth and PyMusic"
        } else {
            "New compositions will use the selected music engine"
        });
    }
    pub(super) fn start_python_music(
        &mut self,
        app: &mut App,
        tools: &mpsc::UnboundedSender<ToolReply>,
    ) {
        if self.music_player.as_ref().is_some_and(Player::is_finished) {
            self.music_player.take();
        }
        let state = if !app.music_enabled() {
            "paused"
        } else if app.music_muted() {
            "muted"
        } else {
            "playing"
        };
        self.daw.stop();
        if let Err(error) = self.python_music.show(state, self.music_alternating, tools) {
            app.push_private_event("PyMusic", error.to_string());
            return;
        }
        if let Some(player) = &self.music_player {
            player.set_muted(app.music_muted());
            if app.music_enabled() {
                player.resume();
            }
            return;
        }
        if self.python_music.rendering.is_none() {
            // A legacy technical-only gate admitted four-second scale exercises.
            // Recover a known full V1 reference without erasing that saved draft.
            if self.python_music.accepted.is_some()
                && self.python_music.report["duration_seconds"]
                    .as_f64()
                    .is_some_and(|seconds| seconds < 30.0)
            {
                if let Ok(draft) = fs::read(&self.python_music.draft) {
                    let backup = self.root.join("music/python").join(format!(
                        "draft-before-reference-{}.py",
                        super::epoch_millis()
                    ));
                    if let Err(error) = fs::write(&backup, draft) {
                        app.push_private_event("PyMusic", format!("Could not preserve the old draft before reference recovery: {error}"));
                        return;
                    }
                }
                app.push_private_event("PyMusic", "The saved fragment is under 30 seconds; restoring the full V1 reference while the Organist develops a new library piece");
                self.submit_python_music(
                    app,
                    tools,
                    include_str!("../../../Music/v1_music_example.py").to_owned(),
                );
                return;
            }
            if let Some(directory) = self.python_music.accepted.clone() {
                self.stop_music_work();
                let ticket = self.music_epoch;
                self.python_music.rendering = Some(ticket);
                let tools = tools.clone();
                std::thread::spawn(move || {
                    let result = fs::read_to_string(directory.join("code.py"))
                        .map_err(|e| e.to_string())
                        .and_then(|code| read_python_artifact(&directory, code, true));
                    let effect = match result {
                        Ok(render) => ToolEffect::PythonReady { ticket, render },
                        Err(error) => ToolEffect::PythonFailed { ticket, error },
                    };
                    let _ = tools.send(ToolReply {
                        source: "PyMusic",
                        result: Ok("Restored complete Python artifact".into()),
                        court_prompt: None,
                        effect: Some(effect),
                    });
                });
            } else {
                self.submit_python_music(app, tools, self.python_music.code.clone());
            }
        }
    }
    pub(super) fn submit_python_music(
        &mut self,
        app: &mut App,
        tools: &mpsc::UnboundedSender<ToolReply>,
        raw: String,
    ) {
        let code = clean_python_source(&raw);
        if code.trim().is_empty() || code.len() > 128 * 1024 {
            app.push_private_event("PyMusic", "Source must be complete and under 128 KiB");
            return;
        }
        self.stop_music_work();
        let ticket = self.music_epoch;
        let cancel = Arc::new(AtomicBool::new(false));
        self.python_music.cancel = Some(cancel.clone());
        self.python_music.rendering = Some(ticket);
        let directory = self
            .root
            .join("music/python")
            .join(format!("render-{}-{ticket}", super::epoch_millis()));
        let python = self.python_music.python.clone();
        let workspace = self.python_music.workspace.clone();
        let tools = tools.clone();
        let synth = self.python_music.synth.clone();
        let _=self.python_music.send(serde_json::json!({"action":"status","message":"Rendering complete Python composition…"}));
        std::thread::spawn(move || {
            let result = render_python(&python, &workspace, &synth, &directory, code, &cancel);
            let (message, effect) = match result {
                Ok(render) => (
                    Ok(format!(
                        "Verified PyMusic: {:.1}s, peak {:.3}",
                        render.audio.report().duration_seconds,
                        render.audio.report().peak
                    )),
                    ToolEffect::PythonReady { ticket, render },
                ),
                Err(error) => (
                    Err(error.clone()),
                    ToolEffect::PythonFailed { ticket, error },
                ),
            };
            let _ = tools.send(ToolReply {
                source: "PyMusic",
                result: message,
                court_prompt: None,
                effect: Some(effect),
            });
        });
        app.set_notice("PyMusic is rendering; only complete verified audio can play");
    }
    pub(super) fn python_ready(
        &mut self,
        app: &mut App,
        tools: &mpsc::UnboundedSender<ToolReply>,
        ticket: u64,
        render: PythonRender,
    ) {
        let generated = self.python_music.rendering == Some(ticket)
            && self.python_music.repair_source.is_some();
        if self.python_music.rendering == Some(ticket) {
            self.python_music.rendering = None;
            self.python_music.cancel = None;
            self.python_music.repair_source = None;
        }
        if ticket != self.music_epoch {
            return;
        }
        if generated
            && !render.restore
            && render.report["duration_seconds"].as_f64().unwrap_or(0.0) < 30.0
        {
            let reason = "Background tracks must be complete arrangements of at least 30 seconds; short sketches remain experiments";
            self.record_music_outcome(app, MusicEngine::Python, "incomplete", "", reason);
            self.schedule_native_evolution();
            app.push_private_event("PyMusic", reason);
            return;
        }
        if generated
            && !render.restore
            && crate::python_score::from_source(&render.code).is_some()
            && let Err(reason) = MusicLibrary::eligibility(&render.report)
        {
            self.record_music_outcome(app, MusicEngine::Python, "incomplete", "", &reason);
            self.schedule_native_evolution();
            app.push_private_event("PyMusic", format!("Current audio retained: {reason}"));
            return;
        }
        if generated
            && !render.restore
            && let Some(reason) = duplicate_python_reason(&self.python_music.report, &render.report)
        {
            self.schedule_native_evolution();
            self.record_music_outcome(
                app,
                MusicEngine::Python,
                "unchanged",
                render.report["audio_digest"].as_str().unwrap_or(""),
                reason,
            );
            let _ = self
                .python_music
                .send(serde_json::json!({"action":"status","message":reason}));
            app.push_private_event(
                "PyMusic",
                format!("Kept the current piece and playhead: {reason}"),
            );
            return;
        }
        let rota_fallback = render.restore && self.music_rota.fallback_ticket == Some(ticket);
        if render.restore && !rota_fallback {
            // Explicit Library Play should respond as soon as loading finishes;
            // automatically composed replacements still finish the current pass.
            self.accept_python_music(app, tools, render);
        } else if let Some(player) = &self.music_player {
            self.python_music.ready = Some(ReadyPython {
                ticket,
                render,
                switch_after: super::next_music_boundary(
                    player.position_seconds(),
                    self.active_music_duration(),
                ),
            });
            app.set_notice("PyMusic is ready; finishing the current playback pass");
        } else {
            self.accept_python_music(app, tools, render);
        }
    }
    pub(super) fn publish_python_if_ready(
        &mut self,
        app: &mut App,
        tools: &mpsc::UnboundedSender<ToolReply>,
    ) {
        let Some(ready) = &self.python_music.ready else {
            return;
        };
        if ready.ticket != self.music_epoch {
            self.python_music.ready = None;
            return;
        }
        if self.music_player.as_ref().is_some_and(|player| {
            !player.is_finished() && player.position_seconds() < ready.switch_after
        }) {
            return;
        }
        let ready = self.python_music.ready.take().unwrap();
        self.accept_python_music(app, tools, ready.render);
    }
    fn accept_python_music(
        &mut self,
        app: &mut App,
        tools: &mpsc::UnboundedSender<ToolReply>,
        render: PythonRender,
    ) {
        let recovered_fragment = self.python_music.report["duration_seconds"]
            .as_f64()
            .is_some_and(|seconds| seconds < 30.0)
            && super::music_library::is_bundled_v1_source(&render.code);
        let looped = render.report["loop"].as_bool().unwrap_or(true);
        let duration = f64::from(render.audio.report().duration_seconds);
        let artifact = render.report["audio_digest"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| render.directory.to_string_lossy().into_owned());
        let measured = format!(
            "Verified {duration:.3}s stereo audio, {} frames, {}/{} note-helper calls; source_digest={}; PCM identity={artifact}. Playback verification is not listener approval.",
            render.report["frames"],
            render.report["notes_rendered"],
            render.report["notes_requested"],
            render.report["source_digest"]
        );
        // Reuse the current stream for replacements. Only initial playback
        // needs a new, paused device; failures must retain the previous player.
        let initial_player = if self.music_player.is_none() {
            match Player::prepare_on(
                level_for_court(&render.audio),
                looped,
                std::env::var("TELEDRA_AUDIO_DEVICE").ok().as_deref(),
            ) {
                Ok(player) => Some(player),
                Err(error) => {
                    self.python_acceptance_failed(app, &artifact, &error.to_string());
                    return;
                }
            }
        } else {
            None
        };
        let home = self.root.join("music/python");
        let pointer =
            serde_json::json!({"artifact":render.directory.file_name().unwrap().to_string_lossy()});
        let persist = fs::write(home.join("current-next.json"), pointer.to_string());
        if let Err(error) = persist {
            self.python_acceptance_failed(
                app,
                &artifact,
                &format!("Could not save completed composition: {error}"),
            );
            return;
        }
        // The outgoing PCM keeps playing while the old window acknowledges
        // shutdown. The actual audio handoff then happens within one stream.
        self.daw.stop();
        if let Some(player) = self.music_player.as_mut() {
            if let Err(error) = player.replace(level_for_court(&render.audio), looped, 0.0) {
                self.python_acceptance_failed(app, &artifact, &error.to_string());
                return;
            }
        } else {
            self.music_player = initial_player;
        }
        // current.json represents successful playback selection. Never advance
        // it ahead of the player operation; a failed replacement keeps recovery intact.
        if let Err(error) = fs::rename(home.join("current-next.json"), home.join("current.json")) {
            app.push_private_event(
                "PyMusic",
                format!(
                    "New audio is playing but the recovery pointer could not be saved: {error}"
                ),
            );
        }
        self.python_music.active_library_id = None;
        if let Some(library) = self.python_music.library.as_mut() {
            let result = if MusicLibrary::eligibility(&render.report).is_ok() {
                Some(library.record_published(&render))
            } else if super::music_library::is_bundled_v1_source(&render.code) {
                Some(library.record_verified_v1_restore(&render))
            } else {
                self.python_music.active_library_id = library.id_for_artifact(&render.directory);
                None
            };
            if let Some(result) = result {
                match result {
                    Ok(id) => self.python_music.active_library_id = Some(id),
                    Err(error) => app.push_private_event(
                        "Music library",
                        format!("Audio selected, but library save failed: {error}"),
                    ),
                }
            }
        }
        self.music_engine = MusicEngine::Python;
        self.python_music.accepted = Some(render.directory);
        self.python_music.accepted_code = Some(render.code.clone());
        if !render.restore {
            self.python_music.code = render.code;
            let _ = fs::write(home.join("draft.py"), &self.python_music.code);
        }
        self.python_music.report = render.report;
        self.python_music.duration = duration;
        if let Some(player) = &self.music_player {
            player.set_muted(app.music_muted());
            if app.music_enabled() {
                player.resume();
            } else {
                player.pause();
            }
        }
        self.save_music_selection();
        self.schedule_native_evolution();
        self.record_music_outcome(app, MusicEngine::Python, "accepted", &artifact, &measured);
        let state = if !app.music_enabled() {
            "paused"
        } else if app.music_muted() {
            "muted"
        } else {
            "playing"
        };
        if let Err(error) = self.python_music.show(state, self.music_alternating, tools) {
            app.push_private_event("PyMusic", error.to_string());
        }
        app.set_notice(format!(
            "PyMusic ready: {}",
            self.python_music.report["title"]
                .as_str()
                .unwrap_or("Python composition")
        ));
        if recovered_fragment {
            self.queue_composition(app, true);
        }
    }

    fn python_acceptance_failed(&mut self, app: &mut App, artifact: &str, error: &str) {
        // Submission disarmed the old deadline. A failed device/persistence
        // commit must not permanently stop future improvement attempts.
        self.schedule_native_evolution();
        self.record_music_outcome(
            app,
            MusicEngine::Python,
            "acceptance_failed",
            artifact,
            error,
        );
        let message = format!("Previous audio retained: {error}");
        let _ = self
            .python_music
            .send(serde_json::json!({"action":"status","message":message}));
        app.push_private_event("PyMusic", message);
    }
    pub(super) fn handle_python_action(
        &mut self,
        app: &mut App,
        tools: &mpsc::UnboundedSender<ToolReply>,
        session: u64,
        action: PythonAction,
    ) {
        if session != self.python_music.session || self.music_engine != MusicEngine::Python {
            return;
        }
        match action {
            PythonAction::Run { code } => {
                self.cancel_music_composition(app);
                app.set_music_enabled(true);
                if let Some(player) = &self.music_player {
                    player.resume();
                }
                self.submit_python_music(app, tools, code);
            }
            PythonAction::Save { code } => {
                let directory = self.root.join("music/python");
                let result = fs::create_dir_all(&directory)
                    .and_then(|_| fs::write(directory.join("draft.py"), &code));
                match result {
                    Ok(()) => self.python_music.code = code,
                    Err(error) => app.push_private_event("PyMusic", error.to_string()),
                }
            }
            PythonAction::Native => self.select_music_engine(app, tools, MusicEngine::Native),
            PythonAction::Alternate => self.toggle_music_comparison(app),
            PythonAction::Compose => self.queue_composition(app, true),
            PythonAction::LibraryPlay { id } => {
                let Some(library) = self.python_music.library.clone() else {
                    app.push_private_event("Music library", "The library is unavailable");
                    return;
                };
                self.cancel_music_composition(app);
                self.stop_music_work();
                app.set_music_enabled(true);
                if let Some(player) = &self.music_player {
                    player.resume();
                }
                let ticket = self.music_epoch;
                self.python_music.rendering = Some(ticket);
                let tools = tools.clone();
                std::thread::spawn(move || {
                    let effect = match library.load_playback(&id) {
                        Ok(render) => ToolEffect::PythonReady { ticket, render },
                        Err(error) => ToolEffect::PythonFailed { ticket, error },
                    };
                    let _ = tools.send(ToolReply {
                        source: "Music library",
                        result: Ok("Loaded library selection".into()),
                        court_prompt: None,
                        effect: Some(effect),
                    });
                });
                app.set_notice(
                    "Loading library selection; current audio continues until it is ready",
                );
            }
            PythonAction::Play => {
                app.set_music_enabled(true);
                self.start_python_music(app, tools);
                self.music_transport(if app.music_muted() {
                    "muted"
                } else {
                    "playing"
                });
            }
            PythonAction::Pause => {
                app.set_music_enabled(false);
                if let Some(player) = &self.music_player {
                    player.pause();
                }
                self.music_transport("paused");
            }
            PythonAction::Stop | PythonAction::Close => {
                self.cancel_music_composition(app);
                self.stop_music_work();
                app.set_music_enabled(false);
                if let Some(player) = self.music_player.take() {
                    player.stop();
                }
                self.music_transport("stopped");
            }
            PythonAction::Keep => self.record_music_feedback(
                app,
                teledra_music::FeedbackVerdict::Keep,
                "kept from PyMusic".into(),
            ),
            PythonAction::Reject => self.record_music_feedback(
                app,
                teledra_music::FeedbackVerdict::Reject,
                "rejected from PyMusic".into(),
            ),
        }
    }
}

impl RuntimeServices {
    pub(super) fn live_python_feedback_snapshot(
        &self,
        verdict: teledra_music::FeedbackVerdict,
        note: &str,
    ) -> io::Result<serde_json::Value> {
        let artifact = self
            .python_music
            .accepted_artifact()
            .and_then(Path::file_name)
            .ok_or_else(|| io::Error::other("No accepted Python music is selected"))?;
        let code = self
            .python_music
            .accepted_source()
            .ok_or_else(|| io::Error::other("No accepted Python source is selected"))?;
        Ok(
            serde_json::json!({"timestamp_ms":super::epoch_millis(),"engine":"python","identity":artifact.to_string_lossy(),
            "library_id":self.python_music.active_library_id,"score_title":self.python_music.report["title"],
            "verdict":verdict,"note":note.trim(),"code":code,"report":self.python_music.report}),
        )
    }
    pub(super) fn record_python_library_verdict(
        &mut self,
        app: &mut App,
        verdict: teledra_music::FeedbackVerdict,
        note: &str,
    ) {
        let choice = match verdict {
            teledra_music::FeedbackVerdict::Keep => LibraryVerdict::Keep,
            teledra_music::FeedbackVerdict::Reject => LibraryVerdict::Reject,
            teledra_music::FeedbackVerdict::Revise => return,
        };
        if let Some(id) = self.python_music.active_library_id.clone()
            && let Some(library) = &mut self.python_music.library
            && let Err(error) = library.set_verdict(&id, choice, note)
        {
            app.push_private_event(
                "Music library",
                format!("Could not save listener verdict: {error}"),
            );
        }
        let _ = self.python_music.send_library();
    }
}

fn clean_python_source(raw: &str) -> String {
    let text = raw.trim();
    if let Some(body) = text
        .strip_prefix("```python")
        .or_else(|| text.strip_prefix("```"))
    {
        body.strip_suffix("```").unwrap_or(body).trim().to_owned()
    } else {
        text.to_owned()
    }
}

fn duplicate_python_reason(
    current: &serde_json::Value,
    candidate: &serde_json::Value,
) -> Option<&'static str> {
    for (field, reason) in [
        (
            "audio_digest",
            "The proposed revision renders exactly the same audio; titles and comments do not establish a musical change.",
        ),
        (
            "source_digest",
            "The proposed revision repeats the accepted source without a compositional change.",
        ),
    ] {
        if let (Some(current), Some(candidate)) =
            (current[field].as_str(), candidate[field].as_str())
            && !current.is_empty()
            && current == candidate
        {
            return Some(reason);
        }
    }
    None
}

fn python_feedback_context(raw: &str) -> String {
    let mut events = Vec::new();
    let mut bytes = 0;
    for event in raw
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .take(8)
    {
        let excerpt = |value: &serde_json::Value, limit| {
            value
                .as_str()
                .unwrap_or_default()
                .chars()
                .take(limit)
                .collect::<String>()
        };
        let report = &event["report"];
        let projected = serde_json::json!({
            "identity": excerpt(&event["identity"], 160),
            "title": excerpt(&event["score_title"], 120),
            "verdict": excerpt(&event["verdict"], 32),
            "note": excerpt(&event["note"], 500),
            "note_truncated": event["note"].as_str().is_some_and(|note| note.chars().count() > 500),
            "measured": {
                "duration_seconds": report["duration_seconds"].as_f64(),
                "peak": report["peak"].as_f64(), "rms": report["rms"].as_f64(),
                "notes_requested": report["notes_requested"].as_u64(),
                "notes_rendered": report["notes_rendered"].as_u64(),
                "key": excerpt(&report["metadata"]["key"], 64),
                "bpm": report["metadata"]["bpm"].as_f64(),
                "bars": report["metadata"]["bars"].as_u64(),
            },
        })
        .to_string();
        if bytes + projected.len() + 1 > 4096 {
            break;
        }
        bytes += projected.len() + 1;
        events.push(projected);
    }
    events.reverse();
    events.join("\n")
}

pub(super) fn render_python(
    python: &Path,
    workspace: &Path,
    synth: &Path,
    directory: &Path,
    code: String,
    cancel: &AtomicBool,
) -> Result<PythonRender, String> {
    fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    fs::write(directory.join("code.py"), &code).map_err(|e| e.to_string())?;
    let log = fs::File::create(directory.join("error.log")).map_err(|e| e.to_string())?;
    let mut command = Command::new(python);
    command
        .arg(workspace.join("Music/python_music_worker.py"))
        .args(["--code"])
        .arg(directory.join("code.py"))
        .arg("--output")
        .arg(directory.join("audio.wav"))
        .arg("--report")
        .arg(directory.join("report.json"))
        .arg("--synth-exe")
        .arg(synth)
        .current_dir(workspace.join("Music"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log));
    super::daw::hide_console(&mut command);
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(120);
    let status = loop {
        if cancel.load(Ordering::Acquire) || Instant::now() >= deadline {
            super::daw::kill_process_tree(&mut child);
            let _ = child.wait();
            return Err(
                "PyMusic render cancelled or exceeded 120 seconds; previous audio retained".into(),
            );
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(40));
    };
    if !status.success() {
        let mut message = fs::read(directory.join("report.json"))
            .ok()
            .and_then(|raw| serde_json::from_slice::<serde_json::Value>(&raw).ok())
            .and_then(|report| report["error"].as_str().map(str::to_owned))
            .unwrap_or_default();
        let _ = fs::File::open(directory.join("error.log"))
            .map(|f| f.take(6000).read_to_string(&mut message));
        return Err(format!("Python composition failed: {message}"));
    }
    read_python_artifact(directory, code, false)
}

pub(super) fn read_python_artifact(
    directory: &Path,
    code: String,
    restore: bool,
) -> Result<PythonRender, String> {
    let audio = read_python_wav(&directory.join("audio.wav"))?;
    let mut report: serde_json::Value = serde_json::from_slice(
        &fs::read(directory.join("report.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("Missing complete PyMusic report: {e}"))?;
    if report["ok"] != true
        || report["sample_rate"] != 48_000
        || report["channels"] != 2
        || report["frames"].as_u64() != Some((audio.samples().len() / 2) as u64)
        || report["notes_requested"].as_u64().is_none()
        || report["notes_requested"] != report["notes_rendered"]
        || report["duration_seconds"].as_f64().is_none_or(|seconds| {
            (seconds - f64::from(audio.report().duration_seconds)).abs() > 0.001
        })
    {
        return Err("PyMusic report does not match the complete audio and note counts".to_owned());
    }
    // This function runs in the render/restore worker. Hash complete PCM and
    // source here rather than scanning a long piece on the interactive loop.
    report["source_digest"] = super::stable_digest(code.bytes()).into();
    report["audio_digest"] = super::stable_digest(
        audio
            .samples()
            .iter()
            .flat_map(|sample| sample.to_bits().to_le_bytes()),
    )
    .into();
    Ok(PythonRender {
        restore,
        code,
        directory: directory.to_owned(),
        report,
        audio,
    })
}

/// Court playback loudness, as RMS of the mixed signal (about -25 dBFS).
/// Buzzart's sets are mastered here; the Organist's renders arrived near
/// -34 dBFS, so every handover jumped about 9 dB.
const COURT_MUSIC_RMS: f32 = 0.056;

/// The same piece at the court's playback loudness. Gain is bounded (never
/// more than 8x, never pushing a peak past 0.9), so a quiet intro or a sparse
/// piece is lifted without clipping, and already-loud audio is left alone.
pub(super) fn level_for_court(audio: &VerifiedAudio) -> VerifiedAudio {
    let samples = audio.samples();
    if samples.is_empty() {
        return audio.clone();
    }
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
    let peak = samples.iter().fold(0.0_f32, |peak, s| peak.max(s.abs()));
    if rms <= 1e-6 || peak <= 1e-6 {
        return audio.clone();
    }
    let gain = (COURT_MUSIC_RMS / rms).min(0.9 / peak).clamp(0.5, 8.0);
    if (gain - 1.0).abs() < 0.05 {
        return audio.clone();
    }
    VerifiedAudio::new(samples.iter().map(|s| s * gain).collect()).unwrap_or_else(|_| audio.clone())
}

pub(super) fn read_python_wav(path: &Path) -> Result<VerifiedAudio, String> {
    let size = fs::metadata(path).map_err(|e| e.to_string())?.len();
    if !(44..=116_000_000).contains(&size) {
        return Err("PyMusic WAV length is outside the complete render limit".into());
    }
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    if &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("PyMusic output is not WAV".into());
    }
    if u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize + 8 != bytes.len() {
        return Err("Incomplete WAV container".to_owned());
    }
    let mut cursor = 12;
    let mut valid_format = false;
    let mut samples = None;
    while cursor + 8 <= bytes.len() {
        let n = u32::from_le_bytes(bytes[cursor + 4..cursor + 8].try_into().unwrap()) as usize;
        let start = cursor + 8;
        let end = start
            .checked_add(n)
            .filter(|end| *end <= bytes.len())
            .ok_or("Incomplete WAV chunk")?;
        match &bytes[cursor..cursor + 4] {
            b"fmt " if n >= 16 => {
                valid_format = u16::from_le_bytes(bytes[start..start + 2].try_into().unwrap()) == 1
                    && u16::from_le_bytes(bytes[start + 2..start + 4].try_into().unwrap()) == 2
                    && u32::from_le_bytes(bytes[start + 4..start + 8].try_into().unwrap())
                        == 48_000
                    && u16::from_le_bytes(bytes[start + 14..start + 16].try_into().unwrap()) == 16;
            }
            b"data" => {
                if !n.is_multiple_of(4) {
                    return Err("Incomplete stereo frames".into());
                }
                samples = Some(
                    bytes[start..end]
                        .chunks_exact(2)
                        .map(|v| f32::from(i16::from_le_bytes([v[0], v[1]])) / 32768.0)
                        .collect::<Vec<_>>(),
                );
            }
            _ => {}
        }
        cursor = end + n % 2;
    }
    if !valid_format {
        return Err("PyMusic must render PCM16 stereo at 48000 Hz".into());
    }
    VerifiedAudio::new(samples.ok_or("Missing WAV samples")?).map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "pymusic_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "pymusic_lifecycle_tests.rs"]
mod lifecycle_tests;
