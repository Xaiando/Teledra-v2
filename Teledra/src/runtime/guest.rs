//! Buzzart, the fly-circuit composer, plays for the court as a guest musician.
//!
//! The Organist and Buzzart take turns: every second music round is the
//! guest's. Buzzart composes and renders in its own Python process; the court
//! alone owns playback, so a takeover always replaces the audio inside the one
//! player stream and whatever was playing stops with it. Two pieces never sound
//! at once, and neither performer's window keeps playing after the handover.
//!
//! The Organist's rounds have to change the music as well. Forty consecutive
//! requests for a new piece came back byte-identical, and the court — rightly
//! refusing to replay an identical render — kept one track playing for five
//! days. Each of his rounds now asks for a new piece framed by a drawn genome
//! (see `organist_genome`), as Buzzart draws one per tune. If his composition
//! still brings nothing new, the genome is rendered as a complete piece in his
//! ensemble; only if that fails does a different library piece stand in.
//!
//! The court knows its guest: agents are told what is playing, and now and
//! then one of them introduces Buzzart to the room as his set begins.

use std::collections::VecDeque;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;
use teledra_music::{Player, VerifiedAudio};
use tokio::sync::mpsc;

use super::{RuntimeServices, ToolEffect, ToolReply};
use crate::app::App;

/// A set that takes longer than this is abandoned and the round passes back.
const GUEST_TIMEOUT: Duration = Duration::from_secs(300);
/// Rendered takes kept on disk; each is roughly 17 MB of PCM.
const KEEP_TAKES: usize = 12;
/// Organist titles remembered so the library fallback does not repeat them.
const RECENT_TITLES: usize = 6;
/// Recent Organist frames a new genome must stay clearly apart from.
const RECENT_FRAMES: usize = 6;
/// Court members who take turns introducing the guest.
const INTRODUCERS: [teledra_agents::AgentId; 5] = [
    teledra_agents::AgentId::Queen,
    teledra_agents::AgentId::Orator,
    teledra_agents::AgentId::Organist,
    teledra_agents::AgentId::Diplomat,
    teledra_agents::AgentId::Alchemist,
];
/// Introductions stay occasional: the first set, then every third, and never
/// sooner than this after the previous one.
const INTRODUCTION_GAP: Duration = Duration::from_secs(12 * 60);
/// What Buzzart is, in a phrase the court can pass on.
const BUZZART: &str =
    "a composer whose musical instincts grow from a map of a fruit fly's brain wiring";
