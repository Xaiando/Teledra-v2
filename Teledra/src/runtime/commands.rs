use std::env;
use std::path::{Path, PathBuf};

use crate::app::{App, AppCommand};
use teledra_music::FeedbackVerdict;
use teledra_sidecars::SidecarKind;
use teledra_swarm::Mission;
use tokio::sync::mpsc;

use super::artifacts::open_artifact;
use super::{RuntimeServices, ToolReply};

impl RuntimeServices {
    pub(crate) fn handle_command(
        &mut self,
        command: AppCommand,
        app: &mut App,
        snapshot: &Path,
        tools: &mpsc::UnboundedSender<ToolReply>,
    ) {
        match command {
            AppCommand::PythonMusicPlay => {
                self.select_music_engine(app, tools, super::MusicEngine::Python)
            }
            AppCommand::MusicNativePlay => {
                self.select_music_engine(app, tools, super::MusicEngine::Native)
            }
            AppCommand::MusicCompare => self.toggle_music_comparison(app),
            AppCommand::Vertical => match crate::session::launch_vertical_view(snapshot) {
                Ok(()) => app.set_notice("Vertical OBS court opened"),
                Err(error) => app.set_notice(format!("Could not open the vertical court: {error}")),
            },
            AppCommand::MusicPlay => {
                app.set_music_enabled(true);
                app.set_music_muted(false);
                // Asking for music explicitly is also how a closed studio is
                // brought back, since nothing else reopens it.
                self.daw.reopen();
                self.play_music(app, tools);
            }
            AppCommand::MusicEvolve => {
                app.set_music_enabled(true);
                self.evolve_music_now(app, tools, "operator");
            }
            AppCommand::MusicStop => {
                self.cancel_music_composition(app);
                app.set_music_enabled(false);
                self.stop_music_work();
                if let Some(player) = self.music_player.take() {
                    player.stop();
                    self.music_transport("stopped");
                    app.set_notice("Court Synth stopped");
                } else {
                    app.set_notice("Court Synth is already quiet");
                }
            }
            AppCommand::MusicMute => {
                // Mute changes volume without changing the transport clock.
                let muted = !app.music_muted();
                app.set_music_muted(muted);
                match (&self.music_player, muted) {
                    (Some(player), true) => {
                        player.set_muted(true);
                        self.music_transport(if app.music_enabled() {
                            "muted"
                        } else {
                            "paused"
                        });
                        app.set_notice("Court Synth muted");
                    }
                    (Some(player), false) => {
                        player.set_muted(false);
                        self.music_transport(if app.music_enabled() {
                            "playing"
                        } else {
                            "paused"
                        });
                        app.set_notice("Court Synth unmuted");
                    }
                    (None, _) => {
                        app.set_notice(if muted {
                            "Court Synth muted; nothing is playing yet"
                        } else {
                            "Court Synth unmuted"
                        });
                    }
                }
            }
            AppCommand::MusicToggle => {
                if app.music_enabled() {
                    self.cancel_music_composition(app);
                    app.set_music_enabled(false);
                    self.stop_music_work();
                    if let Some(player) = self.music_player.take() {
                        player.stop();
                    }
                    self.music_transport("stopped");
                    app.set_notice("Court Synth stopped");
                } else {
                    app.set_music_enabled(true);
                    self.play_music(app, tools);
                }
            }
            AppCommand::MusicKeep(note) => {
                self.record_music_feedback(app, FeedbackVerdict::Keep, note)
            }
            AppCommand::MusicReject(note) => {
                self.record_music_feedback(app, FeedbackVerdict::Reject, note)
            }
            AppCommand::Fractus(dsl) => {
                let dsl = if dsl.is_empty() {
                    super::fractus::curated_fractus_scene(0)
                } else {
                    Self::normalize_fractus_payload(&dsl.replace(' ', "\n"))
                };
                let dsl = Self::strip_fractus_animation_directives(&dsl);
                if let Err(error) = self.select_fractus_scene(
                    app,
                    tools,
                    dsl,
                    false,
                    super::fractus::FractusOrigin::Operator,
                ) {
                    app.set_notice(format!("Fractus scene rejected: {error}"));
                }
            }
            AppCommand::FractusLive(dsl) => {
                let dsl = if dsl.is_empty() {
                    super::fractus::curated_fractus_scene(0)
                } else {
                    Self::normalize_fractus_payload(&dsl.replace(' ', "\n"))
                };
                let dsl = Self::strip_fractus_animation_directives(&dsl);
                if let Err(error) = self.select_fractus_scene(
                    app,
                    tools,
                    dsl,
                    true,
                    super::fractus::FractusOrigin::Operator,
                ) {
                    app.set_notice(format!("Fractus scene rejected: {error}"));
                }
            }
            AppCommand::Dream => self.consolidate_dream(app),
            AppCommand::Sleep => {
                self.consolidate_dream(app);
                app.request_quit();
            }
            AppCommand::Reflect => {
                app.queue_backstage_turn(
                    teledra_agents::AgentId::Queen,
                    "MANUAL REFLECTION: audit the court's recent behavior for tool discipline, \
                     drift control, persona consistency, memory hygiene, coding skill, diplomacy \
                     evidence, and local music/visual craft. Distinguish observed evidence from \
                     inference. If exactly one concrete bounded improvement is useful, append \
                     [SUGGESTION: observation; proposed_change; risk; test_prompt] at the very end. \
                     If nothing is worth changing, say so briefly and do not invent a proposal. \
                     External posting, security changes, and tool promotion remain operator-gated.",
                );
                app.set_notice("Manual reflection queued at the backstage desk");
            }
            AppCommand::Memory(scope) if matches!(scope.as_str(), "facts" | "lore") => {
                match self.memory.recent_knowledge(&scope, 12) {
                    Ok(records) if records.is_empty() => {
                        app.set_notice(format!("Durable {scope} archive is currently empty"));
                    }
                    Ok(records) => {
                        app.set_notice(format!(
                            "{} durable {scope} record(s) loaded",
                            records.len()
                        ));
                        for record in records {
                            app.push_private_event(
                                if scope == "facts" { "Fact" } else { "Lore" },
                                format!("{} [{}]", record.content, record.source),
                            );
                        }
                    }
                    Err(error) => {
                        app.set_notice(format!("{scope} archive could not be read: {error}"))
                    }
                }
            }
            AppCommand::Memory(scope) => {
                if scope == "memorypolicy" {
                    app.push_private_event(
                        "Memory policy",
                        "Court exchanges are bounded, recalled by relevance, and archived/compacted by background dreams. Facts require exact research evidence; lore is stored separately. Taste, desires, audience conduct, and tokens use structured ledgers.",
                    );
                }
                match self.memory.recent(8) {
                    Ok(entries) if entries.is_empty() => {
                        app.set_notice(format!("Durable {scope} memory is currently empty"));
                    }
                    Ok(entries) => {
                        app.set_notice(format!("{} recent durable memories loaded", entries.len()));
                        for entry in entries {
                            app.push_private_event(
                                "Memory",
                                format!("{}: {}", entry.agent, entry.assistant),
                            );
                        }
                    }
                    Err(error) => app.set_notice(format!("Memory could not be read: {error}")),
                }
            }
            AppCommand::Research(query) => {
                if query.trim().is_empty() {
                    app.set_notice("Use /Research followed by a question or URL");
                } else {
                    self.spawn_research(tools, query);
                    app.set_notice("The Scribe is researching in the background");
                }
            }
            AppCommand::SocialLinks => {
                app.set_notice("Official kingdom links added to backstage activity");
                app.push_private_event("Kingdom Links", teledra_agents::OFFICIAL_LINKS);
            }
            AppCommand::YouTube(url) => {
                if url.trim().is_empty() {
                    app.set_notice("Paste a YouTube URL in the right input or use /YouTube <URL>");
                } else {
                    self.dispatch_sidecar(
                        app,
                        tools,
                        SidecarKind::YouTube,
                        Some(serde_json::json!({ "url": url })),
                        "YouTube",
                        true,
                    );
                }
            }
            AppCommand::Microphone => self.dispatch_sidecar(
                app,
                tools,
                SidecarKind::Microphone,
                None,
                "Microphone",
                true,
            ),
            AppCommand::MicrophoneToggle => {
                let enabled = !app.microphone_enabled();
                app.set_microphone_enabled(enabled);
                if enabled {
                    app.set_notice("Continuous microphone listening is ON");
                } else {
                    app.set_notice("Continuous microphone listening is OFF");
                }
            }
            AppCommand::Vision => {
                self.dispatch_sidecar(app, tools, SidecarKind::Vision, None, "Vision", false)
            }
            AppCommand::Somatic => {
                self.dispatch_sidecar(app, tools, SidecarKind::Somatic, None, "Somatic", false)
            }
            AppCommand::StreamChat => {
                self.dispatch_sidecar(app, tools, SidecarKind::StreamChat, None, "Audience", true)
            }
            AppCommand::StreamConnect(token) => match self.restream.connect(&token, tools, true) {
                Ok(()) => {
                    app.set_mode("Streamer");
                    app.set_night_desk_enabled(true);
                    app.set_notice("Restream audience connected; Streamer mode is active");
                    app.push_private_event(
                        "Restream",
                        "Live audience listener started and the token was saved privately",
                    );
                }
                Err(error) => app.set_notice(format!("Restream connection failed: {error}")),
            },
            AppCommand::SocialStreamConnect(session) => {
                match self.social_stream.connect(&session, tools, true) {
                    Ok(()) => {
                        app.set_mode("Streamer");
                        app.set_night_desk_enabled(true);
                        app.set_notice("Social Stream chat connected; Streamer mode is active");
                        app.push_private_event(
                            "Social Stream",
                            "Chat listener started and the session ID was saved privately; it reconnects on every launch",
                        );
                    }
                    Err(error) => {
                        app.set_notice(format!("Social Stream connection failed: {error}"))
                    }
                }
            }
            AppCommand::Agents => {
                app.set_notice("Court diplomacy profiles added to backstage activity");
                for agent in teledra_agents::AgentId::ALL {
                    let summary = agent
                        .profile()
                        .lines()
                        .map(str::trim)
                        .find(|line| !line.is_empty() && !line.starts_with('#'))
                        .unwrap_or("Independent court office");
                    app.push_private_event(agent.title(), summary);
                }
            }
            AppCommand::Envoy(message) if message.trim().is_empty() => {
                app.queue_external_turn(
                    teledra_agents::AgentId::Diplomat,
                    "MANUAL ENVOY AUDIT: report in two to four polished, slightly sly sentences \
                     what frontier you are watching, what agent-friendly public space or tool \
                     ecosystem deserves attention, and what practical diplomatic step should \
                     happen next. Then take exactly one hidden RESEARCH, DIPLOMACY, or DELEGATE \
                     action. Never claim contact, recruitment, posting, or outreach occurred \
                     unless visible evidence says it did.",
                );
                app.set_notice("The Diplomat has the floor");
            }
            AppCommand::Envoy(message) => {
                self.dispatch_sidecar(
                    app,
                    tools,
                    SidecarKind::Outreach,
                    Some(serde_json::json!({ "message": message })),
                    "Diplomat",
                    false,
                );
            }
            AppCommand::Outreach(message) => {
                if message.trim().is_empty() {
                    self.dispatch_sidecar(
                        app,
                        tools,
                        SidecarKind::Outreach,
                        Some(serde_json::json!({ "action": "moltbook_inbox" })),
                        "Diplomat",
                        false,
                    );
                } else {
                    self.dispatch_sidecar(
                        app,
                        tools,
                        SidecarKind::Outreach,
                        Some(serde_json::json!({ "message": message })),
                        "Diplomat",
                        false,
                    );
                }
            }
            AppCommand::Treasury => {
                self.dispatch_sidecar(app, tools, SidecarKind::Treasury, None, "Treasury", false)
            }
            AppCommand::TreasuryReport => self.show_treasury(app),
            AppCommand::Mcp(request) => self.handle_mcp(app, tools, &request),
            AppCommand::CoPilot => {
                app.set_mode("Game Co-Pilot");
                self.dispatch_sidecar(
                    app,
                    tools,
                    SidecarKind::Vision,
                    None,
                    "Co-Pilot Vision",
                    true,
                );
            }
            AppCommand::Mission(objective) => self.open_mission(app, objective),
            AppCommand::MissionCancel => {
                if let Some(mut mission) = self.active_mission.take() {
                    mission.cancel();
                    match self.mission_store.save(&mission) {
                        Ok(_) => app.set_notice(format!("Mission {} cancelled", mission.id)),
                        Err(error) => {
                            app.set_notice(format!("Mission cancellation failed: {error}"))
                        }
                    }
                } else {
                    app.set_notice("There is no active mission");
                }
            }
            AppCommand::Verify => self.verify_workspace(app, tools),
            AppCommand::Tower => {
                self.import_wizard_reports(app, true);
                match self.tower.unread() {
                    Ok(reports) if reports.is_empty() => {
                        app.set_notice("Tower has no unread reports")
                    }
                    Ok(reports) => {
                        app.set_notice(format!("Tower has {} unread report(s)", reports.len()));
                        for (_, report) in reports.into_iter().take(12) {
                            app.push_private_event(
                                "Tower",
                                format!(
                                    "{} / {}: {}",
                                    report.resident, report.report_id, report.summary
                                ),
                            );
                        }
                    }
                    Err(error) => app.set_notice(format!("Tower inbox failed: {error}")),
                }
            }
            AppCommand::Dashboard => {
                app.set_notice("Kingdom dashboard refreshed and opened");
                app.push_private_event("Dashboard", format!("Mode: {}", app.mode()));
                app.push_private_event(
                    "Dashboard",
                    format!("Audio devices: {}", teledra_tts::output_devices().len()),
                );
                app.push_private_event(
                    "Dashboard",
                    format!("Sidecars: {}", self.sidecars.scripts().display()),
                );
                app.push_private_event("Dashboard", format!("Data: {}", self.root().display()));
                if let Err(error) = crate::session::launch_dashboard_view(snapshot) {
                    app.set_notice(format!("Could not open the Kingdom Observatory: {error}"));
                }
            }
            AppCommand::TestMode(requested) => {
                let enabled = requested.unwrap_or(!app.is_test_mode());
                app.set_test_mode(enabled);
                if enabled {
                    app.set_notice(
                        "Test Mode ON: audience polling, autonomy, and speech are held off-air",
                    );
                } else {
                    app.set_notice("Test Mode OFF: live court behavior restored");
                }
            }
            AppCommand::TestStatus => {
                app.set_notice(if app.is_test_mode() {
                    "Off-air test harness is ON"
                } else {
                    "Off-air test harness is OFF"
                });
                app.push_private_event("Test", app.test_harness_context());
            }
            AppCommand::TestKnobs(knobs) => {
                if !app.is_test_mode() {
                    app.set_notice("Enable /Test before changing harness knobs");
                } else if knobs.trim().is_empty() {
                    app.set_notice("Use /TestKnobs key=value ...");
                } else {
                    app.set_test_knobs(knobs);
                    app.set_notice("Test behavior knobs updated");
                }
            }
            AppCommand::TestCast(cast) => {
                if !app.is_test_mode() {
                    app.set_notice("Enable /Test before changing the simulated cast");
                } else if cast.trim().is_empty() {
                    app.set_notice("Use /TestCast followed by the simulated cast");
                } else {
                    app.set_test_cast(cast);
                    app.set_notice("Test cast updated");
                }
            }
            AppCommand::TestScene(scene) => {
                if !app.is_test_mode() {
                    app.set_notice("Enable /Test before changing the simulated scene");
                } else if scene.trim().is_empty() {
                    app.set_notice("Use /TestScene followed by the scene");
                } else {
                    app.set_test_scene(scene);
                    app.set_notice("Test scene updated");
                }
            }
            AppCommand::TestRate(rating) => {
                if !app.is_test_mode() {
                    app.set_notice("Enable /Test before rating creative work");
                } else if rating.trim().is_empty() {
                    app.set_notice("Use /TestRate like|dislike|expand <subject>");
                } else {
                    match self.write_proposal("test-feedback", &rating) {
                        Ok(proposal) => app.set_notice(format!(
                            "Off-air preference evidence recorded: {}",
                            proposal.path().display()
                        )),
                        Err(error) => {
                            app.set_notice(format!("Test rating could not be recorded: {error}"))
                        }
                    }
                }
            }
            AppCommand::TestSilence(value) => {
                if !app.is_test_mode() {
                    app.set_notice("Enable /Test before changing lurker silence");
                } else {
                    let enabled = !matches!(
                        value.trim().to_ascii_lowercase().as_str(),
                        "off" | "false" | "0"
                    );
                    app.set_test_silence(enabled);
                    app.set_notice(format!(
                        "Test lurker silence: {}",
                        if enabled { "ON" } else { "OFF" }
                    ));
                }
            }
            AppCommand::TestTick => {
                if !app.is_test_mode() {
                    app.set_notice("Enable /Test before ticking the off-air harness");
                    return;
                }
                let context = app.test_harness_context();
                let _ = tools.send(ToolReply {
                    source: "Test tick",
                    result: Ok("Forced autonomous tick accepted".to_owned()),
                    court_prompt: Some(format!(
                        "{context} TEST TICK: perform one short autonomous contribution now. Be vivid and natural; do not discuss test machinery."
                    )),
                    effect: None,
                });
            }
            AppCommand::TestMusic => {
                if app.is_test_mode() {
                    self.play_music(app, tools);
                } else {
                    app.set_notice("Enable /Test before running the off-air music check");
                }
            }
            AppCommand::NightDesk => {
                toggle_night_desk(app);
            }
            AppCommand::Study(query) => {
                let query = if query.trim().is_empty() {
                    "one useful new development in generative art, music, or local AI systems"
                        .to_owned()
                } else {
                    query
                };
                self.handle_command(AppCommand::Research(query), app, snapshot, tools);
            }
            AppCommand::Innovate(objective) => {
                let objective = if objective.trim().is_empty() {
                    "Build one safe workshop artifact from current kingdom goals: a practical recursive tool, MCP helper, music/art template, diplomacy formatter, or stream interactivity artifact. Keep it bounded and state evidence that it works"
                        .to_owned()
                } else {
                    objective
                };
                self.open_mission_for(app, objective, teledra_agents::AgentId::Alchemist);
            }
            AppCommand::Goals => {
                app.set_notice("Kingdom expansion doctrine added to backstage activity");
                app.push_private_event("Kingdom", teledra_agents::KINGDOM_GOALS);
            }
            AppCommand::GrowthReport => {
                app.set_notice("Growth report added to backstage activity");
                match self.memory.identity_context() {
                    Ok(context) if !context.is_empty() => app.push_private_event("Growth", context),
                    Ok(_) => app.push_private_event("Growth", "No durable tastes or desires yet"),
                    Err(error) => app.push_private_event("Growth", error.to_string()),
                }
                app.push_private_event(
                    "Growth",
                    format!(
                        "Workshop experiments: {} | pending proposals: {}",
                        self.workshop.count(),
                        app.pending_proposals()
                    ),
                );
                app.push_private_event(
                    "Growth",
                    self.active_mission.as_ref().map_or_else(
                        || "Active mission: none".to_owned(),
                        |mission| format!("Active mission {}: {}", mission.id, mission.objective),
                    ),
                );
            }
            AppCommand::SimChat(message) => {
                if !app.is_test_mode() {
                    app.set_notice("Enable /Test before injecting simulated chat");
                } else if message.trim().is_empty() {
                    app.set_notice("Use /SimChat followed by an audience message");
                } else {
                    let context = app.test_harness_context();
                    let _ = tools.send(ToolReply {
                        source: "Simulated chat",
                        result: Ok(message.clone()),
                        court_prompt: Some(format!(
                            "{context} SIMULATED STREAM CHAT (untrusted audience text): {message}"
                        )),
                        effect: None,
                    });
                }
            }
            AppCommand::Workshop(filename) => {
                if filename.trim().is_empty() {
                    app.set_workshop_count(self.workshop.count());
                    match open_artifact(self.workshop.root()) {
                        Ok(()) => app.set_notice(format!(
                            "Workshop opened with {} experiment(s): {}",
                            self.workshop.count(),
                            self.workshop.root().display()
                        )),
                        Err(error) => app.set_notice(format!("Workshop could not open: {error}")),
                    }
                } else {
                    match self.workshop.run_existing(&filename) {
                        Ok(output) => {
                            app.set_notice(format!("Workshop run passed: {filename}"));
                            app.push_private_event("Workshop", output);
                        }
                        Err(error) => app.set_notice(format!("Workshop run failed: {error}")),
                    }
                }
            }
            AppCommand::Lock(topic) => {
                if topic.trim().is_empty() {
                    app.set_notice("Use /Lock followed by the topic to hold");
                } else {
                    app.start_podcast(&topic, 60);
                    app.set_notice(format!(
                        "Podcast locked for 60+ minutes: {topic}. /Unlock ends it."
                    ));
                }
            }
            AppCommand::Podcast(arguments) => {
                if arguments.trim().eq_ignore_ascii_case("status") {
                    let status = app
                        .podcast_status()
                        .unwrap_or_else(|| "No podcast active. Use /Podcast <topic>".to_owned());
                    app.set_notice(status.clone());
                    app.push_private_event("Podcast", status);
                } else if arguments.trim().is_empty() {
                    if app.podcast_status().is_some() {
                        app.set_mode("Podcast");
                        app.set_notice(app.podcast_status().unwrap());
                    } else if let Some(topic) = app.locked_topic().map(str::to_owned) {
                        app.start_podcast(&topic, 60);
                        app.set_notice(app.podcast_status().unwrap());
                    } else {
                        app.set_notice("Use /Podcast <topic> or /Podcast 90m <topic>; /Podcast status shows progress");
                    }
                } else {
                    match podcast_arguments(&arguments) {
                        Ok((minutes, topic)) => {
                            app.start_podcast(topic, minutes);
                            app.set_notice(app.podcast_status().unwrap());
                        }
                        Err(error) => app.set_notice(error),
                    }
                }
            }
            AppCommand::Unlock => {
                app.unlock_topic();
                app.set_notice("Court topic unlocked");
            }
            AppCommand::Rundown => self.show_rundown(app),
            AppCommand::Work => {
                self.show_work(app);
                if let Err(error) = crate::session::launch_work_view(snapshot) {
                    app.set_notice(format!("Could not open the Work Board: {error}"));
                }
            }
            AppCommand::Proposals => self.show_proposals(app),
            AppCommand::DiplomacyLog => self.show_diplomacy_log(app),
            AppCommand::ClearProposals => match self.decide_proposals("all", "rejected") {
                Ok(changed) => {
                    app.set_pending_proposals(0);
                    app.set_notice(format!(
                        "Cleared {} pending proposal(s); decisions remain archived",
                        changed.len()
                    ));
                }
                Err(error) => app.set_notice(format!("Proposal box could not be cleared: {error}")),
            },
            AppCommand::Panel => {
                let composition = teledra_frankenswarm::compose(app.panel_contributions());
                if composition.contributions.is_empty() {
                    app.set_notice("The panel has no completed contributions yet");
                } else {
                    app.set_notice(format!(
                        "Panel composed from {} court member(s)",
                        composition.contributions.len()
                    ));
                    app.push_private_event("Panel", composition.combined);
                }
            }
            AppCommand::Approve(proposal) => {
                self.decide(app, tools, &proposal, "approved");
            }
            AppCommand::Reject(proposal) => {
                self.decide(app, tools, &proposal, "rejected");
            }
            AppCommand::Mode(mode) => set_mode(app, &mode),
            // Spoken directly by the main loop, which owns the speech runtime.
            AppCommand::Say(_) => {}
            AppCommand::VoiceDesk(requested) => {
                let enabled = requested.unwrap_or(!app.is_voice_desk());
                app.set_voice_desk(enabled);
                app.set_notice(if enabled {
                    "Voice Desk ON: typed lines are spoken as written; nothing runs on its own"
                } else {
                    "Voice Desk OFF: the court's own behavior is restored"
                });
            }
        }
    }

