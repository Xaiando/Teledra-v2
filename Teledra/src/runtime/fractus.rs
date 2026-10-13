use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc;

use super::ToolReply;

#[derive(Clone, Debug)]
pub(super) struct FractusRequest {
    pub(super) ticket: u64,
    pub(super) dsl: String,
    pub(super) live: bool,
}

/// One CPU render and one replaceable request keep the gallery responsive
/// without allowing old work to replace the operator's newest choice.
#[derive(Default)]
pub(super) struct FractusRenderQueue {
    latest: u64,
    active: Option<FractusRequest>,
    pending: Option<FractusRequest>,
}

impl FractusRenderQueue {
    pub(super) fn request(&mut self, dsl: String, live: bool) -> Option<FractusRequest> {
        self.latest = self.latest.wrapping_add(1);
        // Every gallery path, including queued native variations, obeys the
        // operator's one-family preference. Invalid payloads still reach the
        // renderer's normal error reporting rather than becoming defaults.
        let dsl = teledra_fractus::single_family_dsl(&dsl).unwrap_or(dsl);
        let request = FractusRequest {
            ticket: self.latest,
            dsl,
            live,
        };
        if self.active.is_some() {
            self.pending = Some(request);
            None
        } else {
            self.active = Some(request.clone());
            Some(request)
        }
    }

    pub(super) fn is_rendering(&self) -> bool {
        self.active.is_some()
    }

    pub(super) fn active_request(&self, ticket: u64) -> Option<&FractusRequest> {
        self.active
            .as_ref()
            .filter(|request| request.ticket == ticket)
    }

    /// Success and failure both release the worker. Duplicate completions do
    /// not release a newer worker or publish an obsolete image.
    pub(super) fn complete(&mut self, ticket: u64) -> (bool, Option<FractusRequest>) {
        if self.active_request(ticket).is_none() {
            return (false, None);
        }
        let publish = ticket == self.latest;
        let next = self.pending.take();
        self.active = next.clone();
        (publish, next)
    }
}

/// Where the current composition came from.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum FractusOrigin {
    /// A `FRACTUS_*` tag a court member wrote.
    Tag,
    /// A `/Art` or `/Fractus` command.
    Operator,
    /// Recovered from family names in the Artist's untagged prose.
    Prose,
    /// Saved before origins were recorded, so it cannot be told apart from the
    /// prose guesses made while the Artist's one-line tag was being rejected.
    #[default]
    Unrecorded,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub(super) struct FractusLineage {
    pub(super) dsl: String,
    pub(super) live: bool,
    #[serde(default)]
    pub(super) origin: FractusOrigin,
}

impl FractusLineage {
    /// Only a composition someone asked for is restored at launch.
    ///
    /// A prose guess is fine for the session that made it, but a saved one
    /// outlived the fix to the guessing: a Lissajous knot the Artist had only
    /// mentioned, drawn over the mandala it asked for, came back and kept
    /// evolving on every launch after that.
    pub(super) fn load(root: &Path) -> Option<Self> {
        let bytes = fs::read(root.join("fractus").join(".fractus-lineage.json")).ok()?;
        let mut lineage: Self = serde_json::from_slice(&bytes).ok()?;
        lineage.dsl = teledra_fractus::single_family_dsl(&lineage.dsl).ok()?;
        matches!(lineage.origin, FractusOrigin::Tag | FractusOrigin::Operator).then_some(lineage)
    }

    pub(super) fn save(&self, root: &Path) -> io::Result<()> {
        let mut lineage = self.clone();
        lineage.dsl = teledra_fractus::single_family_dsl(&lineage.dsl).map_err(io::Error::other)?;
        let directory = root.join("fractus");
        fs::create_dir_all(&directory)?;
        let temporary = directory.join(".fractus-lineage.tmp");
        fs::write(
            &temporary,
            serde_json::to_vec(&lineage).map_err(io::Error::other)?,
        )?;
        fs::rename(temporary, directory.join(".fractus-lineage.json"))
    }

    pub(super) fn evolved_dsl(&self, _generation: u64) -> String {
        // The artwork already animates internally. Restoration preserves its
        // accepted geometry; creative revisions come from the Artist using
        // recorded outcomes, not a repeating zoom/motion cycle.
        teledra_fractus::single_family_dsl(&self.dsl).unwrap_or_else(|_| self.dsl.clone())
    }
}

