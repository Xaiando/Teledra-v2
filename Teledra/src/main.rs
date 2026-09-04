mod app;
mod environment;
mod lifetime;
mod portrait;
mod runtime;
mod session;
mod terminal;
mod ui;

use std::env;
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use app::{App, AppAction, FocusField, ModelReply};
use crossterm::event;
use runtime::{RuntimeServices, ToolReply};
use teledra_brain::{Brain, BrainConfig};
use teledra_court::Court;
use teledra_sidecars::SidecarKind;
use teledra_tts::{SpeechEvent, SpeechRuntime, Utterance, VoiceName};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Teledra could not start: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = env::args().skip(1).collect();
    if let Some(index) = arguments
        .iter()
        .position(|argument| argument == "--vertical-view")
    {
        let path = arguments
            .get(index + 1)
            .ok_or("--vertical-view requires a session snapshot")?;
        return run_vertical_view(PathBuf::from(path));
    }
    if let Some(index) = arguments
        .iter()
        .position(|argument| argument == "--dashboard-view")
    {
        let path = arguments
            .get(index + 1)
            .ok_or("--dashboard-view requires a session snapshot")?;
        return run_snapshot_view(PathBuf::from(path), SnapshotView::Dashboard);
    }
    if let Some(index) = arguments
        .iter()
        .position(|argument| argument == "--work-view")
    {
        let path = arguments
            .get(index + 1)
            .ok_or("--work-view requires a session snapshot")?;
        return run_snapshot_view(PathBuf::from(path), SnapshotView::Work);
    }

    let mut court = Court::default();
    if let Some(index) = arguments.iter().position(|argument| argument == "--role") {
        let requested = arguments
            .get(index + 1)
            .ok_or("--role requires a court member")?;
        court.select(requested.parse()?);
    }

    // Before anything is spawned, so nothing can escape the court's lifetime.
    lifetime::bind_children_to_this_process();

    let brain = Brain::new(BrainConfig::default())?;
    let root = data_dir();
    // One court at a time: a second instance would contend for the audio
    // device and leave the operator with duplicate studio windows.
    lifetime::close_previous_instance(&root);
    let python = python_executable();
    let environment = environment::inspect(&root, &python);
    if arguments
        .iter()
        .any(|argument| matches!(argument.as_str(), "--check" | "--check-environment"))
    {
        let brain_health = brain.check().await;
        match &brain_health {
            Ok(health) => println!("Teledra V2 | {} | {}", health.model, health.endpoint),
            Err(error) => println!("Teledra V2 | BRAIN DISABLED | {error}"),
        }
        println!("{}", environment.render());
        if brain_health.is_ok() && environment.ready() {
            println!("Environment check passed.");
            return Ok(());
        }
        return Err("environment check found unavailable capabilities".into());
    }
    // Screen capture is unavailable on this machine, so the portrait panel can
    // only be inspected by asking it to draw itself somewhere visible. This
    // renders exactly what the TUI would draw, at a given cell size, into a PNG
    // where every cell becomes a block painted with its two colours.
    if let Some(position) = arguments
        .iter()
        .position(|argument| argument == "--portrait")
    {
        let output = arguments
            .get(position + 1)
            .cloned()
            .unwrap_or_else(|| "portrait_check.png".to_owned());
        let columns: u16 = arguments
            .get(position + 2)
            .and_then(|value| value.parse().ok())
            .unwrap_or(55);
        let rows: u16 = arguments
            .get(position + 3)
            .and_then(|value| value.parse().ok())
            .unwrap_or(46);
        portrait::dump(&portrait::Portrait::load(), &output, columns, rows)?;
        println!("wrote {output} at {columns}x{rows} cells");
        return Ok(());
    }

    let health = brain.check().await?;
    if arguments.iter().any(|argument| argument == "--strict") && !environment.ready() {
        return Err(format!("strict environment check failed:\n{}", environment.render()).into());
    }

    let mut app = App::new(court, health).with_portrait(Some(portrait::Portrait::load()));
    for capability in &environment.capabilities {
        if !capability.available {
            app.push_private_event(
                "Environment",
                format!("{} disabled: {}", capability.name, capability.detail),
            );
        }
    }

    let snapshots = session::SnapshotPublisher::for_current_process();
    snapshots.publish(&app.snapshot())?;
    let speech = start_speech();
    let (reply_tx, mut reply_rx) = mpsc::unbounded_channel::<ModelReply>();
    let (tool_tx, mut tool_rx) = mpsc::unbounded_channel::<ToolReply>();
    let mut services = RuntimeServices::new(root, python)?;
    services.initialize_app_state(&mut app);
    services.refresh_identity_into(&mut app);
    services.connect_saved_restream(&mut app, &tool_tx);
    if env::var_os("TELEDRA_DISABLE_MUSIC").is_some() {
        app.set_music_enabled(false);
    } else {
        services.start_organist(&mut app, &tool_tx);
    }
    if env::var_os("TELEDRA_DISABLE_FRACTUS").is_none() {
        services.start_fractus_gallery(&mut app, &tool_tx);
    }
    // A launched court performs.
    //
    // "Normal" answers the operator but never speaks first, and only a saved
    // Restream token or a typed command moved it off that. With no token
    // present a fresh launch sat silent: music evolved, the gallery rendered,
    // and the court said nothing for as long as it was left alone. That is what
    // "the court died down" looked like from the desk. The mode is still the
    // operator's to change, and this only applies when nothing else has already
    // chosen one.
    if app.mode() == "Normal" {
        match env::var("TELEDRA_START_MODE") {
            Ok(mode) if !mode.trim().is_empty() => app.set_mode(mode.trim()),
            _ => app.set_mode("Babble"),
        }
        app.push_private_event(
            "Mode",
            format!("{} on launch; the court speaks on its own", app.mode()),
        );
    }
    let mut autonomous_mode = app.mode().to_owned();
    let mut autonomy_cadence = autonomous_interval(app.mode());
    let mut next_autonomous = autonomy_cadence.map(|interval| Instant::now() + interval);
    let mut next_stream_poll =
        env::var_os("TELEDRA_STREAM_CHAT_URL").map(|_| Instant::now() + Duration::from_secs(2));
    let mut next_copilot_poll: Option<Instant> = None;
    let mut next_somatic_poll = env::var_os("TELEDRA_DISABLE_SOMATIC")
        .is_none()
        .then(Instant::now);
    let mut next_wizard_poll =
        env::var_os("TELEDRA_WIZARD_REPORTS").map(|_| Instant::now() + Duration::from_secs(8));
    let mut night_desk_enabled = app.night_desk_enabled();
    // Backstage work must be slower than one inference, or it queues behind
    // itself and crowds out everything the audience came for.
    let night_desk_cadence = autonomous_interval("Night Desk")
        .unwrap_or_else(|| Duration::from_secs(90))
        .max(Duration::from_secs(45));
    let mut next_nightdesk = night_desk_enabled.then(|| Instant::now() + night_desk_cadence);
    let mut next_moltbook_poll = night_desk_enabled.then(Instant::now);
    let mut microphone_enabled = app.microphone_enabled();
    let mut next_microphone_poll = microphone_enabled.then(Instant::now);
    // The score only ever changed when the Organist happened to write one, so a
    // single piece could hold the stream for an hour. Evolution is now on a
    // cadence of its own, and it yields to the public court the same way
    // backstage work does.
    let music_evolution_cadence = music_evolution_interval();
    let mut next_music_evolution =
        music_evolution_cadence.map(|interval| Instant::now() + interval);
    let mut next_dream_check = Instant::now() + Duration::from_secs(30 * 60);
    let mut terminal = terminal::TerminalSession::enter()?;
    // Say what the console granted before anything is drawn with it. A session
    // without 24-bit colour renders the portrait as a field of blocks, which
    // reads as a broken image; naming the missing capability is the difference
    // between a bug report and a known limitation.
    // Which build is this? The launcher runs the previous executable when a
    // build fails, so "the fix is in the tree" and "the fix is in the court"
    // are different questions. V1 printed this at startup; V2 had lost it.
    app.push_private_event("Build", build_identity());

    let terminal_report = terminal.report();
    app.set_terminal_report(terminal_report);
    app.push_private_event("Terminal", terminal_report.describe());

    loop {
        drain_model_replies(
            &mut reply_rx,
            &mut app,
            &mut services,
            &tool_tx,
            speech.as_ref(),
        );
        drain_tool_replies(&mut tool_rx, &mut app, &mut services, &tool_tx);
        drain_speech_events(speech.as_ref(), &mut app);
        if app.quit_requested() {
            break;
        }

        if app.mode() != autonomous_mode {
            autonomous_mode = app.mode().to_owned();
            autonomy_cadence = autonomous_interval(app.mode());
            next_autonomous = autonomy_cadence.map(|interval| Instant::now() + interval);
        }
        if app.night_desk_enabled() != night_desk_enabled {
            night_desk_enabled = app.night_desk_enabled();
            next_nightdesk = night_desk_enabled.then(Instant::now);
            next_moltbook_poll = night_desk_enabled.then(Instant::now);
        }
        if app.microphone_enabled() != microphone_enabled {
            microphone_enabled = app.microphone_enabled();
            next_microphone_poll = microphone_enabled.then(Instant::now);
        }

        if let Some(request) = app.begin_queued_external_turn() {
            spawn_turn(&brain, &reply_tx, request);
        }
        // A deadline is only spent when a turn actually started. Advancing it
        // after a refusal let the shortest cadence monopolise the single
        // inference slot: an inference outlasts the Night Desk interval, so the
        // public court was re-armed on every refusal and the backstage
        // scheduler was always the one holding an expired deadline when the
        // slot freed. The court fell silent while maintenance ran on.
        let public_court_due = !app.is_test_mode()
            && app.allows_idle_autonomy()
            && next_autonomous.is_some_and(|deadline| Instant::now() >= deadline);
        if public_court_due {
            if let Some(request) = app.begin_autonomous_turn() {
                spawn_turn(&brain, &reply_tx, request);
                next_autonomous = autonomy_cadence.map(|interval| Instant::now() + interval);
            }
        } else if !app.allows_idle_autonomy() {
            next_autonomous = autonomy_cadence.map(|interval| Instant::now() + interval);
        }
        // The court performing always outranks the court doing chores, so a
        // due public turn holds the slot rather than losing it to Night Desk.
        if !app.is_test_mode()
            && !public_court_due
            && next_nightdesk.is_some_and(|deadline| Instant::now() >= deadline)
            && let Some(request) = app.begin_nightdesk_turn()
        {
            spawn_turn(&brain, &reply_tx, request);
            next_nightdesk = Some(Instant::now() + night_desk_cadence);
        }
        if !app.is_test_mode()
            && next_stream_poll.is_some_and(|deadline| Instant::now() >= deadline)
        {
            services.dispatch_sidecar_quiet(
                &tool_tx,
                SidecarKind::StreamChat,
                None,
                "Audience",
                true,
            );
            next_stream_poll = Some(Instant::now() + Duration::from_secs(15));
        }
        if !app.is_test_mode() && app.is_copilot() && next_copilot_poll.is_none() {
            next_copilot_poll = Some(Instant::now());
        } else if !app.is_copilot() {
            next_copilot_poll = None;
        }
        if next_copilot_poll.is_some_and(|deadline| Instant::now() >= deadline) {
            services.dispatch_sidecar(
                &mut app,
                &tool_tx,
                SidecarKind::Vision,
                None,
                "Co-Pilot Vision",
                true,
            );
            next_copilot_poll = Some(Instant::now() + Duration::from_secs(7));
        }
        if !app.is_test_mode()
            && next_somatic_poll.is_some_and(|deadline| Instant::now() >= deadline)
        {
            services.dispatch_sidecar_quiet(&tool_tx, SidecarKind::Somatic, None, "Somatic", false);
            next_somatic_poll = Some(Instant::now() + Duration::from_secs(5));
        }
        if next_wizard_poll.is_some_and(|deadline| Instant::now() >= deadline) {
            services.import_wizard_reports(&mut app, true);
            next_wizard_poll = Some(Instant::now() + Duration::from_secs(300));
        }
        if !app.is_test_mode()
            && next_moltbook_poll.is_some_and(|deadline| Instant::now() >= deadline)
        {
            services.dispatch_sidecar_quiet(
                &tool_tx,
                SidecarKind::Outreach,
                Some(serde_json::json!({ "action": "moltbook_inbox" })),
                "Diplomat",
                false,
            );
            next_moltbook_poll = Some(Instant::now() + Duration::from_secs(60));
        }
        if !app.is_test_mode()
            && next_microphone_poll.is_some_and(|deadline| Instant::now() >= deadline)
        {
            services.dispatch_sidecar_quiet(
                &tool_tx,
                SidecarKind::Microphone,
                None,
                "Microphone",
                true,
            );
            next_microphone_poll = Some(Instant::now() + Duration::from_secs(12));
        }
        if !app.is_test_mode()
            && !public_court_due
            && app.music_enabled()
            && next_music_evolution.is_some_and(|deadline| Instant::now() >= deadline)
        {
            services.queue_music_evolution(&mut app);
            next_music_evolution =
                music_evolution_cadence.map(|interval| Instant::now() + interval);
        }
        if !app.is_test_mode() {
            services.evolve_music_if_due(&mut app, &tool_tx);
            services.evolve_fractus_if_due(&mut app, &tool_tx);
        }
        if !app.is_test_mode() && Instant::now() >= next_dream_check {
            services.consolidate_memory_if_needed(&mut app);
            next_dream_check = Instant::now() + Duration::from_secs(30 * 60);
        }

        app.tick();
        snapshots.publish(&app.snapshot())?;
        terminal.draw(|frame| ui::render(frame, &app))?;

        if event::poll(Duration::from_millis(70))? {
            match app.handle_event(event::read()?) {
                AppAction::Continue => {}
                AppAction::Quit => break,
                AppAction::Submit => {
                    if let Some(speech) = &speech {
                        let _ = speech.clear();
                    }
                    if app.focus() == FocusField::Chat && !app.input().trim().starts_with('/') {
                        let query = app.input().to_owned();
                        services.recall_into(&mut app, &query);
                    }
                    let Some(request) = app.begin_turn() else {
                        if let Some(command) = app.take_command() {
                            services.handle_command(command, &mut app, snapshots.path(), &tool_tx);
                        }
                        continue;
                    };
                    spawn_turn(&brain, &reply_tx, request);
                    next_autonomous = autonomy_cadence.map(|interval| Instant::now() + interval);
                }
            }
        }
    }

    Ok(())
}

