//! Startup capability report for minimal and strict launch modes.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Debug)]
pub struct Capability {
    pub name: &'static str,
    pub required: bool,
    pub available: bool,
    pub detail: String,
}

#[derive(Clone, Debug)]
pub struct EnvironmentReport {
    pub capabilities: Vec<Capability>,
}

impl EnvironmentReport {
    pub fn ready(&self) -> bool {
        self.capabilities
            .iter()
            .all(|capability| capability.available)
    }

    pub fn render(&self) -> String {
        self.capabilities
            .iter()
            .map(|capability| {
                format!(
                    "{:<14} {:<9} {:<8} {}",
                    capability.name,
                    if capability.available {
                        "AVAILABLE"
                    } else {
                        "DISABLED"
                    },
                    if capability.required {
                        "REQUIRED"
                    } else {
                        "OPTIONAL"
                    },
                    capability.detail
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

pub fn inspect(data: &Path, python: &Path) -> EnvironmentReport {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest.parent().unwrap_or(manifest).to_path_buf();
    let tts = workspace.join("TTS");
    let assets = workspace.join("assets");
    let references = [
        "queen_ref_clean.wav",
        "organist_ref_clean.wav",
        "alchemist_ref_clean.wav",
        "archivist_ref_clean.wav",
        "orator_ref_clean.wav",
        "scribe_ref_clean.wav",
        "artist_ref_clean.wav",
        "diplomat_ref_clean.wav",
        "treasurer_ref_clean.wav",
    ];
    let python_health = Command::new(python)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    let tts_package = python_health
        && python_probe(
            python,
            "import importlib.util,sys; sys.exit(0 if importlib.util.find_spec('zipvoice') else 1)",
        );
    let tts_model_cache = tts_package
        && python_probe(
            python,
            "from huggingface_hub import snapshot_download; snapshot_download('YatharthS/LuxTTS', local_files_only=True); snapshot_download('openai/whisper-base', local_files_only=True); snapshot_download('Systran/faster-whisper-base.en', local_files_only=True)",
        );
    let voice_assets = references.iter().all(|name| assets.join(name).is_file());
    let audio_devices = teledra_tts::output_devices();
    let data_health = std::fs::create_dir_all(data).is_ok_and(|()| {
        let probe = data.join(format!(".write-probe-{}", std::process::id()));
        std::fs::write(&probe, b"teledra")
            .and_then(|()| std::fs::remove_file(probe))
            .is_ok()
    });
    let sidecar_scripts = workspace.join("Sidecars").join("scripts");
    let sidecars_complete = [
        "microphone.py",
        "vision.py",
        "somatic.py",
        "stream_chat.py",
        "restream_listener.py",
        "outreach.py",
        "treasury.py",
        "mcp.py",
        "youtube.py",
    ]
    .iter()
    .all(|script| sidecar_scripts.join(script).is_file());
    let sidecar_packages = python_health
        && python_probe(
            python,
            "import importlib.util,sys; names=('PIL','aiohttp','cv2','faster_whisper','mediapipe','pyaudio','pytesseract','sounddevice','speech_recognition','youtube_transcript_api'); sys.exit(0 if all(importlib.util.find_spec(name) for name in names) else 1)",
        );
    let tesseract = tesseract_executable();

    EnvironmentReport {
        capabilities: vec![
            Capability {
                name: "data",
                required: true,
                available: data_health,
                detail: data.display().to_string(),
            },
            Capability {
                name: "python",
                required: false,
                available: python_health,
                detail: python.display().to_string(),
            },
            Capability {
                name: "TTS worker",
                required: false,
                available: python_health
                    && tts_package
                    && tts_model_cache
                    && tts.join("worker").join("voice_worker.py").is_file()
                    && voice_assets,
                detail: if !tts_package {
                    "zipvoice/LuxTTS is not importable in TELEDRA_PYTHON".to_owned()
                } else if !tts_model_cache {
                    "LuxTTS or Whisper model cache is incomplete; run Setup-Python.ps1".to_owned()
                } else if voice_assets {
                    "worker, model caches, and nine reference voices found".to_owned()
                } else {
                    "one or more reference voices are missing".to_owned()
                },
            },
            Capability {
                name: "audio",
                required: false,
                available: !audio_devices.is_empty(),
                detail: if audio_devices.is_empty() {
                    "no output device found".to_owned()
                } else {
                    audio_devices.join(" | ")
                },
            },
            Capability {
                name: "sidecars",
                required: false,
                available: python_health && sidecars_complete && sidecar_packages,
                detail: if sidecar_packages {
                    sidecar_scripts.display().to_string()
                } else {
                    "one or more packages in Sidecars/requirements-optional.txt are missing"
                        .to_owned()
                },
            },
            Capability {
                name: "vision OCR",
                required: false,
                available: sidecar_packages && tesseract.is_some(),
                detail: tesseract
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "Tesseract was not found; set TELEDRA_TESSERACT".to_owned()),
            },
            Capability {
                name: "music",
                required: false,
                available: !teledra_tts::output_devices().is_empty(),
                detail: "native Court Score renderer".to_owned(),
            },
            Capability {
                name: "Fractus",
                required: false,
                available: data_health,
                detail: "native version-2 raster renderer".to_owned(),
            },
        ],
    }
}

fn python_probe(python: &Path, code: &str) -> bool {
    Command::new(python)
        .args(["-c", code])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn tesseract_executable() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("TELEDRA_TESSERACT").map(PathBuf::from)
        && path.is_file()
    {
        return Some(path);
    }
    #[cfg(windows)]
    {
        let standard = PathBuf::from(r"C:\Program Files\Tesseract-OCR\tesseract.exe");
        if standard.is_file() {
            return Some(standard);
        }
    }
    Command::new("tesseract")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
        .then(|| PathBuf::from("tesseract"))
}
