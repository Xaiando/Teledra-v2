use std::path::PathBuf;
use std::sync::OnceLock;
use serde::{Deserialize, Serialize};

use crate::{AppPaths, EnvironmentReport};

/// The process-wide runtime, installed exactly once during startup after the
/// environment has been validated.
///
/// Deep call sites (background cycles, `spawn_blocking` closures, model-produced
/// action tags) cannot borrow `main`'s locals, and threading `&AppPaths` through
/// every layer is what let launches drift outside the capability gate in the
/// first place. Installing once yields `'static` borrows that any call site can
/// reach through `runtime_context()`.
static APP_PATHS: OnceLock<AppPaths> = OnceLock::new();
static ENVIRONMENT: OnceLock<EnvironmentReport> = OnceLock::new();

/// Installs the runtime. Returns an error if called twice: a second, different
/// environment would silently re-authorize capabilities the first one denied.
pub fn install_runtime(paths: AppPaths, environment: EnvironmentReport) -> Result<(), String> {
    APP_PATHS
        .set(paths)
        .map_err(|_| "AppPaths were already installed".to_string())?;
    ENVIRONMENT
        .set(environment)
        .map_err(|_| "EnvironmentReport was already installed".to_string())?;
    Ok(())
}

pub fn app_paths() -> &'static AppPaths {
    APP_PATHS
        .get()
        .expect("AppPaths accessed before install_runtime(); startup order is wrong")
}

pub fn environment() -> &'static EnvironmentReport {
    ENVIRONMENT
        .get()
        .expect("EnvironmentReport accessed before install_runtime(); startup order is wrong")
}

/// The single accessor every launch site uses.
pub fn runtime_context() -> RuntimeContext<'static> {
    RuntimeContext {
        paths: app_paths(),
        environment: environment(),
    }
}

/// True once the runtime is installed. Lets non-startup paths (unit tests,
/// `--check-environment`) degrade instead of panicking.
pub fn is_installed() -> bool {
    APP_PATHS.get().is_some() && ENVIRONMENT.get().is_some()
}