fn drain_model_replies(
    replies: &mut mpsc::UnboundedReceiver<ModelReply>,
    app: &mut App,
    services: &mut RuntimeServices,
    tools: &mpsc::UnboundedSender<ToolReply>,
    speech: Option<&SpeechRuntime>,
) {
    while let Ok(mut reply) = replies.try_recv() {
        let actions = if let Ok(raw) = &reply.result {
            let parsed = teledra_court::parse_response(raw);
            // A member that asks for something the court cannot execute has
            // done no work, however convincing the reply reads. Say so
            // backstage rather than letting the request disappear.
            for head in &parsed.unrecognised {
                app.push_private_event(
                    "Tags",
                    format!(
                        "{:?} wrote '{head}:' which matches no court action; nothing ran",
                        reply.agent
                    ),
                );
            }
            if parsed.visible.is_empty() {
                // The member spoke only in tags, so it has nothing to say
                // aloud. A stand-in line was published instead, and the same
                // sentence appearing in the public feed read as the court
                // having run out of things to say. The work still happens; it
                // simply belongs backstage.
                reply.audible = false;
                reply.result = Ok(format!("{} acted without speaking.", reply.agent.title()));
            } else {
                reply.result = Ok(parsed.visible);
            }
            parsed.actions
        } else {
            Vec::new()
        };
        if let Ok(answer) = &reply.result {
            services.record_reply(app, reply.agent, &reply.user, answer);
            if reply.audible
                && !app.is_test_mode()
                && let Some(speech) = speech
            {
                let _ = speech.speak(Utterance::new(voice_for(reply.agent), answer));
            }
        }
        let asked_for_a_score = reply.user.contains("COMPOSER NUDGE")
            || reply
                .user
                .contains("described a composition instead of producing one");
        let produced_a_score = actions
            .iter()
            .any(|action| matches!(action, teledra_court::CourtAction::CourtScore(_)));
        // Asked for a score and sent one, but without its tag: the Organist
        // answers a correction request with a fenced JSON block and no wrapper.
        // The reply is unambiguous — a score was requested and an object came
        // back — so the object is taken rather than thrown away for missing
        // punctuation.
        let salvaged = (asked_for_a_score && !produced_a_score)
            .then(|| {
                reply
                    .result
                    .as_ref()
                    .ok()
                    .and_then(|answer| json_object(answer))
            })
            .flatten();
        app.finish_turn(reply);
        for action in actions {
            services.handle_court_action(action, app, tools);
        }
        if let Some(raw) = salvaged {
            app.push_private_event(
                "Organist",
                "A score arrived without its tag and was taken as written".to_owned(),
            );
            services.handle_court_action(teledra_court::CourtAction::CourtScore(raw), app, tools);
            continue;
        }
        // A nudge that comes back as prose has developed nothing. The
        // correction loop covers a payload that will not parse; this covers the
        // commoner case of a member describing the work instead of doing it,
        // which otherwise leaves the score untouched until the next cadence.
        if asked_for_a_score && !produced_a_score {
            services.request_missing_score(app);
        }
    }
}