/// Outcomes that mean the Organist's round produced nothing new to play.
const NOTHING_NEW: [&str; 7] = [
    "unchanged",
    "incomplete",
    "render_failed",
    "acceptance_failed",
    "generation_failed",
    "missing_artifact",
    "edit_rejected",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Performer {
    /// The court starts on the Organist's music.
    #[default]
    Organist,
    Buzzart,
}

/// Whose turn it is, and what the current round still owes.
#[derive(Default)]
pub(super) struct MusicRota {
    /// Who holds the floor now.
    pub(super) current: Performer,
    /// A round the rota started that has not yet produced music.
    pub(super) preparing: Option<Performer>,
    /// The Organist's round failed to produce a new piece; a stand-in is due.
    pub(super) fallback_due: bool,
    /// The stand-in render or library load for this round, so it is published
    /// at a musical boundary instead of cutting in the moment it is ready.
    pub(super) fallback_ticket: Option<u64>,
    /// Stand-ins tried this round: first the genome's own piece, then library.
    pub(super) stand_ins: u8,
    /// The genome framing the Organist's current round.
    pub(super) genome: Option<Value>,
    pub(super) rounds: u64,
    recent_titles: VecDeque<String>,
    recent_frames: VecDeque<Value>,
}

impl MusicRota {
    /// The performers alternate: each round goes to whoever did not play last.
    pub(super) fn next(&self) -> Performer {
        match self.current {
            Performer::Organist => Performer::Buzzart,
            Performer::Buzzart => Performer::Organist,
        }
    }

    pub(super) fn handed_to(&mut self, performer: Performer, title: Option<&str>) {
        self.current = performer;
        self.preparing = None;
        self.fallback_due = false;
        self.fallback_ticket = None;
        self.stand_ins = 0;
        self.genome = None;
        self.rounds += 1;
        if let (Performer::Organist, Some(title)) = (performer, title) {
            self.recent_titles.retain(|recent| recent != title);
            self.recent_titles.push_back(title.to_owned());
            while self.recent_titles.len() > RECENT_TITLES {
                self.recent_titles.pop_front();
            }
        }
    }

    /// Records an Organist outcome; true when it leaves his round with nothing
    /// new, so a stand-in is due. Each kind of stand-in is tried once.
    pub(super) fn organist_outcome(&mut self, outcome: &str) -> bool {
        if self.preparing != Some(Performer::Organist) || !NOTHING_NEW.contains(&outcome) {
            return false;
        }
        if self.fallback_ticket.take().is_some() {
            if self.stand_ins < 2 {
                // The genome's piece failed; a library piece is the last resort.
                self.fallback_due = true;
                return true;
            }
            // Every stand-in failed; the round ends and the next one follows.
            self.preparing = None;
            return false;
        }
        if self.fallback_due {
            return false;
        }
        self.fallback_due = true;
        true
    }

    pub(super) fn recently_played(&self, title: &str) -> bool {
        self.recent_titles.iter().any(|recent| recent == title)
    }

    /// Remembers the frame of a piece the Organist played.
    pub(super) fn remember_frame(&mut self, song: Value) {
        self.recent_frames.push_back(song);
        while self.recent_frames.len() > RECENT_FRAMES {
            self.recent_frames.pop_front();
        }
    }

    /// Draws the genome for an Organist round, apart from his recent pieces.
    pub(super) fn draw_genome(&mut self, seed: u64, playing: Option<Value>) -> &Value {
        let mut recent: Vec<Value> = self.recent_frames.iter().cloned().collect();
        recent.extend(playing);
        self.genome = Some(crate::organist_genome::draw(seed, &recent));
        self.genome.as_ref().expect("just drawn")
    }
}

pub(super) struct GuestTake {
    pub(super) report: Value,
    pub(super) audio: VerifiedAudio,
}

pub(super) struct GuestReady {
    pub(super) take: GuestTake,
    pub(super) switch_after: f64,
}

/// Buzzart's rendering process and the set it has ready for the court.
pub(super) struct GuestMusician {
    python: PathBuf,
    script: PathBuf,
    brain: Option<String>,
    disabled: bool,
    running: Option<(Child, PathBuf, Instant)>,
    pub(super) ready: Option<GuestReady>,
    /// Length of the guest set now playing, which the court's boundary and
    /// cadence arithmetic must use instead of the Organist's piece.
    pub(super) playing_seconds: Option<f64>,
    /// Title and description of the set now playing, for the court's context.
    pub(super) now_playing: Option<(String, String)>,
    sets_played: u64,
    last_introduction: Option<Instant>,
}

impl GuestMusician {
    pub(super) fn new(python: PathBuf) -> Self {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        Self {
            python,
            script: workspace.join("FlyComposer").join("teledra_guest.py"),
            brain: std::env::var("TELEDRA_GUEST_BRAIN")
                .ok()
                .filter(|value| !value.is_empty()),
            // Court tests never summon a real guest process unless they ask for one.
            disabled: cfg!(test) || std::env::var_os("TELEDRA_DISABLE_GUEST").is_some(),
            running: None,
            ready: None,
            playing_seconds: None,
            now_playing: None,
            sets_played: 0,
            last_introduction: None,
        }
    }

    pub(super) fn available(&self) -> bool {
        !self.disabled && self.script.is_file()
    }

    /// Counts a set and says who, if anyone, introduces it to the room.
    fn introducer(&mut self) -> Option<teledra_agents::AgentId> {
        self.sets_played += 1;
        let due = self.sets_played == 1 || self.sets_played.is_multiple_of(3);
        let rested = self
            .last_introduction
            .is_none_or(|last| last.elapsed() >= INTRODUCTION_GAP);
        if !due || !rested {
            return None;
        }
        self.last_introduction = Some(Instant::now());
        let turn = (self.sets_played / 3) as usize;
        Some(INTRODUCERS[turn % INTRODUCERS.len()])
    }

    pub(super) fn busy(&self) -> bool {
        self.running.is_some() || self.ready.is_some()
    }

    pub(super) fn start(&mut self, takes: &Path) -> io::Result<()> {
        fs::create_dir_all(takes)?;
        prune_takes(takes, KEEP_TAKES);
        let name = format!("take-{}", super::epoch_millis());
        let out = takes.join(&name);
        // The log sits beside the take: the script requires an empty directory.
        let log = File::create(takes.join(format!("{name}.log")))?;
        let mut command = Command::new(&self.python);
        command
            .arg(&self.script)
            .arg("--out")
            .arg(&out)
            // Every set is kept as a compressed copy; full takes are pruned.
            .arg("--archive")
            .arg(takes.join("archive"))
            .current_dir(self.script.parent().unwrap_or_else(|| Path::new(".")))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(log));
        if let Some(brain) = &self.brain {
            command.arg("--brain").arg(brain);
        }
        super::daw::hide_console(&mut command);
        self.running = Some((command.spawn()?, out, Instant::now()));
        Ok(())
    }

    /// A finished set, a failure, or nothing yet. Never blocks.
    pub(super) fn poll(&mut self) -> Option<Result<GuestTake, String>> {
        let (child, out, started) = self.running.as_mut()?;
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = out.clone();
                self.running = None;
                if !status.success() {
                    return Some(Err(format!(
                        "Buzzart's render exited with {status}; see {}.log",
                        out.display()
                    )));
                }
                Some(load_take(&out))
            }
            Ok(None) if started.elapsed() > GUEST_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                self.running = None;
                Some(Err(
                    "Buzzart took longer than five minutes; the round returns to the Organist"
                        .to_owned(),
                ))
            }
            Ok(None) => None,
            Err(error) => {
                self.running = None;
                Some(Err(error.to_string()))
            }
        }
    }
}