/// Idempotent installer for unit tests, which share one process and run in
/// parallel. Resolves the real repository root so path-confinement and
/// script-presence checks are exercised as they are in production.
#[cfg(test)]
pub fn test_runtime_context() -> RuntimeContext<'static> {
    APP_PATHS.get_or_init(|| crate::AppPaths::resolve().expect("resolve AppPaths for tests"));
    ENVIRONMENT.get_or_init(crate::EnvironmentReport::all_available_for_tests);
    runtime_context()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "Optional" means a capability may be unavailable at runtime -- not that
    /// its source location is undocumented. A registered sidecar that no clean
    /// checkout contains is a broken install, not an optional feature.
    #[test]
    fn every_registered_sidecar_ships_with_the_repository() {
        let root = &test_runtime_context().paths.root;
        let missing: Vec<String> = ALL_SIDECAR_KINDS
            .iter()
            .map(|kind| (kind, sidecar_spec(*kind)))
            .filter(|(_, spec)| !spec.generated_at_runtime)
            .filter(|(_, spec)| !root.join(spec.relative_script).is_file())
            .map(|(kind, spec)| format!("{:?} -> {}", kind, spec.relative_script))
            .collect();
        assert!(
            missing.is_empty(),
            "registered sidecars absent from the checkout:\n  {}",
            missing.join("\n  ")
        );
    }

    /// The gate is the launcher, not the caller's memory. A disabled capability
    /// must fail before a process is ever constructed.
    #[test]
    fn a_disabled_capability_cannot_produce_a_command() {
        let paths = test_runtime_context().paths;
        let mut environment = crate::EnvironmentReport::all_available_for_tests();
        environment.hearing = crate::Capability::Disabled {
            reason: "Disabled in minimal mode".to_string(),
        };
        let context = RuntimeContext {
            paths,
            environment: &environment,
        };

        let error = sync_python_sidecar_command(&context, SidecarKind::Hearing)
            .err()
            .expect("a disabled capability must not yield a command");
        match error {
            SidecarError::Disabled { capability, reason } => {
                assert_eq!(capability, CapabilityId::Hearing);
                assert!(reason.contains("minimal"), "reason must be actionable: {reason}");
            }
            other => panic!("expected a Disabled error, got: {other}"),
        }

        // A different capability on the same report is unaffected.
        assert!(sync_python_sidecar_command(&context, SidecarKind::Vision).is_ok());
    }

    /// Restream is network/chat ingestion. If it inherited Hearing, enabling a
    /// microphone would silently authorize an external network listener.
    #[test]
    fn restream_does_not_inherit_the_microphone_capability() {
        assert_eq!(
            sidecar_spec(SidecarKind::Restream).capability,
            CapabilityId::StreamingChat
        );
        assert_eq!(
            sidecar_spec(SidecarKind::Hearing).capability,
            CapabilityId::Hearing
        );
    }

    /// Every command is rooted explicitly rather than inheriting the process
    /// working directory, and carries an absolute script path.
    #[test]
    fn launched_commands_are_rooted_and_absolute() {
        let context = test_runtime_context();
        let command = sync_python_sidecar_command(&context, SidecarKind::Voice)
            .expect("voice is available in the test environment");
        assert_eq!(command.get_current_dir(), Some(context.paths.root.as_path()));
        let script = command
            .get_args()
            .next()
            .expect("the script is passed as the first argument");
        assert!(
            std::path::Path::new(script).is_absolute(),
            "script argument must be absolute, got: {}",
            script.to_string_lossy()
        );
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum CapabilityId {
    Voice,
    Somatic,
    Hearing,
    Vision,
    StreamingChat,
    Art,
    Dreaming,
    MemorySearch,
    MusicAuthoring,
    MusicWorkshop,
    NetworkResearch,
    Outreach,
    Mcp,
    TreasuryNetwork,
    /// Executing model-generated experiment code through tools/workshop_runner.py.
    /// Distinct from Art: this runs arbitrary generated Python, so a text-only
    /// profile must be able to deny it without also denying the art window.
    WorkshopTools,
    /// Operator-invoked, read-only local viewers (`/dashboard`, `/work`).
    /// Deliberately not mode-gated: a minimal profile still permits explicitly
    /// requested pure-local helpers. It only requires an interpreter.
    OperatorTools,
}

impl std::fmt::Display for CapabilityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SidecarKind {
    Voice,
    Somatic,
    Hearing,
    Vision,
    Restream,
    Art,
    Dream,
    Memory,
    MusicWorkshop,
    Research,
    Outreach,
    Mcp,
    TreasuryNetwork,
    Dynamic(CapabilityId, &'static str),
}

pub struct SidecarSpec {
    pub capability: CapabilityId,
    pub relative_script: &'static str,
    pub long_running: bool,
    /// True when the court writes this script itself before launching it, so it
    /// is legitimately absent from a clean checkout (`art.py`, like `music.py`,
    /// is model-authored). Everything else must ship with the repository.
    pub generated_at_runtime: bool,
}

/// Every statically-known sidecar. `Dynamic` is deliberately excluded: its
/// script is supplied at the call site and cannot be enumerated here.
pub const ALL_SIDECAR_KINDS: &[SidecarKind] = &[
    SidecarKind::Voice,
    SidecarKind::Somatic,
    SidecarKind::Hearing,
    SidecarKind::Vision,
    SidecarKind::Restream,
    SidecarKind::Art,
    SidecarKind::Dream,
    SidecarKind::Memory,
    SidecarKind::MusicWorkshop,
    SidecarKind::Research,
    SidecarKind::Outreach,
    SidecarKind::Mcp,
    SidecarKind::TreasuryNetwork,
];

pub fn sidecar_spec(kind: SidecarKind) -> SidecarSpec {
    match kind {
        SidecarKind::Voice => SidecarSpec { capability: CapabilityId::Voice, relative_script: "generate_voice.py", long_running: true, generated_at_runtime: false },
        SidecarKind::Somatic => SidecarSpec { capability: CapabilityId::Somatic, relative_script: "somatic_cortex_stream.py", long_running: true, generated_at_runtime: false },
        SidecarKind::Hearing => SidecarSpec { capability: CapabilityId::Hearing, relative_script: "copilot_mic.py", long_running: true, generated_at_runtime: false },
        SidecarKind::Vision => SidecarSpec { capability: CapabilityId::Vision, relative_script: "copilot_vision.py", long_running: true, generated_at_runtime: false },
        SidecarKind::Restream => SidecarSpec { capability: CapabilityId::StreamingChat, relative_script: "restream_listener.py", long_running: true, generated_at_runtime: false },
        SidecarKind::Art => SidecarSpec { capability: CapabilityId::Art, relative_script: "art.py", long_running: false, generated_at_runtime: true },
        SidecarKind::Dream => SidecarSpec { capability: CapabilityId::Dreaming, relative_script: "dream.py", long_running: true, generated_at_runtime: false },
        SidecarKind::Memory => SidecarSpec { capability: CapabilityId::MemorySearch, relative_script: "retrieve_memory.py", long_running: false, generated_at_runtime: false },
        SidecarKind::MusicWorkshop => SidecarSpec { capability: CapabilityId::MusicWorkshop, relative_script: "court_synth/workshop.py", long_running: false, generated_at_runtime: false },
        SidecarKind::Research => SidecarSpec { capability: CapabilityId::NetworkResearch, relative_script: "get_youtube_transcript.py", long_running: false, generated_at_runtime: false },
        SidecarKind::Outreach => SidecarSpec { capability: CapabilityId::Outreach, relative_script: "outreach_poster.py", long_running: false, generated_at_runtime: false },
        SidecarKind::Mcp => SidecarSpec { capability: CapabilityId::Mcp, relative_script: "mcp_bridge.py", long_running: true, generated_at_runtime: false },
        SidecarKind::TreasuryNetwork => SidecarSpec { capability: CapabilityId::TreasuryNetwork, relative_script: "treasury_scout.py", long_running: false, generated_at_runtime: false },
        SidecarKind::Dynamic(capability, relative_script) => SidecarSpec { capability, relative_script, long_running: false, generated_at_runtime: false },
    }
}

/// Copy so a launch site deep in a loop or closure never has to think about
/// ownership of the gate itself.
#[derive(Clone, Copy)]
pub struct RuntimeContext<'a> {
    pub paths: &'a AppPaths,
    pub environment: &'a EnvironmentReport,
}

#[derive(Debug)]
pub enum SidecarError {
    Disabled { capability: CapabilityId, reason: String },
    PythonUnavailable,
    ScriptMissing(PathBuf),
}

impl std::fmt::Display for SidecarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SidecarError::Disabled { capability, reason } => write!(f, "Capability {:?} is disabled: {}", capability, reason),
            SidecarError::PythonUnavailable => write!(f, "Python environment is unavailable"),
            SidecarError::ScriptMissing(path) => write!(f, "Script missing: {}", path.display()),
        }
    }
}

