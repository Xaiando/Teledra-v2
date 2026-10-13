use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::app::App;
use teledra_fractus::Scene;
use teledra_music::FeedbackVerdict;

use super::RuntimeServices;

static ARTIFACT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

const FRACTUS_GALLERY_KEEP: usize = 64;
const FRACTUS_LIVE_FRAMES: u16 = 120;
const FRACTUS_LIVE_FPS: u16 = 24;
const FRACTUS_LIVE_MAX_EDGE: u32 = 800;
const FRACTUS_LIVE_MAX_PIXELS: u32 = 480_000;
const IN_PROGRESS_ARTIFACT_GRACE_MS: u128 = 10 * 60 * 1_000;

/// Names the renderer accepts. The matcher resolves overlapping names
/// longest-first, so `lotus_mandala` is not reduced to `mandala`. The Artist
/// reliably names these in prose even when it omits the action tag entirely.
const FRACTUS_FAMILIES: [&str; 36] = [
    "cellular_automata",
    "reaction_diffusion",
    "strange_attractor",
    "koch_snowflake",
    "flower_of_life",
    "lotus_mandala",
    "star_mandala",
    "barnsley_fern",
    "sierpinski",
    "fractal_tree",
    "dragon_curve",
    "radial_weave",
    "kaleidoscope",
    "phyllotaxis",
    "woven_web",
    "orbital_lace",
    "harmonograph",
    "rose_curve",
    "string_art",
    "flow_field",
    "mandelbrot",
    "burning_ship",
    "multibrot",
    "spirograph",
    "lissajous",
    "guilloche",
    "hex_weave",
    "op_art",
    "particles",
    "mandala",
    "tricorn",
    "newton",
    "moire",
    "truchet",
    "l_system",
    "julia",
];

#[derive(Debug)]
pub(super) struct RenderedFractus {
    pub(super) path: PathBuf,
    pub(super) retention_notice: Option<String>,
}

#[derive(Default)]
struct GalleryRetention {
    removed_artifacts: usize,
    removed_orphan_images: usize,
    removed_orphans: usize,
    warnings: Vec<String>,
}

#[derive(Debug)]
pub(super) struct ProposalDecision {
    pub(super) id: String,
    pub(super) kind: String,
    pub(super) content: String,
}

/// What happened when an outward action asked to be queued for approval.
#[derive(Clone, Debug)]
pub(super) enum Proposal {
    Created(PathBuf),
    /// An identical request was already waiting, so nothing was written.
    AlreadyPending(PathBuf),
}

impl Proposal {
    pub(super) fn path(&self) -> &Path {
        match self {
            Self::Created(path) | Self::AlreadyPending(path) => path,
        }
    }

    pub(super) fn is_new(&self) -> bool {
        matches!(self, Self::Created(_))
    }
}

impl RuntimeServices {
    /// Salvages the concrete scene the Artist described when it narrates
    /// instead of emitting a `FRACTUS_*` action.
    ///
    /// This is deliberately role-gated by the caller. In ordinary court speech
    /// a word such as "mandala" is conversation; in an Artist turn it is the
    /// same unambiguous intent that the model was asked to put in a tag. The
    /// first named family that is more than an accent becomes the manifest
    /// subject and later families are retained as bounded layers.
    pub(crate) fn salvage_fractus_from_prose(text: &str) -> Option<String> {
        let mut families = intended_terms(text, &FRACTUS_FAMILIES);
        // Stable, so the named order holds within subjects and within accents.
        families.sort_by_key(|family| described_only_as_accent(text, family));
        let primary = families.first()?;
        let palettes = intended_terms(text, &teledra_fractus::PALETTE_NAMES);
        let palette = palettes.first().copied().unwrap_or("purple_haze");
        let seed = prose_seed(text);
        let mut dsl = format!(
            "version 2\ntype={primary}\nwidth=600\nheight=800\niterations=180\nseed={seed}\npalette={palette}\nsymmetry=12\nzoom=1\nmotion=0.65"
        );

        let layers = families
            .iter()
            .skip(1)
            .take(2)
            .enumerate()
            .map(|(index, family)| {
                let layer_palette = palettes.get(index + 1).copied().unwrap_or(palette);
                let alpha = (0.22_f32 - index as f32 * 0.07).max(0.14);
                format!("{family}@{alpha:.2}@screen@{layer_palette}")
            })
            .collect::<Vec<_>>();
        if !layers.is_empty() {
            dsl.push_str("\nlayers=");
            dsl.push_str(&layers.join(";"));
        }

        Scene::parse(&dsl).ok().map(|_| dsl)
    }

    pub(crate) fn artist_requested_still(prompt: &str) -> bool {
        let normalized = prompt.to_ascii_lowercase();
        [
            "still image",
            "still artwork",
            "static image",
            "static artwork",
        ]
        .iter()
        .any(|needle| normalized.contains(needle))
    }

    pub(super) fn archive_approved_creative(
        &self,
        kind: &str,
        content: &str,
    ) -> Result<PathBuf, std::io::Error> {
        let kind = kind
            .chars()
            .filter(|character| character.is_ascii_alphanumeric() || *character == '_')
            .take(40)
            .collect::<String>();
        let directory = self.root().join("creative").join("approved");
        std::fs::create_dir_all(&directory)?;
        let extension = match kind.as_str() {
            "python_music" => "py",
            "music_patch" => "json",
            _ => "strudel",
        };
        let path = directory.join(format!(
            "{}-{}-{}.{}",
            kind,
            epoch_millis(),
            ARTIFACT_SEQUENCE.fetch_add(1, Ordering::Relaxed),
            extension
        ));
        std::fs::write(&path, content)?;
        Ok(path)
    }

