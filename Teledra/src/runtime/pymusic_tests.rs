use super::*;

fn test_app() -> App {
    App::new(
        teledra_court::Court::default(),
        teledra_brain::BrainHealth {
            endpoint: "http://localhost:11434".to_owned(),
            model: teledra_brain::QWEN_MODEL,
        },
    )
}

fn test_audio() -> VerifiedAudio {
    let samples = (0..48_000 * 4)
        .flat_map(|frame| {
            let phase = f64::from(frame) * std::f64::consts::TAU * 220.0 / 48_000.0;
            [(phase.sin() * 0.08) as f32, (phase.sin() * 0.04) as f32]
        })
        .collect();
    VerifiedAudio::new(samples).unwrap()
}

fn test_render(directory: &Path) -> PythonRender {
    PythonRender {
        restore: false,
        code: "# complete accepted source".to_owned(),
        directory: directory.to_owned(),
        report: serde_json::json!({"ok":true,"title":"Accepted Python","loop":true}),
        audio: test_audio(),
    }
}

#[test]
fn submitted_v1_reference_is_recognized_without_accepting_source_changes() {
    let reference = include_str!("../../../Music/v1_music_example.py");
    let submitted = clean_python_source(reference);
    assert!(super::super::music_library::is_bundled_v1_source(
        &submitted
    ));
    assert!(super::super::music_library::is_bundled_v1_source(
        &submitted.replace("\r\n", "\n").replace('\n', "\r\n")
    ));
    assert!(!super::super::music_library::is_bundled_v1_source(
        &format!("{submitted}\nprint('changed')")
    ));
}

#[test]
fn generated_short_sketch_never_replaces_the_complete_piece() {
    let root = tempfile::tempdir().unwrap();
    let mut services =
        RuntimeServices::new(root.path().to_owned(), "unused-python".into()).unwrap();
    let mut app = test_app();
    let old =
        serde_json::json!({"title":"Full arrangement","duration_seconds":88,"audio_digest":"old"});
    services.python_music.report = old.clone();
    services.python_music.rendering = Some(services.music_epoch);
    services.python_music.repair_source = Some("model sketch".into());
    let mut sketch = test_render(root.path());
    sketch.report["duration_seconds"] = 4.into();
    let (tx, _) = mpsc::unbounded_channel();
    services.python_ready(&mut app, &tx, services.music_epoch, sketch);
    assert_eq!(services.python_music.report, old);
    assert!(services.python_music.ready.is_none());
    assert!(services.next_native_evolution.is_some());
    assert!(services.music_player.is_none());
}

#[test]
fn listener_feedback_uses_live_selection_when_recovery_pointer_is_stale() {
    let root = tempfile::tempdir().unwrap();
    let mut services =
        RuntimeServices::new(root.path().to_owned(), "unused-python".into()).unwrap();
    let home = root.path().join("music/python");
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join("current.json"), r#"{"artifact":"render-old"}"#).unwrap();
    services.python_music.accepted = Some(home.join("render-current"));
    services.python_music.accepted_code = Some("actual playing source".into());
    services.python_music.report = serde_json::json!({"title":"Playing piece"});
    services.python_music.active_library_id = Some("py-1234567890abcdef".into());
    let event = services
        .live_python_feedback_snapshot(teledra_music::FeedbackVerdict::Keep, "current one")
        .unwrap();
    assert_eq!(event["identity"], "render-current");
    assert_eq!(event["code"], "actual playing source");
    assert_eq!(event["library_id"], "py-1234567890abcdef");
}

