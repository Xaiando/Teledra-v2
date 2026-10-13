//! Published, complete Python pieces. Render validity and listener taste are
//! deliberately separate records; a successful worker report alone imports nothing.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::pymusic::{PythonRender, read_python_artifact};

const MAX_INDEX_BYTES: u64 = 8_000_000;
const MAX_SOURCE_BYTES: u64 = 128 * 1024;
const MAX_REPORT_BYTES: u64 = 2 * 1024 * 1024;

pub(super) fn is_bundled_v1_source(source: &str) -> bool {
    // Submission trims the outer whitespace. Preserve every internal source
    // byte while recognizing the same reference across Windows line endings.
    source.replace("\r\n", "\n").trim()
        == include_str!("../../../Music/v1_music_example.py")
            .replace("\r\n", "\n")
            .trim()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct LibraryEntry {
    pub(super) id: String,
    pub(super) artifact: String,
    pub(super) source_identity: String,
    pub(super) pcm_identity: String,
    report_identity: String,
    pub(super) title: String,
    pub(super) duration_seconds: f64,
    pub(super) note_count: u64,
    pub(super) metadata: Value,
    pub(super) added_ms: u64,
    pub(super) eligibility: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum LibraryVerdict {
    Keep,
    Reject,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ListenerPreference {
    verdict: LibraryVerdict,
    note: String,
    recorded_ms: u64,
}

#[derive(Clone, Deserialize, Serialize)]
struct LibraryState {
    version: u8,
    entries: Vec<LibraryEntry>,
    // Render aliases retain restart identity after source/PCM deduplication.
    aliases: BTreeMap<String, String>,
    // Preferences never alter the frozen artifact, measured facts, or eligibility.
    preferences: BTreeMap<String, ListenerPreference>,
}

impl Default for LibraryState {
    fn default() -> Self {
        Self {
            version: 1,
            entries: Vec::new(),
            aliases: BTreeMap::new(),
            preferences: BTreeMap::new(),
        }
    }
}

#[derive(Clone)]
pub(super) struct MusicLibrary {
    home: PathBuf,
    state: LibraryState,
}

impl MusicLibrary {
    /// Missing is an empty library. Corruption is an error, never an invitation
    /// to overwrite the old catalog; the caller can disable this optional feature.
    pub(super) fn load(root: &Path) -> io::Result<Self> {
        let home = root.join("music/python");
        let state = match read_limited(&home.join("library.json"), MAX_INDEX_BYTES) {
            Ok(bytes) => serde_json::from_slice::<LibraryState>(&bytes)
                .map_err(|error| invalid(format!("Cannot decode music library: {error}")))?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => LibraryState::default(),
            Err(error) => return Err(error),
        };
        validate_state(&state)?;
        Ok(Self { home, state })
    }

    /// Call only after the verified render has successfully become the player
    /// selection. Manual short sketches may play without entering this library.
    pub(super) fn record_published(&mut self, render: &PythonRender) -> io::Result<String> {
        Self::eligibility(&render.report).map_err(invalid)?;
        self.record(render, "verified_complete")
    }

    /// Explicit recovery of the bundled, known full V1 composition. This is not
    /// a directory scanner or a way to promote arbitrary successful experiments.
    pub(super) fn record_verified_v1_restore(
        &mut self,
        render: &PythonRender,
    ) -> io::Result<String> {
        if !is_bundled_v1_source(&render.code)
            || (f64::from(render.audio.report().duration_seconds) - 88.0).abs() > 0.01
            || render.report["notes_rendered"].as_u64().unwrap_or(0) < 500
        {
            return Err(invalid(
                "This artifact is not the verified bundled V1 composition",
            ));
        }
        self.record(render, "verified_v1_reference")
    }

    /// This is a complete-piece admission rule, not a claim that the music is
    /// beautiful or that an operator likes it. Worker audit details are required.
    pub(super) fn eligibility(report: &Value) -> Result<(), String> {
        let duration = report["duration_seconds"].as_f64().unwrap_or(0.0);
        if !duration.is_finite() || !(30.0..=600.0).contains(&duration) {
            return Err("Library pieces must contain 30–600 seconds of complete audio".into());
        }
        let audit = &report["composition_audit"];
        if audit["version"] != 1
            || audit["renderer"] != "render_song"
            || audit["engine"] != "pymusic"
            || audit["complete"] != true
            || audit["notes_scheduled"].as_u64().unwrap_or(0) < 16
            || audit["notes_scheduled"] != audit["notes_mixed"]
            || audit["active_layers"].as_u64().unwrap_or(0) < 5
            || audit["layer_count"] != audit["active_layers"]
            || audit["events"].as_array().is_none_or(|events| {
                Some(events.len() as u64) != audit["notes_mixed"].as_u64()
                    || events.iter().any(|event| event["mixed"] != true)
            })
            || audit["sections"].as_array().is_none_or(|sections| {
                sections.len() < 4
                    || Some(sections.len() as u64) != audit["section_count"].as_u64()
                    || sections
                        .iter()
                        .any(|section| !positive_measure(&section["rms"]))
            })
            || audit["layer_stats"].as_array().is_none_or(|layers| {
                layers
                    .iter()
                    .filter(|layer| {
                        layer["events"].as_u64().unwrap_or(0) > 0 && positive_measure(&layer["rms"])
                    })
                    .count()
                    < 5
            })
            || audit["duration_seconds"]
                .as_f64()
                .is_none_or(|seconds| (seconds - duration).abs() > 0.01)
        {
            return Err("A complete composition audit is required for the music library".into());
        }
        Ok(())
    }

    fn record(&mut self, render: &PythonRender, eligibility: &str) -> io::Result<String> {
        let artifact = self.checked_artifact(&render.directory)?;
        let disk_code = read_limited(&render.directory.join("code.py"), MAX_SOURCE_BYTES)?;
        if disk_code != render.code.as_bytes() {
            return Err(invalid("Published source differs from the stored source"));
        }
        let disk_report: Value = serde_json::from_slice(&read_limited(
            &render.directory.join("report.json"),
            MAX_REPORT_BYTES,
        )?)
        .map_err(io::Error::other)?;
        validate_report(render)?;
        let report_identity = report_digest(&render.report);
        if report_identity != report_digest(&disk_report) {
            return Err(invalid(
                "Published report differs from the stored render report",
            ));
        }
        let source_identity = super::stable_digest(render.code.bytes());
        let pcm_identity = pcm_identity(render);
        if let Some(existing) = self.state.entries.iter().find(|entry| {
            entry.source_identity == source_identity || entry.pcm_identity == pcm_identity
        }) {
            let id = existing.id.clone();
            if self.state.aliases.get(&artifact) != Some(&id) {
                let mut next = self.state.clone();
                next.aliases.insert(artifact, id.clone());
                self.commit(next)?;
            }
            return Ok(id);
        }
        let id = format!("py-{pcm_identity}");
        let title = render.report["title"]
            .as_str()
            .filter(|title| !title.trim().is_empty())
            .unwrap_or("Python composition")
            .chars()
            .take(160)
            .collect();
        let mut next = self.state.clone();
        next.entries.push(LibraryEntry {
            id: id.clone(),
            artifact: artifact.clone(),
            source_identity,
            pcm_identity,
            report_identity,
            title,
            duration_seconds: render.report["duration_seconds"].as_f64().unwrap(),
            note_count: render.report["composition_audit"]["notes_mixed"]
                .as_u64()
                .unwrap_or_else(|| render.report["notes_rendered"].as_u64().unwrap()),
            metadata: render.report["metadata"].clone(),
            added_ms: now_ms(),
            eligibility: eligibility.to_owned(),
        });
        next.aliases.insert(artifact, id.clone());
        self.commit(next)?;
        Ok(id)
    }

    pub(super) fn id_for_artifact(&self, path: &Path) -> Option<String> {
        let artifact = self.checked_artifact(path).ok()?;
        self.state.aliases.get(&artifact).cloned()
    }

    /// Run on the existing restore worker, never on the interactive event loop.
    /// No UI-supplied path reaches this method: IDs resolve only frozen entries.
    pub(super) fn load_playback(&self, id: &str) -> Result<PythonRender, String> {
        let entry = self
            .state
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| "Unknown music library ID".to_owned())?;
        let directory = self.home.join(&entry.artifact);
        self.checked_artifact(&directory)
            .map_err(|error| error.to_string())?;
        let code = String::from_utf8(
            read_limited(&directory.join("code.py"), MAX_SOURCE_BYTES)
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        let render = read_python_artifact(&directory, code, true)?;
        if super::stable_digest(render.code.bytes()) != entry.source_identity
            || pcm_identity(&render) != entry.pcm_identity
            || report_digest(&render.report) != entry.report_identity
        {
            return Err("Library artifact changed after publication; playback refused".into());
        }
        Ok(render)
    }

    /// A different piece for the Organist's round: the newest version of each
    /// distinct title, never one the listener rejected or `skip` excludes.
    /// Pieces the listener kept are preferred; otherwise any eligible title.
    pub(super) fn rotation_pick(&self, skip: impl Fn(&str) -> bool) -> Option<(String, String)> {
        let mut newest: BTreeMap<&str, &LibraryEntry> = BTreeMap::new();
        for entry in &self.state.entries {
            if entry.eligibility != "verified_complete" || skip(&entry.title) {
                continue;
            }
            let slot = newest.entry(entry.title.as_str()).or_insert(entry);
            if entry.added_ms >= slot.added_ms {
                *slot = entry;
            }
        }
        let verdict = |entry: &LibraryEntry| {
            self.state
                .preferences
                .get(&entry.id)
                .map(|preference| preference.verdict)
        };
        let candidates: Vec<&LibraryEntry> = newest
            .into_values()
            .filter(|entry| verdict(entry) != Some(LibraryVerdict::Reject))
            .collect();
        let kept: Vec<&LibraryEntry> = candidates
            .iter()
            .copied()
            .filter(|entry| verdict(entry) == Some(LibraryVerdict::Keep))
            .collect();
        let pool = if kept.is_empty() { candidates } else { kept };
        let entry = pool.get((now_ms() % pool.len().max(1) as u64) as usize)?;
        Some((entry.id.clone(), entry.title.clone()))
    }

    pub(super) fn set_verdict(
        &mut self,
        id: &str,
        verdict: LibraryVerdict,
        note: &str,
    ) -> io::Result<()> {
        if !self.state.entries.iter().any(|entry| entry.id == id) {
            return Err(invalid("Unknown music library ID"));
        }
        let mut next = self.state.clone();
        next.preferences.insert(
            id.to_owned(),
            ListenerPreference {
                verdict,
                note: note.chars().take(2_000).collect(),
                recorded_ms: now_ms(),
            },
        );
        self.commit(next)
    }

    pub(super) fn editor_state(&self, active_id: Option<&str>) -> Value {
        let mut title_counts = BTreeMap::new();
        for entry in &self.state.entries {
            *title_counts.entry(entry.title.as_str()).or_insert(0_usize) += 1;
        }
        let mut versions = BTreeMap::new();
        // Catalog insertion order is immutable. Number before reversing for
        // display so adding a new version never renumbers an older one.
        let mut entries: Vec<_> = self
            .state
            .entries
            .iter()
            .map(|entry| {
                let version = versions.entry(entry.title.as_str()).or_insert(0_usize);
                *version += 1;
                let display_title = if title_counts[entry.title.as_str()] > 1 {
                    format!("{} · v{version}", entry.title)
                } else {
                    entry.title.clone()
                };
                let verdict =
                    self.state
                        .preferences
                        .get(&entry.id)
                        .map_or("unreviewed", |preference| match preference.verdict {
                            LibraryVerdict::Keep => "keep",
                            LibraryVerdict::Reject => "reject",
                        });
                json!({"id":entry.id,"title":entry.title,"display_title":display_title,
                "added_ms":entry.added_ms,"duration_seconds":entry.duration_seconds,
                "note_count":entry.note_count,"metadata":entry.metadata,"verdict":verdict,
                "eligibility":entry.eligibility})
            })
            .collect();
        entries.reverse();
        json!({"action":"library","entries":entries,"active_id":active_id})
    }

    fn checked_artifact(&self, directory: &Path) -> io::Result<String> {
        let name = directory
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| valid_artifact(name))
            .ok_or_else(|| invalid("Invalid render directory"))?;
        let home = self.home.canonicalize()?;
        let actual = directory.canonicalize()?;
        if actual.parent() != Some(home.as_path()) || !actual.is_dir() {
            return Err(invalid(
                "Library render must be a direct child of music/python",
            ));
        }
        for filename in ["code.py", "report.json", "audio.wav"] {
            let path = actual.join(filename);
            let target = path.canonicalize()?;
            if target.parent() != Some(actual.as_path()) || !target.is_file() {
                return Err(invalid(
                    "Library artifact file escapes its render directory",
                ));
            }
        }
        Ok(name.to_owned())
    }

    fn commit(&mut self, next: LibraryState) -> io::Result<()> {
        validate_state(&next)?;
        let bytes = serde_json::to_vec_pretty(&next).map_err(io::Error::other)?;
        if bytes.len() as u64 > MAX_INDEX_BYTES {
            return Err(invalid("Music library index exceeds its read budget"));
        }
        fs::create_dir_all(&self.home)?;
        let temporary = self
            .home
            .join(format!("library-next-{}.json", std::process::id()));
        fs::write(&temporary, bytes)?;
        fs::rename(&temporary, self.home.join("library.json"))?;
        self.state = next;
        Ok(())
    }
}

fn validate_report(render: &PythonRender) -> io::Result<()> {
    let report = &render.report;
    if report["ok"] != true
        || report["sample_rate"] != 48_000
        || report["channels"] != 2
        || report["frames"].as_u64() != Some((render.audio.samples().len() / 2) as u64)
        || report["notes_requested"].as_u64().is_none()
        || report["notes_requested"] != report["notes_rendered"]
        || report["source_digest"].as_str()
            != Some(super::stable_digest(render.code.bytes()).as_str())
        || report["audio_digest"].as_str().is_none_or(|digest| {
            digest.len() != 16 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        || report["duration_seconds"].as_f64().is_none_or(|seconds| {
            !seconds.is_finite()
                || (seconds - f64::from(render.audio.report().duration_seconds)).abs() > 0.001
        })
    {
        return Err(invalid(
            "Library report does not match verified audio and complete note counts",
        ));
    }
    Ok(())
}

fn report_digest(report: &Value) -> String {
    let mut report = report.clone();
    if let Some(object) = report.as_object_mut() {
        object.remove("source_digest");
        object.remove("audio_digest");
    }
    super::stable_digest(report.to_string().bytes())
}

fn pcm_identity(render: &PythonRender) -> String {
    // read_python_artifact computes this from complete PCM on the worker.
    // Publication must not scan a ten-minute waveform on the UI event loop.
    render.report["audio_digest"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn positive_measure(value: &Value) -> bool {
    value
        .as_f64()
        .is_some_and(|measure| measure.is_finite() && measure > 0.0)
}

fn valid_artifact(value: &str) -> bool {
    let mut components = Path::new(value).components();
    value.starts_with("render-")
        && value.len() <= 160
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        && matches!(components.next(), Some(Component::Normal(_)))
        && components.next().is_none()
}

fn validate_state(state: &LibraryState) -> io::Result<()> {
    let mut ids = BTreeSet::new();
    if state.version != 1 || state.entries.len() > 10_000 {
        return Err(invalid("Unsupported or oversized music library"));
    }
    for entry in &state.entries {
        if entry.id != format!("py-{}", entry.pcm_identity)
            || [
                entry.source_identity.as_str(),
                entry.pcm_identity.as_str(),
                entry.report_identity.as_str(),
            ]
            .iter()
            .any(|identity| {
                identity.len() != 16 || !identity.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
            || !valid_artifact(&entry.artifact)
            || !ids.insert(entry.id.as_str())
            || !entry.duration_seconds.is_finite()
            || !(30.0..=600.0).contains(&entry.duration_seconds)
            || !matches!(
                entry.eligibility.as_str(),
                "verified_complete" | "verified_v1_reference"
            )
        {
            return Err(invalid("Invalid music library entry"));
        }
    }
    if state
        .aliases
        .iter()
        .any(|(artifact, id)| !valid_artifact(artifact) || !ids.contains(id.as_str()))
        || state
            .preferences
            .keys()
            .any(|id| !ids.contains(id.as_str()))
    {
        return Err(invalid(
            "Music library contains an unknown alias or preference target",
        ));
    }
    Ok(())
}

fn read_limited(path: &Path, budget: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(budget + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > budget {
        return Err(invalid("Music library input exceeds its read budget"));
    }
    Ok(bytes)
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use teledra_music::VerifiedAudio;

    fn fixture(root: &Path, artifact: &str, code: &str, frequency: f64) -> PythonRender {
        let directory = root.join("music/python").join(artifact);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("code.py"), code).unwrap();
        let samples = (0..48_000 * 30)
            .flat_map(|frame| {
                let phase = f64::from(frame) * std::f64::consts::TAU * frequency / 48_000.0;
                [(phase.sin() * 0.08) as f32, (phase.sin() * 0.04) as f32]
            })
            .collect();
        VerifiedAudio::new(samples)
            .unwrap()
            .write_wav(&directory.join("audio.wav"))
            .unwrap();
        let report = json!({"ok":true,"title":code,"sample_rate":48000,"channels":2,
            "frames":48000*30,"duration_seconds":30.0,"loop":true,
            "notes_requested":100,"notes_rendered":100,"metadata":{"key":"A minor"},
            "composition_audit":{"version":1,"renderer":"render_song","engine":"pymusic","complete":true,
                "notes_scheduled":100,"notes_mixed":100,"active_layers":5,"duration_seconds":30.0,
                "layer_count":5,"section_count":4,"events":vec![json!({"mixed":true});100],
                "layer_stats":[{"events":20,"rms":0.01},{"events":20,"rms":0.01},
                    {"events":20,"rms":0.01},{"events":20,"rms":0.01},{"events":20,"rms":0.01}],
                "sections":[{"name":"A","rms":0.01},{"name":"B","rms":0.01},
                    {"name":"C","rms":0.01},{"name":"A2","rms":0.01}]}});
        fs::write(directory.join("report.json"), report.to_string()).unwrap();
        read_python_artifact(&directory, code.to_owned(), false).unwrap()
    }

    #[test]
    fn only_explicit_verified_publication_enters_catalog_and_reload_preserves_it() {
        let root = tempfile::tempdir().unwrap();
        let render = fixture(
            root.path(),
            "render-one",
            "raise RuntimeError('never execute cached audio')",
            220.0,
        );
        let mut library = MusicLibrary::load(root.path()).unwrap();
        assert!(
            library.state.entries.is_empty(),
            "report.ok directories are not imports"
        );
        let id = library.record_published(&render).unwrap();
        let reloaded = MusicLibrary::load(root.path()).unwrap();
        assert_eq!(reloaded.state.entries.len(), 1);
        assert_eq!(
            reloaded.id_for_artifact(&render.directory),
            Some(id.clone())
        );
        assert_eq!(
            reloaded.editor_state(Some(&id))["entries"][0]["verdict"],
            "unreviewed"
        );
        let cached = reloaded.load_playback(&id).unwrap();
        assert_eq!(cached.audio.samples(), render.audio.samples());
        assert!(cached.restore);
        assert!(
            !reloaded
                .editor_state(None)
                .to_string()
                .contains("audio.wav")
        );
    }

    #[test]
    fn source_or_pcm_duplicates_share_one_piece_and_retain_aliases() {
        let root = tempfile::tempdir().unwrap();
        let first = fixture(root.path(), "render-first", "same source", 220.0);
        let same_source = fixture(root.path(), "render-source", "same source", 330.0);
        let same_audio = fixture(root.path(), "render-pcm", "changed comments", 220.0);
        let mut library = MusicLibrary::load(root.path()).unwrap();
        let id = library.record_published(&first).unwrap();
        assert_eq!(library.record_published(&same_source).unwrap(), id);
        assert_eq!(library.record_published(&same_audio).unwrap(), id);
        let library = MusicLibrary::load(root.path()).unwrap();
        assert_eq!(library.state.entries.len(), 1);
        assert_eq!(library.id_for_artifact(&same_audio.directory), Some(id));
        assert_eq!(library.state.entries[0].artifact, "render-first");
    }

    #[test]
    fn listener_verdict_targets_exact_id_without_mutating_artifact_facts() {
        let root = tempfile::tempdir().unwrap();
        let first = fixture(root.path(), "render-first", "first", 220.0);
        let second = fixture(root.path(), "render-second", "second", 330.0);
        let mut library = MusicLibrary::load(root.path()).unwrap();
        let first_id = library.record_published(&first).unwrap();
        let second_id = library.record_published(&second).unwrap();
        let facts = serde_json::to_value(&library.state.entries).unwrap();
        library
            .set_verdict(&first_id, LibraryVerdict::Keep, "the answering line works")
            .unwrap();
        library
            .set_verdict(&second_id, LibraryVerdict::Reject, "too busy")
            .unwrap();
        assert!(
            library
                .set_verdict("../draft.py", LibraryVerdict::Keep, "")
                .is_err()
        );
        let library = MusicLibrary::load(root.path()).unwrap();
        assert_eq!(serde_json::to_value(&library.state.entries).unwrap(), facts);
        assert_eq!(
            library.state.preferences[&first_id].verdict,
            LibraryVerdict::Keep
        );
        assert_eq!(
            library.state.preferences[&second_id].verdict,
            LibraryVerdict::Reject
        );
    }

    #[test]
    fn playback_refuses_unknown_ids_and_changed_source_audio_or_report() {
        let root = tempfile::tempdir().unwrap();
        let render = fixture(root.path(), "render-one", "original", 220.0);
        let mut library = MusicLibrary::load(root.path()).unwrap();
        let id = library.record_published(&render).unwrap();
        for unknown in [
            "../render-one",
            "render-one",
            "C:\\audio.wav",
            "py-0000000000000000",
        ] {
            assert!(library.load_playback(unknown).is_err());
        }
        fs::write(render.directory.join("code.py"), "edited").unwrap();
        assert!(library.load_playback(&id).unwrap_err().contains("changed"));
        fs::write(render.directory.join("code.py"), "original").unwrap();
        let mut report: Value =
            serde_json::from_slice(&fs::read(render.directory.join("report.json")).unwrap())
                .unwrap();
        report["loop"] = false.into();
        fs::write(render.directory.join("report.json"), report.to_string()).unwrap();
        assert!(library.load_playback(&id).unwrap_err().contains("changed"));
        fixture(root.path(), "render-one", "original", 330.0);
        assert!(library.load_playback(&id).unwrap_err().contains("changed"));
    }

    #[test]
    fn incomplete_audits_and_mismatched_verified_reports_do_not_publish() {
        let root = tempfile::tempdir().unwrap();
        let mut render = fixture(root.path(), "render-one", "original", 220.0);
        let mut library = MusicLibrary::load(root.path()).unwrap();
        render.report["notes_rendered"] = 99.into();
        assert!(library.record_published(&render).is_err());
        let mut report = render.report.clone();
        report["duration_seconds"] = 4.0.into();
        assert!(MusicLibrary::eligibility(&report).is_err());
        report["duration_seconds"] = 30.0.into();
        report["composition_audit"]["notes_mixed"] = 99.into();
        assert!(MusicLibrary::eligibility(&report).is_err());
        report["composition_audit"]["notes_mixed"] = 100.into();
        report["composition_audit"]["layer_stats"][0]["rms"] = 0.0.into();
        assert!(MusicLibrary::eligibility(&report).is_err());
        assert!(library.record_verified_v1_restore(&render).is_err());
        assert!(library.state.entries.is_empty());
    }

    #[test]
    fn unrelated_roots_and_corrupted_or_traversing_indexes_are_preserved_and_rejected() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let render = fixture(outside.path(), "render-outside", "outside", 220.0);
        let mut library = MusicLibrary::load(root.path()).unwrap();
        assert!(library.record_published(&render).is_err());
        let render = fixture(root.path(), "render-local", "local", 220.0);
        library.record_published(&render).unwrap();
        let path = root.path().join("music/python/library.json");
        let mut stored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        stored["entries"][0]["artifact"] = "../render-outside".into();
        fs::write(&path, stored.to_string()).unwrap();
        assert!(MusicLibrary::load(root.path()).is_err());
        let broken = b"{broken library, retain for recovery";
        fs::write(&path, broken).unwrap();
        assert!(MusicLibrary::load(root.path()).is_err());
        assert_eq!(fs::read(path).unwrap(), broken);
    }

    #[test]
    fn rotation_pick_offers_a_different_unrejected_title() {
        let root = tempfile::tempdir().unwrap();
        let entry = |number: u64, title: &str| LibraryEntry {
            id: format!("py-{number:016x}"),
            artifact: format!("render-{number}"),
            source_identity: format!("{number:016x}"),
            pcm_identity: format!("{number:016x}"),
            report_identity: format!("{number:016x}"),
            title: title.into(),
            duration_seconds: 88.0,
            note_count: 100,
            metadata: Value::Null,
            added_ms: number,
            eligibility: "verified_complete".into(),
        };
        let mut library = MusicLibrary::load(root.path()).unwrap();
        let mut next = library.state.clone();
        next.entries.push(entry(1, "Stuck piece"));
        next.entries.push(entry(2, "Stuck piece"));
        next.entries.push(entry(3, "Old piece"));
        next.entries.push(entry(4, "Old piece"));
        next.entries.push(entry(5, "Disliked piece"));
        library.commit(next).unwrap();
        library
            .set_verdict("py-0000000000000005", LibraryVerdict::Reject, "")
            .unwrap();
        for _ in 0..8 {
            let (id, title) = library
                .rotation_pick(|title| title == "Stuck piece")
                .unwrap();
            // The newest version of the only remaining title.
            assert_eq!(
                (id.as_str(), title.as_str()),
                ("py-0000000000000004", "Old piece")
            );
        }
        assert!(library.rotation_pick(|_| true).is_none());
    }

    #[test]
    fn duplicate_title_versions_are_stable_across_new_entries_and_reload_without_renaming_artifacts()
     {
        let root = tempfile::tempdir().unwrap();
        let entry = |number: u64, title: &str, added_ms: u64| LibraryEntry {
            id: format!("py-{number:016x}"),
            artifact: format!("render-{number}"),
            source_identity: format!("{number:016x}"),
            pcm_identity: format!("{number:016x}"),
            report_identity: format!("{number:016x}"),
            title: title.into(),
            duration_seconds: 88.0,
            note_count: 100,
            metadata: Value::Null,
            added_ms,
            eligibility: "verified_complete".into(),
        };
        let mut library = MusicLibrary::load(root.path()).unwrap();
        let mut next = library.state.clone();
        next.entries.push(entry(1, "Evolving Harmony", 1_000));
        library.commit(next).unwrap();
        assert_eq!(
            library.editor_state(None)["entries"][0]["display_title"],
            "Evolving Harmony"
        );
        let mut next = library.state.clone();
        next.entries.push(entry(2, "Another piece", 2_000));
        next.entries.push(entry(3, "Evolving Harmony", 3_000));
        let immutable = serde_json::to_value(&next.entries).unwrap();
        library.commit(next).unwrap();
        let mut library = MusicLibrary::load(root.path()).unwrap();
        let displayed = library.editor_state(None);
        assert_eq!(
            displayed["entries"][0]["display_title"],
            "Evolving Harmony · v2"
        );
        assert_eq!(displayed["entries"][1]["display_title"], "Another piece");
        assert_eq!(
            displayed["entries"][2]["display_title"],
            "Evolving Harmony · v1"
        );
        assert_eq!(displayed["entries"][2]["added_ms"], 1_000);
        assert_eq!(
            serde_json::to_value(&library.state.entries).unwrap(),
            immutable
        );
        let mut next = library.state.clone();
        // Even a clock correction cannot reorder immutable insertion history.
        next.entries.push(entry(4, "Evolving Harmony", 500));
        library.commit(next).unwrap();
        let displayed = MusicLibrary::load(root.path()).unwrap().editor_state(None);
        assert_eq!(
            displayed["entries"][0]["display_title"],
            "Evolving Harmony · v3"
        );
        assert_eq!(
            displayed["entries"][1]["display_title"],
            "Evolving Harmony · v2"
        );
        assert_eq!(
            displayed["entries"][3]["display_title"],
            "Evolving Harmony · v1"
        );
        assert_eq!(displayed["entries"][3]["title"], "Evolving Harmony");
        assert_eq!(displayed["entries"][3]["id"], "py-0000000000000001");
    }
}
