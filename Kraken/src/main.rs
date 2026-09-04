use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use teledra_kraken::hub::{self, HubClient};
use teledra_kraken::queue::{JobQueue, JobStatus};
use teledra_kraken::runner::KrakenRunner;
use teledra_kraken::skills;

fn main() {
    if let Err(error) = dispatch(std::env::args().skip(1).collect()) {
        eprintln!("kraken: {error}");
        std::process::exit(1);
    }
}

fn dispatch(arguments: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    let root = root();
    let workspace = workspace(&root);
    let runner = KrakenRunner::new(&root, &workspace)?;
    let Some(command) = arguments.first().map(String::as_str) else {
        usage();
        std::process::exit(2);
    };
    match command {
        "add" => add(&runner, &arguments[1..])?,
        "run" => {
            let maximum = arguments
                .get(1)
                .and_then(|value| value.parse().ok())
                .unwrap_or(25);
            println!("processed {} terminal job(s)", runner.run(maximum)?);
        }
        "daemon" => daemon(&runner, "kraken-daemon", None)?,
        "worker" => {
            let name = arguments
                .get(1)
                .map(String::as_str)
                .unwrap_or("kraken-worker-1");
            let selected = arguments.get(2).map(PathBuf::from).unwrap_or(workspace);
            let worker = KrakenRunner::new(&root, selected)?;
            daemon(&worker, name, HubClient::from_environment().ok())?;
        }
        "status" => status(runner.queue())?,
        "watch" => watch(runner.queue())?,
        "skills" => {
            for skill in skills::SKILLS {
                println!(
                    "  {:<18} verifier={:<20} timeout={}s children={}",
                    skill.name,
                    skill.verifier.unwrap_or("-"),
                    skill.timeout_seconds,
                    skill.max_children
                );
            }
        }
        "graduate-games" => {
            let input = arguments
                .get(1)
                .cloned()
                .unwrap_or_else(|| "default".into());
            let job = runner.queue().add("graduate_games", input, None, 0)?;
            runner.run(1)?;
            let result = runner
                .queue()
                .get(&job.id)?
                .ok_or("graduation job disappeared")?;
            println!(
                "{} -> {:?}: {}",
                result.id,
                result.status,
                result.output.unwrap_or_default()
            );
            if result.status != JobStatus::Done {
                std::process::exit(1);
            }
        }
        "compact" => {
            runner.queue().compact()?;
            println!("queue compacted");
        }
        _ => {
            usage();
            std::process::exit(2);
        }
    }
    Ok(())
}

fn root() -> PathBuf {
    std::env::var_os("TELEDRA_KRAKEN_ROOT")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .map(|path| path.join("TeledraV2/Kraken"))
        })
        .unwrap_or_else(|| std::env::temp_dir().join("TeledraV2/Kraken"))
}

fn workspace(root: &Path) -> PathBuf {
    std::env::var_os("KRAKEN_WORKSPACE")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("workspace"))
}

fn add(runner: &KrakenRunner, arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if arguments.len() < 2 {
        return Err("usage: kraken add <skill> <input> | --input-file <path> | @<path>".into());
    }
    let skill = &arguments[0];
    if skills::skill(skill).is_none() {
        return Err(format!("unknown skill: {skill}").into());
    }
    let input = if arguments
        .get(1)
        .is_some_and(|value| matches!(value.as_str(), "--input-file" | "-f"))
    {
        fs::read_to_string(arguments.get(2).ok_or("missing input-file path")?)?
    } else if arguments.len() == 2 && arguments[1].starts_with('@') {
        fs::read_to_string(arguments[1].trim_start_matches('@'))?
    } else {
        arguments[1..].join(" ")
    };
    let job = runner.queue().add(skill, input, None, 0)?;
    println!("queued {} -> {}", job.id, job.skill);
    Ok(())
}