#[tokio::test]
#[ignore = "two real configured local model songs and full PyMusic renders; no devices, AppData writes or UI"]
async fn real_model_composes_two_complete_library_tracks() {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    let root = workspace.join(format!(
        "target/song-library-proof/{}",
        super::super::epoch_millis()
    ));
    fs::create_dir_all(&root).unwrap();
    let home = root.join("music/python");
    let brain = teledra_brain::Brain::new(Default::default()).unwrap();
    let mut library = MusicLibrary::load(&root).unwrap();
    let mut proofs = Vec::new();
    let mut previous = serde_json::Value::Null;
    for (index, brief) in [
        "Create a gentle A-minor piano-led piece: a memorable leaping theme, a quieter answering voice, independent bass, changing harmony and a clear return. Prefer classic PyMusic keys/pad/bass and restrained levels.",
        "Compose a distinct D-dorian piece for plucked lead and a bell answer. Use a different theme, rhythmic spacing, progression and contrasting middle section from the previous piece. Preserve a calm pulse and phrase space. Prefer classic PyMusic voices."
    ].iter().enumerate() {
        let directory = home.join(format!("render-proof-{index}"));
        fs::create_dir_all(&directory).unwrap();
        let messages = vec![teledra_brain::Message::system(crate::python_context::song_generation_context(&previous, "No listener verdict exists for these proofs", "Past measurements are execution evidence, not aesthetic approval")), teledra_brain::Message::user(*brief)];
        fs::write(directory.join("input.json"), serde_json::to_vec_pretty(&messages).unwrap()).unwrap();
        println!("Generating real model song {} at {}", index+1, directory.display());
        let output = crate::python_composition::compose_python_song_with_budget(&brain, &messages, Duration::from_secs(90)).await;
        match &output { Ok(value) => fs::write(directory.join("model-output.txt"), value).unwrap(), Err(error) => { fs::write(directory.join("generation-error.txt"), error).unwrap(); panic!("Real song generation failed: {error}; evidence {}",directory.display()); } }
        let parsed = teledra_court::parse_response(&output.unwrap());
        let code = parsed.actions.iter().find_map(|action| match action {
            teledra_court::CourtAction::CreativeArtifact { kind, content } if kind == "python_music" => Some(content.clone()), _ => None,
        }).expect("real song action");
        let rendered = render_python(&workspace.join(".venv/Scripts/python.exe"), &workspace, &workspace.join("target/release/teledra.exe"), &directory, code, &AtomicBool::new(false)).unwrap_or_else(|e| panic!("Render failed: {e}; {}",directory.display()));
        MusicLibrary::eligibility(&rendered.report).unwrap();
        assert_ne!(rendered.report["audio_digest"], previous["audio_digest"]);
        // This isolated publication registry exercises saving/loading without
        // opening a player. Live activation separately uses accept_python_music.
        let id = library.record_published(&rendered).unwrap();
        let restored = library.load_playback(&id).unwrap();
        assert_eq!(restored.report["audio_digest"], rendered.report["audio_digest"]);
        proofs.push(serde_json::json!({"id":id,"title":rendered.report["title"],"duration_seconds":rendered.report["duration_seconds"],"notes_scheduled":rendered.report["composition_audit"]["notes_scheduled"],"notes_mixed":rendered.report["composition_audit"]["notes_mixed"],"pcm_identity":rendered.report["audio_digest"],"path":directory}));
        previous = rendered.report;
    }
    fs::write(root.join("proof.json"), serde_json::to_vec_pretty(&serde_json::json!({"model":teledra_brain::model_name(),"tracks":proofs,"scope":"Two unmodified model recipes, full actual PyMusic worker rendering, placement verification and isolated library save/reload. No listening or taste verdict. No live state changed."})).unwrap()).unwrap();
    assert_eq!(
        library.editor_state(None)["entries"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    println!("Two complete real-model library pieces: {}", root.display());
}

#[test]
fn music_snapshot_exposes_cached_baseline_and_outstanding_work_without_a_device() {
    use crate::session::{MusicSnapshotEngine, MusicSnapshotTransport};

    let root = tempfile::tempdir().unwrap();
    let mut services =
        RuntimeServices::new(root.path().to_owned(), "unused-python".into()).unwrap();
    let mut app = test_app();
    services.music_engine = MusicEngine::Python;
    services.music_alternating = false;
    services.python_music.accepted = Some(root.path().join("music/python/render-accepted"));
    services.python_music.accepted_code = Some("not rehashed while publishing".into());
    services.python_music.report = serde_json::json!({
        "duration_seconds":88.25,
        "source_digest":"9c07df017dd11335",
        "audio_digest":"d4af46fad164d828"
    });
    services.python_music.rendering = Some(12);
    services.next_native_evolution = Some(Instant::now() - Duration::from_secs(1));
    app.set_music_muted(true);
    app.queue_external_turn(
        teledra_agents::AgentId::Organist,
        "PYMUSIC COMPOSITION: improve",
    );

    let rendering = services.music_snapshot(&app);
    assert_eq!(rendering.engine, MusicSnapshotEngine::Python);
    assert_eq!(rendering.transport, MusicSnapshotTransport::Stopped);
    assert!(rendering.muted);
    assert!(!rendering.alternating);
    assert_eq!(
        rendering.accepted_artifact.as_deref(),
        Some("render-accepted")
    );
    assert_eq!(
        rendering.source_identity.as_deref(),
        Some("fnv1a64:9c07df017dd11335")
    );
    assert_eq!(rendering.pcm_identity.as_deref(), Some("d4af46fad164d828"));
    assert_eq!(rendering.position_seconds, 0);
    assert_eq!(rendering.duration_seconds, 89);
    assert_eq!(rendering.next_evolution_seconds, Some(0));
    assert!(rendering.composer_pending);
    assert_eq!(rendering.pending_turn, None);
    assert_eq!(rendering.rendering_ticket, Some(12));
    assert_eq!(rendering.ready_ticket, None);
    assert_eq!(rendering, services.music_snapshot(&app));

    let request = app.begin_queued_external_turn().unwrap();
    services.python_music.rendering = None;
    services.python_music.ready = Some(ReadyPython {
        ticket: 12,
        render: test_render(&root.path().join("render-next")),
        switch_after: 176.5,
    });
    let ready = services.music_snapshot(&app);
    assert_eq!(ready.pending_turn, Some(request.id));
    assert_eq!(ready.rendering_ticket, None);
    assert_eq!(ready.ready_ticket, Some(12));
    assert_eq!(ready.switch_after_seconds, Some(177));
    assert_eq!(ready.source_identity, rendering.source_identity);

    services.music_engine = MusicEngine::Native;
    services.music_epoch = 13;
    services.music_rendering = true;
    services.python_music.ready = None;
    services.next_native_evolution = None;
    let native = services.music_snapshot(&app);
    assert_eq!(native.engine, MusicSnapshotEngine::Native);
    assert_eq!(native.rendering_ticket, Some(13));
    assert_eq!(native.next_evolution_seconds, None);
    assert_eq!(native.source_identity, None);
    assert_eq!(native.pcm_identity, None);
}

#[test]
fn recursive_python_context_uses_the_accepted_source_key_and_tempo() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("music/python");
    let accepted = home.join("render-accepted");
    fs::create_dir_all(&accepted).unwrap();
    fs::write(
        home.join("current.json"),
        r#"{"artifact":"render-accepted"}"#,
    )
    .unwrap();
    let source = "# exact accepted music baseline\nSR = 44100";
    fs::write(accepted.join("code.py"), source).unwrap();
    fs::write(
        accepted.join("report.json"),
        r#"{"title":"Accepted study","metadata":{"key":"A harmonic minor","bpm":109.09}}"#,
    )
    .unwrap();
    fs::write(home.join("draft.py"), "# unplayed broken editor draft").unwrap();
    let services = RuntimeServices::new(root.path().to_owned(), "unused-python".into()).unwrap();
    assert_eq!(
        services.python_music.accepted_source().as_deref(),
        Some(source)
    );
    assert_eq!(
        services.python_music.accepted_artifact(),
        Some(accepted.as_path())
    );
    let context = services.python_composition_context();
    assert!(context.contains(source));
    assert!(!context.contains("unplayed broken editor draft"));
    assert!(context.contains("A harmonic minor and 109.09 BPM"));
    assert!(context.contains("PYTHON_MUSIC_EDIT"));
    assert!(context.contains(&super::super::music_edits::python_source_identity(source)));
    assert!(context.contains("RECURSIVE MUSIC WORK"));
    assert!(context.contains("Preserve the accepted source's sample rate"));
    let generation = services.python_generation_context();
    assert!(generation.contains("A harmonic minor"));
    assert!(generation.contains("piano (MIDI"));
    assert!(generation.contains("theremin (MIDI"));
    assert!(!generation.contains(source));
    assert!(!generation.contains("unplayed broken editor draft"));
    assert_eq!(
        services
            .python_edit_source("EDIT ACCEPTED SOURCE: change the answer")
            .as_deref(),
        Some(source)
    );
    assert!(
        services
            .python_edit_source("PYMUSIC COMPOSITION: complete new piece")
            .is_none()
    );
    assert_eq!(
        services
            .python_edit_source("Compose an improved counterline for the current PyMusic piece")
            .as_deref(),
        Some(source)
    );
    assert!(
        services
            .python_edit_source("PYMUSIC COMPOSITION: COMPOSER CORRECTION: repair failed source")
            .is_none()
    );
}

#[test]
fn historical_python_feedback_is_bounded_evidence_without_old_source_code() {
    let raw = (0..10).map(|index| serde_json::json!({
        "identity":format!("render-{index}"), "score_title":format!("Study {index}"),
        "verdict":"keep", "note":format!("Keep this exact phrase {index}. {}", "x".repeat(900)),
        "code":format!("historical_source_must_not_enter_context{}", "z".repeat(100_000)),
        "report":{"duration_seconds":88.0,"peak":0.8,"rms":0.05,"notes_requested":754,"notes_rendered":754,
            "metadata":{"key":"A harmonic minor","bpm":109.09,"bars":40},"extra":"ignored"},
    }).to_string()).collect::<Vec<_>>().join("\n");
    let context = python_feedback_context(&raw);
    assert!(context.len() <= 4096);
    assert!(!context.contains("historical_source_must_not_enter_context"));
    assert!(!context.contains("ignored"));
    assert!(context.contains("Keep this exact phrase 9."));
    assert!(context.contains("A harmonic minor"));
    for line in context.lines() {
        let event: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(event["note"].as_str().unwrap().chars().count() <= 500);
        assert_eq!(event["note_truncated"], true);
        assert_eq!(event["measured"]["notes_rendered"], 754);
    }
}

#[test]
fn complete_artifact_fingerprints_distinguish_source_edits_from_audio_changes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("audio.wav");
    let audio = test_audio();
    audio.write_wav(&path).unwrap();
    let report = serde_json::json!({"ok":true,"title":"First name","sample_rate":48000,"channels":2,"frames":192000,"duration_seconds":4.0,"notes_requested":0,"notes_rendered":0});
    fs::write(root.path().join("report.json"), report.to_string()).unwrap();
    let first = read_python_artifact(root.path(), "# first source".into(), false).unwrap();
    let mut renamed = report;
    renamed["title"] = "New title, same audio".into();
    fs::write(root.path().join("report.json"), renamed.to_string()).unwrap();
    let second = read_python_artifact(root.path(), "# changed comment".into(), false).unwrap();
    assert_eq!(first.report["audio_digest"], second.report["audio_digest"]);
    assert_ne!(
        first.report["source_digest"],
        second.report["source_digest"]
    );
    assert!(
        duplicate_python_reason(&first.report, &second.report)
            .unwrap()
            .contains("same audio")
    );
    let changed =
        VerifiedAudio::new(audio.samples().iter().map(|sample| sample * 0.6).collect()).unwrap();
    changed.write_wav(&path).unwrap();
    let third = read_python_artifact(root.path(), "# actual changed source".into(), false).unwrap();
    assert_ne!(first.report["audio_digest"], third.report["audio_digest"]);
    assert!(duplicate_python_reason(&first.report, &third.report).is_none());
    let same_source = read_python_artifact(root.path(), "# first source".into(), false).unwrap();
    assert!(
        duplicate_python_reason(&first.report, &same_source.report)
            .unwrap()
            .contains("repeats the accepted source")
    );
    assert!(duplicate_python_reason(&serde_json::json!({}), &serde_json::json!({})).is_none());
}

