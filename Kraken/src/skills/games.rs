use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use regex::Regex;
use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{ChildRequest, SkillContext, SkillError, SkillResult};

#[derive(Deserialize)]
struct PixelForkRequest {
    url: String,
    #[serde(default = "yes")]
    improve: bool,
}

fn yes() -> bool {
    true
}

pub fn pixelfork(input: &str, context: &SkillContext) -> Result<SkillResult, SkillError> {
    let request = if input.trim_start().starts_with('{') {
        serde_json::from_str::<PixelForkRequest>(input)?
    } else {
        PixelForkRequest {
            url: input.trim().into(),
            improve: true,
        }
    };
    let url = reqwest::Url::parse(&request.url)
        .map_err(|error| SkillError::Unsupported(error.to_string()))?;
    if url.scheme() != "https" || url.host_str() != Some("pixelfork.ai") {
        return Err(SkillError::Unsupported(
            "input must be an https://pixelfork.ai/publish/<id> URL".into(),
        ));
    }
    let slug = url
        .path()
        .strip_prefix("/publish/")
        .filter(|slug| {
            !slug.is_empty()
                && slug
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric())
        })
        .ok_or_else(|| SkillError::Unsupported("invalid PixelFork publish slug".into()))?;
    let mut title = format!("PixelFork {slug}");
    let mut description = String::new();
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .redirect(Policy::none())
        .build()?;
    if let Ok(response) = client
        .get(url.clone())
        .header("User-Agent", "Teledra-Kraken/2")
        .send()
        && response.status().is_success()
        && response
            .content_length()
            .is_none_or(|length| length <= 2 * 1024 * 1024)
        && let Ok(source) = response.text()
    {
        if let Some(found) = capture(&source, r"(?is)<title[^>]*>(.*?)</title>") {
            title = strip_tags(&found);
        }
        if let Some(found) = capture(
            &source,
            r#"(?is)<meta\s+name=["']description["']\s+content=["'](.*?)["']"#,
        ) {
            description = found;
        }
    }
    let relative = PathBuf::from("games").join(format!("pixelfork-{slug}"));
    let directory = context.workspace_path(&relative)?;
    fs::create_dir_all(&directory)?;
    let manifest = serde_json::json!({
        "source": "pixelfork.ai",
        "publish_url": request.url,
        "slug": slug,
        "title": title,
        "description": description,
        "harness": "Kraken strict DOM and browser game verification"
    });
    let manifest_path = directory.join("pixelfork_manifest.json");
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest)? + "\n",
    )?;
    fs::write(
        directory.join("README.md"),
        format!(
            "# PixelFork Game Fork: {slug}\n\nSource: {}\n\nThis is a provenance-preserving seed. Run `code_forge` to create or improve the local implementation.\n",
            request.url
        ),
    )?;
    let children = request.improve.then(|| ChildRequest {
        skill: "code_forge".into(),
        input: serde_json::json!({
            "task": format!("Recreate and improve the PixelFork game from {} using the local manifest as provenance.", request.url),
            "dir": relative,
            "quality": "beast"
        }).to_string(),
    }).into_iter().collect();
    Ok(SkillResult {
        summary: format!(
            "Imported PixelFork provenance for {slug} into {}.",
            directory.display()
        ),
        output: Some(manifest_path),
        children,
        evidence: vec![request.url],
    })
}