fn daemon(
    runner: &KrakenRunner,
    name: &str,
    client: Option<HubClient>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (_lock, pidfile) = worker_lock(runner.queue().root(), name)?;
    let running = Arc::new(AtomicBool::new(true));
    let flag = Arc::clone(&running);
    ctrlc::set_handler(move || flag.store(false, Ordering::SeqCst))?;
    if let Some(client) = &client {
        client.register(name);
        client.signal(name, &format!("{name} activated; workspace worker online"));
        for job in runner.queue().all()?.iter().rev().take(12) {
            client.mirror_job(runner.queue().root(), job, name);
        }
    }
    println!("{name} on duty; Ctrl+C to stop");
    let mut announced = BTreeMap::new();
    while running.load(Ordering::SeqCst) {
        if let Some(client) = &client {
            client.register(name);
            for mission in client.missions() {
                if hub::claim_mission(runner.queue().root(), &mission.id).unwrap_or(false) {
                    let duplicate = runner.queue().all()?.iter().any(|job| {
                        matches!(job.status, JobStatus::Queued | JobStatus::Running)
                            && job.skill == mission.skill
                            && job.input == mission.input
                    });
                    if duplicate {
                        client.signal(
                            name,
                            &format!("skipped duplicate mission {} already in flight", mission.id),
                        );
                        continue;
                    }
                    match runner.queue().add(&mission.skill, &mission.input, None, 0) {
                        Ok(job) => {
                            client.mirror_job(runner.queue().root(), &job, name);
                            client.signal(name, &format!("queued {} -> {}", job.id, job.skill))
                        }
                        Err(error) => client
                            .signal(name, &format!("mission {} rejected: {error}", mission.id)),
                    }
                }
            }
        }
        let processed = runner.run(10)?;
        if let Some(client) = &client {
            for job in runner.queue().all()?.iter().rev().take(12) {
                client.mirror_job(runner.queue().root(), job, name);
                let state = format!("{:?}", job.status).to_ascii_lowercase();
                if announced.get(&job.id) != Some(&state) {
                    announced.insert(job.id.clone(), state.clone());
                    if job.status.terminal() {
                        client.signal(
                            name,
                            &format!(
                                "{} {} -> {}{}",
                                job.id,
                                job.skill,
                                state,
                                job.output
                                    .as_deref()
                                    .map(|output| format!(" ({output})"))
                                    .unwrap_or_default()
                            ),
                        );
                    }
                }
            }
        }
        if processed == 0 {
            thread::sleep(Duration::from_secs(2));
        }
    }
    if let Some(client) = &client {
        client.signal(name, &format!("{name} standing down"));
    }
    let _ = fs::remove_file(pidfile);
    Ok(())
}

fn worker_lock(root: &Path, name: &str) -> Result<(File, PathBuf), Box<dyn std::error::Error>> {
    let directory = root.join("hub");
    fs::create_dir_all(&directory)?;
    let path = hub::pidfile(root, name);
    for _ in 0..2 {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                use std::io::Write;
                writeln!(file, "{}", std::process::id())?;
                return Ok((file, path));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let mut owner = String::new();
                File::open(&path)?.read_to_string(&mut owner)?;
                let pid = owner.trim().parse::<u32>().ok();
                if pid.is_some_and(process_alive) {
                    return Err(format!("{name} is already running (pid {})", owner.trim()).into());
                }
                fs::remove_file(&path)?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    Err(format!("could not acquire worker pidfile for {name}").into())
}

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
        .is_ok_and(|output| {
            String::from_utf8_lossy(&output.stdout).lines().any(|line| {
                line.split(',')
                    .nth(1)
                    .is_some_and(|field| field.trim_matches('"').trim() == pid.to_string())
            })
        })
}

#[cfg(not(windows))]
fn process_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .is_ok_and(|status| status.success())
}

fn status(queue: &JobQueue) -> Result<(), Box<dyn std::error::Error>> {
    let jobs = queue.all()?;
    let mut counts = BTreeMap::new();
    for job in &jobs {
        *counts
            .entry(format!("{:?}", job.status).to_ascii_lowercase())
            .or_insert(0usize) += 1;
    }
    println!(
        "queue: {}",
        if counts.is_empty() {
            "empty".into()
        } else {
            serde_json::to_string(&counts)?
        }
    );
    for job in jobs.iter().rev().take(8).rev() {
        println!(
            "  {} {:<9} {:<18} {}",
            job.id,
            format!("{:?}", job.status).to_ascii_lowercase(),
            job.skill,
            job.input.chars().take(60).collect::<String>()
        );
    }
    Ok(())
}

fn watch(queue: &JobQueue) -> Result<(), Box<dyn std::error::Error>> {
    let running = Arc::new(AtomicBool::new(true));
    let flag = Arc::clone(&running);
    ctrlc::set_handler(move || flag.store(false, Ordering::SeqCst))?;
    while running.load(Ordering::SeqCst) {
        print!("\x1b[2J\x1b[H");
        status(queue)?;
        thread::sleep(Duration::from_millis(1_500));
    }
    Ok(())
}

fn usage() {
    eprintln!(
        "Kraken v2\n\n  kraken add <skill> <input>\n  kraken run [N]\n  kraken daemon\n  kraken worker [name] [workspace]\n  kraken status\n  kraken watch\n  kraken skills\n  kraken graduate-games [inventory]\n  kraken compact"
    );
}