#[test]
fn generated_duplicate_keeps_the_accepted_artifact_and_rearms_improvement() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("music/python");
    fs::create_dir_all(&home).unwrap();
    let pointer = home.join("current.json");
    fs::write(&pointer, r#"{"artifact":"accepted"}"#).unwrap();
    let mut services =
        RuntimeServices::new(root.path().to_owned(), "unused-python".into()).unwrap();
    let mut app = test_app();
    let (tx, _rx) = mpsc::unbounded_channel();
    services.music_engine = MusicEngine::Python;
    services.music_epoch = 7;
    services.next_native_evolution = None;
    services.python_music.rendering = Some(7);
    services.python_music.repair_source = Some("model-generated change".into());
    services.python_music.accepted_code = Some("accepted source".into());
    services.python_music.report = serde_json::json!({"title":"Accepted","audio_digest":"same-audio","source_digest":"previous-source"});
    let accepted_report = services.python_music.report.clone();
    let mut duplicate = test_render(&root.path().join("new-name"));
    duplicate.report = serde_json::json!({"title":"Renamed","audio_digest":"same-audio","source_digest":"new-comment"});
    services.python_ready(&mut app, &tx, 7, duplicate);
    assert!(services.python_music.ready.is_none());
    assert!(services.python_music.rendering.is_none());
    assert!(
        services.music_player.is_none(),
        "duplicate must not open an audio device"
    );
    assert_eq!(services.python_music.report, accepted_report);
    assert_eq!(
        services.python_music.accepted_source().as_deref(),
        Some("accepted source")
    );
    assert_eq!(fs::read(&pointer).unwrap(), br#"{"artifact":"accepted"}"#);
    assert!(services.next_native_evolution.is_some());
    assert!(
        services
            .music_learning
            .context(MusicEngine::Python)
            .contains("unchanged")
    );
}

#[test]
fn failed_python_commit_preserves_accepted_state_and_rearms_improvement() {
    let root = tempfile::tempdir().unwrap();
    let mut services =
        RuntimeServices::new(root.path().to_owned(), "unused-python".into()).unwrap();
    let mut app = test_app();
    services.python_music.accepted_code = Some("accepted source".into());
    services.python_music.report =
        serde_json::json!({"title":"Accepted","audio_digest":"accepted-pcm"});
    services.next_native_evolution = None;
    services.python_acceptance_failed(&mut app, "candidate-pcm", "current.json rename denied");
    assert!(services.next_native_evolution.is_some());
    assert_eq!(
        services.python_music.accepted_source().as_deref(),
        Some("accepted source")
    );
    assert_eq!(services.python_music.report["title"], "Accepted");
    let evidence = services.music_learning.context(MusicEngine::Python);
    assert!(evidence.contains("acceptance_failed"));
    assert!(evidence.contains("current.json rename denied"));
}

#[test]
fn python_wav_reader_preserves_completed_stereo_frames() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("complete.wav");
    let original = test_audio();
    original.write_wav(&path).unwrap();
    let decoded = read_python_wav(&path).unwrap();
    assert_eq!(decoded.samples().len(), 48_000 * 4 * 2);
    assert_eq!(decoded.report().duration_seconds, 4.0);
    for (before, after) in original.samples().iter().zip(decoded.samples()) {
        assert!((before - after).abs() < 2.0 / 32768.0);
    }
    assert!(decoded.report().peak > 0.079);
}

