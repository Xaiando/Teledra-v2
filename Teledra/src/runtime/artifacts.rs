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
    /// first named family becomes the manifest subject and later families are
    /// retained as bounded layers.
    pub(crate) fn salvage_fractus_from_prose(text: &str) -> Option<String> {
        let families = named_terms(text, &FRACTUS_FAMILIES);
        let primary = families.first()?;
        let palettes = named_terms(text, &teledra_fractus::PALETTE_NAMES);
        let palette = palettes.first().copied().unwrap_or("purple_haze");
        let seed = prose_seed(text);
        let mut dsl = format!(
            "version 2\ntype={primary}\nwidth=600\nheight=800\niterations=180\nseed={seed}\npalette={palette}"
        );

        let layers = families
            .iter()
            .skip(1)
            .take(7)
            .enumerate()
            .map(|(index, family)| {
                let layer_palette = palettes.get(index + 1).copied().unwrap_or(palette);
                let alpha = (0.38_f32 - index as f32 * 0.04).max(0.14);
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
) -> Result<RenderedFractus, Box<dyn std::error::Error>> {
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
    let retention = prune_fractus_gallery(&directory, FRACTUS_GALLERY_KEEP);
    let retention_notice = if retention.removed_artifacts == 0
        && retention.removed_orphans == 0
        && retention.warnings.is_empty()
    {
        None
    } else {
        let mut notice = format!(
            "gallery retention removed {} artifact(s) and {} orphan manifest(s)",
            retention.removed_artifacts, retention.removed_orphans
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

/// Keeps the newest bounded window, then removes exact duplicates within it.
///
/// Capping before hashing is intentional. The first deployment may encounter
/// several gigabytes of history; everything older than the newest window is
/// going away regardless, so reading it merely to prove it is duplicate would
/// turn a small retention pass into a multi-gigabyte scan.
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
}
