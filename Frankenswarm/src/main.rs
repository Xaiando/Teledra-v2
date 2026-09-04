use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use teledra_frankenswarm::autopsy;
use teledra_frankenswarm::blackboard::Blackboard;
use teledra_frankenswarm::evaluation;
use teledra_frankenswarm::models;
use teledra_frankenswarm::runtime;
use teledra_frankenswarm::sandbox::{self, SandboxRequest};
use teledra_frankenswarm::tournament;

fn main() {
    if let Err(error) = dispatch(std::env::args().skip(1).collect()) {
        eprintln!("frankenswarm: {error}");
        std::process::exit(1);
    }
}

fn dispatch(arguments: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    let root = root();
    let artifacts = data_root(&root);
    let command = arguments.first().map(String::as_str).unwrap_or("help");
    match command {
        "preflight" => preflight(&root, &artifacts)?,
        "lock-models" => {
            let lock = models::lock_models(
                &root.join("configs/models.yaml"),
                &root.join("configs/models.lock.json"),
            )?;
            println!("locked {} models", lock.models.len());
        }
        "download-models" => {
            let manifest = models::download_models(
                &root.join("configs/models.lock.json"),
                std::env::var_os("HF_HOME").as_deref().map(Path::new),
            )?;
            let output = artifacts.join("models/downloads.json");
            models::write_download_manifest(&manifest, &output)?;
            for model in manifest.models.values() {
                println!("{}", model.snapshot_path.display());
            }
            println!("manifest={}", output.display());
        }
        "autopsy" => {
            let mut report = autopsy::run(
                &root.join("configs/models.lock.json"),
                &artifacts.join("autopsy"),
            )?;
            if arguments.iter().any(|argument| argument == "--weights") {
                autopsy::run_tensor(
                    &mut report,
                    &artifacts.join("models/downloads.json"),
                    &artifacts.join("autopsy"),
                )?;
            }
            println!(
                "autopsy metadata_safe={} merge_safe={} donors={}",
                report.metadata_safe,
                report.merge_safe,
                report.donors.len()
            );
        }
        "eval-baselines" => {
            let endpoint =
                option(&arguments, "--endpoint").unwrap_or_else(|| "http://127.0.0.1:8001/".into());
            let model = option(&arguments, "--model").unwrap_or_else(|| "base".into());
            let report = evaluation::run(&root.join("data/evals"), &endpoint, &model)?;
            let output = artifacts
                .join("evals")
                .join(format!("{}.json", safe_name(&model)));
            evaluation::write_report(&report, &output)?;
            println!(
                "{} score={:.3} report={}",
                model,
                report.score,
                output.display()
            );
        }
        "build-candidate" => {
            let config = required_option(&arguments, "--config")?;
            let seed = required_option(&arguments, "--seed")?.parse::<u64>()?;
            let config = resolve_inside(&root, Path::new(&config))?;
            let directory = config.parent().ok_or("config has no parent")?;
            let candidates = tournament::plan(
                directory,
                &root.join("configs/models.lock.json"),
                &artifacts,
                &[seed],
            )?;
            let canonical = fs::canonicalize(&config)?;
            let candidate = candidates
                .into_iter()
                .find(|candidate| {
                    fs::canonicalize(&candidate.config).ok().as_ref() == Some(&canonical)
                })
                .ok_or("candidate config was not planned")?;
            let candidate = tournament::build(
                candidate,
                arguments.iter().any(|argument| argument == "--execute"),
                &artifacts.join("autopsy/report.json"),
                &artifacts.join("models/downloads.json"),
            )?;
            println!(
                "{} {} -> {}",
                candidate.id,
                candidate.status,
                candidate.output.display()
            );
        }
        "tournament" => {
            let seeds = option(&arguments, "--seeds")
                .map(|raw| {
                    raw.split(',')
                        .filter_map(|value| value.parse().ok())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_else(|| vec![17, 29, 43]);
            let candidates = tournament::plan(
                &root.join("configs/merge"),
                &root.join("configs/models.lock.json"),
                &artifacts,
                &seeds,
            )?;
            println!("planned {} locked candidates", candidates.len());
            if arguments.iter().any(|argument| argument == "--execute") {
                for candidate in candidates {
                    let built = tournament::build(
                        candidate,
                        true,
                        &artifacts.join("autopsy/report.json"),
                        &artifacts.join("models/downloads.json"),
                    )?;
                    println!("built {}", built.id);
                }
            }
        }
        "select-winner" => {
            let minimum = option(&arguments, "--minimum-category-score")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0.5);
            let winner = evaluation::select_winner(&artifacts.join("evals"), minimum)?;
            fs::create_dir_all(&artifacts)?;
            fs::write(
                artifacts.join("winner.json"),
                serde_json::to_string_pretty(&winner)? + "\n",
            )?;
            println!("winner={} score={:.3}", winner.model, winner.score);
        }
        "serve-command" => serve_command(
            &artifacts,
            arguments.iter().any(|argument| argument == "--execute"),
        )?,
        "init-db" => {
            let board = Blackboard::open(database(&artifacts))?;
            println!("database initialized at {}", board.path().display());
        }
        "runtime" => {
            let address = runtime::parse_loopback_address(
                &option(&arguments, "--addr").unwrap_or_else(|| "127.0.0.1:8000".into()),
            )?;
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?
                .block_on(runtime::run(database(&artifacts), address))?;
        }
        "sandbox-build" => {
            let result = sandbox::build_image(
                &root,
                &option(&arguments, "--runtime").unwrap_or_else(|| "podman".into()),
            )?;
            print_result(result.exit_code, &result.stdout, &result.stderr);
        }
        "sandbox" => {
            let workspace = required_option(&arguments, "--workspace")?;
            let separator = arguments
                .iter()
                .position(|argument| argument == "--")
                .ok_or("sandbox requires -- followed by the container command")?;
            let command = arguments[separator + 1..].join(" ");
            let result = sandbox::run(&SandboxRequest {
                workspace: PathBuf::from(workspace),
                command,
                runtime: option(&arguments, "--runtime").unwrap_or_else(|| "podman".into()),
                timeout_seconds: option(&arguments, "--timeout")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(60),
            })?;
            print_result(result.exit_code, &result.stdout, &result.stderr);
            if result.timed_out || result.exit_code != Some(0) {
                std::process::exit(1);
            }
        }
        "demo" => demo(&artifacts)?,
        _ => usage(),
    }
    Ok(())
}

fn root() -> PathBuf {
    std::env::var_os("FRANKENSWARM_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}
fn data_root(source: &Path) -> PathBuf {
    std::env::var_os("FRANKENSWARM_DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .map(|path| path.join("TeledraV2/Frankenswarm"))
        })
        .unwrap_or_else(|| source.join(".runtime"))
}
fn database(data: &Path) -> PathBuf {
    std::env::var_os("FS_DB_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| data.join("blackboard.db"))
}

fn preflight(root: &Path, output: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let checks: [(&str, bool, &str); 8] = [
        (
            "models_config",
            root.join("configs/models.yaml").is_file(),
            "configs/models.yaml",
        ),
        (
            "model_lock",
            root.join("configs/models.lock.json").is_file(),
            "configs/models.lock.json",
        ),
        ("eval_suite", root.join("data/evals").is_dir(), "data/evals"),
        (
            "huggingface_downloader",
            command_available("hf") || command_available("huggingface-cli"),
            "hf or huggingface-cli",
        ),
        (
            "tensor_python",
            python_modules_available(),
            "Python torch + safetensors",
        ),
        ("podman", command_available("podman"), "podman --version"),
        (
            "mergekit",
            command_available("mergekit-yaml"),
            "mergekit-yaml --help",
        ),
        ("nvidia", command_available("nvidia-smi"), "nvidia-smi"),
    ];
    let report = serde_json::json!({"checks": checks.iter().map(|(name,ok,detail)| serde_json::json!({"name":name,"ok":ok,"detail":detail})).collect::<Vec<_>>()});
    fs::create_dir_all(output)?;
    fs::write(
        output.join("preflight.json"),
        serde_json::to_string_pretty(&report)? + "\n",
    )?;
    let markdown = checks
        .iter()
        .map(|(name, ok, detail)| {
            format!(
                "- [{}] **{}** — {}",
                if *ok { "x" } else { " " },
                name,
                detail
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(
        output.join("preflight.md"),
        format!("# FrankenSwarm preflight\n\n{markdown}\n"),
    )?;
    println!("{markdown}");
    Ok(())
}

fn serve_command(artifacts: &Path, execute: bool) -> Result<(), Box<dyn std::error::Error>> {
    let winner: evaluation::EvaluationReport =
        serde_json::from_str(&fs::read_to_string(artifacts.join("winner.json"))?)?;
    let executable = std::env::var("FRANKENSWARM_VLLM").unwrap_or_else(|_| "vllm".into());
    let arguments = [
        "serve",
        winner.model.as_str(),
        "--host",
        "127.0.0.1",
        "--port",
        "8001",
    ];
    println!("{} {}", executable, arguments.join(" "));
    if execute {
        let status = Command::new(executable).args(arguments).status()?;
        if !status.success() {
            return Err(format!("vLLM exited with {status}").into());
        }
    }
    Ok(())
}

fn demo(data: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let board = Blackboard::open(database(data))?;
    let project =
        board.create_project("FrankenSwarm demo", Some("Atomic blackboard demonstration"))?;
    let task = board.create_task(&project.id, "Inspect merge plan", None, &[])?;
    let lease = board.claim_task(&task.id, "planner", Duration::from_secs(900))?;
    board.transition_task(&task.id, "running")?;
    let completed = board.transition_task(&task.id, "done")?;
    println!(
        "project={} task={} lease={} final={}",
        project.id, task.id, lease.agent_role, completed.status
    );
    Ok(())
}

fn command_available(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}
fn python_modules_available() -> bool {
    let python = std::env::var("FRANKENSWARM_PYTHON")
        .or_else(|_| std::env::var("TELEDRA_PYTHON"))
        .unwrap_or_else(|_| "python".into());
    Command::new(python)
        .args(["-c", "import torch, safetensors"])
        .output()
        .is_ok_and(|output| output.status.success())
}
fn option(arguments: &[String], name: &str) -> Option<String> {
    arguments
        .iter()
        .position(|argument| argument == name)
        .and_then(|index| arguments.get(index + 1))
        .cloned()
}
fn required_option(arguments: &[String], name: &str) -> Result<String, Box<dyn std::error::Error>> {
    option(arguments, name).ok_or_else(|| format!("missing {name}").into())
}
fn safe_name(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .take(100)
        .collect()
}
fn resolve_inside(root: &Path, raw: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = fs::canonicalize(if raw.is_absolute() {
        raw.into()
    } else {
        root.join(raw)
    })?;
    if !path.starts_with(fs::canonicalize(root)?) {
        return Err("path escapes FrankenSwarm root".into());
    }
    Ok(path)
}
fn print_result(code: Option<i32>, stdout: &str, stderr: &str) {
    println!("exit={code:?}\n{stdout}");
    if !stderr.is_empty() {
        eprintln!("{stderr}");
    }
}

fn usage() {
    println!(
        "FrankenSwarm v2\n\n  preflight | lock-models | download-models | autopsy [--weights]\n  eval-baselines [--endpoint URL] [--model NAME]\n  build-candidate --config PATH --seed N [--execute]\n  tournament [--seeds 17,29,43] [--execute]\n  select-winner | serve-command [--execute] | init-db\n  runtime [--addr 127.0.0.1:8000] | sandbox-build\n  sandbox --workspace PATH -- COMMAND | demo"
    );
}
