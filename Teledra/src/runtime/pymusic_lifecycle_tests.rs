use super::*;

fn app() -> App {
    App::new(
        teledra_court::Court::default(),
        teledra_brain::BrainHealth {
            endpoint: "http://localhost:11434".into(),
            model: teledra_brain::QWEN_MODEL,
        },
    )
}

#[test]
#[ignore = "requires local Python, Tk and audio output; playback is muted"]
fn engines_switch_exclusively_and_reject_stale_work() {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    let directory = tempfile::tempdir().unwrap();
    let mut services = RuntimeServices::new(
        directory.path().to_owned(),
        workspace.join(".venv/Scripts/python.exe"),
    )
    .unwrap();
    services.python_music.synth = workspace.join("target/release/teledra.exe");
    let daw_state = services.daw.use_probe(directory.path());
    let py_state = directory.path().join("python-state.json");
    let wrapper = directory.path().join("python-probe.py");
    fs::write(&wrapper,format!(r#"import sys, json, os, tkinter as tk
from pathlib import Path
sys.path.insert(0, {module})
from python_music_editor import PythonMusicEditor
class Probe(PythonMusicEditor):
    def apply_command(self, command):
        if command.get('action') == 'probe_edit':
            self.editor.delete('1.0', 'end')
            self.editor.insert('1.0', command['code'])
        else:
            super().apply_command(command)
        if not self.closing:
            p = Path({state})
            t = p.with_suffix('.tmp')
            t.write_text(json.dumps({{'pid':os.getpid(), 'transport':self.transport, 'code':self.code(), 'catalog':len(self.instrument_catalog)}}),encoding='utf-8')
            t.replace(p)
root = tk.Tk()
root.withdraw()
Probe(root)
root.mainloop()
"#,module=serde_json::to_string(&workspace.join("Music")).unwrap(),state=serde_json::to_string(&py_state).unwrap())).unwrap();
    services.python_music.script = wrapper;
    services.python_music.code="from teledra_synth import *\nTELEDRA_SCORE={'title':'Exclusive proof'}\na=synth_note('C4',4,volume=.05,sr=48000)\nb=instrument_note('E4',1,instrument='piano',volume=.1,sr=48000)\na=mix_waves(a,b,start_time=.5,sr=48000)\nplay_sound(a,sr=48000,loop=True)".into();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut app = app();
    app.set_music_muted(true);
    let pump = |services: &mut RuntimeServices,
                app: &mut App,
                rx: &mut mpsc::UnboundedReceiver<ToolReply>| {
        let until = Instant::now() + Duration::from_secs(40);
        while services.music_rendering || services.python_music.rendering.is_some() {
            while let Ok(reply) = rx.try_recv() {
                if let Some(effect) = reply.effect {
                    services.apply_tool_effect(app, effect, &tx);
                }
            }
            assert!(
                Instant::now() < until,
                "render timed out: {:?}",
                app.snapshot().private_events
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    services.start_organist(&mut app, &tx);
    pump(&mut services, &mut app, &mut rx);
    let native = super::super::daw::tests::wait_state(&daw_state, |v| v["transport"] == "muted");
    let old_native_session = services.daw.session;
    assert!(services.music_player.is_some());
    services.select_music_engine(&mut app, &tx, MusicEngine::Python);
    assert!(
        services.music_player.is_none(),
        "manual switch stops old audio before starting the other render"
    );
    assert_ne!(services.daw.session, old_native_session);
    pump(&mut services, &mut app, &mut rx);
    let python = super::super::daw::tests::wait_state(&py_state, |v| v["catalog"] == 72);
    assert_ne!(native["pid"], python["pid"]);
    assert_eq!(services.music_engine, MusicEngine::Python);
    assert_eq!(services.python_music.report["notes_requested"], 2);
    assert_eq!(services.python_music.report["notes_rendered"], 2);
    let first_pointer = fs::read(directory.path().join("music/python/current.json")).unwrap();
    let old_python_session = services.python_music.session;
    // A generated no-op must not restart the current tune or claim a new
    // accepted artifact. Explicit restore/Play below remains supported.
    let accepted = services.python_music.accepted.clone().unwrap();
    let duplicate = read_python_artifact(
        &accepted,
        fs::read_to_string(accepted.join("code.py")).unwrap(),
        false,
    )
    .unwrap();
    services
        .music_player
        .as_ref()
        .unwrap()
        .seek_seconds(1.0)
        .unwrap();
    services.python_music.rendering = Some(services.music_epoch);
    services.python_music.repair_source = Some(duplicate.code.clone());
    services.python_ready(&mut app, &tx, services.music_epoch, duplicate);
    assert!(services.python_music.ready.is_none());
    assert!(services.music_player.as_ref().unwrap().position_seconds() >= 1.0);
    assert_eq!(
        fs::read(directory.path().join("music/python/current.json")).unwrap(),
        first_pointer
    );
    // Exercise the real persistence failure with an existing muted player.
    // The commit must keep playback and re-arm future development.
    let blocked = directory.path().join("music/python/current-next.json");
    fs::create_dir(&blocked).unwrap();
    let candidate = read_python_artifact(
        &accepted,
        fs::read_to_string(accepted.join("code.py")).unwrap(),
        false,
    )
    .unwrap();
    services.next_native_evolution = None;
    services.accept_python_music(&mut app, &tx, candidate);
    assert!(services.next_native_evolution.is_some());
    assert!(services.music_player.as_ref().unwrap().position_seconds() >= 1.0);
    assert_eq!(
        fs::read(directory.path().join("music/python/current.json")).unwrap(),
        first_pointer
    );
    fs::remove_dir(&blocked).unwrap();
    services.apply_tool_effect(
        &mut app,
        ToolEffect::StudioAction {
            session: old_native_session,
            action: super::super::daw::DawAction::Stop,
        },
        &tx,
    );
    assert!(
        app.music_enabled(),
        "stale old window cannot stop current playback"
    );
    services.submit_python_music(
        &mut app,
        &tx,
        "from teledra_synth import *\nraise ValueError('intentional failure')".into(),
    );
    pump(&mut services, &mut app, &mut rx);
    assert_eq!(
        fs::read(directory.path().join("music/python/current.json")).unwrap(),
        first_pointer
    );
    assert!(
        services.music_player.is_some(),
        "failed render retains current player"
    );
    let draft = "# unsaved editor work survives switching\nTITLE='kept draft'";
    services
        .python_music
        .send(serde_json::json!({"action":"probe_edit","code":draft}))
        .unwrap();
    super::super::daw::tests::wait_state(&py_state, |v| v["code"] == draft);
    services.select_music_engine(&mut app, &tx, MusicEngine::Native);
    assert!(services.python_music.child.is_none());
    assert_eq!(
        fs::read_to_string(directory.path().join("music/python/draft.py")).unwrap(),
        draft
    );
    pump(&mut services, &mut app, &mut rx);
    assert_eq!(services.music_engine, MusicEngine::Native);
    assert!(services.music_player.is_some());
    services.handle_python_action(&mut app, &tx, old_python_session, PythonAction::Stop);
    assert!(
        app.music_enabled(),
        "stale Python controls cannot stop native playback"
    );
    // A draft can be incomplete while the accepted piece remains playable.
    let accepted = services.python_music.accepted.clone().unwrap();
    let prepared = read_python_artifact(
        &accepted,
        fs::read_to_string(accepted.join("code.py")).unwrap(),
        true,
    )
    .unwrap();
    services.python_ready(&mut app, &tx, services.music_epoch, prepared);
    let boundary = services.python_music.ready.as_ref().unwrap().switch_after;
    services.publish_music_if_ready(&mut app, &tx);
    assert_eq!(
        services.music_engine,
        MusicEngine::Native,
        "automatic replacement waits for the complete current pass"
    );
    services
        .music_player
        .as_ref()
        .unwrap()
        .seek_seconds(boundary + 0.1)
        .unwrap();
    services.publish_music_if_ready(&mut app, &tx);
    assert_eq!(services.music_engine, MusicEngine::Python);
    assert_eq!(services.python_music.report["title"], "Exclusive proof");
    assert_eq!(services.python_music.code, draft);
    assert!(services.music_player.is_some());
    assert_eq!(
        fs::read_to_string(directory.path().join("music/python/draft.py")).unwrap(),
        draft
    );
    // Reprepare a completed one-shot from its accepted artifact, preserving
    // the editor draft instead of resuming an exhausted audio sink.
    let accepted = services.python_music.accepted.clone().unwrap();
    let mut replay = read_python_artifact(
        &accepted,
        fs::read_to_string(accepted.join("code.py")).unwrap(),
        true,
    )
    .unwrap();
    replay.report["loop"] = false.into();
    services.accept_python_music(&mut app, &tx, replay);
    services
        .music_player
        .as_ref()
        .unwrap()
        .seek_seconds(3.98)
        .unwrap();
    let until = Instant::now() + Duration::from_secs(3);
    while !services.music_player.as_ref().unwrap().is_finished() {
        assert!(Instant::now() < until, "one-shot did not finish");
        std::thread::sleep(Duration::from_millis(20));
    }
    let session = services.python_music.session;
    services.handle_python_action(&mut app, &tx, session, PythonAction::Play);
    pump(&mut services, &mut app, &mut rx);
    assert!(!services.music_player.as_ref().unwrap().is_finished());
    assert_eq!(services.python_music.code, draft);
    let request = app
        .begin_external_turn(teledra_agents::AgentId::Organist, "compose")
        .unwrap();
    services.cancel_music_composition(&mut app);
    assert!(services.music_reply_cancelled(request.id, request.agent));
    services.stop_music_work();
    services.music_player.take().unwrap().stop();
    services.daw.stop();
    services.python_music.close();
}

#[test]
fn automatic_comparison_queues_the_other_engine_and_can_be_disabled() {
    let root = tempfile::tempdir().unwrap();
    let mut services =
        RuntimeServices::new(root.path().to_owned(), "unused-python".into()).unwrap();
    assert!(services.music_alternating);
    let mut app = app();
    services.queue_composition(&mut app, false);
    let request = app.begin_queued_external_turn().unwrap();
    assert!(request.user.starts_with("PYMUSIC COMPOSITION:"));
    app.finish_turn(crate::app::ModelReply {
        id: request.id,
        agent: request.agent,
        user: request.user,
        result: Err("fixture".into()),
        audible: false,
    });
    services.music_engine = MusicEngine::Python;
    services.queue_composition(&mut app, false);
    let request = app.begin_queued_external_turn().unwrap();
    assert!(request.user.starts_with("NATIVE COMPOSITION:"));
    app.finish_turn(crate::app::ModelReply {
        id: request.id,
        agent: request.agent,
        user: request.user,
        result: Err("fixture".into()),
        audible: false,
    });
    services.toggle_music_comparison(&mut app);
    services.queue_composition(&mut app, false);
    assert!(
        app.begin_queued_external_turn()
            .unwrap()
            .user
            .starts_with("PYMUSIC COMPOSITION:")
    );
    let restored = RuntimeServices::new(root.path().to_owned(), "unused-python".into()).unwrap();
    assert!(!restored.music_alternating);
    assert_eq!(restored.music_engine, MusicEngine::Python);
}

#[test]
fn python_model_repair_uses_actual_error_and_is_bounded_per_accepted_piece() {
    let root = tempfile::tempdir().unwrap();
    let mut services =
        RuntimeServices::new(root.path().to_owned(), "unused-python".into()).unwrap();
    let mut app = app();
    app.set_music_enabled(true);
    services.request_python_repair(&mut app, "bad source".into(), "duration supplied twice");
    let request = app.begin_queued_external_turn().unwrap();
    assert!(request.user.contains("duration supplied twice"));
    assert!(request.user.contains("bad source"));
    app.finish_turn(crate::app::ModelReply {
        id: request.id,
        agent: request.agent,
        user: request.user,
        result: Err("fixture".into()),
        audible: false,
    });
    services.request_python_repair(&mut app, "another bad source".into(), "different error");
    assert!(app.begin_queued_external_turn().is_none());
}