    /// Turns whatever the Artist wrote into a scene the renderer can read.
    ///
    /// The fallback used to be `palette={payload}`, which accepted anything:
    /// a layer stack, a command line, even a paragraph of prose all became the
    /// palette, parsed without complaint, and rendered the default scene. Every
    /// artwork looked identical because the request was being thrown away in a
    /// field nobody validated. Each shape is now recognised for what it is, and
    /// what cannot be recognised is refused so the correction loop can ask for
    /// it again.
    pub(super) fn normalize_fractus_payload(payload: &str) -> String {
        let payload = payload.trim().trim_matches('`').trim();
        // The Artist writes `version 2; type=julia; ...` on one line. Directive
        // stripping, lineage evolution and the exploration brief all work line
        // by line, so fields are put on their own lines before any of them.
        if payload
            .lines()
            .next()
            .is_some_and(|line| line.trim().starts_with("version"))
        {
            return teledra_fractus::canonical_dsl(payload);
        }
        if payload.contains('=') || payload.contains('\n') {
            return format!(
                "version 2
{}",
                teledra_fractus::canonical_dsl(payload)
            );
        }

        // `--type mandala --iterations 220` and friends.
        let mut fields = Vec::new();
        let mut tokens = payload.split_whitespace();
        while let Some(token) = tokens.next() {
            if let Some(key) = token.strip_prefix("--")
                && let Some(value) = tokens.next()
            {
                fields.push(format!("{key}={value}"));
            }
        }
        if !fields.is_empty() {
            return format!(
                "version 2
{}",
                fields.join(
                    "
"
                )
            );
        }

        // A bare layer stack: `kaleidoscope@0.45@screen@violet;spirograph@0.3`.
        if payload.contains('@') {
            return format!(
                "version 2
layers={payload}"
            );
        }

        // A single family name on its own is a legitimate, minimal request.
        let bare = payload.trim().to_ascii_lowercase();
        if !bare.is_empty()
            && bare.len() <= 32
            && bare
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            if teledra_fractus::is_known_palette(&bare) {
                return format!(
                    "version 2
palette={bare}"
                );
            }
            return format!(
                "version 2
type={bare}"
            );
        }