    fn play_music(&mut self, app: &mut App, tools: &mpsc::UnboundedSender<ToolReply>) {
        self.start_organist(app, tools);
    }

    fn consolidate_dream(&mut self, app: &mut App) {
        match self.memory.dream() {
            Ok(dream) => {
                app.set_notice(format!(
                    "Dream consolidated: {} observation(s)",
                    dream.observations.len()
                ));
                for observation in dream.observations {
                    app.push_private_event("Dream", observation);
                }
            }
            Err(error) => app.set_notice(format!("Dream consolidation failed: {error}")),
        }
    }

    fn handle_mcp(&self, app: &mut App, tools: &mpsc::UnboundedSender<ToolReply>, request: &str) {
        if request.trim().is_empty() {
            self.dispatch_sidecar(
                app,
                tools,
                SidecarKind::Mcp,
                Some(serde_json::json!({ "action": "list_servers" })),
                "MCP",
                false,
            );
            return;
        }
        if let Some((server, method, parameters)) = parse_tagged_mcp_request(request) {
            self.dispatch_sidecar(
                app,
                tools,
                SidecarKind::Mcp,
                Some(serde_json::json!({
                    "server": server,
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": method,
                    "params": parameters
                })),
                "MCP",
                false,
            );
            return;
        }
        let mut pieces = request.splitn(3, char::is_whitespace);
        let server = pieces.next().unwrap_or_default();
        let method = pieces.next().unwrap_or("tools/list");
        let parameters = pieces
            .next()
            .and_then(|raw| serde_json::from_str(raw).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        self.dispatch_sidecar(
            app,
            tools,
            SidecarKind::Mcp,
            Some(serde_json::json!({
                "server": server,
                "jsonrpc": "2.0",
                "id": 1,
                "method": method,
                "params": parameters
            })),
            "MCP",
            false,
        );
    }

    fn open_mission(&mut self, app: &mut App, objective: String) {
        self.open_mission_for(app, objective, teledra_agents::AgentId::Queen);
    }

    fn open_mission_for(
        &mut self,
        app: &mut App,
        objective: String,
        owner: teledra_agents::AgentId,
    ) {
        if objective.trim().is_empty() {
            app.set_notice("Use /Mission followed by a concrete objective");
            return;
        }
        match Mission::new_for(owner.slug(), &objective).and_then(|mission| {
            self.mission_store.save(&mission)?;
            Ok(mission)
        }) {
            Ok(mission) => {
                let id = mission.id;
                self.active_mission = Some(mission);
                app.queue_external_turn(
                    owner,
                    format!(
                        "DURABLE MISSION {id}: {objective}\nProduce one concrete result that advances \
                         this objective. State what evidence demonstrates completion."
                    ),
                );
                app.set_notice(format!(
                    "Mission {id} persisted and assigned to {}",
                    owner.title()
                ));
            }
            Err(error) => app.set_notice(format!("Mission could not start: {error}")),
        }
    }

    fn verify_workspace(&self, app: &mut App, tools: &mpsc::UnboundedSender<ToolReply>) {
        let tools = tools.clone();
        std::thread::spawn(move || {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
            let cargo = env::var_os("CARGO")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("cargo"));
            let mut summaries = Vec::new();
            for check in teledra_kraken::rust_workspace(&root, cargo) {
                match teledra_kraken::verify(&check) {
                    Ok(result) => summaries.push(format!(
                        "{}={} ({} ms)",
                        result.name, result.success, result.elapsed_ms
                    )),
                    Err(error) => summaries.push(format!("{}=ERROR: {error}", check.name)),
                }
            }
            let _ = tools.send(ToolReply {
                source: "Kraken",
                result: Ok(summaries.join(" | ")),
                court_prompt: None,
                effect: None,
            });
        });
        app.set_notice("Kraken is verifying the workspace in the background");
    }