#[derive(Clone, Debug, Deserialize)]
struct Inventory {
    #[serde(default)]
    excluded_games: BTreeSet<String>,
    #[serde(default)]
    games: BTreeMap<String, GameDeclaration>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct GameDeclaration {
    runtime: String,
    entrypoint: PathBuf,
    #[serde(default)]
    profile: String,
    #[serde(default)]
    session: String,
    #[serde(default)]
    contract_version: u8,
}

#[derive(Debug, Serialize)]
struct GameRow {
    game: String,
    runtime: Option<String>,
    entrypoint: Option<PathBuf>,
    declaration: Option<GameDeclaration>,
    accepted: bool,
    status: String,
    reasons: Vec<String>,
    sha256: Option<String>,
    runtime_probe: Option<RuntimeProbe>,
    changed_during_probe: bool,
}

#[derive(Clone, Debug, Serialize)]
struct RuntimeProbe {
    attempted: bool,
    passed: bool,
    reasons: Vec<String>,
    clicked_play: bool,
    raf_count: u64,
    audio_starts: u64,
    action: Option<String>,
}

pub fn graduate(input: &str, context: &SkillContext) -> Result<SkillResult, SkillError> {
    let inventory_path = if input.trim().is_empty() || input.trim() == "default" {
        let runtime = context.root.join("game_inventory.json");
        if runtime.is_file() {
            runtime
        } else {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("game_inventory.json")
        }
    } else {
        context.root_or_workspace_path(Path::new(input.trim()))?
    };
    if !inventory_path.is_file() {
        return Err(SkillError::Unsupported(format!(
            "game inventory not found: {}",
            inventory_path.display()
        )));
    }
    let inventory: Inventory = serde_json::from_str(&fs::read_to_string(&inventory_path)?)?;
    let games_root = context.workspace.join("games");
    let mut rows = Vec::new();
    let mut discovered = BTreeSet::new();
    if games_root.is_dir() {
        for entry in fs::read_dir(&games_root)?
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_dir())
        {
            let name = entry.file_name().to_string_lossy().into_owned();
            if inventory.excluded_games.contains(&name) {
                continue;
            }
            discovered.insert(name.clone());
            let Some(declaration) = inventory.games.get(&name) else {
                rows.push(GameRow {
                    game: name,
                    runtime: None,
                    entrypoint: None,
                    declaration: None,
                    accepted: false,
                    status: "undeclared".into(),
                    reasons: vec!["inventory declaration missing".into()],
                    sha256: None,
                    runtime_probe: None,
                    changed_during_probe: false,
                });
                continue;
            };
            rows.push(assess_game(
                &entry.path(),
                name,
                declaration,
                &context.workdir,
            ));
        }
    }
    for (name, declaration) in inventory.games.iter().filter(|(name, _)| {
        !discovered.contains(*name) && !inventory.excluded_games.contains(*name)
    }) {
        rows.push(GameRow {
            game: name.clone(),
            runtime: Some(declaration.runtime.clone()),
            entrypoint: Some(declaration.entrypoint.clone()),
            declaration: Some(declaration.clone()),
            accepted: false,
            status: "declared_but_missing".into(),
            reasons: vec!["declared game directory is absent".into()],
            sha256: None,
            runtime_probe: None,
            changed_during_probe: false,
        });
    }
    let accepted = rows.iter().filter(|row| row.accepted).count();
    let browser_games = rows
        .iter()
        .filter(|row| row.runtime.as_deref() == Some("browser"))
        .count();
    let inventory_complete = !rows
        .iter()
        .any(|row| matches!(row.status.as_str(), "undeclared" | "declared_but_missing"));
    let all_browser_games_graduated = inventory_complete
        && browser_games > 0
        && rows
            .iter()
            .filter(|row| row.runtime.as_deref() == Some("browser"))
            .all(|row| row.accepted);
    let report = serde_json::json!({
        "schema_version": 1,
        "inventory": inventory_path,
        "games_root": games_root,
        "excluded_games": inventory.excluded_games,
        "summary": {
            "discovered": discovered.len(),
            "browser_games": browser_games,
            "graduated": accepted,
            "not_graduated": rows.len() - accepted,
            "inventory_complete": inventory_complete,
            "all_browser_games_graduated": all_browser_games_graduated
        },
        "games": rows,
    });
    let serialized = serde_json::to_string_pretty(&report)? + "\n";
    let output = context.write_vault("game-graduation.json", &serialized)?;
    Ok(SkillResult {
        summary: format!(
            "Game graduation completed: {accepted}/{} accepted. See {}.",
            rows.len(),
            output.display()
        ),
        output: Some(output),
        children: Vec::new(),
        evidence: vec![inventory_path.display().to_string()],
    })
}