/// The first balanced JSON object in a reply, ignoring any code fence.
///
/// Used only where a structured artifact was explicitly requested, so a stray
/// object in ordinary conversation is never mistaken for one.
fn json_object(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let mut depth = 0_usize;
    let mut quoted = false;
    let mut escaped = false;
    for (offset, character) in text[start..].char_indices() {
        if quoted {
            match character {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => quoted = false,
                _ => {}
            }
            continue;
        }
        match character {
            '"' => quoted = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(text[start..start + offset + 1].to_owned());
                }
            }
            _ => {}
        }
    }
    None
}

fn drain_tool_replies(
    replies: &mut mpsc::UnboundedReceiver<ToolReply>,
    app: &mut App,
    services: &mut RuntimeServices,
    tools: &mpsc::UnboundedSender<ToolReply>,
) {
    while let Ok(reply) = replies.try_recv() {
        let quiet = matches!(
            &reply.effect,
            Some(
                runtime::ToolEffect::Somatic(_)
                    | runtime::ToolEffect::DiplomatInbox(_)
                    | runtime::ToolEffect::Audience { .. }
                    | runtime::ToolEffect::DawAction(_)
            )
        );
        let effect = reply.effect;
        match reply.result {
            Ok(message) => {
                if !quiet {
                    app.set_notice(format!("{} completed", reply.source));
                    app.push_private_event(reply.source, message);
                }
            }
            Err(error) => {
                app.set_notice(format!("{} failed: {error}", reply.source));
                app.push_private_event(reply.source, error);
            }
        }
        if let Some(prompt) = reply.court_prompt {
            app.queue_external_turn(teledra_agents::AgentId::Queen, prompt);
        }
        if let Some(effect) = effect {
            services.apply_tool_effect(app, effect, tools);
        }
    }
}