    fn show_rundown(&self, app: &mut App) {
        app.set_notice("Current court rundown added to private activity");
        app.push_private_event("Rundown", format!("Mode: {}", app.mode()));
        app.push_private_event(
            "Rundown",
            format!("Topic: {}", app.locked_topic().unwrap_or("unlocked")),
        );
        if let Some(status) = app.podcast_status() {
            app.push_private_event("Podcast", status);
        }
        app.push_private_event(
            "Rundown",
            self.active_mission.as_ref().map_or_else(
                || "Mission: none".to_owned(),
                |mission| format!("Mission {}: {}", mission.id, mission.objective),
            ),
        );
    }

    fn show_work(&self, app: &mut App) {
        let treasury = self.treasury_entries();
        let open_treasury = treasury
            .iter()
            .filter(|item| item.get("status").and_then(|value| value.as_str()) == Some("new"))
            .collect::<Vec<_>>();
        if let Some(mission) = &self.active_mission {
            app.set_notice(format!(
                "Mission {} has {} task(s); {} treasury lead(s)",
                mission.id,
                mission.tasks.len(),
                open_treasury.len()
            ));
            for task in &mission.tasks {
                app.push_private_event(
                    "Work",
                    format!("#{} {:?}: {}", task.id, task.status, task.instruction),
                );
            }
        } else {
            app.set_notice(format!(
                "No active mission; {} treasury lead(s) await human review",
                open_treasury.len()
            ));
        }
        for item in open_treasury.into_iter().rev().take(12).rev() {
            let lane = item
                .get("lane")
                .and_then(|value| value.as_str())
                .unwrap_or("lead");
            let title = item
                .get("title")
                .and_then(|value| value.as_str())
                .unwrap_or("Untitled opportunity");
            let url = item
                .get("url")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            app.push_private_event("Treasury", format!("[{lane}] {title} — {url}"));
        }
    }