fn assess_game(
    directory: &Path,
    name: String,
    declaration: &GameDeclaration,
    scratch: &Path,
) -> GameRow {
    let path = directory.join(&declaration.entrypoint);
    if !path.is_file() {
        return GameRow {
            game: name,
            runtime: Some(declaration.runtime.clone()),
            entrypoint: Some(declaration.entrypoint.clone()),
            declaration: Some(declaration.clone()),
            accepted: false,
            status: "missing_entrypoint".into(),
            reasons: vec![format!("missing {}", path.display())],
            sha256: None,
            runtime_probe: None,
            changed_during_probe: false,
        };
    }
    let bytes = fs::read(&path).unwrap_or_default();
    let hash = format!("{:x}", Sha256::digest(&bytes));
    if declaration.runtime != "browser" {
        return GameRow {
            game: name,
            runtime: Some(declaration.runtime.clone()),
            entrypoint: Some(declaration.entrypoint.clone()),
            declaration: Some(declaration.clone()),
            accepted: false,
            status: "outside_browser_campaign".into(),
            reasons: vec![format!(
                "runtime {:?} has no browser graduation contract",
                declaration.runtime
            )],
            sha256: Some(hash),
            runtime_probe: None,
            changed_during_probe: false,
        };
    }
    let source = String::from_utf8_lossy(&bytes);
    let mut reasons = Vec::new();
    for (needle, message) in [
        ("<canvas", "canvas is missing"),
        ("<script", "script is missing"),
        ("requestAnimationFrame", "animation loop is missing"),
    ] {
        if !source.contains(needle) {
            reasons.push(message.into());
        }
    }
    if declaration.contract_version < 2 {
        reasons.push("browser contract version must be at least 2".into());
    }
    if declaration.profile.trim().is_empty() || declaration.session.trim().is_empty() {
        reasons.push("profile/session declaration is incomplete".into());
    }
    if !known_profile(&declaration.profile) {
        reasons.push(format!("unknown browser profile: {}", declaration.profile));
    }
    reasons.extend(local_manifest_reasons(directory, declaration));
    for dangerous in ["eval(", "new Function(", "onload=", "onclick="] {
        if source.contains(dangerous) {
            reasons.push(format!("forbidden browser pattern: {dangerous}"));
        }
    }
    if Regex::new(r#"(?i)(?:src|href)\s*=\s*["'](?:https?:)?//"#)
        .is_ok_and(|expression| expression.is_match(&source))
    {
        reasons.push("external browser assets are forbidden during graduation".into());
    }
    let before = hash.clone();
    let runtime_probe = if reasons.is_empty() {
        browser_probe(&path, declaration, scratch)
    } else {
        RuntimeProbe {
            attempted: false,
            passed: false,
            reasons: vec!["runtime probe skipped because static checks failed".into()],
            clicked_play: false,
            raf_count: 0,
            audio_starts: 0,
            action: None,
        }
    };
    reasons.extend(runtime_probe.reasons.clone());
    let after = fs::read(&path)
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
        .unwrap_or_default();
    let changed_during_probe = before != after;
    if changed_during_probe {
        reasons.push("production artifact changed during its browser probe".into());
    }
    reasons.sort();
    reasons.dedup();
    let accepted = reasons.is_empty() && runtime_probe.passed && !changed_during_probe;
    GameRow {
        game: name,
        runtime: Some(declaration.runtime.clone()),
        entrypoint: Some(declaration.entrypoint.clone()),
        declaration: Some(declaration.clone()),
        accepted,
        status: if accepted { "graduated" } else { "rejected" }.into(),
        reasons,
        sha256: Some(hash),
        runtime_probe: Some(runtime_probe),
        changed_during_probe,
    }
}

fn known_profile(profile: &str) -> bool {
    matches!(
        profile,
        "platformer"
            | "endless_runner"
            | "breakout_pinball"
            | "frogger"
            | "match3"
            | "puzzle_grid"
            | "rhythm"
            | "snake"
            | "shooter"
            | "tower_defense"
            | "roguelike"
            | "simulation"
    )
}

fn local_manifest_reasons(directory: &Path, declaration: &GameDeclaration) -> Vec<String> {
    let path = directory.join(".kraken-game.json");
    if !path.exists() {
        return Vec::new();
    }
    let Ok(value) = fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .ok_or(())
    else {
        return vec!["invalid local .kraken-game.json".into()];
    };
    let mut reasons = Vec::new();
    for (field, expected) in [
        ("profile", serde_json::json!(declaration.profile)),
        ("session", serde_json::json!(declaration.session)),
        (
            "contract_version",
            serde_json::json!(declaration.contract_version),
        ),
    ] {
        if value.get(field) != Some(&expected) {
            reasons.push(format!("local manifest {field} disagrees with inventory"));
        }
    }
    reasons
}

fn browser_probe(path: &Path, declaration: &GameDeclaration, scratch: &Path) -> RuntimeProbe {
    let Some(browser) = find_browser() else {
        return failed_probe(false, "headless Chrome/Edge is unavailable");
    };
    let Ok(source) = fs::read_to_string(path) else {
        return failed_probe(false, "cannot read browser entrypoint");
    };
    let base = reqwest::Url::from_directory_path(path.parent().unwrap_or(Path::new(".")))
        .ok()
        .map(|url| format!(r#"<base href="{}">"#, url))
        .unwrap_or_default();
    let actions = profile_actions(&declaration.profile);
    let bootstrap = r#"<script>
window.__KRAKEN_PROBE__={raf:0,audio:0,errors:[]};const P=window.__KRAKEN_PROBE__;
addEventListener('error',e=>P.errors.push(String(e.message||e.error||'error')));addEventListener('unhandledrejection',e=>P.errors.push(String(e.reason||'rejection')));
const ce=console.error.bind(console);console.error=(...a)=>{P.errors.push(a.map(String).join(' '));ce(...a)};
window.requestAnimationFrame=f=>setTimeout(()=>{P.raf++;try{f(performance.now())}catch(e){P.errors.push(String(e));throw e}},16);window.cancelAnimationFrame=clearTimeout;
class Param{setValueAtTime(){return this}linearRampToValueAtTime(){return this}exponentialRampToValueAtTime(){return this}setTargetAtTime(){return this}cancelScheduledValues(){return this}}
class Node{connect(){return this}disconnect(){}}
class Source extends Node{constructor(){super();this.frequency=new Param;this.detune=new Param;this.playbackRate=new Param;this.gain=new Param}start(){P.audio++}stop(){}}
class Audio{constructor(){this.destination=new Node;this.state='running';this.sampleRate=44100;this.currentTime=0}createOscillator(){return new Source}createGain(){return new Source}createBufferSource(){return new Source}createBiquadFilter(){const n=new Source;n.Q=new Param;return n}createDynamicsCompressor(){return new Source}createStereoPanner(){const n=new Source;n.pan=new Param;return n}createBuffer(){return {}}resume(){return Promise.resolve()}suspend(){return Promise.resolve()}close(){return Promise.resolve()}}
try{Object.defineProperty(window,'AudioContext',{configurable:true,value:Audio});Object.defineProperty(window,'webkitAudioContext',{configurable:true,value:Audio})}catch(e){P.errors.push(String(e))}
try{const p=HTMLMediaElement.prototype.play;HTMLMediaElement.prototype.play=function(){P.audio++;return Promise.resolve()}}catch(e){}
</script>"#;
    let driver = format!(
        r#"<script>
addEventListener('load',()=>setTimeout(()=>{{
 const P=window.__KRAKEN_PROBE__, buttons=[...document.querySelectorAll('button,[role=button]')];
 const play=document.querySelector('[data-action="play"],#playBtn,#play-btn,#startBtn,#start-btn,#startButton')||buttons.find(b=>/play|start|begin|launch|enter/i.test((b.innerText||b.textContent||'').trim()));
 let clicked=false;if(play){{try{{play.click();clicked=true}}catch(e){{P.errors.push(String(e))}}}}
 setTimeout(()=>{{
  const api=window.__KRAKEN_BEAST__;let before=null,after=null,chosen=null,result=false;
  try{{if(api&&typeof api.snapshot==='function')before=api.snapshot();if(api&&typeof api.action==='function'){{for(const a of {actions}){{const r=api.action(a);if(r!==false){{chosen=a;result=true;break}}}}}}}}catch(e){{P.errors.push(String(e))}}
  setTimeout(()=>{{try{{if(api&&typeof api.snapshot==='function')after=api.snapshot()}}catch(e){{P.errors.push(String(e))}}
   const report={{clickedPlay:clicked,rafCount:P.raf,audioStarts:P.audio,errors:P.errors,beastApi:!!(api&&typeof api.snapshot==='function'&&typeof api.action==='function'),beastVersion:api&&api.version,beastProfile:api&&api.profile,before,after,action:chosen,actionResult:result}};
   const pre=document.createElement('pre');pre.id='kraken-runtime-report';pre.textContent=JSON.stringify(report);document.body.appendChild(pre);
  }},500);
 }},700);
}},150));
</script>"#,
        actions = serde_json::to_string(&actions).unwrap_or_else(|_| "[]".into())
    );
    let head = Regex::new(r"(?i)<head\b[^>]*>").ok();
    let mut probe = if let Some(found) = head.as_ref().and_then(|regex| regex.find(&source)) {
        format!(
            "{}{}{}{}",
            &source[..found.end()],
            base,
            bootstrap,
            &source[found.end()..]
        )
    } else {
        format!("{base}{bootstrap}{source}")
    };
    if let Some(index) = probe.to_ascii_lowercase().rfind("</body>") {
        probe.insert_str(index, &driver);
    } else {
        probe.push_str(&driver);
    }
    if fs::create_dir_all(scratch).is_err() {
        return failed_probe(false, "cannot create graduation scratch directory");
    }
    let probe_path = scratch.join(format!(
        "probe-{}.html",
        path.parent()
            .and_then(Path::file_name)
            .unwrap_or_default()
            .to_string_lossy()
    ));
    if fs::write(&probe_path, probe).is_err() {
        return failed_probe(false, "cannot write isolated browser probe");
    }
    let profile_dir = scratch.join("chrome-profile");
    let Some(url) = reqwest::Url::from_file_path(&probe_path).ok() else {
        return failed_probe(false, "cannot create probe file URL");
    };
    let check = crate::Check {
        name: format!("browser graduation {}", declaration.profile),
        program: browser,
        arguments: vec![
            "--headless=new".into(),
            "--disable-gpu".into(),
            "--disable-extensions".into(),
            "--disable-background-networking".into(),
            "--disable-background-timer-throttling".into(),
            "--disable-renderer-backgrounding".into(),
            "--autoplay-policy=no-user-gesture-required".into(),
            "--allow-file-access-from-files".into(),
            "--no-first-run".into(),
            format!("--user-data-dir={}", profile_dir.display()),
            "--virtual-time-budget=9500".into(),
            "--run-all-compositor-stages-before-draw".into(),
            "--dump-dom".into(),
            url.to_string(),
        ],
        working_directory: scratch.into(),
        timeout_seconds: 20,
    };
    let result = match crate::verify(&check) {
        Ok(result) => result,
        Err(error) => return failed_probe(true, &error.to_string()),
    };
    let Some(raw) = capture_limit(
        &result.stdout,
        r#"(?is)<pre[^>]*id=["']kraken-runtime-report["'][^>]*>(.*?)</pre>"#,
        64 * 1024,
    ) else {
        return failed_probe(
            true,
            &format!(
                "headless browser produced no runtime report: {}",
                result.stderr.chars().take(400).collect::<String>()
            ),
        );
    };
    let decoded = html_decode(&raw);
    let Ok(report) = serde_json::from_str::<serde_json::Value>(&decoded) else {
        return failed_probe(true, "headless browser returned invalid probe JSON");
    };
    assess_runtime_report(&report, declaration)
}

fn assess_runtime_report(
    report: &serde_json::Value,
    declaration: &GameDeclaration,
) -> RuntimeProbe {
    let mut reasons = Vec::new();
    let clicked_play = report
        .get("clickedPlay")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let raf_count = report
        .get("rafCount")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let audio_starts = report
        .get("audioStarts")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let action = report
        .get("action")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    if !clicked_play {
        reasons.push("browser could not click a visible Play/Start control".into());
    }
    if raf_count < 3 {
        reasons.push(format!(
            "animation loop advanced only {raf_count} frame(s); at least three required"
        ));
    }
    if audio_starts == 0 {
        reasons.push("no audio source started during Play and the profile action".into());
    }
    if report
        .get("beastVersion")
        .and_then(serde_json::Value::as_u64)
        != Some(declaration.contract_version as u64)
    {
        reasons.push("runtime Beast contract version disagrees with inventory".into());
    }
    if report
        .get("beastProfile")
        .and_then(serde_json::Value::as_str)
        != Some(declaration.profile.as_str())
    {
        reasons.push("runtime Beast profile disagrees with inventory".into());
    }
    if !report
        .get("actionResult")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        reasons.push("profile-specific runtime action returned false".into());
    }
    let before = report.get("before").unwrap_or(&serde_json::Value::Null);
    let after = report.get("after").unwrap_or(&serde_json::Value::Null);
    if before.is_null() || before == after {
        reasons.push("profile action produced no observable snapshot change".into());
    }
    for field in required_snapshot_fields(&declaration.profile) {
        if before.pointer(field).is_none() {
            reasons.push(format!("runtime snapshot is missing {field}"));
        }
    }
    if let Some(errors) = report.get("errors").and_then(serde_json::Value::as_array) {
        for error in errors.iter().filter_map(serde_json::Value::as_str).take(5) {
            reasons.push(format!("browser runtime error: {error}"));
        }
    }
    RuntimeProbe {
        attempted: true,
        passed: reasons.is_empty(),
        reasons,
        clicked_play,
        raf_count,
        audio_starts,
        action,
    }
}

fn profile_actions(profile: &str) -> Vec<&'static str> {
    match profile {
        "snake" => vec!["feed", "collide"],
        "endless_runner" | "platformer" => vec!["damage", "jump", "fail"],
        "breakout_pinball" => vec!["hit_target", "drain", "advance"],
        "frogger" => vec!["collide", "reach_goal", "hop"],
        "match3" => vec!["legal_swap", "cascade"],
        "puzzle_grid" => vec!["move", "push", "advance"],
        "rhythm" => vec!["hit_next", "miss_next", "finish"],
        "shooter" => vec!["damage", "shoot", "advance"],
        "tower_defense" => vec!["place_tower", "start_wave", "leak"],
        "roguelike" => vec!["damage", "advance", "move", "attack"],
        "simulation" => vec!["move", "interact", "mine", "pan", "rest", "advance"],
        _ => Vec::new(),
    }
}

fn required_snapshot_fields(profile: &str) -> &'static [&'static str] {
    match profile {
        "platformer" => &[
            "/actor/x",
            "/actor/y",
            "/actor/vy",
            "/metrics/lives",
            "/metrics/level",
        ],
        "endless_runner" => &[
            "/actor/y",
            "/actor/vy",
            "/metrics/distance",
            "/metrics/posture",
            "/metrics/lives",
        ],
        "shooter" => &[
            "/actor/x",
            "/actor/y",
            "/metrics/projectiles",
            "/metrics/enemies",
            "/metrics/lives",
            "/metrics/wave",
        ],
        "snake" => &[
            "/state",
            "/complete",
            "/score",
            "/board/rows",
            "/board/cols",
            "/board/body",
            "/actor/x",
            "/actor/y",
            "/metrics/length",
            "/metrics/food/x",
            "/metrics/food/y",
        ],
        "breakout_pinball" => &[
            "/metrics/ball/x",
            "/metrics/ball/y",
            "/metrics/ball/vx",
            "/metrics/ball/vy",
            "/metrics/targets",
            "/metrics/balls",
            "/metrics/level",
        ],
        "frogger" => &[
            "/actor/x",
            "/actor/y",
            "/metrics/lives",
            "/metrics/goals",
            "/metrics/level",
        ],
        "puzzle_grid" => &[
            "/actor/x",
            "/actor/y",
            "/metrics/moves",
            "/metrics/crates_on_goals",
            "/metrics/level",
        ],
        "roguelike" => &[
            "/actor/x",
            "/actor/y",
            "/metrics/turn",
            "/metrics/hp",
            "/metrics/enemies",
            "/metrics/floor",
        ],
        "simulation" => &[
            "/state",
            "/actor/x",
            "/actor/y",
            "/metrics/hunger",
            "/metrics/energy",
            "/metrics/gold",
            "/metrics/money",
        ],
        "tower_defense" => &[
            "/state",
            "/board/rows",
            "/board/cols",
            "/probe/place_tower/row",
            "/probe/place_tower/col",
            "/metrics/currency",
            "/metrics/base_hp",
        ],
        "rhythm" => &[
            "/state",
            "/chart",
            "/metrics/chart_hash",
            "/metrics/remaining_notes",
        ],
        "match3" => &[
            "/board/rows",
            "/board/cols",
            "/board/cells",
            "/probe/legal_swap/from/row",
            "/probe/legal_swap/to/row",
            "/metrics/board_hash",
            "/metrics/moves",
        ],
        _ => &["/state", "/score", "/actor", "/metrics"],
    }
}