pub(super) fn curated_fractus_scene(generation: u64) -> String {
    const STUDIES: [(&str, &str, u64, u32); 8] = [
        ("lotus_mandala", "solar_gold", 731, 12),
        ("julia", "electric_cyan", 2741, 12),
        ("kaleidoscope", "purple_haze", 4093, 10),
        ("mandelbrot", "neon_sunset", 811, 12),
        ("star_mandala", "ice_fire", 1571, 16),
        ("flower_of_life", "emerald", 3187, 6),
        ("newton", "twilight", 6029, 12),
        ("spirograph", "royal", 4783, 18),
    ];
    let (family, palette, seed, symmetry) = STUDIES[(generation as usize / 4) % STUDIES.len()];
    let step = [0.0, 1.0, 2.0, 1.0][generation as usize % 4];
    format!(
        "version 2\ntype={family}\nwidth=600\nheight=800\niterations=180\nseed={seed}\npalette={palette}\nsymmetry={symmetry}\nzoom={:.3}\nmotion={:.3}",
        1.0 + step * 0.025,
        0.6 + step * 0.025
    )
}

pub(super) fn usable_research_excerpt(title: &str, excerpt: &str) -> bool {
    let text = format!("{title} {excerpt}").to_ascii_lowercase();
    excerpt.chars().count() >= 120
        && ![
            "access denied",
            "captcha",
            "bot protection",
            "verify you are human",
            "just a moment",
            "404 not found",
            "automated requests",
            "checking your browser",
        ]
        .iter()
        .any(|blocked| text.contains(blocked))
}

pub(super) struct FractusStudio {
    python: PathBuf,
    script: PathBuf,
    root: PathBuf,
    command_path: PathBuf,
    status_path: PathBuf,
    generation: u64,
    child: Option<Child>,
}

impl FractusStudio {
    pub(super) fn new(python: PathBuf, data_root: &Path) -> Self {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let workspace = manifest.parent().unwrap_or_else(|| Path::new("."));
        let root = data_root.join("fractus");
        Self {
            python,
            script: workspace.join("Fractus").join("studio.py"),
            command_path: root.join(".fractus-studio-command.json"),
            status_path: root.join(".fractus-studio-status.json"),
            root,
            generation: 0,
            child: None,
        }
    }

    pub(super) fn show(
        &mut self,
        path: &Path,
        live: bool,
        tools: &mpsc::UnboundedSender<ToolReply>,
    ) -> io::Result<()> {
        let root = canonical_directory(&self.root)?;
        let path = path.canonicalize()?;
        if root != path && !path.starts_with(&root) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Fractus artifact is outside the managed gallery",
            ));
        }
        match path.extension().and_then(|extension| extension.to_str()) {
            Some(extension) if extension.eq_ignore_ascii_case("png") => {}
            Some(extension) if extension.eq_ignore_ascii_case("gif") => {}
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Fractus Studio accepts PNG and GIF artifacts only",
                ));
            }
        }
        self.ensure_running()?;
        self.generation = next_command_id(self.generation);
        let presentation_path = presentation_path(&path);
        let command = serde_json::json!({
            "action": "show",
            "command_id": self.generation,
            "path": presentation_path,
            "live": live,
        });
        write_mailbox(&self.command_path, &command)?;
        await_status(self.status_path.clone(), self.generation, tools.clone());
        Ok(())
    }

    fn ensure_running(&mut self) -> io::Result<()> {
        let stopped = self
            .child
            .as_mut()
            .is_some_and(|child| child.try_wait().ok().flatten().is_some());
        if stopped {
            self.stop();
        }
        if self.child.is_some() {
            return Ok(());
        }
        if !self.script.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("Fractus Studio is missing: {}", self.script.display()),
            ));
        }
        fs::create_dir_all(&self.root)?;
        let _ = fs::remove_file(&self.command_path);
        let _ = fs::remove_file(&self.status_path);
        let mut command = Command::new(&self.python);
        command
            .arg(&self.script)
            .arg("--root")
            .arg(&self.root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        hide_console(&mut command);
        self.child = Some(command.spawn()?);
        Ok(())
    }

    fn stop(&mut self) {
        if self.child.is_some() {
            self.generation = next_command_id(self.generation);
            let command = serde_json::json!({
                "action": "shutdown",
                "command_id": self.generation,
            });
            let _ = write_mailbox(&self.command_path, &command);
        }
        if let Some(mut child) = self.child.take() {
            if !wait_for_exit(&mut child) {
                kill_process_tree(&mut child);
            }
            let _ = child.wait();
        }
    }
}