#[test]
fn python_wav_reader_rejects_truncated_wrong_format_and_silent_artifacts() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("candidate.wav");
    test_audio().write_wav(&path).unwrap();
    let complete = fs::read(&path).unwrap();
    let mut variants = Vec::new();
    variants.push(complete[..complete.len() - 1].to_vec());
    let mut not_wav = complete.clone();
    not_wav[..4].copy_from_slice(b"NOPE");
    variants.push(not_wav);
    for (offset, bytes) in [
        (20, 3_u16.to_le_bytes()),
        (22, 1_u16.to_le_bytes()),
        (34, 8_u16.to_le_bytes()),
    ] {
        let mut wrong = complete.clone();
        wrong[offset..offset + 2].copy_from_slice(&bytes);
        variants.push(wrong);
    }
    let mut wrong_rate = complete.clone();
    wrong_rate[24..28].copy_from_slice(&44_100_u32.to_le_bytes());
    variants.push(wrong_rate);
    let mut half_frame = complete.clone();
    half_frame.truncate(half_frame.len() - 2);
    let remaining = (half_frame.len() - 44) as u32;
    half_frame[40..44].copy_from_slice(&remaining.to_le_bytes());
    variants.push(half_frame);
    let mut silent = complete;
    silent[44..].fill(0);
    variants.push(silent);
    for (index, bytes) in variants.iter().enumerate() {
        fs::write(&path, bytes).unwrap();
        assert!(
            read_python_wav(&path).is_err(),
            "invalid WAV variant {index} was accepted"
        );
    }
}