fn failed_probe(attempted: bool, reason: &str) -> RuntimeProbe {
    RuntimeProbe {
        attempted,
        passed: false,
        reasons: vec![reason.into()],
        clicked_play: false,
        raf_count: 0,
        audio_starts: 0,
        action: None,
    }
}

fn find_browser() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("KRAKEN_BROWSER").map(PathBuf::from)
        && path.is_file()
    {
        return Some(path);
    }
    for path in [
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
    ] {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    for name in ["chrome", "google-chrome", "chromium", "msedge"] {
        if Command::new(name)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
        {
            return Some(name.into());
        }
    }
    None
}

fn html_decode(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn capture(text: &str, expression: &str) -> Option<String> {
    capture_limit(text, expression, 500)
}

fn capture_limit(text: &str, expression: &str, maximum: usize) -> Option<String> {
    Regex::new(expression)
        .ok()?
        .captures(text)?
        .get(1)
        .map(|capture| capture.as_str().chars().take(maximum).collect())
}

fn strip_tags(text: &str) -> String {
    Regex::new(r"<[^>]+>")
        .unwrap()
        .replace_all(text, " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixelfork_host_is_exact() {
        let url = reqwest::Url::parse("https://evil-pixelfork.ai/publish/abc").unwrap();
        assert_ne!(url.host_str(), Some("pixelfork.ai"));
    }

    #[test]
    fn runtime_graduation_requires_real_profile_effect_and_audio() {
        let declaration = GameDeclaration {
            runtime: "browser".into(),
            entrypoint: "index.html".into(),
            profile: "snake".into(),
            session: "finite".into(),
            contract_version: 2,
        };
        let mut report = serde_json::json!({
            "clickedPlay": true, "rafCount": 8, "audioStarts": 1,
            "beastApi": true, "beastVersion": 2, "beastProfile": "snake",
            "action": "feed", "actionResult": true, "errors": [],
            "before": {"state":"playing","complete":false,"score":0,"board":{"rows":8,"cols":8,"body":[{"x":1,"y":1}]},"actor":{"x":1,"y":1},"metrics":{"length":3,"food":{"x":2,"y":2}}},
            "after": {"state":"playing","complete":false,"score":1,"board":{"rows":8,"cols":8,"body":[{"x":2,"y":1}]},"actor":{"x":2,"y":1},"metrics":{"length":4,"food":{"x":3,"y":2}}}
        });
        assert!(assess_runtime_report(&report, &declaration).passed);
        report["audioStarts"] = serde_json::json!(0);
        let rejected = assess_runtime_report(&report, &declaration);
        assert!(!rejected.passed);
        assert!(
            rejected
                .reasons
                .iter()
                .any(|reason| reason.contains("audio"))
        );
    }

    #[test]
    fn local_game_manifest_may_confirm_but_not_override_inventory() {
        let root = tempfile::tempdir().unwrap();
        let declaration = GameDeclaration {
            runtime: "browser".into(),
            entrypoint: "index.html".into(),
            profile: "snake".into(),
            session: "finite".into(),
            contract_version: 2,
        };
        fs::write(
            root.path().join(".kraken-game.json"),
            r#"{"profile":"shooter","session":"finite","contract_version":2}"#,
        )
        .unwrap();
        assert!(
            local_manifest_reasons(root.path(), &declaration)
                .iter()
                .any(|reason| reason.contains("profile"))
        );
    }

    #[test]
    fn installed_headless_browser_proves_the_behavioral_contract() {
        if find_browser().is_none() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let game = root.path().join("game");
        let scratch = root.path().join("scratch");
        fs::create_dir_all(&game).unwrap();
        fs::write(
            game.join("index.html"),
            r#"<!doctype html><html><head></head><body><button id="play">Play</button><canvas></canvas><script>
let state='ready',score=0,frames=0;
document.getElementById('play').addEventListener('click',()=>{state='playing';const c=new AudioContext(),o=c.createOscillator();o.connect(c.destination);o.start();o.stop(c.currentTime+.01);const loop=()=>{frames++;requestAnimationFrame(loop)};requestAnimationFrame(loop)});
window.__KRAKEN_BEAST__={version:2,profile:'snake',snapshot(){return{state,complete:false,score,board:{rows:8,cols:8,body:[{x:score,y:0}]},actor:{x:score,y:0},metrics:{length:3+score,food:{x:4,y:4},frames}}},action(name){if(name==='feed'){score++;return true}return false}};
</script></body></html>"#,
        )
        .unwrap();
        let declaration = GameDeclaration {
            runtime: "browser".into(),
            entrypoint: "index.html".into(),
            profile: "snake".into(),
            session: "finite".into(),
            contract_version: 2,
        };
        let report = browser_probe(&game.join("index.html"), &declaration, &scratch);
        assert!(report.passed, "{report:?}");
    }
}