fn write_mailbox(path: &Path, command: &serde_json::Value) -> io::Result<()> {
    let encoded = serde_json::to_vec(command).map_err(io::Error::other)?;
    fs::write(path, encoded)
}

fn presentation_path(path: &Path) -> String {
    let displayed = path.to_string_lossy();
    #[cfg(windows)]
    if let Some(normal) = displayed.strip_prefix(r"\\?\") {
        return normal.to_owned();
    }
    displayed.into_owned()
}

fn next_command_id(previous: u64) -> u64 {
    let clock = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros()
        .min(u128::from(u64::MAX)) as u64;
    clock.max(previous.saturating_add(1))
}

fn await_status(status_path: PathBuf, command_id: u64, tools: mpsc::UnboundedSender<ToolReply>) {
    std::thread::spawn(move || {
        for _ in 0..240 {
            std::thread::sleep(Duration::from_millis(50));
            let Ok(raw) = fs::read_to_string(&status_path) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
                continue;
            };
            if value.get("command_id").and_then(serde_json::Value::as_u64) != Some(command_id) {
                continue;
            }
            let event = value
                .get("event")
                .and_then(|value| value.as_str())
                .unwrap_or("error");
            let result = if event == "loaded" {
                let path = value
                    .get("path")
                    .and_then(|value| value.as_str())
                    .unwrap_or("Fractus artifact");
                let frames = value
                    .get("frames")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                Ok(format!("Displayed {path} with {frames} frame(s)"))
            } else {
                Err(value
                    .get("message")
                    .and_then(|value| value.as_str())
                    .unwrap_or("unknown presentation error")
                    .to_owned())
            };
            let _ = tools.send(ToolReply {
                source: "Fractus Studio",
                result,
                court_prompt: None,
                effect: None,
            });
            return;
        }
        let _ = tools.send(ToolReply {
            source: "Fractus Studio",
            result: Err("Studio did not acknowledge the rendered art within 12 seconds".to_owned()),
            court_prompt: None,
            effect: None,
        });
    });
}

impl Drop for FractusStudio {
    fn drop(&mut self) {
        self.stop();
    }
}

fn canonical_directory(path: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(path)?;
    path.canonicalize()
}