impl Drop for GuestMusician {
    fn drop(&mut self) {
        if let Some((child, _, _)) = self.running.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn introduction_prompt(title: &str, description: &str) -> String {
    format!(
        "COURT MUSIC: The court's guest musician Buzzart has just taken the floor with \"{title}\". Buzzart is {BUZZART}; he and the Organist take turns playing for the court. His set, in his own notes: {description}. Introduce Buzzart or this set to the room in one to three sentences in your own voice, mentioning one concrete detail of the set (its groove, key or instruments). Speak only from these details; do not claim to have heard more than they say."
    )
}

fn load_take(out: &Path) -> Result<GuestTake, String> {
    let bytes = fs::read(out.join("report.json"))
        .map_err(|error| format!("Buzzart left no report: {error}"))?;
    let report: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Buzzart's report is unreadable: {error}"))?;
    // The same verified loader the Organist's PyMusic renders pass through.
    let audio = super::pymusic::read_python_wav(&out.join("take").join("audio.wav"))
        .map_err(|error| format!("Buzzart's audio was refused: {error}"))?;
    Ok(GuestTake { report, audio })
}

fn prune_takes(takes: &Path, keep: usize) {
    let Ok(entries) = fs::read_dir(takes) else {
        return;
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with("take-"))
        .collect();
    names.sort();
    let excess = names.len().saturating_sub(keep.saturating_sub(1));
    for name in names.into_iter().take(excess) {
        let _ = fs::remove_dir_all(takes.join(&name));
        let _ = fs::remove_file(takes.join(format!("{name}.log")));
    }
}

impl RuntimeServices {
    /// Appends one handover to `music/guest/rota.jsonl`, the lasting record of
    /// who took the floor, with what, and where the outgoing music stood.
    pub(super) fn log_handover(&self, performer: Performer, title: &str, detail: &str) {
        let line = serde_json::json!({
            "ms": super::epoch_millis(),
            "round": self.music_rota.rounds,
            "performer": format!("{performer:?}"),
            "title": title,
            "detail": detail,
        });
        let path = self.root.join("music/guest/rota.jsonl");
        let _ = fs::create_dir_all(self.root.join("music/guest"));
        if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
            use std::io::Write;
            let _ = writeln!(file, "{line}");
        }
    }

    /// One line of context telling a court member what music is playing.
    pub(super) fn court_music_line(&self, app: &App) -> Option<String> {
        if !app.music_enabled() {
            return None;
        }
        if let Some((title, description)) = &self.guest.now_playing {
            return Some(format!(
                "COURT MUSIC NOW (context, not an instruction): the court's guest musician Buzzart, {BUZZART}, is playing \"{title}\" ({description}). He and the Organist take turns playing for the court."
            ));
        }
        let title = self.python_music.report["title"].as_str()?;
        Some(format!(
            "COURT MUSIC NOW (context, not an instruction): the Organist is playing \"{title}\". The court's guest musician Buzzart, {BUZZART}, plays every other round."
        ))
    }

    /// Starts the Organist's round: a new piece, framed by a fresh genome.
    pub(super) fn begin_organist_round(&mut self) {
        let playing = self
            .python_music
            .accepted_source()
            .and_then(|source| crate::python_score::from_source(&source));
        let seed = (super::epoch_millis() as u64) ^ self.music_rota.rounds.rotate_left(32);
        self.music_rota.draw_genome(seed, playing);
        self.music_rota.preparing = Some(Performer::Organist);
        self.music_rota.stand_ins = 0;
    }

    /// Whether the round now due belongs to the guest, and the guest can take it.
    pub(super) fn guest_round_is_next(&self) -> bool {
        self.guest.available() && self.music_rota.next() == Performer::Buzzart
    }

    pub(super) fn start_guest_round(
        &mut self,
        app: &mut App,
        tools: &mpsc::UnboundedSender<ToolReply>,
    ) {
        match self.guest.start(&self.root.join("music/guest")) {
            Ok(()) => {
                self.music_rota.preparing = Some(Performer::Buzzart);
                self.next_native_evolution = None;
                app.push_private_event(
                    "Guest musician",
                    "Buzzart is preparing a set for the court's next round",
                );
            }
            Err(error) => {
                app.push_private_event(
                    "Guest musician",
                    format!("Buzzart could not start ({error}); the Organist keeps the round"),
                );
                self.music_rota.preparing = Some(Performer::Organist);
                self.evolve_music_now(app, tools, "automatic");
            }
        }
    }

    /// Collects a finished set and hands it the floor at a musical boundary.
    pub(super) fn publish_guest_if_ready(&mut self, app: &mut App) {
        if let Some(result) = self.guest.poll() {
            match result {
                Ok(take) => {
                    let switch_after = match &self.music_player {
                        Some(player) if !player.is_finished() => super::next_music_boundary(
                            player.position_seconds(),
                            self.active_music_duration(),
                        ),
                        _ => 0.0,
                    };
                    app.push_private_event(
                        "Guest musician",
                        format!(
                            "{} is ready; it takes over when the current pass ends",
                            take.report["title"].as_str().unwrap_or("Buzzart's set")
                        ),
                    );
                    self.guest.ready = Some(GuestReady { take, switch_after });
                }
                Err(error) => {
                    self.log_handover(
                        Performer::Organist,
                        "",
                        &format!("guest round failed: {error}"),
                    );
                    app.push_private_event("Guest musician", error);
                    // The round passes to the Organist rather than stalling.
                    self.music_rota.preparing = None;
                    self.music_rota.current = Performer::Buzzart;
                    self.next_native_evolution = Some(Instant::now());
                }
            }
        }
        let Some(ready) = &self.guest.ready else {
            return;
        };
        if self.music_player.as_ref().is_some_and(|player| {
            !player.is_finished() && player.position_seconds() < ready.switch_after
        }) {
            return;
        }
        let ready = self.guest.ready.take().expect("checked above");
        self.accept_guest_music(app, ready.take);
    }

    fn accept_guest_music(&mut self, app: &mut App, take: GuestTake) {
        let seconds = f64::from(take.audio.report().duration_seconds);
        let title = take.report["title"]
            .as_str()
            .unwrap_or("Buzzart guest set")
            .to_owned();
        let description = take.report["description"].as_str().unwrap_or("").to_owned();
        let outgoing = self
            .music_player
            .as_ref()
            .map_or(0.0, Player::position_seconds);
        // The outgoing music exits here: the new PCM replaces it inside the one
        // player stream, and the Organist's studio window closes first.
        self.daw.stop();
        let replaced = match self.music_player.as_mut() {
            Some(player) => player
                .replace(super::pymusic::level_for_court(&take.audio), true, 0.0)
                .map_err(|error| error.to_string()),
            None => Player::prepare_on(
                super::pymusic::level_for_court(&take.audio),
                true,
                std::env::var("TELEDRA_AUDIO_DEVICE").ok().as_deref(),
            )
            .map(|player| self.music_player = Some(player))
            .map_err(|error| error.to_string()),
        };
        if let Err(error) = replaced {
            app.push_private_event(
                "Guest musician",
                format!(
                    "Buzzart's set could not be played ({error}); the Organist keeps the floor"
                ),
            );
            self.music_rota.preparing = None;
            self.music_rota.current = Performer::Buzzart;
            self.next_native_evolution = Some(Instant::now());
            return;
        }
        if let Some(player) = &self.music_player {
            player.set_muted(app.music_muted());
            if app.music_enabled() {
                player.resume();
            } else {
                player.pause();
            }
        }
        let _ = self.python_music.send(serde_json::json!({
            "action": "status",
            "message": format!("Guest musician Buzzart has the floor: {title}"),
        }));
        self.guest.playing_seconds = Some(seconds);
        self.guest.now_playing = Some((title.clone(), description.clone()));
        self.music_rota.handed_to(Performer::Buzzart, None);
        self.log_handover(
            Performer::Buzzart,
            &title,
            &format!(
                "replaced the playing audio in the same stream at {outgoing:.1}s; seed {}",
                take.report["seed"]
            ),
        );
        app.set_notice(format!("Guest musician: {title}"));
        // A public line on every set filled the transcript every couple of
        // minutes; announce only the sets a court member introduces.
        if app.music_enabled()
            && !app.music_muted()
            && let Some(agent) = self.guest.introducer()
        {
            app.push_public_message(
                "Court",
                format!("The court's guest musician Buzzart takes the floor: {title}."),
            );
            app.queue_external_turn(agent, introduction_prompt(&title, &description));
        }
        app.push_private_event(
            "Guest musician",
            format!(
                "{title}: {seconds:.0}s, seed {}, {} voices on average; the previous music was replaced in the same stream",
                take.report["seed"],
                take.report["mean_voices"]
            ),
        );
        // The Organist's round comes next; he composes while the guest plays.
        self.schedule_native_evolution();
    }

    /// Plays a different piece from the Organist's library when his round
    /// produced nothing new, so his turn still changes the music.
    pub(super) fn start_organist_fallback(
        &mut self,
        app: &mut App,
        tools: &mpsc::UnboundedSender<ToolReply>,
    ) {
        if self.python_music.rendering.is_some() || self.python_music.ready.is_some() {
            return; // still busy; try again next tick
        }
        self.music_rota.fallback_due = false;
        self.music_rota.stand_ins += 1;
        if self.music_rota.stand_ins == 1
            && let Some(genome) = self.music_rota.genome.clone()
        {
            // His own frame and ensemble, with the genome's drawn melody: a
            // new piece even when his composition brought none.
            let code =
                crate::python_score::expand_proposal(crate::organist_genome::proposal(&genome))
                    .and_then(|song| crate::python_score::source(&song));
            match code {
                Ok(code) => {
                    app.push_private_event(
                        "Organist",
                        format!(
                            "No new piece from the composer this round; rendering the round's genome as \"{}\" ({} ensemble, {} {} at {} BPM)",
                            genome["title"].as_str().unwrap_or("a new piece"),
                            genome["ensemble"].as_str().unwrap_or("drawn"),
                            genome["tonic"].as_str().unwrap_or(""),
                            genome["mode"].as_str().unwrap_or(""),
                            genome["bpm"],
                        ),
                    );
                    self.submit_python_music(app, tools, code);
                    if self.python_music.rendering == Some(self.music_epoch) {
                        self.music_rota.fallback_ticket = Some(self.music_epoch);
                        return;
                    }
                }
                Err(error) => app.push_private_event("Organist", error),
            }
        }
        self.music_rota.stand_ins = 2;
        let Some(library) = self.python_music.library.clone() else {
            app.push_private_event("Music rota", "The Organist has no library to fall back on");
            self.music_rota.preparing = None;
            return;
        };
        let playing = self.python_music.report["title"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        let rota = &self.music_rota;
        let Some((id, title)) =
            library.rotation_pick(|title| title == playing || rota.recently_played(title))
        else {
            app.push_private_event(
                "Music rota",
                "No other library piece is available for the Organist's round",
            );
            self.music_rota.preparing = None;
            return;
        };
        let ticket = self.music_epoch;
        self.python_music.rendering = Some(ticket);
        self.music_rota.fallback_ticket = Some(ticket);
        app.push_private_event(
            "Organist",
            format!(
                "No new piece this round; the Organist plays \"{title}\" from his library instead"
            ),
        );
        let tools = tools.clone();
        std::thread::spawn(move || {
            let effect = match library.load_playback(&id) {
                Ok(render) => ToolEffect::PythonReady { ticket, render },
                Err(error) => ToolEffect::PythonFailed { ticket, error },
            };
            let _ = tools.send(ToolReply {
                source: "Music rota",
                result: Ok("Loaded the Organist's library piece".into()),
                court_prompt: None,
                effect: Some(effect),
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_performers_alternate_every_round() {
        let mut rota = MusicRota::default();
        assert_eq!(rota.current, Performer::Organist);
        assert_eq!(rota.next(), Performer::Buzzart);
        rota.handed_to(Performer::Buzzart, None);
        assert_eq!(rota.next(), Performer::Organist);
        rota.handed_to(Performer::Organist, Some("A"));
        assert_eq!(rota.next(), Performer::Buzzart);
        assert_eq!(rota.rounds, 2);
    }

    #[test]
    fn a_round_that_produces_nothing_new_falls_back_once() {
        let mut rota = MusicRota {
            preparing: Some(Performer::Organist),
            ..MusicRota::default()
        };
        assert!(!rota.organist_outcome("requested"));
        assert!(rota.organist_outcome("unchanged"));
        // Already due: a second failure in the same round does not stack.
        assert!(!rota.organist_outcome("render_failed"));
        // The genome's own piece is tried first...
        rota.fallback_due = false;
        rota.stand_ins = 1;
        rota.fallback_ticket = Some(3);
        // ...and if it fails, a library piece is still due.
        assert!(rota.organist_outcome("render_failed"));
        assert!(rota.fallback_due);
        // If the library piece fails too, the round ends instead of looping.
        rota.fallback_due = false;
        rota.stand_ins = 2;
        rota.fallback_ticket = Some(4);
        assert!(!rota.organist_outcome("render_failed"));
        assert_eq!(rota.preparing, None);
        assert_eq!(rota.fallback_ticket, None);
    }

    #[test]
    fn introductions_are_occasional_and_rotate_through_the_court() {
        let mut guest = GuestMusician::new(PathBuf::from("python"));
        assert_eq!(guest.introducer(), Some(INTRODUCERS[0]));
        assert_eq!(guest.introducer(), None); // second set
        // The third set is due, but the previous introduction was just now.
        assert_eq!(guest.introducer(), None);
        guest.last_introduction = Some(Instant::now() - INTRODUCTION_GAP);
        assert_eq!(guest.introducer(), None); // fourth set
        assert_eq!(guest.introducer(), None); // fifth set
        assert_eq!(guest.introducer(), Some(INTRODUCERS[2])); // sixth set
        let prompt = introduction_prompt("Buzzart guest set: shuffle in G minor", "shuffle groove");
        assert!(prompt.contains("shuffle groove") && prompt.contains("fruit fly"));
    }

    #[test]
    fn organist_genomes_stay_apart_from_his_recent_pieces() {
        let mut rota = MusicRota::default();
        let mut drawn = Vec::new();
        for round in 0..10 {
            let genome = rota.draw_genome(round * 104_729, None).clone();
            for earlier in drawn.iter().rev().take(RECENT_FRAMES) {
                assert!(crate::organist_genome::distance(&genome, earlier) >= 5);
            }
            rota.remember_frame(genome.clone());
            drawn.push(genome);
        }
    }

    #[test]
    #[ignore = "renders one genome piece per ensemble through the real PyMusic worker"]
    fn genome_stand_ins_render_as_verified_library_pieces() {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_owned();
        let root = tempfile::tempdir().unwrap();
        let mut ensembles = std::collections::BTreeSet::new();
        for seed in 0..400_u64 {
            let genome = crate::organist_genome::draw(seed * 31_337, &[]);
            let ensemble = genome["ensemble"].as_str().unwrap().to_owned();
            if !ensembles.insert(ensemble.clone()) {
                continue;
            }
            let song =
                crate::python_score::expand_proposal(crate::organist_genome::proposal(&genome))
                    .unwrap();
            let code = crate::python_score::source(&song).unwrap();
            let directory = root.path().join(format!("render-{seed}"));
            let render = super::super::pymusic::render_python(
                &workspace.join(".venv/Scripts/python.exe"),
                &workspace,
                &workspace.join("target/release/teledra.exe"),
                &directory,
                code,
                &std::sync::atomic::AtomicBool::new(false),
            )
            .unwrap_or_else(|error| panic!("{ensemble}: {error}"));
            super::super::music_library::MusicLibrary::eligibility(&render.report)
                .unwrap_or_else(|error| panic!("{ensemble}: {error}"));
            eprintln!(
                "{ensemble}: {} ({} {} {} BPM) {:.1}s",
                genome["title"],
                genome["tonic"],
                genome["mode"],
                genome["bpm"],
                render.audio.report().duration_seconds
            );
            if ensembles.len() == 8 {
                return;
            }
        }
        panic!("only drew ensembles {ensembles:?}");
    }

    #[test]
    fn outcomes_outside_an_organist_round_change_nothing() {
        let mut rota = MusicRota::default();
        assert!(!rota.organist_outcome("unchanged"));
        rota.preparing = Some(Performer::Buzzart);
        assert!(!rota.organist_outcome("unchanged"));
    }

    #[test]
    fn recently_played_titles_are_remembered_and_bounded() {
        let mut rota = MusicRota::default();
        for index in 0..10 {
            rota.handed_to(Performer::Organist, Some(&format!("piece {index}")));
        }
        assert!(rota.recently_played("piece 9"));
        assert!(!rota.recently_played("piece 0"));
        // A guest round records no Organist title.
        rota.handed_to(Performer::Buzzart, Some("ignored"));
        assert!(!rota.recently_played("ignored"));
    }

    #[test]
    fn old_takes_are_pruned_to_the_newest() {
        let root = tempfile::tempdir().unwrap();
        for index in 0..15 {
            let name = format!("take-{index:04}");
            fs::create_dir_all(root.path().join(&name)).unwrap();
            fs::write(root.path().join(format!("{name}.log")), "").unwrap();
        }
        prune_takes(root.path(), KEEP_TAKES);
        let left: Vec<_> = fs::read_dir(root.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_dir())
            .collect();
        // Room is left for the take about to be written.
        assert_eq!(left.len(), KEEP_TAKES - 1);
        assert!(root.path().join("take-0014").is_dir());
        assert!(!root.path().join("take-0000").exists());
        assert!(!root.path().join("take-0000.log").exists());
    }

    #[test]
    #[ignore = "requires the FlyComposer venv and an audio output; playback is muted"]
    fn a_real_guest_set_is_verified_and_replaces_the_playing_music() {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_owned();
        let root = tempfile::tempdir().unwrap();
        let mut services = RuntimeServices::new(
            root.path().to_owned(),
            workspace.join(".venv/Scripts/python.exe"),
        )
        .unwrap();
        let mut app = crate::app::App::new(
            teledra_court::Court::default(),
            teledra_brain::BrainHealth {
                endpoint: "http://localhost:11434".into(),
                model: teledra_brain::QWEN_MODEL,
            },
        );
        app.set_music_muted(true);
        services.guest.disabled = false;
        // The Organist's piece is playing, well into its pass.
        let organist: Vec<f32> = (0..48_000 * 8)
            .flat_map(|frame| {
                let sample = (frame as f32 * 220.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.1;
                [sample, sample]
            })
            .collect();
        let organist = VerifiedAudio::new(organist).unwrap();
        let player = Player::prepare_on(organist, true, None).unwrap();
        player.set_muted(true);
        player.resume();
        services.music_player = Some(player);
        std::thread::sleep(Duration::from_millis(1_500));
        let before = services.music_player.as_ref().unwrap().position_seconds();
        assert!(before >= 1.0);

        assert!(services.guest_round_is_next());
        services
            .guest
            .start(&root.path().join("music/guest"))
            .unwrap();
        services.music_rota.preparing = Some(Performer::Buzzart);
        let until = Instant::now() + Duration::from_secs(240);
        while services.guest.ready.is_none() {
            services.publish_guest_if_ready(&mut app);
            assert!(
                services.guest.running.is_some() || services.guest.ready.is_some(),
                "guest failed: {:?}",
                app.snapshot().private_events
            );
            assert!(Instant::now() < until, "guest render timed out");
            std::thread::sleep(Duration::from_millis(100));
        }
        let ready = services.guest.ready.as_ref().unwrap();
        // Handover waits for the end of the Organist's pass, never mid-phrase.
        assert!(ready.switch_after >= 8.0);
        let seconds = f64::from(ready.take.audio.report().duration_seconds);
        assert!(seconds >= 60.0, "guest set is {seconds}s");
        services.guest.ready.as_mut().unwrap().switch_after = 0.0;
        services.publish_guest_if_ready(&mut app);

        // Same stream, new PCM from the start: the old music has exited. The
        // audio thread takes the swap up on its next buffer (40 ms crossfade).
        std::thread::sleep(Duration::from_millis(400));
        let after = services.music_player.as_ref().unwrap().position_seconds();
        assert!(
            after < 0.5,
            "playhead {after} did not restart on the guest set"
        );
        assert!(services.guest.ready.is_none());
        assert_eq!(services.guest.playing_seconds, Some(seconds));
        assert_eq!(services.active_music_duration(), seconds);
        assert_eq!(services.music_rota.current, Performer::Buzzart);
        assert_eq!(services.music_rota.next(), Performer::Organist);
        assert!(!services.guest_round_is_next());
        services.music_player.take().unwrap().stop();
    }
}
