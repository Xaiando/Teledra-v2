use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::app::App;
use teledra_fractus::Scene;
use teledra_music::FeedbackVerdict;

use super::RuntimeServices;

static ARTIFACT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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
        if payload
            .lines()
            .next()
            .is_some_and(|line| line.trim().starts_with("version"))
        {
            return payload.to_owned();
        }
        if payload.contains('=') || payload.contains('\n') {
            return format!(
                "version 2
{payload}"
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
    /// Animation chooses PNG versus GIF at the runtime boundary; it is not a
    /// scene layer. Keeping the token in `layers=` produced a verified image
    /// with an `unknown_layers=["animate=true"]` warning and no animation.
    pub(super) fn strip_fractus_animation_directives(dsl: &str) -> String {
        dsl.lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                let key = trimmed
                    .split_once('=')
                    .map(|(key, _)| key.trim().to_ascii_lowercase());
                if key.as_deref().is_some_and(|key| {
                    matches!(
                        key,
                        "animate" | "animated" | "animation" | "motion" | "live"
                    )
                }) {
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
            let still_open = value.get("status").and_then(|v| v.as_str()) == Some("pending");
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
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                continue;
            };
            if value.get("status").and_then(|value| value.as_str()) != Some("pending") {
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
        &self,
        app: &mut App,
        verdict: FeedbackVerdict,
        note: String,
    ) {
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
}

pub(super) fn render_fractus_at(
    root: &Path,
    dsl: &str,
    live: bool,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let directory = root.join("fractus");
    std::fs::create_dir_all(&directory)?;
    let id = format!(
        "{}-{}-{}",
        epoch_millis(),
        std::process::id(),
        ARTIFACT_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let (image, manifest_path) = if live {
        (
            directory.join(format!("fractus-{id}.gif")),
            directory.join(format!("fractus-{id}.manifest.json")),
        )
    } else {
        (
            directory.join(format!("fractus-{id}.png")),
            directory.join(format!("fractus-{id}.manifest.json")),
        )
    };
    let scene = Scene::parse(dsl)?;
    let manifest = if live {
        scene.render_gif(&image, 24)?
    } else {
        scene.render(&image)?
    };
    teledra_fractus::write_manifest(&manifest_path, &manifest)?;
    Ok(image)
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
    use super::RuntimeServices;

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
}