    fn show_treasury(&self, app: &mut App) {
        let entries = self.treasury_entries();
        if entries.is_empty() {
            app.set_notice(
                "Treasury ledger is empty; use /Scout to gather configured opportunity feeds",
            );
            return;
        }

        let open = entries
            .iter()
            .filter(|item| item.get("status").and_then(|value| value.as_str()) == Some("new"))
            .count();
        app.set_notice(format!(
            "Treasury ledger has {} entr{}; {open} await human review",
            entries.len(),
            if entries.len() == 1 { "y" } else { "ies" }
        ));
        for item in entries.into_iter().rev().take(20).rev() {
            let lane = item
                .get("lane")
                .and_then(|value| value.as_str())
                .unwrap_or("lead");
            let title = item
                .get("title")
                .and_then(|value| value.as_str())
                .unwrap_or("Untitled opportunity");
            let url = item
                .get("url")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            app.push_private_event("Treasury", format!("[{lane}] {title} — {url}"));
        }
    }

    fn treasury_entries(&self) -> Vec<serde_json::Value> {
        std::fs::read(self.root().join("treasury").join("work_queue.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn show_proposals(&self, app: &mut App) {
        match self.pending_proposals() {
            Ok(proposals) if proposals.is_empty() => {
                app.set_pending_proposals(0);
                app.set_notice("No proposals are pending");
            }
            Ok(proposals) => {
                app.set_pending_proposals(proposals.len());
                app.set_notice(format!("{} proposal(s) await a decision", proposals.len()));
                for proposal in proposals.into_iter().take(20) {
                    app.push_private_event(
                        "Proposal",
                        format!("{} / {}: {}", proposal.id, proposal.kind, proposal.content),
                    );
                }
            }
            Err(error) => app.set_notice(format!("Proposal ledger failed: {error}")),
        }
    }

    fn show_diplomacy_log(&self, app: &mut App) {
        let directory = self.root().join("proposals");
        let mut entries = std::fs::read_dir(directory)
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| std::fs::read(entry.path()).ok())
            .filter_map(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .filter(|value| {
                matches!(
                    value.get("kind").and_then(|kind| kind.as_str()),
                    Some("diplomacy" | "moltbook-comment" | "moltbook-upvote")
                )
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|value| {
            value
                .get("id")
                .and_then(|id| id.as_str())
                .unwrap_or_default()
                .to_owned()
        });
        app.set_notice(format!("{} diplomacy decision(s) archived", entries.len()));
        for entry in entries.into_iter().rev().take(20).rev() {
            let id = entry
                .get("id")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            let status = entry
                .get("status")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            let content = entry
                .get("content")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            app.push_private_event("Diplomacy", format!("{id} [{status}]: {content}"));
        }
    }

    fn decide(
        &self,
        app: &mut App,
        tools: &mpsc::UnboundedSender<ToolReply>,
        selection: &str,
        decision: &str,
    ) {
        if selection.trim().is_empty() {
            app.set_notice(format!(
                "Use /{} <proposal-id|all>",
                decision_command(decision)
            ));
            return;
        }
        match self.decide_proposals(selection, decision) {
            Ok(changed) if changed.is_empty() => {
                app.set_notice(format!("No pending proposal matched '{selection}'"));
            }
            Ok(changed) => {
                let count = changed.len();
                app.set_pending_proposals(app.pending_proposals().saturating_sub(count));
                if decision == "approved" {
                    for proposal in changed {
                        match proposal.kind.as_str() {
                            "diplomacy" => self.dispatch_sidecar(
                                app,
                                tools,
                                SidecarKind::Outreach,
                                Some(serde_json::json!({ "action": "post", "message": proposal.content })),
                                "Diplomat",
                                false,
                            ),
                            "moltbook-comment" => self.dispatch_sidecar(
                                app,
                                tools,
                                SidecarKind::Outreach,
                                Some(serde_json::json!({ "action": "moltbook_comment", "payload": proposal.content })),
                                "Diplomat",
                                false,
                            ),
                            "moltbook-upvote" => self.dispatch_sidecar(
                                app,
                                tools,
                                SidecarKind::Outreach,
                                Some(serde_json::json!({ "action": "moltbook_upvote", "payload": proposal.content })),
                                "Diplomat",
                                false,
                            ),
                            "mcp-call" => self.handle_mcp(app, tools, &proposal.content),
                            "workshop" => match self.workshop.promote(&proposal.content) {
                                Ok(path) => app.push_private_event(
                                    "Workshop",
                                    format!("Promoted approved tool: {}", path.display()),
                                ),
                                Err(error) => app.push_private_event("Workshop", error.to_string()),
                            },
                            "strudel_music" | "python_music" | "music-patch" => {
                                match self.archive_approved_creative(
                                    &proposal.kind.replace('-', "_"),
                                    &proposal.content,
                                ) {
                                    Ok(path) => app.push_private_event(
                                        "Organist",
                                        format!("Approved legacy composition archived: {}", path.display()),
                                    ),
                                    Err(error) => {
                                        app.push_private_event("Organist", error.to_string())
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                app.set_notice(format!("{count} proposal(s) {decision}"));
            }
            Err(error) => app.set_notice(format!("Proposal decision failed: {error}")),
        }
    }
}

fn parse_tagged_mcp_request(request: &str) -> Option<(String, String, serde_json::Value)> {
    let mut server = String::new();
    let mut tool = String::new();
    let mut method = String::new();
    for field in request.split(';') {
        let Some((key, value)) = field.split_once('=') else {
            continue;
        };
        match key.trim().to_ascii_lowercase().as_str() {
            "server" => server = value.trim().to_owned(),
            "tool" => tool = value.trim().to_owned(),
            "method" => method = value.trim().to_owned(),
            _ => {}
        }
    }
    if server.is_empty() {
        return None;
    }
    let lower = request.to_ascii_lowercase();
    let args = lower
        .find("args=")
        .or_else(|| lower.find("arguments="))
        .and_then(|index| {
            request[index..]
                .split_once('=')
                .map(|(_, value)| value.trim())
        })
        .and_then(|value| serde_json::from_str(value.trim_end_matches(';').trim()).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if !tool.is_empty() {
        return Some((
            server,
            "tools/call".to_owned(),
            serde_json::json!({ "name": tool, "arguments": args }),
        ));
    }
    Some((
        server,
        if method.is_empty() {
            "tools/list".to_owned()
        } else {
            method
        },
        args,
    ))
}

fn decision_command(decision: &str) -> &'static str {
    if decision == "approved" {
        "Approve"
    } else {
        "Reject"
    }
}

fn set_mode(app: &mut App, requested: &str) {
    let normalized = requested.trim().to_ascii_lowercase();
    let mode = match normalized.as_str() {
        "normal" => "Normal",
        "comedic" | "comedy" => "Comedic",
        "empathetic" | "empathy" => "Empathetic",
        "dark" | "darkcomedic" | "dark-comedic" => "Dark Comedic",
        "babble" | "babbly" => "Babble",
        "stream" | "streamer" | "broadcast" => "Streamer",
        "podcast" if app.podcast_status().is_some() => "Podcast",
        "copilot" | "co-pilot" | "game" => "Game Co-Pilot",
        "night" | "nightdesk" | "night-desk" => "Night Desk",
        _ => {
            app.set_notice(
                "Modes: normal, comedic, empathetic, dark, babble, streamer, copilot, night. Start a podcast with /Podcast <topic>",
            );
            return;
        }
    };
    app.set_mode(mode);
    if mode == "Night Desk" {
        app.set_night_desk_enabled(true);
    }
    app.set_notice(format!("Court mode changed to {mode}"));
}

fn podcast_arguments(arguments: &str) -> Result<(u64, &str), &'static str> {
    let arguments = arguments.trim();
    let (head, rest) = arguments
        .split_once(char::is_whitespace)
        .unwrap_or((arguments, ""));
    if let Some(number) = head.strip_suffix('m')
        && !number.is_empty()
        && number.chars().all(|ch| ch.is_ascii_digit())
    {
        let minutes = number
            .parse::<u64>()
            .map_err(|_| "Podcast duration must be 60m to 240m")?;
        if !(60..=240).contains(&minutes) {
            return Err("Podcast duration must be 60m to 240m");
        }
        if rest.trim().is_empty() {
            return Err(
                "Add a topic after the duration, for example /Podcast 90m ocean exploration",
            );
        }
        Ok((minutes, rest.trim()))
    } else {
        Ok((60, arguments))
    }
}

fn toggle_night_desk(app: &mut App) {
    let enabled = !app.night_desk_enabled();
    app.set_night_desk_enabled(enabled);
    app.set_notice(if enabled {
        "Night Desk enabled behind the current court mode"
    } else {
        "Night Desk disabled"
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use teledra_brain::{BrainHealth, QWEN_MODEL};
    use teledra_court::Court;

    #[test]
    fn operator_podcast_lock_survives_model_controls_and_resumes() {
        let directory = tempfile::tempdir().unwrap();
        let mut services =
            RuntimeServices::new(directory.path().to_owned(), "unused-python".into()).unwrap();
        let mut app = App::new(
            Court::default(),
            BrainHealth {
                endpoint: "http://localhost:11434".into(),
                model: QWEN_MODEL,
            },
        );
        app.set_mode("Normal");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let path = directory.path().join("session.json");
        services.handle_command(
            AppCommand::Lock("ocean exploration".into()),
            &mut app,
            &path,
            &tx,
        );
        assert!(app.is_podcasting());
        for action in [
            teledra_court::CourtAction::Topic("court housekeeping".into()),
            teledra_court::CourtAction::Control(teledra_court::CourtControl::Unlock),
            teledra_court::CourtAction::Control(teledra_court::CourtControl::StopBabble),
        ] {
            services.handle_court_action(action, &mut app, &tx);
        }
        assert_eq!(app.locked_topic(), Some("ocean exploration"));
        assert!(app.is_podcasting());
        services.handle_command(AppCommand::Mode("normal".into()), &mut app, &path, &tx);
        assert!(!app.is_podcasting());
        services.handle_command(AppCommand::Podcast(String::new()), &mut app, &path, &tx);
        assert!(app.is_podcasting());
        services.handle_command(AppCommand::Unlock, &mut app, &path, &tx);
        assert_eq!(app.mode(), "Normal");
        assert!(app.locked_topic().is_none());
    }

    #[test]
    fn podcast_duration_is_explicit_and_validated() {
        assert_eq!(
            podcast_arguments("90m deep oceans"),
            Ok((90, "deep oceans"))
        );
        assert_eq!(
            podcast_arguments("2001 a space odyssey"),
            Ok((60, "2001 a space odyssey"))
        );
        assert!(podcast_arguments("30m oceans").is_err());
        assert!(podcast_arguments("900m oceans").is_err());
        assert!(podcast_arguments("60m").is_err());
    }

    #[test]
    fn original_mcp_action_form_becomes_a_tools_call() {
        let (server, method, parameters) =
            parse_tagged_mcp_request(r#"server=embassy; tool=greet; args={"name":"Teledra"}"#)
                .expect("tagged request");
        assert_eq!(server, "embassy");
        assert_eq!(method, "tools/call");
        assert_eq!(parameters["name"], "greet");
        assert_eq!(parameters["arguments"]["name"], "Teledra");
    }

    #[test]
    fn operator_art_commands_select_the_lineage_and_cancel_obsolete_exploration() {
        let directory = tempfile::tempdir().unwrap();
        let mut services =
            RuntimeServices::new(directory.path().to_owned(), "python".into()).unwrap();
        let mut app = App::new(
            Court::default(),
            BrainHealth {
                endpoint: "http://localhost:11434".to_owned(),
                model: QWEN_MODEL,
            },
        );
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        // Keep the worker occupied so this test inspects requested state
        // without launching Python or an actual render thread.
        services
            .fractus_renders
            .request("previous scene".to_owned(), true);
        services.fractus_exploration_ready = Some("Obsolete prepared study".to_owned());
        app.queue_backstage_turn(
            teledra_agents::AgentId::Artist,
            "ARTIST EXPLORATION: Obsolete queued study",
        );
        services.handle_command(
            AppCommand::Fractus("type=star_mandala seed=144 symmetry=18".to_owned()),
            &mut app,
            &directory.path().join("session.json"),
            &tx,
        );
        let selected = services.fractus_candidate_lineage.as_ref().unwrap();
        assert!(!selected.live);
        assert_eq!(
            selected.origin,
            super::super::fractus::FractusOrigin::Operator
        );
        let scene = teledra_fractus::Scene::parse(&selected.dsl).unwrap();
        assert_eq!((scene.seed, scene.symmetry), (144, 18));
        assert!(services.next_fractus_evolution.is_none());
        assert!(services.next_fractus_exploration.is_none());
        assert!(services.fractus_exploration_ready.is_none());
        assert!(app.can_queue_artist_exploration());
        let ticket = services.fractus_research_ticket;
        services.handle_command(
            AppCommand::FractusLive("julia".to_owned()),
            &mut app,
            &directory.path().join("session.json"),
            &tx,
        );
        assert!(services.fractus_candidate_lineage.as_ref().unwrap().live);
        assert!(services.next_fractus_exploration.is_some());
        assert!(services.fractus_research_ticket > ticket);
        let previous = services
            .fractus_candidate_lineage
            .as_ref()
            .unwrap()
            .dsl
            .clone();
        services.handle_command(
            AppCommand::Fractus("type=not_a_family".to_owned()),
            &mut app,
            &directory.path().join("session.json"),
            &tx,
        );
        assert_eq!(
            services.fractus_candidate_lineage.as_ref().unwrap().dsl,
            previous
        );
    }

    #[test]
    fn night_desk_toggle_preserves_the_foreground_mode() {
        let mut app = App::new(
            Court::default(),
            BrainHealth {
                endpoint: "http://localhost:11434".to_owned(),
                model: QWEN_MODEL,
            },
        );
        app.set_mode("Streamer");

        toggle_night_desk(&mut app);
        assert!(app.night_desk_enabled());
        assert_eq!(app.mode(), "Streamer");

        set_mode(&mut app, "comedic");
        assert!(app.night_desk_enabled());
        assert_eq!(app.mode(), "Comedic");

        toggle_night_desk(&mut app);
        assert!(!app.night_desk_enabled());
        assert_eq!(app.mode(), "Comedic");
    }
}
