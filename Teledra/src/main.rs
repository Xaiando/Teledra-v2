mod app;
mod composition;
mod environment;
mod lifetime;
mod organist_genome;
mod podcast;
mod portrait;
mod python_composition;
mod python_context;
mod python_parameters;
mod python_score;
mod runtime;
mod session;
mod terminal;
mod ui;

use std::env;
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use app::{App, AppAction, ModelReply};
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
    if arguments
        .iter()
        .any(|argument| argument == "--music-note-server")
    {
        return teledra_music::run_python_note_server().map_err(Into::into);
    }
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

    // Used by Music/organist_train.py: the court's own drawing, expansion and
    // validation, so trained weights are learned on exactly what it plays.
    if let Some(index) = arguments
        .iter()
        .position(|argument| argument == "--organist-genomes")
    {
        let first: u64 = arguments
            .get(index + 1)
            .and_then(|value| value.parse().ok())
            .ok_or("--organist-genomes requires FIRST_SEED COUNT [STYLE_JSON]")?;
        let count: u64 = arguments
            .get(index + 2)
            .and_then(|value| value.parse().ok())
            .ok_or("--organist-genomes requires FIRST_SEED COUNT [STYLE_JSON]")?;
        let style = match arguments.get(index + 3) {
            Some(path) => organist_genome::Style::load(std::path::Path::new(path))?,
            None => organist_genome::Style::bundled(),
        };
        for seed in first..first.saturating_add(count.min(10_000)) {
            let Some(genome) = organist_genome::draw_with(seed, &[], &style) else {
                continue;
            };
            let code = python_score::expand_proposal(organist_genome::proposal(&genome))
                .and_then(|song| python_score::source(&song))?;
            println!(
                "{}",
                serde_json::json!({"seed": seed, "genome": genome, "code": code})
            );
        }
        return Ok(());
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

    let root = data_dir();
    let brain = Brain::new(BrainConfig::default())?
        .with_oversize_log(root.join("diagnostics").join("oversized-prompts.jsonl"));
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
    // Read-only diagnostics above must never terminate the running court.
    lifetime::close_previous_instance(&root);
    if arguments.iter().any(|argument| argument == "--strict") && !environment.ready() {
        return Err(format!("strict environment check failed:\n{}", environment.render()).into());
    }

    let mut app = App::new(court, health).with_portrait(Some(portrait::Portrait::load()));
    // Start as a plain speaking desk: nothing runs unless the operator types it.
    if env::var_os("TELEDRA_VOICE_DESK").is_some_and(|value| !value.is_empty() && value != "0") {
        app.set_voice_desk(true);
    }
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
    let mut inference = ActiveInference::default();
    let mut turn_balance = CourtTurnBalance::default();
    let mut services = RuntimeServices::new(root, python)?;
    services.initialize_app_state(&mut app);
    services.refresh_identity_into(&mut app);
    services.connect_saved_chat(&mut app, &tool_tx);
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
        cancel_stopped_composition(&mut inference, &mut app, &services).await;
        services.publish_music_if_ready(&mut app, &tool_tx);
        drain_speech_events(speech.as_ref(), &mut app);
        drain_completed_speech(&mut app, &mut services);
        if app.quit_requested() {
            break;
        }

        if app.mode() != autonomous_mode {
            autonomous_mode = app.mode().to_owned();
            autonomy_cadence = autonomous_interval(app.mode());
            next_autonomous = autonomy_cadence.map(|interval| {
                if app.is_podcasting() {
                    Instant::now()
                } else {
                    Instant::now() + interval
                }
            });
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

        // Prepare one reply ahead while the current voice is heard. Without
        // this backpressure, fast model turns build minutes of delayed speech.
        let speech_has_room = speech
            .as_ref()
            .is_none_or(|speech| speech.pending_count() < 2);
        let public_court_due = !app.is_hands_off()
            && app.allows_idle_autonomy()
            && next_autonomous.is_some_and(|deadline| Instant::now() >= deadline);
        let allow_backstage = turn_balance.allows_backstage(public_court_due, speech_has_room);
        let nightdesk_due = !app.is_hands_off()
            && next_nightdesk.is_some_and(|deadline| Instant::now() >= deadline);
        if !app.is_hands_off()
            && let Some((request, origin)) = begin_ready_work(
                &mut app,
                &turn_balance,
                speech_has_room,
                allow_backstage,
                nightdesk_due,
            )
        {
            match origin {
                WorkOrigin::External => turn_balance.started(request.audible),
                WorkOrigin::NightDesk => turn_balance.started_nightdesk(request.audible),
            }
            inference
                .start(|| spawn_turn(&brain, &reply_tx, request, &services, &mut app))
                .await;
            if origin == WorkOrigin::NightDesk {
                next_nightdesk = Some(Instant::now() + night_desk_cadence);
            }
        }
        // A deadline is only spent when a turn actually started. Advancing it
        // after a refusal let the shortest cadence monopolise the single
        // inference slot: an inference outlasts the Night Desk interval, so the
        // public court was re-armed on every refusal and the backstage
        // scheduler was always the one holding an expired deadline when the
        // slot freed. The court fell silent while maintenance ran on.
        if public_court_due && speech_has_room {
            if let Some(request) = app.begin_autonomous_turn() {
                if app.mode() == "Night Desk" {
                    turn_balance.started_nightdesk(request.audible);
                } else {
                    turn_balance.started(request.audible);
                }
                inference
                    .start(|| spawn_turn(&brain, &reply_tx, request, &services, &mut app))
                    .await;
                next_autonomous = autonomy_cadence.map(|interval| Instant::now() + interval);
            }
        } else if !app.allows_idle_autonomy() {
            next_autonomous = autonomy_cadence.map(|interval| Instant::now() + interval);
        }
        if !app.is_hands_off()
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
        if !app.is_hands_off() && app.is_copilot() && next_copilot_poll.is_none() {
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
        if !app.is_hands_off()
            && next_somatic_poll.is_some_and(|deadline| Instant::now() >= deadline)
        {
            services.dispatch_sidecar_quiet(&tool_tx, SidecarKind::Somatic, None, "Somatic", false);
            next_somatic_poll = Some(Instant::now() + Duration::from_secs(5));
        }
        if !app.is_hands_off()
            && next_wizard_poll.is_some_and(|deadline| Instant::now() >= deadline)
        {
            services.import_wizard_reports(&mut app, true);
            next_wizard_poll = Some(Instant::now() + Duration::from_secs(300));
        }
        if !app.is_hands_off()
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
        if !app.is_hands_off()
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
        if !app.is_hands_off() {
            services.evolve_music_if_due(&mut app, &tool_tx);
            services.evolve_fractus_if_due(&mut app, &tool_tx);
        }
        if !app.is_hands_off() && Instant::now() >= next_dream_check {
            services.consolidate_memory_if_needed(&mut app);
            next_dream_check = Instant::now() + Duration::from_secs(30 * 60);
        }

        app.tick();
        let mut snapshot = app.snapshot();
        snapshot.music = Some(services.music_snapshot(&app));
        snapshots.publish(&snapshot)?;
        terminal.draw(|frame| ui::render(frame, &app))?;

        if event::poll(Duration::from_millis(70))? {
            match app.handle_event(event::read()?) {
                AppAction::Continue => {}
                AppAction::Quit => break,
                AppAction::Submit => {
                    let Some(request) = app.begin_turn() else {
                        if let Some(command) = app.take_command() {
                            if app.podcast_status().is_some() && interrupts_podcast(&command) {
                                if let Some(pending) = app.pending().cloned()
                                    && pending.user.starts_with("PODCAST TURN:")
                                {
                                    inference.cancel().await;
                                    app.finish_turn(ModelReply {
                                        id: pending.id,
                                        agent: pending.agent,
                                        user: pending.user,
                                        result: Err("Podcast interrupted by the operator".into()),
                                        audible: true,
                                    });
                                }
                                drain_speech_events(speech.as_ref(), &mut app);
                                if let Some(speech) = &speech {
                                    let _ = speech.clear();
                                }
                                app.interrupt_speech_captions();
                                drain_completed_speech(&mut app, &mut services);
                            }
                            if let app::AppCommand::Say(text) = &command {
                                // Any /Say hands the floor to the operator: the court
                                // goes quiet and waits. /VoiceDesk off releases it.
                                let entered = !app.is_voice_desk();
                                app.set_voice_desk(true);
                                match say_target(text) {
                                    None => app.set_notice(if entered {
                                        "Voice Desk ON: type a line to hear it; /VoiceDesk off releases the court"
                                    } else {
                                        "Voice Desk is on: type a line to hear it"
                                    }),
                                    Some((voice, words)) => {
                                        let queued = speech.as_ref().is_some_and(|speech| {
                                            speech.speak(Utterance::new(voice, words))
                                        });
                                        if queued {
                                            app.set_notice(format!("{voice} will say it"));
                                        } else {
                                            app.set_notice(
                                                "Speech unavailable or busy; nothing was spoken",
                                            );
                                        }
                                    }
                                }
                                continue;
                            }
                            services.handle_command(command, &mut app, snapshots.path(), &tool_tx);
                        }
                        continue;
                    };
                    drain_completed_speech(&mut app, &mut services);
                    if let Some(speech) = &speech {
                        let _ = speech.clear();
                    }
                    turn_balance.started(request.audible);
                    inference
                        .start(|| spawn_turn(&brain, &reply_tx, request, &services, &mut app))
                        .await;
                    next_autonomous = autonomy_cadence.map(|interval| Instant::now() + interval);
                }
            }
        }
    }

    // Preserve the heard fragment on a normal exit as well as an interruption.
    drain_speech_events(speech.as_ref(), &mut app);
    if let Some(speech) = &speech {
        let _ = speech.clear();
    }
    app.interrupt_speech_captions();
    drain_completed_speech(&mut app, &mut services);
    Ok(())
}

fn is_music_action(action: &teledra_court::CourtAction) -> bool {
    matches!(action, teledra_court::CourtAction::CourtScore(_))
        || matches!(action, teledra_court::CourtAction::CreativeArtifact { kind, .. }
            if kind == "python_music" || kind == "python_music_edit")
}

fn drain_model_replies(
    replies: &mut mpsc::UnboundedReceiver<ModelReply>,
    app: &mut App,
    services: &mut RuntimeServices,
    tools: &mpsc::UnboundedSender<ToolReply>,
    speech: Option<&SpeechRuntime>,
) {
    while let Ok(mut reply) = replies.try_recv() {
        if !app
            .pending()
            .is_some_and(|pending| pending.id == reply.id && pending.agent == reply.agent)
        {
            app.push_private_event(
                "Court",
                "Ignored a stale model reply before executing actions or speech",
            );
            continue;
        }
        if services.music_reply_cancelled(reply.id, reply.agent) {
            reply.result =
                Err("Composition cancelled by the operator's music selection or stop".to_owned());
            app.finish_turn(reply);
            continue;
        }
        let python_requested = services.python_composition_requested(&reply.user);
        if python_requested
            && reply.agent == teledra_agents::AgentId::Organist
            && let Ok(raw) = &mut reply.result
            && let Some(code) = fenced_python_composition(raw)
        {
            *raw = format!("[PYTHON_MUSIC: {code}]");
        }
        let mut recovered_fractus = None;
        let actions = if let Ok(raw) = &reply.result {
            let parsed = teledra_court::parse_response(raw);
            let mut actions = parsed.actions;
            // The live research-to-render probe returned the supplied URL as
            // another RESEARCH tag despite already having its evidence. Do not
            // turn a completed Artist study into duplicate Queen research.
            if reply.agent == teledra_agents::AgentId::Artist {
                remove_redundant_art_research(&reply.user, &mut actions);
            }
            // Recover the score from raw model output before presentation
            // sanitizing removes machine payloads from speech.
            if reply.agent == teledra_agents::AgentId::Organist
                && !python_requested
                && !actions.iter().any(is_music_action)
                && let Some(raw_score) = json_object(raw)
                && teledra_music::Score::from_json(&raw_score).is_ok()
            {
                actions.push(teledra_court::CourtAction::CourtScore(raw_score));
                app.push_private_event(
                    "Organist",
                    "Recovered a complete untagged score before speech sanitization",
                );
            }
            recovered_fractus = salvaged_artist_fractus(reply.agent, &reply.user, raw, &actions);
            if recovered_fractus.is_some() {
                app.push_private_event(
                    "Artist",
                    "A scene named in prose was recovered as the next Fractus lineage".to_owned(),
                );
            }
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
            let visible = teledra_court::naturalize_spoken_text(&parsed.visible);
            if visible.is_empty() {
                // The member spoke only in tags, so it has nothing to say
                // aloud. A stand-in line was published instead, and the same
                // sentence appearing in the public feed read as the court
                // having run out of things to say. The work still happens; it
                // simply belongs backstage.
                reply.audible = false;
                reply.result = Ok(format!("{} acted without speaking.", reply.agent.title()));
            } else {
                reply.result = Ok(visible);
            }
            actions
        } else {
            Vec::new()
        };
        let mut queued_for_speech = false;
        if let Ok(answer) = &reply.result
            && !(reply.audible && app.is_recent_repetition(answer))
        {
            if reply.audible
                && !app.is_test_mode()
                && !app.is_voice_desk()
                && let Some(speech) = speech
            {
                queued_for_speech =
                    speech.speak(Utterance::new(voice_for(reply.agent), answer).with_id(reply.id));
                if !queued_for_speech {
                    app.push_private_event(
                        "Speech",
                        "Utterance was not queued: worker unavailable, queue full, or text too long",
                    );
                }
            }
            if !queued_for_speech {
                services.record_reply(app, reply.agent, &reply.user, answer);
            }
        }
        if reply.agent == teledra_agents::AgentId::Organist
            && composition_turn_requested(reply.agent, &reply.user)
            && let Err(error) = &reply.result
        {
            services.music_generation_failed(app, &reply.user, error);
        }
        let asked_for_a_score = reply.result.is_ok()
            && !python_requested
            && (reply.user.contains("COMPOSER NUDGE")
                || reply
                    .user
                    .contains("described a composition instead of producing one"));
        let produced_a_score = actions.iter().any(is_music_action);
        let asked_for_python = reply.result.is_ok()
            && python_requested
            && composition_turn_requested(reply.agent, &reply.user);
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
        // While streaming, the audience's turns need the one shared GPU more
        // than a second attempt at backstage maintenance does.
        let maintenance_retry = reply.user.starts_with("NIGHT DESK")
            && app.mode() != "Streamer"
            && reply.agent != teledra_agents::AgentId::Treasurer
            && !reply.user.starts_with("NIGHT DESK RETRY:")
            && reply.result.is_ok()
            && recovered_fractus.is_none()
            && !actions.iter().any(|action| !matches!(action,
                teledra_court::CourtAction::Reflection { .. }
                | teledra_court::CourtAction::PrivateTrace { .. }))
            && !actions.iter().any(|action| matches!(action,
                teledra_court::CourtAction::PrivateTrace { content, .. } if content.contains("NO_ACTION")));
        let maintenance_agent = reply.agent;
        let maintenance_task = reply.user.chars().take(1600).collect::<String>();
        if queued_for_speech {
            app.finish_turn_with_speech(reply, true);
        } else {
            app.finish_turn(reply);
        }
        if actions.len() + usize::from(recovered_fractus.is_some()) > 4 {
            app.push_private_event(
                "Court",
                "Limited this turn to four actions; excess actions did not run",
            );
        }
        // Kept apart from tagged actions so a guess at the Artist's prose is
        // never restored at the next launch as if the Artist had asked for it.
        // It still counts as the last of the turn's four actions.
        let recovered_fractus = recovered_fractus.filter(|_| actions.len() < 4);
        for action in actions.into_iter().take(4) {
            services.handle_court_action(action, app, tools);
        }
        if let Some((payload, live)) = recovered_fractus {
            services.recover_fractus_from_prose(app, tools, &payload, live);
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
        if asked_for_python && !produced_a_score {
            services.request_missing_python_music(app);
        }
        if maintenance_retry {
            app.push_private_event(
                "Night Desk",
                "No executable work was produced; requesting one bounded correction",
            );
            app.queue_backstage_turn(maintenance_agent, format!(
                "NIGHT DESK RETRY: Your previous reply described work but emitted no executable action. Complete one supported action for this task, or emit [OBSERVE: NO_ACTION; the specific missing evidence]. Do not claim any operation already succeeded. Original task: {maintenance_task}"));
        }
    }
}

/// The scene payload and live flag recovered from an untagged Artist reply.
fn salvaged_artist_fractus(
    agent: teledra_agents::AgentId,
    prompt: &str,
    raw: &str,
    actions: &[teledra_court::CourtAction],
) -> Option<(String, bool)> {
    if agent != teledra_agents::AgentId::Artist
        || actions
            .iter()
            .any(|action| matches!(action, teledra_court::CourtAction::Fractus { .. }))
    {
        return None;
    }
    RuntimeServices::salvage_fractus_from_prose(raw)
        .map(|payload| (payload, !RuntimeServices::artist_requested_still(prompt)))
}

fn remove_redundant_art_research(prompt: &str, actions: &mut Vec<teledra_court::CourtAction>) {
    if !prompt.starts_with("ARTIST EXPLORATION:") {
        return;
    }
    let evidence = prompt
        .split_once("PUBLIC RESEARCH EVIDENCE")
        .map(|(_, evidence)| evidence)
        .unwrap_or("");
    actions.retain(|action| {
        !matches!(action, teledra_court::CourtAction::Research(query)
        if !query.trim().is_empty() && evidence.contains(query.trim()))
    });
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
                    app.record_execution_report(reply.source, &message);
                    app.set_notice(format!("{} completed", reply.source));
                    app.push_private_event(reply.source, message);
                }
            }
            Err(error) => {
                app.record_execution_report(reply.source, &format!("FAILED: {error}"));
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
            SpeechEvent::Failed { id, reason, .. } => {
                app.fail_speech_captions(id);
                app.set_notice(format!("Speech: {reason}"));
                app.push_private_event("Speech", reason);
            }
            SpeechEvent::Started { id, voice, seconds } => {
                app.advance_speech_caption(id, 0.0, seconds);
                app.set_notice(format!("{voice} is speaking"));
            }
            SpeechEvent::Progress {
                id,
                elapsed,
                seconds,
                ..
            } => {
                app.advance_speech_caption(id, elapsed, seconds);
            }
            SpeechEvent::Finished { id, .. } => app.finish_speech_caption(id),
        }
    }
}