#[test]
fn python_current_manifest_loads_accepted_provenance_and_keeps_unsent_draft() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("music/python");
    let accepted = home.join("render-12-3");
    fs::create_dir_all(&accepted).unwrap();
    fs::write(home.join("current.json"), r#"{"artifact":"render-12-3"}"#).unwrap();
    fs::write(accepted.join("code.py"), "# accepted source").unwrap();
    fs::write(
        accepted.join("report.json"),
        r#"{"ok":true,"title":"Accepted work"}"#,
    )
    .unwrap();
    let accepted_state = PythonMusic::new("unused-python".into(), root.path());
    assert_eq!(accepted_state.code, "# accepted source");
    assert_eq!(accepted_state.report["title"], "Accepted work");
    assert!(accepted_state.child.is_none());
    fs::write(home.join("draft.py"), "# unplayed editor draft").unwrap();
    let edited_state = PythonMusic::new("unused-python".into(), root.path());
    assert_eq!(edited_state.code, "# unplayed editor draft");
    assert_eq!(
        edited_state.report, accepted_state.report,
        "draft edits cannot claim accepted provenance"
    );
    fs::remove_file(home.join("draft.py")).unwrap();
    fs::write(home.join("current.json"), r#"{"artifact":"../../outside"}"#).unwrap();
    let invalid = PythonMusic::new("unused-python".into(), root.path());
    assert!(invalid.report.is_null());
    assert_ne!(invalid.code, "# accepted source");
}

#[test]
fn python_stop_invalidates_queued_and_late_results_without_opening_audio() {
    let root = tempfile::tempdir().unwrap();
    let mut services =
        RuntimeServices::new(root.path().to_owned(), "unused-python".into()).unwrap();
    let mut app = test_app();
    let (tx, _rx) = mpsc::unbounded_channel();
    services.music_engine = MusicEngine::Python;
    services.music_epoch = 8;
    services.python_music.session = 4;
    let cancellation = Arc::new(AtomicBool::new(false));
    services.python_music.cancel = Some(cancellation.clone());
    services.python_music.rendering = Some(8);
    services.python_music.ready = Some(ReadyPython {
        ticket: 8,
        render: test_render(&root.path().join("queued")),
        switch_after: 4.0,
    });
    services.music_pending = Some(services.score.clone());
    services.music_draft = Some(services.score.clone());
    services.music_deferred = Some(services.score.clone());
    app.set_music_enabled(true);
    services.handle_python_action(&mut app, &tx, 4, PythonAction::Stop);
    assert!(cancellation.load(Ordering::Acquire));
    assert_eq!(services.music_epoch, 9);
    assert!(services.python_music.rendering.is_none() && services.python_music.ready.is_none());
    assert!(
        services.music_pending.is_none()
            && services.music_draft.is_none()
            && services.music_deferred.is_none()
    );
    assert!(!app.music_enabled());
    services.python_ready(&mut app, &tx, 8, test_render(&root.path().join("late")));
    assert!(services.music_player.is_none() && services.python_music.ready.is_none());
    assert!(!root.path().join("music/python/current.json").exists());
    services.python_music.ready = Some(ReadyPython {
        ticket: 8,
        render: test_render(&root.path().join("stale-queued")),
        switch_after: 0.0,
    });
    services.publish_python_if_ready(&mut app, &tx);
    assert!(services.music_player.is_none() && services.python_music.ready.is_none());
}

#[test]
fn inactive_editor_controls_cannot_save_or_stop_the_selected_engine() {
    let root = tempfile::tempdir().unwrap();
    let mut services =
        RuntimeServices::new(root.path().to_owned(), "unused-python".into()).unwrap();
    let mut app = test_app();
    let (tx, _rx) = mpsc::unbounded_channel();
    let original = services.python_music.code.clone();
    services.python_music.session = 6;
    for (engine, session) in [(MusicEngine::Native, 6), (MusicEngine::Python, 5)] {
        services.music_engine = engine;
        app.set_music_enabled(true);
        let epoch = services.music_epoch;
        services.handle_python_action(
            &mut app,
            &tx,
            session,
            PythonAction::Save {
                code: "stale edit".into(),
            },
        );
        services.handle_python_action(&mut app, &tx, session, PythonAction::Stop);
        assert_eq!(services.music_epoch, epoch);
        assert_eq!(services.python_music.code, original);
        assert!(app.music_enabled());
        assert!(!root.path().join("music/python/draft.py").exists());
    }
}

#[test]
fn composition_routing_preserves_explicit_native_and_python_requests() {
    let root = tempfile::tempdir().unwrap();
    let mut services =
        RuntimeServices::new(root.path().to_owned(), "unused-python".into()).unwrap();
    assert!(!services.python_composition_requested("Compose another piece"));
    assert!(services.python_composition_requested("PYMUSIC COMPOSITION: write an answer"));
    assert!(services.python_composition_requested("Use PYTHON_MUSIC"));
    services.music_engine = MusicEngine::Python;
    assert!(services.python_composition_requested("Compose another piece"));
    assert!(!services.python_composition_requested("NATIVE COMPOSITION: write an answer"));
    assert_eq!(MusicEngine::Native.other().other(), MusicEngine::Native);
    let context = services.python_composition_context();
    assert!(context.contains("instrument_note('C4', duration, instrument='piano'"));
    assert!(context.contains("TELEDRA_SCORE"));
    for instrument in teledra_music::Instrument::ALL {
        assert!(context.contains(&format!("{} (MIDI ", instrument.id())));
    }
}

#[test]
#[ignore = "real local Python/NumPy and built native note bridge; writes only isolated proof artifacts, opens no device"]
fn real_python_worker_verifies_full_example_and_rejects_incomplete_replacements() {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    let python = std::env::var_os("TELEDRA_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.join(".venv/Scripts/python.exe"));
    let synth = std::env::var_os("TELEDRA_SYNTH_EXE")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.join("target/release/examples/python_note_server.exe"));
    assert!(
        python.is_file(),
        "Python/NumPy executable required: {}",
        python.display()
    );
    assert!(
        synth.is_file(),
        "built native bridge required: {}",
        synth.display()
    );
    let proof = workspace
        .join("target/python-music-proof")
        .join(format!("runtime-{}", super::super::epoch_millis()));
    let source = fs::read_to_string(workspace.join("Music/pymusic_example.py")).unwrap();
    let render = render_python(
        &python,
        &workspace,
        &synth,
        &proof.join("accepted"),
        source.clone(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(render.code, source);
    assert_eq!(render.report["ok"], true);
    assert_eq!(render.report["sample_rate"], 48_000);
    assert_eq!(render.report["channels"], 2);
    assert_eq!(
        render.report["frames"].as_u64().unwrap() as usize,
        render.audio.samples().len() / 2
    );
    assert_eq!(render.report["duration_seconds"], 80.0);
    assert_eq!(render.audio.report().duration_seconds, 80.0);
    assert_eq!(render.report["notes_requested"], 285);
    assert_eq!(render.report["notes_rendered"], 285);
    assert!(
        render.report["note_audit_scope"]
            .as_str()
            .unwrap()
            .contains("arbitrary NumPy")
    );
    let accepted_wav = fs::read(render.directory.join("audio.wav")).unwrap();
    let accepted_report = fs::read(render.directory.join("report.json")).unwrap();
    let pointer = proof.join("current.json");
    fs::write(&pointer, r#"{"artifact":"accepted"}"#).unwrap();
    let cases = [
        ("truncated", "from teledra_synth import *\nfull_track = ["),
        (
            "missing-play",
            "import numpy as np\nfull_track = np.ones(192000) * .1",
        ),
        (
            "nan",
            "from teledra_synth import *\nplay_sound(np.ones(192000) * float('nan'), sr=48000)",
        ),
        (
            "caught-note-failure",
            "from teledra_synth import *\ntry:\n    synth_note('H4', .4)\nexcept ValueError:\n    pass\nplay_sound(np.ones(192000) * .1, sr=48000)",
        ),
    ];
    for (name, code) in cases {
        let directory = proof.join(name);
        assert!(
            render_python(
                &python,
                &workspace,
                &synth,
                &directory,
                code.to_owned(),
                &AtomicBool::new(false)
            )
            .is_err(),
            "{name}"
        );
        assert!(
            !directory.join("audio.wav").exists(),
            "{name} must not produce publishable audio"
        );
        assert_eq!(fs::read(&pointer).unwrap(), br#"{"artifact":"accepted"}"#);
        assert_eq!(
            fs::read(render.directory.join("audio.wav")).unwrap(),
            accepted_wav
        );
        assert_eq!(
            fs::read(render.directory.join("report.json")).unwrap(),
            accepted_report
        );
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let trigger = cancel.clone();
    let killer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(250));
        trigger.store(true, Ordering::Release);
    });
    let started = Instant::now();
    let error = render_python(
        &python,
        &workspace,
        &synth,
        &proof.join("cancelled"),
        "while True:\n    pass".to_owned(),
        &cancel,
    )
    .unwrap_err();
    killer.join().unwrap();
    assert!(error.contains("cancelled"));
    assert!(started.elapsed() < Duration::from_secs(8));
    assert!(!proof.join("cancelled/audio.wav").exists());
    eprintln!(
        "Verified 80 s Python artifact and rejected replacements at {}",
        proof.display()
    );
}

#[test]
#[ignore = "requires the user's V1 score and local Python/NumPy; produces only isolated verified audio, opens no device"]
fn unchanged_v1_python_score_renders_and_verifies() {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    let python = std::env::var_os("TELEDRA_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.join(".venv/Scripts/python.exe"));
    let synth = std::env::var_os("TELEDRA_SYNTH_EXE")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.join("target/release/teledra.exe"));
    let legacy = std::env::var_os("TELEDRA_V1_MUSIC")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("D:/Teledra/music.py"));
    let original = fs::read(&legacy).expect("read the unchanged V1 score");
    let source = String::from_utf8(original.clone()).unwrap();
    let proof = workspace
        .join("target/python-music-proof")
        .join(format!("v1-unchanged-{}", super::super::epoch_millis()));
    let render = render_python(
        &python,
        &workspace,
        &synth,
        &proof,
        source,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(fs::read(&legacy).unwrap(), original);
    assert_eq!(fs::read(proof.join("code.py")).unwrap(), original);
    assert_eq!(render.report["title"], "Fivefold Court Engine");
    assert_eq!(render.report["duration_seconds"], 88.0);
    assert_eq!(render.audio.report().duration_seconds, 88.0);
    assert_eq!(render.report["metadata"]["bars"], 40);
    assert_eq!(
        render.report["metadata"]["sections"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    assert_eq!(
        render.report["notes_requested"],
        render.report["notes_rendered"]
    );
    assert!(render.report["notes_requested"].as_u64().unwrap() > 500);
    eprintln!(
        "Verified unchanged V1 score: {} helper calls, {:.3} peak, {:.5} RMS, {:.2} seam at {}",
        render.report["notes_requested"],
        render.audio.report().peak,
        render.audio.report().rms,
        render.audio.report().seam_delta,
        proof.display()
    );
}

#[test]
#[ignore = "real cleaned bundled V1 worker and isolated library proof; requires local Python/NumPy and release synth, opens no player or UI"]
fn cleaned_bundled_v1_enters_library_and_reloads_verified_audio() {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    let python = std::env::var_os("TELEDRA_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.join(".venv/Scripts/python.exe"));
    let synth = std::env::var_os("TELEDRA_SYNTH_EXE")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.join("target/release/teledra.exe"));
    assert!(
        python.is_file(),
        "local Python required: {}",
        python.display()
    );
    assert!(
        synth.is_file(),
        "release synth required: {}",
        synth.display()
    );
    let proof_parent = workspace.join("target/python-music-proof");
    fs::create_dir_all(&proof_parent).unwrap();
    // Keep the isolated proof under the workspace, including failures, so no
    // application data or live library is read or mutated by this test.
    let proof = tempfile::Builder::new()
        .prefix("v1-library-normalized-")
        .tempdir_in(&proof_parent)
        .unwrap()
        .keep();
    let bundled = include_str!("../../../Music/v1_music_example.py");
    let source = clean_python_source(bundled);
    assert!(!source.ends_with('\n'));
    assert_ne!(
        source, bundled,
        "exercise the actual submission normalization"
    );
    assert!(super::super::music_library::is_bundled_v1_source(&source));
    let render = render_python(
        &python,
        &workspace,
        &synth,
        &proof.join("music/python/render-v1-reference"),
        source.clone(),
        &AtomicBool::new(false),
    )
    .unwrap_or_else(|error| panic!("V1 worker failed; evidence at {}: {error}", proof.display()));
    assert_eq!(render.code, source);
    assert_eq!(
        fs::read_to_string(render.directory.join("code.py")).unwrap(),
        source
    );
    assert_eq!(render.report["title"], "Fivefold Court Engine");
    assert_eq!(render.report["duration_seconds"], 88.0);
    assert_eq!(render.report["notes_requested"], 754);
    assert_eq!(render.report["notes_rendered"], 754);
    let mut library = MusicLibrary::load(&proof).unwrap();
    assert_eq!(library.editor_state(None)["entries"], serde_json::json!([]));
    assert!(
        MusicLibrary::eligibility(&render.report).is_err(),
        "the trusted V1 reference uses explicit admission, not a forged song audit"
    );
    let id = library.record_verified_v1_restore(&render).unwrap();
    assert_eq!(library.record_verified_v1_restore(&render).unwrap(), id);
    let library = MusicLibrary::load(&proof).unwrap();
    assert_eq!(library.id_for_artifact(&render.directory), Some(id.clone()));
    let state = library.editor_state(Some(&id));
    assert_eq!(state["entries"].as_array().unwrap().len(), 1);
    assert_eq!(state["entries"][0]["eligibility"], "verified_v1_reference");
    assert_eq!(state["entries"][0]["verdict"], "unreviewed");
    // Cached loading rechecks the actual source/report/WAV. It executes no
    // Python and opens no audio device; taste remains a listener decision.
    let loaded = library.load_playback(&id).unwrap();
    assert!(loaded.restore);
    assert_eq!(loaded.code, source);
    assert_eq!(loaded.audio.samples(), render.audio.samples());
    assert_eq!(loaded.report["audio_digest"], render.report["audio_digest"]);
    assert_eq!(
        loaded.report["source_digest"],
        render.report["source_digest"]
    );
    let evidence = serde_json::json!({"status":"passed","library_id":id,
        "source_normalization":"actual clean_python_source; outer trim and CRLF recognition",
        "duration_seconds":render.report["duration_seconds"],
        "notes_requested":render.report["notes_requested"],"notes_rendered":render.report["notes_rendered"],
        "source_digest":render.report["source_digest"],"audio_digest":render.report["audio_digest"],
        "library_reloaded":true,"cached_pcm_identical":true,"listener_verdict":"unreviewed",
        "model_calls":0,"audio_devices_opened":0,"live_application_data_changed":false});
    fs::write(
        proof.join("proof.json"),
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();
    eprintln!(
        "Cleaned bundled V1 admitted and reloaded, 88s/754 notes, proof: {}",
        proof.display()
    );
}

#[test]
fn the_composition_context_stays_inside_the_model_window() {
    let root = tempfile::tempdir().unwrap();
    let mut services =
        RuntimeServices::new(root.path().to_owned(), "unused-python".into()).unwrap();
    // A real accepted report carries a per-note audit of ~78,000 characters.
    let notes: Vec<_> = (0..2_000)
        .map(|index| serde_json::json!({"voice":"piano","start":index,"pitch":"C4","mixed":true}))
        .collect();
    services.python_music.report = serde_json::json!({
        "title": "Big piece", "metadata": {"key": "A dorian", "bpm": 108},
        "composition_audit": {"notes": notes, "layer_stats": {"lead": {"notes": 40}}, "notes_scheduled": 200, "notes_mixed": 200},
    });
    let context = services.python_composition_context();
    assert!(
        // Room remains for the persona, runtime facts and recent exchanges.
        context.chars().count() < teledra_brain::PROMPT_CHAR_BUDGET * 2 / 3,
        "composition context is {} characters",
        context.chars().count()
    );
    assert!(context.contains("layer_stats") || context.contains("\"layers\""));
    assert!(context.contains("piano (MIDI"));
}

#[test]
fn quiet_renders_are_lifted_to_court_loudness_without_clipping() {
    let sine = |amplitude: f32| {
        let samples: Vec<f32> = (0..48_000 * 6)
            .flat_map(|frame| {
                let v = (frame as f32 * 330.0 * std::f32::consts::TAU / 48_000.0).sin() * amplitude;
                [v, v]
            })
            .collect();
        VerifiedAudio::new(samples).unwrap()
    };
    let rms = |audio: &VerifiedAudio| {
        let s = audio.samples();
        (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt()
    };
    // An Organist-level render (-34 dBFS) is lifted to the court's -25 dBFS.
    let quiet = level_for_court(&sine(0.028));
    assert!((rms(&quiet) - 0.056).abs() < 0.003, "rms {}", rms(&quiet));
    // Loud material never has its peak pushed past 0.9.
    let loud = level_for_court(&sine(0.05));
    assert!(loud.report().peak <= 0.901);
    // Near-target audio is left exactly as it was.
    let near = sine(0.079);
    assert_eq!(level_for_court(&near).samples(), near.samples());
}