fn drain_speech_events(speech: Option<&SpeechRuntime>, app: &mut App) {
    let Some(speech) = speech else {
        return;
    };
    while let Some(event) = speech.try_event() {
        match event {
            SpeechEvent::Failed { reason, .. } => {
                app.set_notice(format!("Speech disabled: {reason}"));
            }
            SpeechEvent::Started { voice } => app.set_notice(format!("{voice} is speaking")),
            SpeechEvent::Finished { .. } => {}
        }
    }
}

fn run_vertical_view(path: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let portrait = portrait::Portrait::load();
    let mut terminal = terminal::TerminalSession::enter_vertical()?;
    let mut snapshot = session::read_snapshot(&path)?;
    let mut missing_since = None;
    loop {
        if let Ok(updated) = session::read_snapshot(&path) {
            snapshot = updated;
            missing_since = None;
        } else if !path.exists() {
            let since = missing_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= Duration::from_secs(2) {
                break;
            }
        } else {
            missing_since = None;
        }
        terminal.draw(|frame| ui::render_vertical(frame, &snapshot, &portrait))?;
        if event::poll(Duration::from_millis(100))? {
            let event::Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind == event::KeyEventKind::Press
                && ((key.modifiers.contains(event::KeyModifiers::CONTROL)
                    && matches!(
                        key.code,
                        event::KeyCode::Char('q') | event::KeyCode::Char('c')
                    ))
                    || key.code == event::KeyCode::Esc)
            {
                break;
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum SnapshotView {
    Dashboard,
    Work,
}

fn run_snapshot_view(path: PathBuf, view: SnapshotView) -> Result<(), Box<dyn std::error::Error>> {
    let mut terminal = match view {
        SnapshotView::Dashboard => terminal::TerminalSession::enter_dashboard()?,
        SnapshotView::Work => terminal::TerminalSession::enter_work_board()?,
    };
    let mut snapshot = session::read_snapshot(&path)?;
    let mut missing_since = None;
    loop {
        if let Ok(updated) = session::read_snapshot(&path) {
            snapshot = updated;
            missing_since = None;
        } else if !path.exists() {
            let since = missing_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= Duration::from_secs(2) {
                break;
            }
        } else {
            missing_since = None;
        }
        terminal.draw(|frame| match view {
            SnapshotView::Dashboard => ui::render_dashboard(frame, &snapshot),
            SnapshotView::Work => ui::render_work_board(frame, &snapshot),
        })?;
        if event::poll(Duration::from_millis(100))? {
            let event::Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind == event::KeyEventKind::Press
                && ((key.modifiers.contains(event::KeyModifiers::CONTROL)
                    && matches!(
                        key.code,
                        event::KeyCode::Char('q') | event::KeyCode::Char('c')
                    ))
                    || key.code == event::KeyCode::Esc)
            {
                break;
            }
        }
    }
    Ok(())
}

fn spawn_turn(
    brain: &Brain,
    replies: &mpsc::UnboundedSender<ModelReply>,
    request: app::InferenceRequest,
) {
    let request_brain = brain.clone();
    let request_tx = replies.clone();
    tokio::spawn(async move {
        let result = request_brain
            .complete_reviewed(&request.messages, request.temperature, request.agent.slug())
            .await
            .map_err(|error| error.to_string());
        let _ = request_tx.send(ModelReply {
            id: request.id,
            agent: request.agent,
            user: request.user,
            result,
            audible: request.audible,
        });
    });
}

/// Revision, working-tree state, and build time, as one line.
fn build_identity() -> String {
    let raw = env!("TELEDRA_BUILD_ID");
    let (head, built_at) = raw.rsplit_once('.').unwrap_or((raw, "0"));
    let seconds: u64 = built_at.parse().unwrap_or_default();
    let age = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |now| now.as_secs().saturating_sub(seconds));
    format!(
        "{} v{} ({head}, built {} ago)",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        humanised(age)
    )
}

/// A duration a person can read at a glance.
fn humanised(seconds: u64) -> String {
    match seconds {
        0..=90 => format!("{seconds}s"),
        91..=5_400 => format!("{}m", seconds / 60),
        5_401..=172_800 => format!("{}h", seconds / 3_600),
        _ => format!("{}d", seconds / 86_400),
    }
}

fn autonomous_interval(mode: &str) -> Option<Duration> {
    let configured = env::var("TELEDRA_AUTONOMOUS_INTERVAL_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok());
    let seconds = configured.unwrap_or(match mode {
        "Night Desk" => 90,
        "Babble" | "Streamer" => 20,
        _ => 45,
    });
    (seconds > 0).then(|| Duration::from_secs(seconds.max(1)))
}

/// How often the Organist is asked to develop the current score.
///
/// Bounded below because an evolution costs a full inference and competes with
/// the court for the one slot; bounded above so a long stream is never left on
/// a single loop. `0` leaves the score alone until the operator asks.
fn music_evolution_interval() -> Option<Duration> {
    let configured = env::var("TELEDRA_MUSIC_EVOLVE_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(300);
    (configured > 0).then(|| Duration::from_secs(configured.clamp(60, 3_600)))
}

fn start_speech() -> Option<SpeechRuntime> {
    if env::var_os("TELEDRA_DISABLE_TTS").is_some() {
        return None;
    }
    let worker = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("TTS")
        .join("worker")
        .join("voice_worker.py");
    let mut command = Command::new(python_executable());
    command.arg(worker);
    SpeechRuntime::start_on(command, env::var("TELEDRA_AUDIO_DEVICE").ok()).ok()
}

fn data_dir() -> PathBuf {
    env::var_os("TELEDRA_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(env::temp_dir)
                .join("Teledra V2")
        })
}

fn python_executable() -> PathBuf {
    env::var_os("TELEDRA_PYTHON")
        .map(PathBuf::from)
        .or_else(|| {
            let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let bundled = manifest
                .parent()
                .unwrap_or(&manifest)
                .join(".venv")
                .join("Scripts")
                .join("python.exe");
            bundled.is_file().then_some(bundled)
        })
        .unwrap_or_else(|| PathBuf::from("python"))
}

fn voice_for(agent: teledra_agents::AgentId) -> VoiceName {
    use teledra_agents::AgentId;
    match agent {
        AgentId::Queen | AgentId::Malthus | AgentId::Wizard => VoiceName::Queen,
        AgentId::Organist => VoiceName::Organist,
        AgentId::Archivist => VoiceName::Archivist,
        AgentId::Alchemist => VoiceName::Alchemist,
        AgentId::Orator => VoiceName::Orator,
        AgentId::Scribe => VoiceName::Scribe,
        AgentId::Artist => VoiceName::Artist,
        AgentId::Diplomat => VoiceName::Diplomat,
        AgentId::Treasurer => VoiceName::Treasurer,
    }
}