impl std::error::Error for SidecarError {}

pub enum SidecarOutcome<T> {
    Started(T),
    Disabled { reason: String },
}

pub fn tokio_python_sidecar_command(
    context: &RuntimeContext,
    kind: SidecarKind,
) -> Result<tokio::process::Command, SidecarError> {
    let spec = sidecar_spec(kind);
    
    context.environment.require(spec.capability)?;

    let python = context
        .paths
        .python
        .as_ref()
        .ok_or(SidecarError::PythonUnavailable)?;

    let script = context.paths.root.join(spec.relative_script);

    if !script.is_file() {
        return Err(SidecarError::ScriptMissing(script));
    }

    let mut command = tokio::process::Command::new(python);
    command
        .arg(script)
        .current_dir(&context.paths.root);

    Ok(command)
}

/// Python for an inline `-c` / `-m` invocation that has no script file of its
/// own. Still gated and still rooted: the caller supplies only the arguments.
pub fn sync_python_inline_command(
    context: &RuntimeContext,
    capability: CapabilityId,
) -> Result<std::process::Command, SidecarError> {
    context.environment.require(capability)?;

    let python = context
        .paths
        .python
        .as_ref()
        .ok_or(SidecarError::PythonUnavailable)?;

    let mut command = std::process::Command::new(python);
    command.current_dir(&context.paths.root);
    Ok(command)
}

pub fn sync_python_sidecar_command(
    context: &RuntimeContext,
    kind: SidecarKind,
) -> Result<std::process::Command, SidecarError> {
    let spec = sidecar_spec(kind);
    
    context.environment.require(spec.capability)?;

    let python = context
        .paths
        .python
        .as_ref()
        .ok_or(SidecarError::PythonUnavailable)?;

    let script = context.paths.root.join(spec.relative_script);

    if !script.is_file() {
        return Err(SidecarError::ScriptMissing(script));
    }

    let mut command = std::process::Command::new(python);
    command
        .arg(script)
        .current_dir(&context.paths.root);

    Ok(command)
}