        // Prose. Refused rather than silently rendered as something else.
        format!(
            "version 2
type={payload}"
        )
    }

    /// Whether the Artist asked for motion while using a static action head.
    ///
    /// Live transcripts contained `animate=true` inside a `FRACTUS_ART` layer
    /// list. The renderer treated it as an unknown layer and wrote a PNG. The
    /// intent is unambiguous, so recover it before parsing instead of silently
    /// discarding motion.
    pub(super) fn fractus_requests_animation(payload: &str) -> bool {
        let lowered = payload.to_ascii_lowercase();
        [
            "animate=true",
            "animated=true",
            "animation=true",
            "motion=true",
            "live=true",
            "--animate true",
            "--live true",
        ]
        .iter()
        .any(|needle| lowered.contains(needle))
    }

    /// Removes presentation-only animation fields from the scene DSL.
    ///
    /// Animation chooses a still PNG versus APNG at the runtime boundary; it is not a
    /// scene layer. Keeping the token in `layers=` produced a verified image
    /// with an `unknown_layers=["animate=true"]` warning and no animation.
    pub(super) fn strip_fractus_animation_directives(dsl: &str) -> String {
        dsl.lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                let key = trimmed
                    .split_once('=')
                    .map(|(key, _)| key.trim().to_ascii_lowercase());
                if key
                    .as_deref()
                    .is_some_and(|key| matches!(key, "animate" | "animated" | "animation" | "live"))
                    || (key.as_deref() == Some("motion")
                        && trimmed.split_once('=').is_some_and(|(_, value)| {
                            matches!(value.trim().to_ascii_lowercase().as_str(), "true" | "false")
                        }))
                {
                    return None;
                }
                if key.as_deref() == Some("layers") {
                    let (_, layers) = trimmed.split_once('=')?;
                    let layers = layers
                        .split(';')
                        .filter(|layer| {
                            let layer_key = layer
                                .trim()
                                .split(['=', '@'])
                                .next()
                                .unwrap_or_default()
                                .to_ascii_lowercase();
                            !matches!(
                                layer_key.as_str(),
                                "animate" | "animated" | "animation" | "motion" | "live"
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(";");
                    return (!layers.is_empty()).then(|| format!("layers={layers}"));
                }
                Some(line.to_owned())
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub(super) fn write_scribe_record(
        &self,
        path: &str,
        content: &str,
        append: bool,
    ) -> Result<PathBuf, std::io::Error> {
        let normalized = path.replace('\\', "/");
        let mapped = normalized
            .to_ascii_lowercase()
            .find("/knowledge/")
            .map(|index| &normalized[index + "/knowledge/".len()..])
            .unwrap_or(&normalized);
        let relative = Path::new(mapped);
        if relative.as_os_str().is_empty()
            || relative.is_absolute()
            || relative.components().any(|component| {
                !matches!(
                    component,
                    std::path::Component::Normal(_) | std::path::Component::CurDir
                )
            })
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Scribe paths must be non-empty, relative, and may not traverse directories",
            ));
        }
        let destination = self.root().join("records").join(relative);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if append {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&destination)?;
            writeln!(file, "{content}")?;
        } else {
            std::fs::write(&destination, content)?;
        }
        Ok(destination)
    }

    pub(super) fn write_proposal(
        &self,
        kind: &str,
        content: &str,
    ) -> Result<Proposal, std::io::Error> {
        let directory = self.root().join("proposals");
        std::fs::create_dir_all(&directory)?;

        // The same request, still waiting, does not need queueing twice. The
        // court re-proposes the same outward action every time the subject
        // comes round: one hundred and thirteen proposals were pending of which
        // thirty-four were distinct, and a single post had twenty-four
        // identical upvotes waiting on it. An approval queue that deep is one
        // nobody reads.
        if let Some(existing) = self.pending_proposal_matching(kind, content) {
            return Ok(Proposal::AlreadyPending(existing));
        }

        let id = format!(
            "{kind}-{}-{}-{}",
            epoch_millis(),
            std::process::id(),
            ARTIFACT_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        let path = directory.join(format!("{id}.json"));
        let bytes = serde_json::to_vec_pretty(&serde_json::json!({
            "id": id,
            "kind": kind,
            "content": content,
            "created_at_ms": epoch_millis(),
            "status": "pending"
        }))
        .map_err(std::io::Error::other)?;
        std::fs::write(&path, bytes)?;
        Ok(Proposal::Created(path))
    }

    /// Reduces identical pending requests to one apiece.
    ///
    /// Duplicates were written for as long as the court re-raised the same
    /// subject, so an existing installation carries a queue that is mostly
    /// repeats. Approving one of a set of identical requests is approving all
    /// of them, so collapsing them loses no decision the operator could have
    /// made — and a queue of thirty-four is one that can actually be read.
    /// The earliest of each set is kept.
    pub(super) fn collapse_duplicate_proposals(&self) -> Result<usize, std::io::Error> {
        let directory = self.root().join("proposals");
        let Ok(entries) = std::fs::read_dir(&directory) else {
            return Ok(0);
        };
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
            .collect();
        paths.sort();

        let mut seen: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();
        let mut removed = 0;
        for path in paths {
            let Ok(raw) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
                continue;
            };
            if value.get("status").and_then(|v| v.as_str()) != Some("pending") {
                continue;
            }
            let key = (
                value
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_owned(),
                value
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .trim()
                    .to_owned(),
            );
            if !seen.insert(key) && std::fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// An identical proposal of the same kind that is still awaiting a decision.
    fn pending_proposal_matching(&self, kind: &str, content: &str) -> Option<PathBuf> {
        let directory = self.root().join("proposals");
        let wanted = content.trim();
        for entry in std::fs::read_dir(directory).ok()?.flatten() {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
                continue;
            };
            let same_kind = value.get("kind").and_then(|v| v.as_str()) == Some(kind);
            let still_open = value.get("status").and_then(|v| v.as_str()) == Some("pending")
                && !proposal_expired(&value, epoch_millis());
            let same_content = value
                .get("content")
                .and_then(|v| v.as_str())
                .is_some_and(|existing| existing.trim() == wanted);
            if same_kind && still_open && same_content {
                return Some(path);
            }
        }
        None
    }

    pub(super) fn pending_proposals(&self) -> Result<Vec<ProposalDecision>, std::io::Error> {
        let directory = self.root().join("proposals");
        let Ok(entries) = std::fs::read_dir(directory) else {
            return Ok(Vec::new());
        };
        let mut proposals = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                continue;
            };
            if value.get("status").and_then(|value| value.as_str()) != Some("pending") {
                continue;
            }
            if proposal_expired(&value, epoch_millis()) {
                value["status"] = serde_json::json!("expired");
                value["expired_at_ms"] = serde_json::json!(epoch_millis());
                std::fs::write(
                    &path,
                    serde_json::to_vec_pretty(&value).map_err(std::io::Error::other)?,
                )?;
                continue;
            }
            proposals.push(ProposalDecision {
                id: value
                    .get("id")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_owned(),
                kind: value
                    .get("kind")
                    .and_then(|value| value.as_str())
                    .unwrap_or("suggestion")
                    .to_owned(),
                content: value
                    .get("content")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_owned(),
            });
        }
        proposals.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(proposals)
    }

    pub(super) fn decide_proposals(
        &self,
        selection: &str,
        decision: &str,
    ) -> Result<Vec<ProposalDecision>, std::io::Error> {
        let mut changed = Vec::new();
        let directory = self.root().join("proposals");
        let all = selection.eq_ignore_ascii_case("all");
        for proposal in self.pending_proposals()? {
            if !all && proposal.id != selection {
                continue;
            }
            let path = directory.join(format!("{}.json", proposal.id));
            let bytes = std::fs::read(&path)?;
            let mut value: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
            value["status"] = serde_json::Value::String(decision.to_owned());
            value["decided_at_ms"] = serde_json::json!(epoch_millis());
            std::fs::write(
                path,
                serde_json::to_vec_pretty(&value).map_err(std::io::Error::other)?,
            )?;
            changed.push(proposal);
        }
        Ok(changed)
    }

    pub(super) fn record_music_feedback(
        &mut self,
        app: &mut App,
        verdict: FeedbackVerdict,
        note: String,
    ) {
        if self.music_engine == super::pymusic::MusicEngine::Python {
            self.record_python_feedback(app, verdict, note);
            return;
        }
        let directory = self.root().join("music").join("feedback");
        let normalized = note.trim().to_ascii_lowercase();
        match teledra_music::record_feedback(&directory, &self.score, verdict, note.clone()) {
            Ok(path) => {
                app.set_notice(format!("Music verdict recorded: {}", path.display()));
                if matches!(verdict, FeedbackVerdict::Keep)
                    && matches!(normalized.as_str(), "like" | "expand" | "playlist")
                {
                    let collection = if normalized == "playlist" {
                        "playlist"
                    } else {
                        "keepers"
                    };
                    let archive = self.root().join("music").join(collection);
                    let saved = std::fs::create_dir_all(&archive).and_then(|_| {
                        let destination = archive.join(format!(
                            "{}-{}-{}.json",
                            epoch_millis(),
                            std::process::id(),
                            ARTIFACT_SEQUENCE.fetch_add(1, Ordering::Relaxed)
                        ));
                        let bytes = serde_json::to_vec_pretty(&self.score)
                            .map_err(std::io::Error::other)?;
                        std::fs::write(&destination, bytes)?;
                        Ok(destination)
                    });
                    match saved {
                        Ok(destination) => app.push_private_event(
                            "Organist",
                            format!("Score saved to {collection}: {}", destination.display()),
                        ),
                        Err(error) => app.push_private_event("Organist", error.to_string()),
                    }
                }
                let reflection_kind = if matches!(verdict, FeedbackVerdict::Reject) {
                    "opinion"
                } else {
                    "taste"
                };
                let _ = self.memory.apply_reflection(
                    reflection_kind,
                    &format!("music '{}': {normalized}", self.score.title),
                    "operator:music-feedback",
                );
                self.refresh_identity_into(app);
            }
            Err(error) => app.set_notice(format!("Could not record music verdict: {error}")),
        }
    }

    fn record_python_feedback(&mut self, app: &mut App, verdict: FeedbackVerdict, note: String) {
        self.record_python_library_verdict(app, verdict, &note);
        let directory = self.root().join("music").join("python");
        let result = self
            .live_python_feedback_snapshot(verdict, &note)
            .and_then(|event| {
                let path = directory.join("feedback.jsonl");
                let mut log = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)?;
                serde_json::to_writer(&mut log, &event).map_err(std::io::Error::other)?;
                writeln!(log)?;
                log.flush()?;
                Ok((path, event))
            });
        match result {
            Ok((path, event)) => {
                app.set_notice(format!("Python music verdict recorded: {}", path.display()));
                let normalized = note.trim().to_ascii_lowercase();
                if matches!(verdict, FeedbackVerdict::Keep)
                    && matches!(normalized.as_str(), "like" | "expand" | "playlist")
                {
                    let collection = if normalized == "playlist" {
                        "playlist"
                    } else {
                        "keepers"
                    };
                    let archive = directory.join(collection);
                    let saved = std::fs::create_dir_all(&archive).and_then(|_| {
                        let destination = archive.join(format!(
                            "{}-{}-{}.json",
                            epoch_millis(),
                            std::process::id(),
                            ARTIFACT_SEQUENCE.fetch_add(1, Ordering::Relaxed)
                        ));
                        let bytes =
                            serde_json::to_vec_pretty(&event).map_err(std::io::Error::other)?;
                        std::fs::write(&destination, bytes)?;
                        Ok(destination)
                    });
                    match saved {
                        Ok(path) => app.push_private_event(
                            "Organist",
                            format!(
                                "Python source and render report saved to {collection}: {}",
                                path.display()
                            ),
                        ),
                        Err(error) => app.push_private_event("Organist", error.to_string()),
                    }
                }
                let kind = if matches!(verdict, FeedbackVerdict::Reject) {
                    "opinion"
                } else {
                    "taste"
                };
                let title = event["score_title"]
                    .as_str()
                    .unwrap_or("Python composition");
                let _ = self.memory.apply_reflection(
                    kind,
                    &format!("Python music '{title}': {normalized}"),
                    "operator:python-music-feedback",
                );
                self.refresh_identity_into(app);
            }
            Err(error) => app.set_notice(format!("Could not record Python music verdict: {error}")),
        }
    }
}