fn drain_completed_speech(app: &mut App, services: &mut RuntimeServices) {
    while let Some(spoken) = app.take_completed_speech() {
        services.record_reply(app, spoken.agent, &spoken.user, &spoken.answer);
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

fn fenced_python_composition(raw: &str) -> Option<&str> {
    let code = raw
        .trim()
        .strip_prefix("```python")?
        .strip_suffix("```")?
        .trim();
    (!code.is_empty() && !code.contains("```")).then_some(code)
}

#[derive(Default)]
struct CourtTurnBalance {
    public_turns: u8,
    last_quiet_was_nightdesk: bool,
}

impl CourtTurnBalance {
    fn allows_backstage(&self, public_due: bool, speech_has_room: bool) -> bool {
        !public_due || !speech_has_room || self.public_turns >= 3
    }

    fn started(&mut self, audible: bool) {
        self.public_turns = if audible {
            self.public_turns.saturating_add(1).min(3)
        } else {
            0
        };
        if !audible {
            self.last_quiet_was_nightdesk = false;
        }
    }

    fn started_nightdesk(&mut self, audible: bool) {
        self.started(audible);
        self.last_quiet_was_nightdesk = true;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorkOrigin {
    External,
    NightDesk,
}

/// Share permitted backstage slots between the standing rota and queued art/
/// music work. A continuous creative queue must not keep Wizard at rota slot
/// twelve forever; ready audience speech retains its existing priority.
fn begin_ready_work(
    app: &mut App,
    balance: &CourtTurnBalance,
    can_speak: bool,
    allow_backstage: bool,
    nightdesk_due: bool,
) -> Option<(app::InferenceRequest, WorkOrigin)> {
    let audience_has_priority =
        can_speak && app.has_queued_audible_work() && balance.public_turns < 3;
    let nightdesk_first = nightdesk_due
        && allow_backstage
        && !audience_has_priority
        && (!balance.last_quiet_was_nightdesk || !app.has_queued_backstage_work());
    if nightdesk_first
        && let Some(request) = app.begin_nightdesk_turn_with_speech_capacity(can_speak)
    {
        return Some((request, WorkOrigin::NightDesk));
    }
    if let Some(request) = app.begin_queued_external_turn_with_fairness(
        can_speak,
        allow_backstage,
        balance.public_turns >= 3,
    ) {
        return Some((request, WorkOrigin::External));
    }
    if nightdesk_due && allow_backstage {
        return app
            .begin_nightdesk_turn_with_speech_capacity(can_speak)
            .map(|request| (request, WorkOrigin::NightDesk));
    }
    None
}

#[derive(Default)]
struct ActiveInference(Option<tokio::task::JoinHandle<()>>);

impl ActiveInference {
    async fn start(&mut self, spawn: impl FnOnce() -> tokio::task::JoinHandle<()>) {
        self.cancel().await;
        self.0 = Some(spawn());
    }

    async fn cancel(&mut self) {
        if let Some(previous) = self.0.take() {
            previous.abort();
            // Wait for the HTTP future to drop before starting its replacement.
            let _ = previous.await;
        }
    }
}

impl Drop for ActiveInference {
    fn drop(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
        }
    }
}

async fn cancel_stopped_composition(
    inference: &mut ActiveInference,
    app: &mut App,
    services: &RuntimeServices,
) {
    let Some(pending) = app
        .pending()
        .filter(|pending| services.music_reply_cancelled(pending.id, pending.agent))
        .cloned()
    else {
        return;
    };
    inference.cancel().await;
    app.finish_turn(ModelReply {
        id: pending.id,
        agent: pending.agent,
        user: pending.user,
        result: Err("Composition cancelled by the operator's music selection or stop".into()),
        audible: false,
    });
}

fn spawn_turn(
    brain: &Brain,
    replies: &mpsc::UnboundedSender<ModelReply>,
    mut request: app::InferenceRequest,
    services: &RuntimeServices,
    app: &mut App,
) -> tokio::task::JoinHandle<()> {
    let podcast_turn = request.user.starts_with("PODCAST TURN:");
    let recent_podcast = app.recent_podcast_speech();
    let python_requested = !podcast_turn && services.python_composition_requested(&request.user);
    let composing = !podcast_turn && composition_turn_requested(request.agent, &request.user);
    // Executable Python needs an artifact-only conversation. Court dialogue,
    // persona review and historical action examples were turning revisions
    // into prose or stale JSON instead of a playable candidate.
    let python_edit_source = if python_requested && composing {
        let source = services.python_edit_source(&request.user);
        let recipe = source
            .as_deref()
            .is_none_or(|source| crate::python_score::from_source(source).is_some());
        request.messages = vec![
            teledra_brain::Message::system(if recipe {
                services.python_song_generation_context()
            } else {
                services.python_generation_context()
            }),
            teledra_brain::Message::user(request.user.clone()),
        ];
        source
    } else {
        services.enrich_request(app, &mut request);
        None
    };
    let playing_source = (python_requested && composing && python_edit_source.is_none())
        .then(|| services.accepted_python_source())
        .flatten();
    // A rota round carries its drawn genome; the frame is applied to his proposal.
    let round_genome = organist_genome::from_prompt(&request.user);
    // Retries and private review share this budget instead of restarting a
    // two-minute timer. Background composition gets room for a complete score.
    let budget = inference_budget(request.audible, app.allows_idle_autonomy(), composing);
    let request_brain = brain.clone();
    let request_tx = replies.clone();
    tokio::spawn(async move {
        let result = if podcast_turn {
            podcast::complete_turn(
                &request_brain,
                &request.messages,
                request.temperature,
                budget,
                &recent_podcast,
            )
            .await
        } else if python_requested && composing {
            if let Some(source) = python_edit_source {
                python_composition::compose_python_edit_with_budget(
                    &request_brain,
                    &request.messages,
                    &source,
                    budget,
                )
                .await
            } else {
                python_composition::compose_python_song_avoiding(
                    &request_brain,
                    &request.messages,
                    budget,
                    playing_source.as_deref(),
                    round_genome.as_ref(),
                )
                .await
            }
        } else if request.agent == teledra_agents::AgentId::Organist
            && !python_requested
            && teledra_music::composition_requested(&request.user)
        {
            composition::compose_score_with_budget(
                &request_brain,
                &request.messages,
                request.temperature,
                budget,
            )
            .await
            .map(|json| format!("[COURT_SCORE: {json}]"))
        } else {
            request_brain
                .complete_reviewed_with_budget(
                    &request.messages,
                    request.temperature,
                    request.agent.slug(),
                    budget,
                )
                .await
                .map_err(|error| error.to_string())
        };
        let _ = request_tx.send(ModelReply {
            id: request.id,
            agent: request.agent,
            user: request.user,
            result,
            audible: request.audible,
        });
    })
}

fn composition_turn_requested(agent: teledra_agents::AgentId, prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    agent == teledra_agents::AgentId::Organist
        && (teledra_music::composition_requested(prompt)
            || lower.contains("pymusic composition")
            || lower.contains("native composition")
            || lower.contains("python_music"))
}

fn inference_budget(audible: bool, public_mode: bool, composing: bool) -> Duration {
    Duration::from_secs(if composing {
        90
    } else if audible {
        60
    } else if public_mode {
        // Ollama serves one request at a time; part of this can be spent
        // waiting behind a composition, which timed out at 30 seconds.
        45
    } else {
        90
    })
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
    // Explicit podcast sessions keep the floor moving even if ordinary idle
    // chatter is disabled. Speech capacity still prevents an audio backlog.
    if mode == "Podcast" {
        return Some(Duration::from_secs(2));
    }
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

fn interrupts_podcast(command: &app::AppCommand) -> bool {
    use app::AppCommand;
    match command {
        AppCommand::Unlock | AppCommand::TestMode(_) | AppCommand::CoPilot => true,
        AppCommand::Lock(topic) => !topic.trim().is_empty(),
        AppCommand::Podcast(arguments) => {
            !arguments.trim().is_empty() && !arguments.trim().eq_ignore_ascii_case("status")
        }
        AppCommand::Mode(mode) => !mode.trim().eq_ignore_ascii_case("podcast"),
        _ => false,
    }
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

/// Splits `/Say` arguments into a voice and the exact words. A leading word that
/// names a court voice selects it; anything else is spoken by the Queen whole.
fn say_target(text: &str) -> Option<(VoiceName, &str)> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if let Some((first, rest)) = text.split_once(char::is_whitespace)
        && let Ok(voice) = first.parse::<VoiceName>()
        && !rest.trim().is_empty()
    {
        return Some((voice, rest.trim()));
    }
    Some((VoiceName::Queen, text))
}

fn voice_for(agent: teledra_agents::AgentId) -> VoiceName {
    use teledra_agents::AgentId;
    match agent {
        AgentId::Queen => VoiceName::Queen,
        AgentId::Malthus => VoiceName::Malthus,
        AgentId::Wizard => VoiceName::Wizard,
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

#[cfg(test)]
mod tests {
    use super::salvaged_artist_fractus;
    use teledra_agents::AgentId;
    use teledra_court::CourtAction;

    #[test]
    fn say_defaults_to_the_queen_and_accepts_a_named_voice() {
        use super::{VoiceName, say_target};
        assert_eq!(say_target("   "), None);
        assert_eq!(
            say_target("Sing a bright chorus"),
            Some((VoiceName::Queen, "Sing a bright chorus"))
        );
        assert_eq!(
            say_target("wizard  One finding. "),
            Some((VoiceName::Wizard, "One finding."))
        );
        assert_eq!(
            say_target("queen"),
            Some((VoiceName::Queen, "queen")),
            "a bare voice name is words to speak, not a voice with nothing to say"
        );
    }

    #[test]
    fn every_court_member_has_their_own_matching_voice() {
        let mut assigned = std::collections::HashSet::new();
        for agent in AgentId::ALL {
            let voice = super::voice_for(agent);
            assert_eq!(voice.slug(), agent.slug(), "wrong voice for {agent}");
            assert!(assigned.insert(voice), "voice reused by {agent}");
        }
        assert_eq!(
            assigned,
            super::VoiceName::ALL.into_iter().collect(),
            "the voice roster must cover the entire court"
        );
    }

    #[test]
    fn interrupted_audio_cannot_return_as_complete_durable_recollection() {
        let root = tempfile::tempdir().unwrap();
        let mut services =
            super::RuntimeServices::new(root.path().to_owned(), "unused-python".into()).unwrap();
        let memory = teledra_memory::MemoryStore::new(root.path().join("memory")).unwrap();
        let mut app = super::App::new(
            teledra_court::Court::default(),
            teledra_brain::BrainHealth {
                endpoint: "http://localhost:11434".into(),
                model: teledra_brain::QWEN_MODEL,
            },
        );
        let first = app
            .begin_external_turn(AgentId::Queen, "Describe the room")
            .unwrap();
        app.finish_turn_with_speech(
            super::ModelReply {
                id: first.id,
                agent: first.agent,
                user: first.user,
                result: Ok("Copper glows by the door. An unheard obsidian telescope waits behind the curtain.".into()),
                audible: true,
            },
            true,
        );
        app.advance_speech_caption(first.id, 0.0, 10.0);
        let second = app
            .begin_external_turn(AgentId::Artist, "Add a detail")
            .unwrap();
        app.finish_turn_with_speech(
            super::ModelReply {
                id: second.id,
                agent: second.agent,
                user: second.user,
                result: Ok("An unheard amethyst crown rests against the painting.".into()),
                audible: true,
            },
            true,
        );
        super::drain_completed_speech(&mut app, &mut services);
        assert!(
            memory
                .recall("obsidian telescope amethyst crown")
                .unwrap()
                .is_empty()
        );

        app.interrupt_speech_captions();
        super::drain_completed_speech(&mut app, &mut services);
        assert!(
            memory
                .recall("obsidian telescope amethyst crown")
                .unwrap()
                .is_empty()
        );
        assert!(memory.recall("copper").unwrap().contains("queen: Copper …"));
        let mut request = app
            .begin_external_turn(
                AgentId::Queen,
                "Tell me about the obsidian telescope and amethyst crown",
            )
            .unwrap();
        services.enrich_request(&mut app, &mut request);
        assert!(request.messages.iter().all(|message| {
            !message.content.starts_with("Durable recollections")
                && !message.content.contains("waits behind the curtain")
                && !message.content.contains("rests against the painting")
        }));
        let transcript =
            std::fs::read_to_string(root.path().join("memory/court_transcript.jsonl")).unwrap();
        assert_eq!(
            transcript.lines().count(),
            1,
            "only the heard partial is durable"
        );
    }

    #[test]
    fn completed_audio_and_text_fallback_are_saved_to_durable_memory() {
        let root = tempfile::tempdir().unwrap();
        let mut services =
            super::RuntimeServices::new(root.path().to_owned(), "unused-python".into()).unwrap();
        let memory = teledra_memory::MemoryStore::new(root.path().join("memory")).unwrap();
        let mut app = super::App::new(
            teledra_court::Court::default(),
            teledra_brain::BrainHealth {
                endpoint: "http://localhost:11434".into(),
                model: teledra_brain::QWEN_MODEL,
            },
        );
        let request = app
            .begin_external_turn(AgentId::Queen, "Describe the instrument")
            .unwrap();
        app.finish_turn_with_speech(
            super::ModelReply {
                id: request.id,
                agent: request.agent,
                user: request.user,
                result: Ok("A sapphire compass marks the northern doorway.".into()),
                audible: true,
            },
            true,
        );
        super::drain_completed_speech(&mut app, &mut services);
        assert!(memory.recall("sapphire compass").unwrap().is_empty());
        app.finish_speech_caption(request.id);
        super::drain_completed_speech(&mut app, &mut services);
        assert!(
            memory
                .recall("sapphire compass")
                .unwrap()
                .contains("queen: A sapphire compass")
        );

        let request = app
            .begin_external_turn(AgentId::Queen, "Describe the garden")
            .unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let (tools, _) = tokio::sync::mpsc::unbounded_channel();
        tx.send(super::ModelReply {
            id: request.id,
            agent: request.agent,
            user: request.user,
            result: Ok("The juniper hedge shelters the robins from the wind.".into()),
            audible: true,
        })
        .unwrap();
        super::drain_model_replies(&mut rx, &mut app, &mut services, &tools, None);
        super::drain_completed_speech(&mut app, &mut services);
        assert!(
            memory
                .recall("juniper hedge robins")
                .unwrap()
                .contains("queen: The juniper hedge")
        );
        let transcript =
            std::fs::read_to_string(root.path().join("memory/court_transcript.jsonl")).unwrap();
        assert_eq!(
            transcript.lines().count(),
            2,
            "each completed response is saved once"
        );
    }

    #[tokio::test]
    async fn spawn_turn_routes_accepted_python_edits_through_the_artifact_schema() {
        use std::io::Write;
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        };
        use std::time::Duration;

        // Serve every request so an accidental generic critic call is counted,
        // rather than hidden behind a connection failure after the first reply.
        struct LocalModel {
            stop: Arc<AtomicBool>,
            worker: Option<std::thread::JoinHandle<Result<(), String>>>,
        }
        impl LocalModel {
            fn finish(mut self) {
                self.stop.store(true, Ordering::Release);
                self.worker
                    .take()
                    .unwrap()
                    .join()
                    .expect("mock worker panicked")
                    .expect("mock request failed");
            }
        }
        impl Drop for LocalModel {
            fn drop(&mut self) {
                self.stop.store(true, Ordering::Release);
                if let Some(worker) = self.worker.take() {
                    // Cleanup must not panic again if a routing assertion failed.
                    let _ = worker.join();
                }
            }
        }
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let endpoint = format!("http://{address}");
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let requests = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
        let captured = requests.clone();
        let proposal = serde_json::json!({
            "intent":"Let the answering voice enter later",
            "edits":[{"line":4, "new":"answer_start = 0.75"}]
        });
        let response = serde_json::json!({
            "message":{"content":proposal.to_string()}, "done_reason":"stop"
        })
        .to_string();
        let model = LocalModel {
            stop,
            worker: Some(std::thread::spawn(move || {
                while !done.load(Ordering::Acquire) {
                    let Ok((mut socket, _)) = listener.accept() else {
                        std::thread::sleep(Duration::from_millis(1));
                        continue;
                    };
                    let Some(request) = super::python_composition::read_mock_request(&mut socket)
                        .map_err(|error| error.to_string())?
                    else {
                        continue;
                    };
                    captured
                        .lock()
                        .map_err(|error| error.to_string())?
                        .push(request);
                    let _ = write!(
                        socket,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        response.len(),
                        response
                    );
                }
                Ok(())
            })),
        };
        // Exercise cancelled connection cleanup before the real dispatch. Neither
        // an empty socket nor an incomplete JSON body is a model request.
        drop(std::net::TcpStream::connect(address).unwrap());
        {
            let mut partial = std::net::TcpStream::connect(address).unwrap();
            partial
                .write_all(b"POST /api/chat HTTP/1.1\r\nContent-Length: 40\r\n\r\n{\"partial\":")
                .unwrap();
        }
        let brain = super::Brain::new(super::BrainConfig {
            ollama_url: endpoint.clone(),
            connect_timeout: Duration::from_secs(2),
            response_timeout: Duration::from_secs(5),
        })
        .unwrap();

        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("music/python");
        let accepted = home.join("render-dispatch");
        std::fs::create_dir_all(&accepted).unwrap();
        std::fs::write(
            root.path().join("music/playback.json"),
            r#"{"engine":"python","alternating":false}"#,
        )
        .unwrap();
        std::fs::write(
            home.join("current.json"),
            r#"{"artifact":"render-dispatch"}"#,
        )
        .unwrap();
        let source = concat!(
            "import numpy as np\n",
            "from teledra_synth import synth_note, mix_waves, play_sound\n",
            "SR = 44100\n",
            "answer_start = 0.5\n",
            "full_track = np.zeros(SR * 4)\n",
            "answer = synth_note('A4', 0.4, sr=SR)\n",
            "full_track = mix_waves(full_track, answer, start_time=answer_start, sr=SR)\n",
            "play_sound(full_track, sr=SR, loop=True)\n"
        );
        std::fs::write(accepted.join("code.py"), source).unwrap();
        std::fs::write(
            accepted.join("report.json"),
            r#"{"title":"Dispatch study","duration_seconds":4,"metadata":{"key":"A minor","bpm":96}}"#,
        )
        .unwrap();
        std::fs::write(home.join("draft.py"), "UNPLAYED_DRAFT_MUST_NOT_BE_EDITED").unwrap();
        let services =
            super::RuntimeServices::new(root.path().to_owned(), "unused-python".into()).unwrap();
        let mut court = teledra_court::Court::default();
        court.record_exchange_for(
            AgentId::Organist,
            "STALE_COURT_HISTORY_REQUEST",
            "STALE_COURT_HISTORY_PROSE: I would improve the orchestra later.",
        );
        let mut app = super::App::new(
            court,
            teledra_brain::BrainHealth {
                endpoint,
                model: teledra_brain::QWEN_MODEL,
            },
        );
        let brief = "PYMUSIC COMPOSITION: EDIT ACCEPTED SOURCE: Give the answer a later entrance; preserve the other notes and tempo.";
        app.queue_backstage_turn(AgentId::Organist, brief);
        let request = app.begin_queued_external_turn().unwrap();
        assert!(
            request
                .messages
                .iter()
                .any(|message| message.content.contains("STALE_COURT_HISTORY_PROSE"))
        );
        assert!(
            request
                .messages
                .iter()
                .any(|message| message.content.contains("Your current court identity:"))
        );
        let id = request.id;
        let (replies, mut incoming) = tokio::sync::mpsc::unbounded_channel();
        let mut worker = super::spawn_turn(&brain, &replies, request, &services, &mut app);
        match tokio::time::timeout(Duration::from_secs(5), &mut worker).await {
            Ok(result) => result.unwrap(),
            Err(_) => {
                worker.abort();
                let _ = worker.await;
                panic!("the mock-backed production dispatch must finish within five seconds");
            }
        }
        let reply = incoming.try_recv().unwrap();
        assert_eq!(reply.id, id);
        assert_eq!(reply.agent, AgentId::Organist);
        assert_eq!(reply.user, brief);
        assert!(!reply.audible);
        let parsed = teledra_court::parse_response(&reply.result.unwrap());
        let [CourtAction::CreativeArtifact { kind, content }] = parsed.actions.as_slice() else {
            panic!("production dispatch must return one executable edit action");
        };
        assert_eq!(kind, "python_music_edit");
        let edit: serde_json::Value = serde_json::from_str(content).unwrap();
        assert_eq!(
            edit["base"],
            super::runtime::music_edits::python_source_identity(source)
        );
        assert_eq!(edit["intent"], proposal["intent"]);
        assert_eq!(edit["edits"], proposal["edits"]);
        assert_eq!(
            super::runtime::music_edits::apply_python_edit(source, content).unwrap(),
            source.replace("answer_start = 0.5", "answer_start = 0.75")
        );

        model.finish();
        let requests = requests.lock().unwrap();
        assert_eq!(
            requests.len(),
            1,
            "a valid artifact must not invoke the generic prose critic"
        );
        let sent = &requests[0];
        assert_eq!(sent["model"], teledra_brain::QWEN_MODEL);
        assert_eq!(sent["format"]["type"], "object");
        assert_eq!(sent["format"]["additionalProperties"], false);
        assert_eq!(
            sent["format"]["required"],
            serde_json::json!(["intent", "edits"])
        );
        assert_eq!(
            sent["format"]["properties"]["edits"]["items"]["required"],
            serde_json::json!(["line", "new"])
        );
        let replacements = &sent["format"]["properties"]["edits"]["items"]["properties"];
        assert_eq!(replacements["line"]["type"], "integer");
        assert_eq!(
            replacements["line"]["enum"],
            serde_json::json!([1, 2, 3, 4, 5, 6, 7, 8])
        );
        assert!(replacements.get("old").is_none());
        assert!(sent["format"]["properties"].get("base").is_none());
        let messages = sent["messages"].as_array().unwrap();
        let context = messages
            .iter()
            .map(|message| message["content"].as_str().unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        for (index, line) in source.lines().enumerate() {
            let numbered = format!("{}: {line}", index + 1);
            assert_eq!(
                context.lines().filter(|line| *line == numbered).count(),
                1,
                "include accepted source line {} exactly once",
                index + 1
            );
        }
        assert_eq!(
            context.matches("NUMBERED ACCEPTED PYTHON SOURCE").count(),
            1
        );
        assert!(context.contains(brief));
        assert!(
            messages
                .iter()
                .rev()
                .find(|message| message["role"] == "user")
                .unwrap()["content"]
                .as_str()
                .unwrap()
                .contains(brief)
        );
        assert!(context.contains("A minor"));
        assert!(context.contains("theremin (MIDI"));
        for stale in [
            "STALE_COURT_HISTORY",
            "Your current court identity:",
            "UNPLAYED_DRAFT_MUST_NOT_BE_EDITED",
        ] {
            assert!(!context.contains(stale), "artifact request leaked {stale}");
        }
        assert!(
            !messages
                .iter()
                .any(|message| message["role"] == "assistant")
        );
        // Inspect the produced action only: dispatch must not execute Python,
        // open an audio device, or mutate the accepted artifact itself.
        assert_eq!(
            std::fs::read_to_string(accepted.join("code.py")).unwrap(),
            source
        );
        assert_eq!(
            std::fs::read_to_string(home.join("current.json")).unwrap(),
            r#"{"artifact":"render-dispatch"}"#
        );
        let mut files = std::fs::read_dir(&home)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        files.sort();
        assert_eq!(files, ["current.json", "draft.py", "render-dispatch"]);
    }

    #[test]
    fn a_composition_timeout_records_the_error_without_a_fake_missing_score_retry() {
        let root = tempfile::tempdir().unwrap();
        let mut services =
            super::RuntimeServices::new(root.path().to_owned(), "unused-python".into()).unwrap();
        let mut app = super::App::new(
            teledra_court::Court::default(),
            teledra_brain::BrainHealth {
                endpoint: "http://localhost:11434".into(),
                model: teledra_brain::QWEN_MODEL,
            },
        );
        let request = app
            .begin_external_turn(
                AgentId::Organist,
                "NATIVE COMPOSITION: COMPOSER NUDGE: Compose a complete piece",
            )
            .unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let (tools, _) = tokio::sync::mpsc::unbounded_channel();
        tx.send(super::ModelReply {
            id: request.id,
            agent: request.agent,
            user: request.user,
            result: Err("total composition deadline exceeded".into()),
            audible: false,
        })
        .unwrap();
        super::drain_model_replies(&mut rx, &mut app, &mut services, &tools, None);
        assert!(app.pending().is_none());
        assert!(
            app.begin_queued_external_turn().is_none(),
            "a timeout is not a missing-score prose reply"
        );
        let evidence = std::fs::read_to_string(root.path().join("music/learning.json")).unwrap();
        assert!(evidence.contains("generation_failed"));
        assert!(evidence.contains("total composition deadline exceeded"));
    }

    #[test]
    fn court_priority_offers_backstage_a_bounded_opportunity_after_three_public_turns() {
        let mut balance = super::CourtTurnBalance::default();
        let mut app = super::App::new(
            teledra_court::Court::default(),
            teledra_brain::BrainHealth {
                endpoint: "http://localhost:11434".into(),
                model: teledra_brain::QWEN_MODEL,
            },
        );
        app.queue_backstage_turn(AgentId::Organist, "Develop the current composition");
        for _ in 0..3 {
            assert!(!balance.allows_backstage(true, true));
            assert!(
                app.begin_queued_external_turn_with_priority(
                    true,
                    balance.allows_backstage(true, true)
                )
                .is_none()
            );
            let request = app.begin_autonomous_turn().unwrap();
            balance.started(request.audible);
            app.finish_turn(super::ModelReply {
                id: request.id,
                agent: request.agent,
                user: request.user,
                result: Ok(format!("Public reply {}.", request.id)),
                audible: true,
            });
        }
        assert!(balance.allows_backstage(true, true));
        let quiet = app
            .begin_queued_external_turn_with_priority(true, balance.allows_backstage(true, true))
            .unwrap();
        assert_eq!(quiet.agent, AgentId::Organist);
        assert!(!quiet.audible);
        balance.started(quiet.audible);
        assert!(!balance.allows_backstage(true, true));
        assert!(balance.allows_backstage(false, true));
        assert!(balance.allows_backstage(true, false));
    }

    #[test]
    fn maintenance_has_a_shorter_budget_without_cutting_complete_compositions_short() {
        use std::time::Duration;
        assert_eq!(
            super::inference_budget(false, true, false),
            Duration::from_secs(45)
        );
        assert_eq!(
            super::inference_budget(false, true, true),
            Duration::from_secs(90)
        );
        assert_eq!(
            super::inference_budget(false, false, false),
            Duration::from_secs(90)
        );
        assert_eq!(
            super::inference_budget(true, true, false),
            Duration::from_secs(60)
        );
        assert!(!super::composition_turn_requested(
            AgentId::Organist,
            "The floor is yours. Discuss the motif and let the current piece breathe."
        ));
        for prompt in [
            "PYMUSIC COMPOSITION: develop the motif",
            "NATIVE COMPOSITION nudge",
            "Return one complete [PYTHON_MUSIC: code]",
            "Compose a layered nocturne",
        ] {
            assert!(super::composition_turn_requested(AgentId::Organist, prompt));
            assert!(!super::composition_turn_requested(AgentId::Queen, prompt));
        }
    }

    #[test]
    fn continuously_queued_music_and_art_share_slots_until_both_wizard_jobs_dispatch() {
        let mut app = super::App::new(
            teledra_court::Court::default(),
            teledra_brain::BrainHealth {
                endpoint: "http://unused.invalid".into(),
                model: teledra_brain::QWEN_MODEL,
            },
        );
        let mut balance = super::CourtTurnBalance::default();
        let mut wizard_jobs = Vec::new();
        let mut origins = Vec::new();
        for slot in 0..48 {
            app.queue_backstage_turn(AgentId::Organist, format!("Music development {slot}"));
            app.queue_backstage_turn(AgentId::Artist, format!("Art development {slot}"));
            // Three intervening public starts make the next quiet slot eligible,
            // even while the public cadence remains overdue.
            for _ in 0..3 {
                balance.started(true);
            }
            let allow = balance.allows_backstage(true, true);
            let (request, origin) =
                super::begin_ready_work(&mut app, &balance, true, allow, true).unwrap();
            origins.push(origin);
            if request.agent == AgentId::Wizard {
                wizard_jobs.push(request.user.clone());
            }
            match origin {
                super::WorkOrigin::External => balance.started(request.audible),
                super::WorkOrigin::NightDesk => balance.started_nightdesk(request.audible),
            }
            assert!(
                super::begin_ready_work(&mut app, &balance, true, true, true).is_none(),
                "another worker cannot start while this request is pending"
            );
            app.finish_turn(super::ModelReply {
                id: request.id,
                agent: request.agent,
                user: request.user,
                result: Ok(format!("Completed local dispatch {}", request.id)),
                audible: request.audible,
            });
        }
        assert!(origins.iter().enumerate().all(|(index, origin)| *origin
            == if index % 2 == 0 {
                super::WorkOrigin::NightDesk
            } else {
                super::WorkOrigin::External
            }));
        assert_eq!(wizard_jobs.len(), 2);
        assert!(wizard_jobs[0].starts_with(crate::app::WIZARD_APP_EXPERIMENT));
        assert!(wizard_jobs[1].starts_with(crate::app::WIZARD_MUSIC_STUDY));
        assert_eq!(app.wizard_work_turn(), 2);
    }

    #[test]
    fn nightdesk_sharing_preserves_audience_priority_and_does_not_start_before_due() {
        let make_app = || {
            super::App::new(
                teledra_court::Court::default(),
                teledra_brain::BrainHealth {
                    endpoint: "http://unused.invalid".into(),
                    model: teledra_brain::QWEN_MODEL,
                },
            )
        };
        let mut app = make_app();
        app.queue_external_turn(AgentId::Queen, "An audience question");
        app.queue_backstage_turn(AgentId::Artist, "Queued painting");
        let balance = super::CourtTurnBalance::default();
        let (audience, origin) =
            super::begin_ready_work(&mut app, &balance, true, true, true).unwrap();
        assert!(audience.audible);
        assert_eq!(origin, super::WorkOrigin::External);
        assert_eq!(audience.user, "An audience question");
        let mut app = make_app();
        assert!(super::begin_ready_work(&mut app, &balance, true, true, false).is_none());
        assert!(super::begin_ready_work(&mut app, &balance, true, false, true).is_none());
        let (due, origin) = super::begin_ready_work(&mut app, &balance, true, true, true).unwrap();
        assert_eq!(origin, super::WorkOrigin::NightDesk);
        assert!(due.user.starts_with("NIGHT DESK REVIEW"));
        assert_eq!(
            app.wizard_work_turn(),
            0,
            "non-Wizard jobs do not spend its work slots"
        );
    }

    #[tokio::test]
    async fn replacing_inference_drops_the_old_worker_before_starting_the_next() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        struct Dropped(Arc<AtomicBool>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let flag = dropped.clone();
        let mut inference = super::ActiveInference::default();
        let (started, ready) = tokio::sync::oneshot::channel();
        inference
            .start(|| {
                tokio::spawn(async move {
                    let _guard = Dropped(flag);
                    let _ = started.send(());
                    std::future::pending::<()>().await;
                })
            })
            .await;
        ready.await.unwrap();
        inference
            .start(|| {
                assert!(dropped.load(Ordering::SeqCst));
                tokio::spawn(async {})
            })
            .await;
        inference.cancel().await;
        assert!(inference.0.is_none());
    }

    #[test]
    fn python_composition_counts_as_music_without_counting_legacy_or_unrelated_artifacts() {
        let parsed = teledra_court::parse_response(
            "[PYTHON_MUSIC: import numpy as np\nfull_track = np.zeros(44100)\n]",
        );
        assert!(parsed.actions.iter().any(super::is_music_action));
        assert!(super::is_music_action(&CourtAction::CourtScore(
            "{}".into()
        )));
        for kind in ["strudel_music", "python_art", "suggestion"] {
            assert!(!super::is_music_action(&CourtAction::CreativeArtifact {
                kind: kind.into(),
                content: "a draft".into(),
            }));
        }
    }

    #[test]
    fn an_art_exploration_does_not_research_its_supplied_source_again() {
        let prompt = "ARTIST EXPLORATION: Create a new loop. PUBLIC RESEARCH EVIDENCE: https://mathworld.wolfram.com/Hypotrochoid.html describes rolling circles.";
        let mut actions = vec![
            CourtAction::Research("https://mathworld.wolfram.com/Hypotrochoid.html".to_owned()),
            CourtAction::Fractus {
                payload: "version 2\ntype=spirograph".to_owned(),
                live: true,
            },
            CourtAction::Research("phyllotaxis spiral packing".to_owned()),
        ];
        super::remove_redundant_art_research(prompt, &mut actions);
        assert_eq!(actions.len(), 2);
        assert!(matches!(
            &actions[0],
            CourtAction::Fractus { live: true, .. }
        ));
        let mut ordinary = vec![CourtAction::Research(
            "https://mathworld.wolfram.com/Hypotrochoid.html".to_owned(),
        )];
        super::remove_redundant_art_research("Please research this source", &mut ordinary);
        assert_eq!(ordinary.len(), 1);
    }

    #[test]
    fn stale_replies_cannot_write_files_and_current_replies_execute_at_most_four_actions() {
        let root = tempfile::tempdir().unwrap();
        let mut services =
            super::RuntimeServices::new(root.path().to_owned(), "python".into()).unwrap();
        let mut app = super::App::new(
            teledra_court::Court::default(),
            teledra_brain::BrainHealth {
                endpoint: "http://localhost:11434".to_owned(),
                model: teledra_brain::QWEN_MODEL,
            },
        );
        let request = app
            .begin_external_turn(AgentId::Scribe, "Record the work")
            .unwrap();
        let (replies, mut incoming) = tokio::sync::mpsc::unbounded_channel();
        let (tools, _work) = tokio::sync::mpsc::unbounded_channel();
        let raw = "[SCRIBE_APPEND: notes/evidence.md|A real record.]".repeat(5);
        let reply = |id| super::ModelReply {
            id,
            agent: request.agent,
            user: request.user.clone(),
            result: Ok(raw.clone()),
            audible: false,
        };
        replies.send(reply(request.id + 1)).unwrap();
        super::drain_model_replies(&mut incoming, &mut app, &mut services, &tools, None);
        let record = root.path().join("records/notes/evidence.md");
        assert!(!record.exists());
        assert_eq!(app.pending().unwrap().id, request.id);
        replies.send(reply(request.id)).unwrap();
        super::drain_model_replies(&mut incoming, &mut app, &mut services, &tools, None);
        assert_eq!(std::fs::read_to_string(record).unwrap().lines().count(), 4);
        assert!(app.pending().is_none());
    }

    #[test]
    fn python_requests_can_recover_one_complete_code_fence_without_guessing_at_prose() {
        assert_eq!(
            super::fenced_python_composition("```python\nplay_sound(piece)\n```"),
            Some("play_sound(piece)")
        );
        assert!(
            super::fenced_python_composition("Here is some Python: ```python\nx=1\n```").is_none()
        );
        assert!(super::fenced_python_composition("```json\n{}\n```").is_none());
        assert!(super::fenced_python_composition("```python\nx=1").is_none());
        assert!(
            super::fenced_python_composition("```python\nx=1\n```\n```python\ny=2\n```").is_none()
        );
    }

    #[tokio::test]
    async fn stopping_music_cancels_inference_and_discards_its_late_reply_before_actions() {
        let root = tempfile::tempdir().unwrap();
        let mut services =
            super::RuntimeServices::new(root.path().to_owned(), "unused-python".into()).unwrap();
        let mut app = super::App::new(
            teledra_court::Court::default(),
            teledra_brain::BrainHealth {
                endpoint: "http://localhost:11434".into(),
                model: teledra_brain::QWEN_MODEL,
            },
        );
        let request = app
            .begin_external_turn(AgentId::Organist, "PYMUSIC COMPOSITION: compose")
            .unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let (tools, mut work) = tokio::sync::mpsc::unbounded_channel();
        services.handle_command(
            crate::app::AppCommand::MusicStop,
            &mut app,
            &root.path().join("snapshot.json"),
            &tools,
        );
        let mut inference = super::ActiveInference::default();
        inference
            .start(|| tokio::spawn(std::future::pending()))
            .await;
        super::cancel_stopped_composition(&mut inference, &mut app, &services).await;
        assert!(inference.0.is_none());
        assert!(
            app.pending().is_none(),
            "Stop must free the court without awaiting the model"
        );
        tx.send(super::ModelReply {
            id: request.id,
            agent: request.agent,
            user: request.user,
            result: Ok("[PYTHON_MUSIC: print('stale')]".into()),
            audible: false,
        })
        .unwrap();
        super::drain_model_replies(&mut rx, &mut app, &mut services, &tools, None);
        assert!(app.pending().is_none());
        assert!(!root.path().join("music/python").exists());
        assert!(work.try_recv().is_err());
    }

    #[test]
    fn only_untagged_artist_prose_is_salvaged() {
        let prose = "The Mandala family should unfold in neon sunset light.";
        assert!(matches!(
            salvaged_artist_fractus(AgentId::Artist, "make art", prose, &[]),
            Some((_, true))
        ));
        assert!(salvaged_artist_fractus(AgentId::Queen, "make art", prose, &[]).is_none());

        let existing = vec![CourtAction::Fractus {
            payload: "mandala".to_owned(),
            live: true,
        }];
        assert!(salvaged_artist_fractus(AgentId::Artist, "make art", prose, &existing).is_none());
    }

    #[test]
    fn an_explicit_still_request_is_not_turned_into_a_gif() {
        let action = salvaged_artist_fractus(
            AgentId::Artist,
            "Please make a still image for the archive",
            "A lissajous study in electric cyan.",
            &[],
        )
        .unwrap();
        assert!(!action.1);
    }
}