fn wait_for_exit(child: &mut Child) -> bool {
    for _ in 0..20 {
        if child.try_wait().ok().flatten().is_some() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    false
}

#[cfg(windows)]
fn kill_process_tree(child: &mut Child) {
    let mut command = Command::new("taskkill");
    command
        .args(["/PID", &child.id().to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_console(&mut command);
    let _ = command.status();
}

#[cfg(not(windows))]
fn kill_process_tree(child: &mut Child) {
    let _ = child.kill();
}

#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app() -> crate::app::App {
        crate::app::App::new(
            teledra_court::Court::default(),
            teledra_brain::BrainHealth {
                endpoint: "http://localhost:11434".to_owned(),
                model: teledra_brain::QWEN_MODEL,
            },
        )
    }

    #[test]
    fn rapid_art_requests_render_once_and_publish_only_the_newest() {
        let mut queue = FractusRenderQueue::default();
        let first = queue.request("first".to_owned(), true).unwrap();
        assert!(
            queue
                .request("discarded middle".to_owned(), false)
                .is_none()
        );
        assert!(queue.request("newest".to_owned(), true).is_none());
        let (publish, next) = queue.complete(first.ticket);
        assert!(!publish);
        let next = next.unwrap();
        assert_eq!(next.dsl, "newest");
        assert!(queue.is_rendering());
        let (publish, duplicate) = queue.complete(first.ticket);
        assert!(!publish && duplicate.is_none());
        assert!(
            queue.is_rendering(),
            "stale completion must not release the newer worker"
        );
        let (publish, pending) = queue.complete(next.ticket);
        assert!(publish && pending.is_none());
        assert!(!queue.is_rendering());
        assert!(
            queue
                .request("after failure or success".to_owned(), false)
                .is_some()
        );
    }

    #[test]
    fn automatic_studies_satisfy_the_richness_floor_without_overlays() {
        for generation in (0..32).step_by(4) {
            let scene = teledra_fractus::Scene::parse(&curated_fractus_scene(generation)).unwrap();
            assert!(scene.layers.is_empty());
            scene
                .require_visual_richness()
                .unwrap_or_else(|error| panic!("generation {generation}: {error}"));
        }
    }

    #[test]
    fn saved_artist_lineage_preserves_subject_without_reviving_overlays() {
        let directory = tempfile::tempdir().unwrap();
        let lineage = FractusLineage {
            dsl: "version 2\ntype=star_mandala\nseed=1999\npalette=ice_fire\nsymmetry=18\nzoom=2\nmotion=0.4\nlayers=rose_curve@0.14@screen@royal".to_owned(),
            live: true,
            origin: FractusOrigin::Tag,
        };
        FractusLineage {
            dsl: "type=lotus_mandala\nseed=1".to_owned(),
            live: false,
            origin: FractusOrigin::Operator,
        }
        .save(directory.path())
        .unwrap();
        lineage.save(directory.path()).unwrap();
        let saved = FractusLineage::load(directory.path()).unwrap();
        assert_eq!(
            saved.dsl,
            teledra_fractus::single_family_dsl(&lineage.dsl).unwrap()
        );
        assert!(saved.live);
        assert_eq!(saved.origin, FractusOrigin::Tag);
        assert!(
            !directory
                .path()
                .join("fractus/.fractus-lineage.tmp")
                .exists()
        );
        for generation in 0..40 {
            let scene = teledra_fractus::Scene::parse(&saved.evolved_dsl(generation)).unwrap();
            assert!(matches!(
                scene.kind,
                teledra_fractus::SceneKind::StarMandala
            ));
            assert_eq!(
                (scene.seed, scene.symmetry, scene.layers.len()),
                (1999, 18, 0)
            );
            assert_eq!(scene.palette, "ice_fire");
            assert_eq!(scene.zoom, 2.0);
            assert_eq!(scene.motion, 0.4);
        }
    }

    #[test]
    fn loading_the_current_tagged_overlay_is_clean_and_does_not_rewrite_the_original() {
        let directory = tempfile::tempdir().unwrap();
        let gallery = directory.path().join("fractus");
        fs::create_dir_all(&gallery).unwrap();
        let original = r#"{"dsl":"version 2\ntype=kaleidoscope\nwidth=600\nheight=800\niterations=180\nseed=7843791494995425550\npalette=violet\nsymmetry=12\nzoom=1\nmotion=0.65\nlayers=rose_curve@0.22@screen@electric_cyan","live":true,"origin":"tag"}"#;
        let path = gallery.join(".fractus-lineage.json");
        fs::write(&path, original).unwrap();
        let saved = FractusLineage::load(directory.path()).unwrap();
        let scene = teledra_fractus::Scene::parse(&saved.dsl).unwrap();
        assert!(scene.layers.is_empty());
        assert!(matches!(
            scene.kind,
            teledra_fractus::SceneKind::Kaleidoscope
        ));
        assert_eq!(scene.seed, 7_843_791_494_995_425_550);
        assert_eq!(scene.palette, "violet");
        assert_eq!(fs::read_to_string(path).unwrap(), original);
    }

    #[test]
    fn immediate_and_queued_gallery_requests_both_remove_unrelated_families() {
        let mut queue = FractusRenderQueue::default();
        let first = queue
            .request(
                "type=lissajous; layers=mandala@0.2;fractal_tree@0.15".to_owned(),
                true,
            )
            .unwrap();
        assert!(
            teledra_fractus::Scene::parse(&first.dsl)
                .unwrap()
                .layers
                .is_empty()
        );
        assert!(
            queue
                .request("type=julia; layers=rose_curve@0.22".to_owned(), true)
                .is_none()
        );
        let (_, next) = queue.complete(first.ticket);
        let next = next.unwrap();
        let scene = teledra_fractus::Scene::parse(&next.dsl).unwrap();
        assert!(scene.layers.is_empty());
        assert!(matches!(scene.kind, teledra_fractus::SceneKind::Julia));
    }

    #[test]
    fn a_gallery_action_stores_and_renders_the_clean_subject_through_evolution() {
        use super::super::{RuntimeServices, artifacts::render_fractus_at};

        let directory = tempfile::tempdir().unwrap();
        let mut services =
            RuntimeServices::new(directory.path().to_owned(), "missing-python".into()).unwrap();
        let mut app = test_app();
        services.fractus.script = directory.path().join("absent-studio.py");
        let (tx, _rx) = mpsc::unbounded_channel();
        // Hold the worker to inspect the actual selected request without
        // starting a Studio window, process or model request.
        let busy = services
            .fractus_renders
            .request("busy".to_owned(), true)
            .unwrap();
        services.handle_court_action(
            teledra_court::CourtAction::Fractus {
                payload: "version 2; type=lissajous; width=96; height=128; seed=18365444092935997690; palette=amethyst; symmetry=12; zoom=1.025; motion=0.675; layers=mandala@0.22@screen@electric_cyan;fractal_tree@0.15@screen@amethyst".to_owned(),
                live: true,
            },
            &mut app,
            &tx,
        );
        assert!(
            services.fractus_lineage.is_none(),
            "unrendered candidate cannot be the accepted baseline"
        );
        let selected = services.fractus_candidate_lineage.as_ref().unwrap();
        assert_eq!(selected.origin, FractusOrigin::Tag);
        assert!(!selected.dsl.contains("layers="));
        let selected_dsl = selected.dsl.clone();
        let (_, request) = services.fractus_renders.complete(busy.ticket);
        let request = request.unwrap();
        assert_eq!(request.dsl, selected_dsl);
        assert!(request.live);

        let artifact = render_fractus_at(directory.path(), &request.dsl, true).unwrap();
        let manifest: teledra_fractus::RenderManifest = serde_json::from_slice(
            &fs::read(artifact.path.with_extension("manifest.json")).unwrap(),
        )
        .unwrap();
        assert!(manifest.verified && artifact.path.is_file());
        assert!(manifest.scene.layers.is_empty());
        assert!(matches!(
            manifest.scene.kind,
            teledra_fractus::SceneKind::Lissajous
        ));
        assert_eq!(manifest.scene.seed, 18_365_444_092_935_997_690);
        assert_eq!(manifest.scene.palette, "amethyst");
        assert!(manifest.animation.unwrap().distinct_frames > 1);
        services.apply_tool_effect(
            &mut app,
            super::super::ToolEffect::FractusReady {
                ticket: request.ticket,
                path: artifact.path,
                live: true,
            },
            &tx,
        );
        assert!(services.fractus_candidate_lineage.is_none());
        assert_eq!(services.fractus_lineage.as_ref().unwrap().dsl, selected_dsl);
        assert_eq!(
            FractusLineage::load(directory.path()).unwrap().dsl,
            selected_dsl
        );
        assert!(services.art_learning_context().contains("outcome=Accepted"));

        // Routine evolution queues a reasoned revision with this measured
        // baseline. It does not launch another identical native render.
        services.next_fractus_evolution = Some(std::time::Instant::now() - Duration::from_secs(1));
        services.evolve_fractus_if_due(&mut app, &tx);
        assert!(!services.fractus_renders.is_rendering());
        let mut next = app.begin_queued_external_turn().unwrap();
        assert!(next.user.starts_with("ARTIST EVOLUTION:"));
        services.enrich_request(&mut app, &mut next);
        assert!(
            next.messages
                .iter()
                .any(|message| message.content.contains("outcome=Accepted")
                    && message.content.contains("seed=18365444092935997690"))
        );
    }

    #[test]
    fn stale_success_and_current_failure_cannot_promote_an_unaccepted_lineage() {
        let directory = tempfile::tempdir().unwrap();
        let baseline = FractusLineage {
            dsl: "type=phyllotaxis\nseed=927".to_owned(),
            live: true,
            origin: FractusOrigin::Tag,
        };
        baseline.save(directory.path()).unwrap();
        let mut services = super::super::RuntimeServices::new(
            directory.path().to_owned(),
            "missing-python".into(),
        )
        .unwrap();
        let mut app = test_app();
        let (tx, _rx) = mpsc::unbounded_channel();
        let old = services
            .fractus_renders
            .request("type=julia; seed=1".to_owned(), true)
            .unwrap();
        let candidate = FractusLineage {
            dsl: "type=mandala\nseed=2".to_owned(),
            live: true,
            origin: FractusOrigin::Tag,
        };
        services.fractus_candidate_lineage = Some(candidate.clone());
        services.fractus_renders.request(candidate.dsl, true);
        let (_, current) = services.fractus_renders.complete(old.ticket);
        let current = current.unwrap();
        services.apply_tool_effect(
            &mut app,
            super::super::ToolEffect::FractusReady {
                ticket: old.ticket,
                path: directory.path().join("old.png"),
                live: true,
            },
            &tx,
        );
        assert_eq!(services.fractus_lineage.as_ref().unwrap().dsl, baseline.dsl);
        assert!(services.fractus_candidate_lineage.is_some());
        services.apply_tool_effect(
            &mut app,
            super::super::ToolEffect::FractusFailed {
                ticket: current.ticket,
                error: "test renderer exceeded its budget".to_owned(),
            },
            &tx,
        );
        assert_eq!(services.fractus_lineage.as_ref().unwrap().dsl, baseline.dsl);
        assert_eq!(
            FractusLineage::load(directory.path()).unwrap().dsl,
            baseline.dsl
        );
        assert!(services.fractus_candidate_lineage.is_none());
        assert!(
            services
                .art_learning_context()
                .contains("test renderer exceeded its budget")
        );
        assert!(services.next_fractus_evolution.is_some());
    }

    #[test]
    fn stale_research_cannot_clear_a_new_research_request() {
        let directory = tempfile::tempdir().unwrap();
        let mut services = super::super::RuntimeServices::new(
            directory.path().to_owned(),
            "missing-python".into(),
        )
        .unwrap();
        let mut app = test_app();
        let (tx, _rx) = mpsc::unbounded_channel();
        services.fractus_research_ticket = 8;
        services.fractus_researching = true;
        services.next_fractus_exploration =
            Some(std::time::Instant::now() + Duration::from_secs(180));
        services.apply_tool_effect(
            &mut app,
            super::super::ToolEffect::FractusResearch {
                ticket: 7,
                direction: "superseded source".to_owned(),
                brief: None,
            },
            &tx,
        );
        assert!(services.fractus_researching);
        assert!(services.fractus_exploration_ready.is_none());
        services.apply_tool_effect(
            &mut app,
            super::super::ToolEffect::FractusResearch {
                ticket: 8,
                direction: "current source".to_owned(),
                brief: None,
            },
            &tx,
        );
        assert!(!services.fractus_researching);
        assert!(
            services
                .fractus_exploration_ready
                .as_ref()
                .unwrap()
                .contains("current source")
        );
    }

    /// The lineage file verbatim from the live data root: the prose guess that
    /// drew a Lissajous knot over the mandala, saved before origins existed.
    #[test]
    fn a_prose_guess_or_an_unrecorded_composition_is_not_restored_at_launch() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(directory.path().join("fractus")).unwrap();
        std::fs::write(
            directory.path().join("fractus/.fractus-lineage.json"),
            r#"{"dsl":"version 2\ntype=lissajous\nwidth=600\nheight=800\niterations=180\nseed=18365444092935997690\npalette=amethyst\nsymmetry=12\nzoom=1\nmotion=0.65\nlayers=mandala@0.22@screen@electric_cyan;fractal_tree@0.15@screen@amethyst","live":true}"#,
        )
        .unwrap();
        assert!(FractusLineage::load(directory.path()).is_none());

        let guess = FractusLineage {
            dsl: "version 2\ntype=mandala\nseed=5".to_owned(),
            live: true,
            origin: FractusOrigin::Prose,
        };
        guess.save(directory.path()).unwrap();
        assert!(FractusLineage::load(directory.path()).is_none());

        FractusLineage {
            origin: FractusOrigin::Tag,
            ..guess
        }
        .save(directory.path())
        .unwrap();
        assert!(FractusLineage::load(directory.path()).is_some());
    }

    #[test]
    fn native_variation_preserves_explicitly_frozen_geometry() {
        let lineage = FractusLineage {
            dsl: "type=lotus_mandala\nseed=731\nmotion=0".to_owned(),
            live: true,
            origin: FractusOrigin::Tag,
        };
        for generation in 0..8 {
            let scene = teledra_fractus::Scene::parse(&lineage.evolved_dsl(generation)).unwrap();
            assert_eq!(scene.motion, 0.0);
        }
    }

    #[test]
    fn curated_gallery_starts_with_mandala_and_fractal_studies_without_overlays() {
        for generation in 0..32 {
            let scene = teledra_fractus::Scene::parse(&curated_fractus_scene(generation)).unwrap();
            assert!(scene.layers.is_empty());
            assert_eq!((scene.width, scene.height), (600, 800));
            let next =
                teledra_fractus::Scene::parse(&curated_fractus_scene(generation / 4 * 4)).unwrap();
            assert_eq!(scene.seed, next.seed);
            assert_eq!(scene.palette, next.palette);
        }
    }

    #[test]
    fn art_research_rejects_empty_stubs_and_bot_challenges() {
        assert!(!usable_research_excerpt(
            "Superformula",
            "Under construction"
        ));
        assert!(!usable_research_excerpt(
            "Just a moment",
            &"Please verify you are human. ".repeat(10)
        ));
        assert!(usable_research_excerpt(
            "Hypotrochoid",
            &"The curve describes a point attached to a circle rolling within another circle. "
                .repeat(3)
        ));
    }
}