/// Feedback belongs to accepted audio, even when the editor contains a newer
/// unsaved or rejected draft. The accepted pointer is advanced after verification.
#[cfg(test)]
fn python_feedback_snapshot(
    directory: &Path,
    verdict: FeedbackVerdict,
    note: &str,
) -> std::io::Result<serde_json::Value> {
    let pointer: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("current.json"))?)
            .map_err(std::io::Error::other)?;
    let artifact = pointer["artifact"].as_str().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "No accepted Python music artifact is selected",
        )
    })?;
    let mut components = Path::new(artifact).components();
    if !matches!(components.next(), Some(std::path::Component::Normal(_)))
        || components.next().is_some()
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "The accepted Python artifact must be a local render directory",
        ));
    }
    let accepted = directory.join(artifact);
    let code = std::fs::read_to_string(accepted.join("code.py"))?;
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(accepted.join("report.json"))?)
            .map_err(std::io::Error::other)?;
    Ok(serde_json::json!({
        "timestamp_ms": epoch_millis(), "engine": "python", "identity": artifact,
        "score_title": report["title"].as_str().unwrap_or("Python composition"),
        "verdict": verdict, "note": note.trim(), "code": code, "report": report,
    }))
}

pub(super) fn render_fractus_at(
    root: &Path,
    dsl: &str,
    live: bool,
) -> Result<RenderedFractus, Box<dyn std::error::Error>> {
    let directory = root.join("fractus");
    std::fs::create_dir_all(&directory)?;
    let id = format!(
        "{}-{}-{}",
        epoch_millis(),
        std::process::id(),
        ARTIFACT_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    // Animated PNG retains the renderer's full color and smooth gradients.
    // Existing GIFs remain readable and subject to the same gallery retention.
    let image = directory.join(format!("fractus-{id}.png"));
    let manifest_path = directory.join(format!("fractus-{id}.manifest.json"));
    let mut scene = Scene::parse(dsl)?;
    if live {
        bound_live_scene(&mut scene);
    }
    // Reject primitive-looking candidates before allocating 120 frames or
    // publishing a manifest. Existing displayed art remains the accepted parent.
    scene.require_visual_richness()?;
    let manifest = if live {
        scene.render_animation(&image, FRACTUS_LIVE_FRAMES, FRACTUS_LIVE_FPS)?
    } else {
        scene.render(&image)?
    };
    teledra_fractus::write_manifest(&manifest_path, &manifest)?;
    let retention = prune_fractus_gallery(&directory, FRACTUS_GALLERY_KEEP);
    let retention_notice = if retention.removed_artifacts == 0
        && retention.removed_orphan_images == 0
        && retention.removed_orphans == 0
        && retention.warnings.is_empty()
    {
        None
    } else {
        let mut notice = format!(
            "gallery retention removed {} artifact(s), {} stale orphan image(s), and {} orphan manifest(s)",
            retention.removed_artifacts, retention.removed_orphan_images, retention.removed_orphans
        );
        if !retention.warnings.is_empty() {
            notice.push_str(&format!(
                "; {} cleanup error(s): {}",
                retention.warnings.len(),
                retention.warnings.join("; ")
            ));
        }
        Some(notice)
    };
    Ok(RenderedFractus {
        path: image,
        retention_notice,
    })
}

fn bound_live_scene(scene: &mut Scene) {
    let edge_scale = f64::from(FRACTUS_LIVE_MAX_EDGE) / f64::from(scene.width.max(scene.height));
    // A very wide or tall artwork may exceed the preferred display edge to
    // preserve its aspect ratio and the renderer's minimum dimension.
    let minimum_scale = 64.0 / f64::from(scene.width.min(scene.height));
    let pixel_scale = (f64::from(FRACTUS_LIVE_MAX_PIXELS)
        / (f64::from(scene.width) * f64::from(scene.height)))
    .sqrt();
    let scale = edge_scale.max(minimum_scale).min(pixel_scale).min(1.0);
    scene.width = (f64::from(scene.width) * scale).floor().max(64.0) as u32;
    scene.height = (f64::from(scene.height) * scale).floor().max(64.0) as u32;
}

/// Keeps the newest bounded window, then removes exact duplicates within it.
///
/// Capping before hashing is intentional. The first deployment may encounter
/// several gigabytes of history; everything older than the newest window is
/// going away regardless, so reading it merely to prove it is duplicate would
/// turn a small retention pass into a multi-gigabyte scan.
fn proposal_expired(value: &serde_json::Value, now: u128) -> bool {
    let created = value
        .get("created_at_ms")
        .and_then(serde_json::Value::as_u64)
        .map(u128::from)
        .or_else(|| {
            value
                .get("id")
                .and_then(serde_json::Value::as_str)
                .and_then(|id| {
                    id.split('-')
                        .filter_map(|part| part.parse::<u128>().ok())
                        .find(|stamp| *stamp >= 1_000_000_000_000)
                })
        });
    let hours = if value
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|kind| kind.starts_with("moltbook-"))
    {
        48
    } else {
        7 * 24
    };
    created.is_some_and(|created| now.saturating_sub(created) >= hours * 60 * 60 * 1000)
}

fn prune_fractus_gallery(directory: &Path, keep: usize) -> GalleryRetention {
    let mut report = GalleryRetention::default();
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            report.warnings.push(error.to_string());
            return report;
        }
    };
    let mut images = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("fractus-"))
                && matches!(
                    path.extension().and_then(|extension| extension.to_str()),
                    Some("gif" | "png")
                )
        })
        .collect::<Vec<_>>();
    images.sort_by(|left, right| right.file_name().cmp(&left.file_name()));

    for image in images.iter().skip(keep) {
        remove_fractus_pair(image, &mut report);
    }

    let mut digests = HashSet::new();
    for image in images.iter().take(keep) {
        match content_digest(image) {
            Ok(digest) if !digests.insert(digest) => {
                remove_fractus_pair(image, &mut report);
            }
            Ok(_) => {}
            Err(error) => report
                .warnings
                .push(format!("could not hash {}: {error}", image.display())),
        }
    }

    remove_stale_orphan_images(directory, &mut report);
    remove_orphan_manifests(directory, &mut report);
    report
}

fn remove_fractus_pair(image: &Path, report: &mut GalleryRetention) {
    match std::fs::remove_file(image) {
        Ok(()) => {
            report.removed_artifacts += 1;
            let manifest = image.with_extension("manifest.json");
            if let Err(error) = std::fs::remove_file(&manifest)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                report
                    .warnings
                    .push(format!("could not remove {}: {error}", manifest.display()));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => report
            .warnings
            .push(format!("could not remove {}: {error}", image.display())),
    }
}

fn remove_orphan_manifests(directory: &Path, report: &mut GalleryRetention) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for path in entries.filter_map(Result::ok).map(|entry| entry.path()) {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(stem) = name.strip_suffix(".manifest.json") else {
            continue;
        };
        if !stem.starts_with("fractus-")
            || path.with_file_name(format!("{stem}.gif")).exists()
            || path.with_file_name(format!("{stem}.png")).exists()
        {
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => report.removed_orphans += 1,
            Err(error) => report
                .warnings
                .push(format!("could not remove {}: {error}", path.display())),
        }
    }
}

/// Deletes unverifiable images left by old interrupted renders, while leaving
/// a recent image alone because another bounded render thread may still be
/// writing its frames and has not reached the manifest step yet.
fn remove_stale_orphan_images(directory: &Path, report: &mut GalleryRetention) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let now = epoch_millis();
    for path in entries.filter_map(Result::ok).map(|entry| entry.path()) {
        if !matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("gif" | "png")
        ) || path.with_extension("manifest.json").exists()
        {
            continue;
        }
        let Some(created) = fractus_artifact_epoch(&path) else {
            continue;
        };
        if now.saturating_sub(created) < IN_PROGRESS_ARTIFACT_GRACE_MS {
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => report.removed_orphan_images += 1,
            Err(error) => report
                .warnings
                .push(format!("could not remove {}: {error}", path.display())),
        }
    }
}

fn fractus_artifact_epoch(path: &Path) -> Option<u128> {
    path.file_stem()?
        .to_str()?
        .strip_prefix("fractus-")?
        .split('-')
        .next()?
        .parse()
        .ok()
}

fn content_digest(path: &Path) -> std::io::Result<(u64, u64)> {
    let mut file = std::fs::File::open(path)?;
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut length = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        length += read as u64;
        for byte in &buffer[..read] {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    Ok((length, hash))
}

/// Words that mark a clause as recalling or turning away from other work.
const RETROSPECTIVE_MARKERS: [&str; 11] = [
    "last",
    "previous",
    "prior",
    "earlier",
    "considering",
    "building on",
    "instead of",
    "rather than",
    "unlike",
    "away from",
    "moving on from",
];

/// Words that describe a family as an accent on the piece, not its subject.
const ACCENT_MARKERS: [&str; 6] = ["subtle", "faint", "accent", "accents", "hint", "whisper"];

/// Whether every mention of a family sits within two words of an accent word.
///
/// "I propose adding a subtle lissajous curve to our existing mandala" named
/// the accent first, so it became the subject: a full-canvas Lissajous knot
/// over the mandala the Artist meant to enrich.
fn described_only_as_accent(text: &str, name: &str) -> bool {
    let searchable = searchable_art_text(text);
    let words = searchable.split(' ').collect::<Vec<_>>();
    let phrase = name.split('_').collect::<Vec<_>>();
    let mut mentioned = false;
    for (start, window) in words.windows(phrase.len()).enumerate() {
        if window != phrase.as_slice() {
            continue;
        }
        mentioned = true;
        let end = (start + phrase.len() + 2).min(words.len());
        if !words[start.saturating_sub(2)..end]
            .iter()
            .any(|word| ACCENT_MARKERS.contains(word))
        {
            return false;
        }
    }
    mentioned
}

/// Names from the clauses that describe the new piece, in order.
///
/// The Artist opens by recalling the last one: "Considering the last
/// composition's exploration of Lissajous harmonics, let's venture into ... a
/// mandala with ... a fractal tree." The first family named became the
/// subject, so a Lissajous knot the Artist had moved on from was drawn over
/// the mandala it asked for, and native evolution kept re-rendering it.
fn intended_terms<'a>(text: &str, names: &'a [&'a str]) -> Vec<&'a str> {
    let mut terms = Vec::new();
    for clause in text.split(['.', ',', ';', ':', '!', '?', '\n', '—', '–']) {
        let words = format!(" {} ", searchable_art_text(clause));
        if RETROSPECTIVE_MARKERS
            .iter()
            .any(|marker| words.contains(&format!(" {marker} ")))
        {
            continue;
        }
        for name in named_terms(clause, names) {
            if !terms.contains(&name) {
                terms.push(name);
            }
        }
    }
    terms
}

fn named_terms<'a>(text: &str, names: &'a [&'a str]) -> Vec<&'a str> {
    let searchable = searchable_art_text(text);
    let mut matches = Vec::new();
    for &name in names {
        let phrase = name.replace('_', " ");
        for (start, _) in searchable.match_indices(&phrase) {
            let end = start + phrase.len();
            let before_is_word = searchable[..start]
                .chars()
                .next_back()
                .is_some_and(|character| character.is_ascii_alphanumeric());
            let after_is_word = searchable[end..]
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_alphanumeric());
            if !before_is_word && !after_is_word {
                matches.push((start, end, name));
            }
        }
    }
    matches.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| (right.1 - right.0).cmp(&(left.1 - left.0)))
    });

    let mut selected: Vec<(usize, usize, &'a str)> = Vec::new();
    let mut seen = HashSet::new();
    for candidate in matches {
        if selected
            .iter()
            .any(|(start, end, _)| candidate.0 < *end && candidate.1 > *start)
            || !seen.insert(candidate.2)
        {
            continue;
        }
        selected.push(candidate);
    }
    selected.into_iter().map(|(_, _, name)| name).collect()
}

fn searchable_art_text(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut separator = true;
    for character in text.chars() {
        let character = match character {
            'é' | 'É' => 'e',
            character if character.is_ascii_alphanumeric() => character.to_ascii_lowercase(),
            _ => ' ',
        };
        if character == ' ' {
            if !separator {
                normalized.push(' ');
                separator = true;
            }
        } else {
            normalized.push(character);
            separator = false;
        }
    }
    normalized
}

fn prose_seed(text: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn epoch_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(windows)]
pub(super) fn open_artifact(path: &Path) -> std::io::Result<()> {
    Command::new("explorer.exe").arg(path).spawn().map(|_| ())
}

#[cfg(not(windows))]
pub(super) fn open_artifact(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{RuntimeServices, prune_fractus_gallery};

    #[test]
    fn python_feedback_uses_accepted_source_and_report_instead_of_edited_draft() {
        let directory = tempfile::tempdir().unwrap();
        let artifact = directory.path().join("render-accepted");
        std::fs::create_dir(&artifact).unwrap();
        std::fs::write(
            directory.path().join("current.json"),
            r#"{"artifact":"render-accepted"}"#,
        )
        .unwrap();
        std::fs::write(artifact.join("code.py"), "accepted = 1\n").unwrap();
        std::fs::write(
            artifact.join("report.json"),
            r#"{"title":"The accepted piece","duration":80}"#,
        )
        .unwrap();
        std::fs::write(directory.path().join("draft.py"), "rejected_draft = 2\n").unwrap();
        let event = super::python_feedback_snapshot(
            directory.path(),
            teledra_music::FeedbackVerdict::Reject,
            " too bright ",
        )
        .unwrap();
        assert_eq!(event["engine"], "python");
        assert_eq!(event["identity"], "render-accepted");
        assert_eq!(event["code"], "accepted = 1\n");
        assert_eq!(event["report"]["duration"], 80);
        assert_eq!(event["score_title"], "The accepted piece");
        assert_eq!(event["note"], "too bright");
        assert_eq!(event["verdict"], "reject");
    }

    #[test]
    fn python_feedback_needs_an_accepted_local_render() {
        let directory = tempfile::tempdir().unwrap();
        assert!(
            super::python_feedback_snapshot(
                directory.path(),
                teledra_music::FeedbackVerdict::Keep,
                ""
            )
            .is_err()
        );
        std::fs::write(
            directory.path().join("current.json"),
            r#"{"artifact":"../outside"}"#,
        )
        .unwrap();
        let error = super::python_feedback_snapshot(
            directory.path(),
            teledra_music::FeedbackVerdict::Keep,
            "",
        )
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn stale_proposals_expire_without_treating_new_or_undated_work_as_old() {
        let created = 1_780_000_000_000_u128;
        let day = 24 * 60 * 60 * 1000;
        let mut value = serde_json::json!({"created_at_ms": created, "kind": "moltbook-comment"});
        assert!(!super::proposal_expired(&value, created + day));
        assert!(super::proposal_expired(&value, created + 2 * day));
        value["kind"] = serde_json::json!("suggestion");
        assert!(!super::proposal_expired(&value, created + 6 * day));
        assert!(super::proposal_expired(&value, created + 7 * day));
        assert!(!super::proposal_expired(
            &serde_json::json!({}),
            created + 20 * day
        ));
        let legacy = serde_json::json!({"id": format!("moltbook-comment-{created}-12-1"), "kind": "moltbook-comment"});
        assert!(super::proposal_expired(&legacy, created + 2 * day));
    }
    use std::path::{Path, PathBuf};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "teledra-{name}-{}-{}",
                std::process::id(),
                super::ARTIFACT_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn animation_intent_is_recovered_without_becoming_a_layer() {
        let payload = "layers=moire@0.5@screen@electric_cyan;animate=true\nmotion=true";
        assert!(RuntimeServices::fractus_requests_animation(payload));
        let dsl = RuntimeServices::normalize_fractus_payload(payload);
        let cleaned = RuntimeServices::strip_fractus_animation_directives(&dsl);
        assert!(cleaned.contains("layers=moire@0.5@screen@electric_cyan"));
        assert!(!cleaned.to_ascii_lowercase().contains("animate"));
        assert!(!cleaned.to_ascii_lowercase().contains("motion"));
        let scene = teledra_fractus::Scene::parse(&cleaned).unwrap();
        assert!(
            scene.unknown_layers.is_empty(),
            "{:?}",
            scene.unknown_layers
        );
    }

    /// The one-line form the Artist is taught, carrying presentation flags the
    /// renderer must not see. Cleanup works line by line, so the payload has to
    /// be split into fields before it, or `motion=true` reaches the parser.
    #[test]
    fn a_one_line_artist_payload_is_cleaned_like_a_multi_line_one() {
        let payload = "version 2; type=julia; width=600; height=800; iterations=180; seed=3947825609874563210; palette=solar_gold; symmetry=12; zoom=2; motion=true; layers=mandala@0.18@screen@electric_cyan;fractal_tree@0.1@screen@solar_gold;animate=true";
        assert!(RuntimeServices::fractus_requests_animation(payload));
        let dsl = RuntimeServices::normalize_fractus_payload(payload);
        let cleaned = RuntimeServices::strip_fractus_animation_directives(&dsl);
        let scene = teledra_fractus::Scene::parse(&cleaned).unwrap();
        assert!(matches!(scene.kind, teledra_fractus::SceneKind::Julia));
        assert_eq!(scene.layers.len(), 2, "{cleaned}");
        assert!(
            scene.unknown_layers.is_empty(),
            "{:?}",
            scene.unknown_layers
        );
        assert!(
            !cleaned.to_ascii_lowercase().contains("animate"),
            "{cleaned}"
        );
        assert_eq!(
            scene.motion, 0.65,
            "a boolean motion flag leaves the default"
        );
    }

    #[test]
    fn numeric_motion_survives_animation_directive_cleanup() {
        let cleaned = RuntimeServices::strip_fractus_animation_directives(
            "version 2\ntype=mandala\nanimate=true\nmotion=0.72\nsymmetry=18\nzoom=1.5",
        );
        let scene = teledra_fractus::Scene::parse(&cleaned).unwrap();
        assert_eq!(scene.motion, 0.72);
        assert_eq!(scene.symmetry, 18);
        assert_eq!(scene.zoom, 1.5);
    }

    #[test]
    fn live_dimensions_preserve_portrait_landscape_and_panorama_within_budget() {
        for (width, height) in [
            (600, 800),
            (1600, 1200),
            (2048, 2048),
            (64, 2048),
            (2048, 64),
        ] {
            let mut scene = teledra_fractus::Scene {
                width,
                height,
                ..Default::default()
            };
            super::bound_live_scene(&mut scene);
            scene.validate().unwrap();
            assert!(scene.width * scene.height <= super::FRACTUS_LIVE_MAX_PIXELS);
            let expected = f64::from(width) / f64::from(height);
            let actual = f64::from(scene.width) / f64::from(scene.height);
            assert!((expected - actual).abs() <= expected * 0.015);
            assert!(scene.width <= width && scene.height <= height);
        }
    }

    #[test]
    fn live_artifact_is_full_color_apng_and_still_remains_single_png() {
        let directory = TestDirectory::new("fractus-live-pipeline");
        let dsl =
            "version 2\ntype=mandala\nwidth=64\nheight=64\niterations=24\nsymmetry=12\nmotion=0.65";
        let live = super::render_fractus_at(directory.path(), dsl, true).unwrap();
        assert_eq!(live.path.extension().unwrap(), "png");
        let bytes = std::fs::read(&live.path).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(bytes[25], 2, "PNG IHDR color type must be true-color RGB");
        assert!(bytes.windows(4).any(|chunk| chunk == b"acTL"));
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(live.path.with_extension("manifest.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["animation"]["frame_count"], 120);
        assert_eq!(manifest["animation"]["fps"], 24);
        assert!(manifest["animation"]["distinct_frames"].as_u64().unwrap() > 1);
        let still = super::render_fractus_at(directory.path(), dsl, false).unwrap();
        let bytes = std::fs::read(&still.path).unwrap();
        assert!(!bytes.windows(4).any(|chunk| chunk == b"acTL"));
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(still.path.with_extension("manifest.json")).unwrap(),
        )
        .unwrap();
        assert!(
            manifest
                .get("animation")
                .is_none_or(serde_json::Value::is_null)
        );
    }

    #[test]
    fn simple_art_is_rejected_before_publication_and_existing_art_is_preserved() {
        let directory = TestDirectory::new("fractus-richness-gate");
        let gallery = directory.path().join("fractus");
        std::fs::create_dir_all(&gallery).unwrap();
        let preserved = gallery.join("fractus-accepted.png");
        std::fs::write(&preserved, b"existing accepted artifact").unwrap();
        let dsl = "type=lissajous; width=600; height=800; iterations=180; palette=solar_gold; symmetry=12; seed=11036126381597215977";
        for live in [false, true] {
            let result = super::render_fractus_at(directory.path(), dsl, live);
            let error = match result {
                Ok(_) => panic!("simple outline must not publish"),
                Err(error) => error,
            };
            assert!(
                error.to_string().contains("visual richness check"),
                "{error}"
            );
            assert_eq!(std::fs::read_dir(&gallery).unwrap().count(), 1);
            assert_eq!(
                std::fs::read(&preserved).unwrap(),
                b"existing accepted artifact"
            );
        }
    }

    #[test]
    fn artist_prose_becomes_the_named_scene_and_layers() {
        let prose = "The Mandala family will hold the symmetry while Lissajous curves move through it. A neon sunset field gives way to electric cyan highlights.";
        let dsl = RuntimeServices::salvage_fractus_from_prose(prose).unwrap();
        let scene = teledra_fractus::Scene::parse(&dsl).unwrap();
        assert!(matches!(scene.kind, teledra_fractus::SceneKind::Mandala));
        assert_eq!(scene.palette, "neon_sunset");
        assert!(
            scene
                .layers
                .iter()
                .any(|layer| matches!(layer.kind, teledra_fractus::SceneKind::Lissajous))
        );
        assert!(dsl.contains("electric_cyan"), "{dsl}");
    }

    #[test]
    fn longer_family_names_are_not_reduced_to_mandala() {
        let dsl = RuntimeServices::salvage_fractus_from_prose(
            "Begin with a lotus mandala in solar gold, then let the petals breathe.",
        )
        .unwrap();
        let scene = teledra_fractus::Scene::parse(&dsl).unwrap();
        assert!(matches!(
            scene.kind,
            teledra_fractus::SceneKind::LotusMandala
        ));
        assert!(scene.layers.is_empty(), "{dsl}");
    }

    /// Verbatim from the stream. The opening clause recalls the previous
    /// piece; the Lissajous knot it names was drawn over the mandala the
    /// Artist described and re-rendered with every evolution of that lineage.
    #[test]
    fn a_family_recalled_from_earlier_work_is_not_drawn_into_the_new_one() {
        let prose = "Considering the last composition's exploration of Lissajous harmonics and kaleidoscopic symmetry, let's venture into a new geometric experiment that combines the elegance of a mandala with the dynamic complexity of a fractal tree. This piece, \"Blossoming Fractal Forest,\" will feature a central mandala that grows and transforms into intricate fractal branches, each branch carrying a unique pattern that evolves as it extends. The palette will be a soft blend of amethyst and electric cyan, with subtle gradients to emphasize the depth and movement. The motion will be smooth and organic, mimicking the growth of a forest over time. This composition aims to create a harmonious and ever-evolving scene that captures the beauty of both static and dynamic forms.";
        let dsl = RuntimeServices::salvage_fractus_from_prose(prose).unwrap();
        let scene = teledra_fractus::Scene::parse(&dsl).unwrap();
        assert!(
            matches!(scene.kind, teledra_fractus::SceneKind::Mandala),
            "{dsl}"
        );
        assert_eq!(scene.palette, "amethyst");
        assert!(
            scene
                .layers
                .iter()
                .all(|layer| !matches!(layer.kind, teledra_fractus::SceneKind::Lissajous)),
            "{dsl}"
        );
        assert!(
            scene
                .layers
                .iter()
                .any(|layer| matches!(layer.kind, teledra_fractus::SceneKind::FractalTree)),
            "{dsl}"
        );
    }

    /// Verbatim from the stream: the new piece was an orbit weave, but the
    /// Lissajous harmonics of the last experiment became its subject.
    #[test]
    fn a_composition_that_turns_away_from_the_last_one_keeps_its_own_subject() {
        let prose = "Considering the last experiment's intricate interplay of Lissajous harmonics and mandalas, let's shift our focus to a different yet equally captivating composition. This new piece, \"Orbital Weave,\" will explore the beauty of hypotrochoids and epicycloids, interwoven with a kaleidoscopic effect to create a dynamic and harmonious visualization. The motion will be smooth, with a gentle glide, and the palette will be a subtle blend of amethyst and electric cyan, emphasizing the elegant curves and symmetry. The layers will be kept minimal, focusing on the primary design while adding a subtle guilloche pattern to enhance the visual depth and complexity.";
        let dsl = RuntimeServices::salvage_fractus_from_prose(prose).unwrap();
        let scene = teledra_fractus::Scene::parse(&dsl).unwrap();
        assert!(
            matches!(scene.kind, teledra_fractus::SceneKind::Guilloche),
            "{dsl}"
        );
        assert!(!dsl.contains("lissajous"), "{dsl}");
    }

    #[test]
    fn a_reply_that_only_recalls_earlier_work_is_not_invented_into_art() {
        assert!(
            RuntimeServices::salvage_fractus_from_prose(
                "Building on the last Lissajous study, the gallery can rest a while."
            )
            .is_none()
        );
    }

    /// Verbatim from the stream: an untagged Night Desk reply that salvaged to
    /// a Lissajous subject with the mandala it meant to enrich as a faint layer.
    #[test]
    fn a_subtle_accent_does_not_displace_the_form_it_accents() {
        let prose = "Ah, my dear courtiers, let us enhance the current visual artifact with a layer of intricate detail. I propose adding a subtle lissajous curve to our existing mandala, intertwining its patterns with fluid grace. This will not replace the core mandala but rather enrich its complexity, creating a harmonious blend of symmetry and motion.";
        let dsl = RuntimeServices::salvage_fractus_from_prose(prose).unwrap();
        let scene = teledra_fractus::Scene::parse(&dsl).unwrap();
        assert!(
            matches!(scene.kind, teledra_fractus::SceneKind::Mandala),
            "{dsl}"
        );
        assert!(
            matches!(
                scene.layers.as_slice(),
                [layer] if matches!(layer.kind, teledra_fractus::SceneKind::Lissajous)
            ),
            "{dsl}"
        );
        // An accent that is the only family named is still the request.
        let dsl = RuntimeServices::salvage_fractus_from_prose(
            "The layers stay minimal while adding a subtle guilloche pattern.",
        )
        .unwrap();
        assert!(dsl.contains("type=guilloche"), "{dsl}");
    }

    #[test]
    fn ordinary_artist_prose_without_a_family_is_not_invented_into_art() {
        assert!(
            RuntimeServices::salvage_fractus_from_prose(
                "The balance is promising, but the negative space still needs judgment."
            )
            .is_none()
        );
    }

    #[test]
    fn retention_keeps_the_newest_window_and_deduplicates_it() {
        let directory = TestDirectory::new("fractus-retention");
        for index in 0..6 {
            let stem = format!("fractus-{index:013}-1-0");
            let image = directory.path().join(format!("{stem}.gif"));
            let bytes = if index == 4 || index == 5 {
                b"same-render".to_vec()
            } else {
                format!("render-{index}").into_bytes()
            };
            std::fs::write(&image, bytes).unwrap();
            std::fs::write(
                directory.path().join(format!("{stem}.manifest.json")),
                b"{}",
            )
            .unwrap();
        }
        std::fs::write(directory.path().join("fractus-orphan.manifest.json"), b"{}").unwrap();

        let report = prune_fractus_gallery(directory.path(), 4);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_eq!(report.removed_artifacts, 3);
        assert_eq!(report.removed_orphan_images, 0);
        assert_eq!(report.removed_orphans, 1);

        let remaining = std::fs::read_dir(directory.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("gif"))
            .collect::<Vec<_>>();
        assert_eq!(remaining.len(), 3, "{remaining:?}");
        assert!(
            directory
                .path()
                .join("fractus-0000000000005-1-0.gif")
                .exists()
        );
        assert!(
            !directory
                .path()
                .join("fractus-0000000000004-1-0.gif")
                .exists()
        );
        for image in remaining {
            assert!(image.with_extension("manifest.json").exists());
        }
    }

    #[test]
    fn retention_removes_only_stale_orphan_images() {
        let directory = TestDirectory::new("fractus-orphan-images");
        let stale = directory.path().join("fractus-0000000000001-1-0.gif");
        let recent = directory
            .path()
            .join(format!("fractus-{}-1-1.gif", super::epoch_millis()));
        std::fs::write(&stale, b"interrupted-old-render").unwrap();
        std::fs::write(&recent, b"render-still-in-progress").unwrap();

        let report = prune_fractus_gallery(directory.path(), 64);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_eq!(report.removed_orphan_images, 1);
        assert!(!stale.exists());
        assert!(recent.exists());
    }
}
