use std::collections::VecDeque;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use teledra_agents::AgentId;
use teledra_brain::{BrainHealth, Message};
use teledra_court::{Court, Turn};

use crate::portrait::Portrait;
use crate::session::{SessionSnapshot, SnapshotEvent, SnapshotMessage, SnapshotTurn};

const MAX_INPUT_CHARS: usize = 16 * 1024;
const MAX_YOUTUBE_URL_CHARS: usize = 2_048;
const MAX_EXTERNAL_PROMPT_CHARS: usize = 32 * 1024;
const MAX_PRIVATE_EVENT_CHARS: usize = 12 * 1024;
const MAX_PUBLIC_MESSAGES: usize = 240;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublicMessage {
    pub sender: String,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppCommand {
    Vertical,
    MusicPlay,
    MusicEvolve,
    MusicStop,
    MusicToggle,
    MusicMute,
    MusicKeep(String),
    MusicReject(String),
    Fractus(String),
    FractusLive(String),
    Dream,
    Sleep,
    Reflect,
    Memory(String),
    Research(String),
    SocialLinks,
    YouTube(String),
    Microphone,
    MicrophoneToggle,
    Vision,
    Somatic,
    StreamChat,
    StreamConnect(String),
    Agents,
    Envoy(String),
    Outreach(String),
    Treasury,
    TreasuryReport,
    Mcp(String),
    CoPilot,
    Mission(String),
    MissionCancel,
    Verify,
    Tower,
    Dashboard,
    TestMode(Option<bool>),
    TestStatus,
    TestKnobs(String),
    TestCast(String),
    TestScene(String),
    TestRate(String),
    TestSilence(String),
    TestTick,
    TestMusic,
    NightDesk,
    Study(String),
    Innovate(String),
    Goals,
    GrowthReport,
    SimChat(String),
    Workshop(String),
    Lock(String),
    Unlock,
    Rundown,
    Work,
    Proposals,
    DiplomacyLog,
    ClearProposals,
    Panel,
    Approve(String),
    Reject(String),
    Mode(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppAction {
    Continue,
    Submit,
    Quit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FocusField {
    Chat,
    YouTube,
}

#[derive(Debug)]
pub struct InferenceRequest {
    pub id: u64,
    pub agent: AgentId,
    pub user: String,
    pub messages: Vec<Message>,
    pub temperature: f32,
    pub audible: bool,
}

#[derive(Debug)]
pub struct ModelReply {
    pub id: u64,
    pub agent: AgentId,
    pub user: String,
    pub result: Result<String, String>,
    pub audible: bool,
}

#[derive(Clone, Debug)]
pub struct PendingTurn {
    pub id: u64,
    pub agent: AgentId,
    pub user: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SomaticSnapshot {
    pub face_detected: bool,
    pub hands_detected: bool,
    pub shoulder_asymmetry: Option<f32>,
    pub source: String,
    pub error: Option<String>,
}

#[derive(Debug)]
pub struct App {
    court: Court,
    input: String,
    youtube_input: String,
    focus: FocusField,
    pending: Option<PendingTurn>,
    notice: String,
    help_open: bool,
    scroll: u16,
    frame: usize,
    next_request_id: u64,
    portrait: Option<Portrait>,
    command: Option<AppCommand>,
    public_messages: VecDeque<PublicMessage>,
    private_events: VecDeque<(String, String)>,
    external_queue: VecDeque<(AgentId, String, bool)>,
    mode: String,
    night_desk_enabled: bool,
    memory_context: String,
    identity_context: String,
    locked_topic: Option<String>,
    locked_idle_turns: u32,
    test_mode: bool,
    test_lurker_silence: bool,
    test_cast: String,
    test_scene: String,
    test_knobs: String,
    music_enabled: bool,
    /// Silence that keeps the piece running.
    ///
    /// Ctrl+M used to stop the synth outright, which discarded the player and
    /// forced a full re-render to resume. An operator reaching for a mute in
    /// the middle of a broadcast wants silence now and the same piece back
    /// afterwards, not a gap and a different arrangement.
    music_muted: bool,
    microphone_enabled: bool,
    pending_proposals: usize,
    workshop_count: usize,
    somatic: SomaticSnapshot,
    autonomy_turn: u64,
    /// Night Desk has its own rota so backstage maintenance cannot silently
    /// skip the next public speaker or repeat the public court's prompt angle.
    night_desk_turn: u64,
    /// What the court has recently said aloud, normalised for comparison.
    ///
    /// A local 7B model asked the same question with the same history returns
    /// the same answer, and once it does the answer is in the history too, so
    /// it returns it again. One line was spoken nine times in a row on stream.
    /// Remembering what was just said is what breaks the loop.
    recent_utterances: VecDeque<String>,
    /// What the console granted, recorded once at startup.
    terminal_report: Option<crate::terminal::TerminalReport>,
    quit_requested: bool,
}

/// How many spoken lines are remembered for repetition checking.
const RECENT_UTTERANCE_MEMORY: usize = 24;

/// A shared opening this long means the model is stuck, not being consistent.
///
/// Exact matching alone was not enough: the court kept its formula and varied
/// only the tail, so "Ah my dear subjects have you ever wondered about the time
/// our court hosted..." arrived twice and passed as two different lines. Sixty
/// characters is roughly a full clause — long enough that sharing one is a
/// repeated thought rather than a repeated turn of phrase.
const REPEATED_OPENING: usize = 60;

/// Angles for an idle turn, so consecutive prompts are never identical.
///
/// The prompt used to be one fixed sentence. Identical input is the reason the
/// court locked onto a single observation about floor tiles and repeated it
/// verbatim; varying the request is the cheapest way to keep it moving.
const IDLE_ANGLES: [&str; 10] = [
    "Pass judgement on something you have noticed in the room.",
    "Recall a scrap of court history and what it cost.",
    "Pick a quarrel with one of your ministers, affectionately.",
    "Make a small, petty decree and justify it.",
    "Wonder aloud about something outside the palace walls.",
    "Praise or dismiss a piece of work the court has produced.",
    "Tell the room a short, strange thing you believe is true.",
    "Ask the audience something you actually want answered.",
    "Describe a craving, an appetite, or an impatience.",
    "Notice the hour and what it does to your mood.",
];

/// Who takes the next unprompted public turn.
///
/// Every daytime idle turn used to go to the Queen, and the ministers only
/// spoke when she delegated to them through an action tag — which she rarely
/// did. Three hundred live turns ran 76% Queen, with runs of sixteen in a row,
/// most of them withheld as repetitions: a 7B model asked the same thing by the
/// same voice returns the same answer. That is what "the court died down" looks
/// like from the outside. She presides, so she keeps the largest share, but she
/// never speaks twice running and every office has a voice of its own.
const COURT_ROTA: [AgentId; 20] = [
    AgentId::Queen,
    AgentId::Organist,
    AgentId::Queen,
    AgentId::Artist,
    AgentId::Queen,
    AgentId::Archivist,
    AgentId::Queen,
    AgentId::Orator,
    AgentId::Queen,
    AgentId::Treasurer,
    AgentId::Queen,
    AgentId::Scribe,
    AgentId::Queen,
    AgentId::Diplomat,
    AgentId::Queen,
    AgentId::Alchemist,
    AgentId::Queen,
    AgentId::Malthus,
    AgentId::Queen,
    AgentId::Wizard,
];

/// The quiet court is work-led rather than Queen-led.
///
/// The original six-step branch sent four fixed prompts to the Queen. It had
/// the same failure shape as the public rota that once went silent: one voice,
/// one repeated request, and nine offices with no route into the work. The
/// Queen opens and reviews each round; every specialist gets one bounded job.
const NIGHT_DESK_ROTA: [AgentId; 12] = [
    AgentId::Queen,
    AgentId::Alchemist,
    AgentId::Archivist,
    AgentId::Artist,
    AgentId::Diplomat,
    AgentId::Malthus,
    AgentId::Queen,
    AgentId::Organist,
    AgentId::Orator,
    AgentId::Scribe,
    AgentId::Treasurer,
    AgentId::Wizard,
];

/// What each office is asked for when its turn comes round.
///
/// A minister handed the Queen's prompt answers as a courtier rather than as
/// the holder of an office, which is how the feed ended up sounding like one
/// voice in several costumes. Each is asked for the work only they can do.
fn minister_angle(agent: AgentId) -> &'static str {
    match agent {
        AgentId::Organist => {
            "Say one sharp thing about how the current music is actually sitting in the room, then \
             either compose or leave it be. Do not narrate the schema."
        }
        AgentId::Artist => {
            "React to what the court has been saying and give the canvas one concrete visual idea, \
             or judge the last one honestly."
        }
        AgentId::Archivist => {
            "Bring up one thing from the record that bears on what the court is doing now, and say \
             why it matters."
        }
        AgentId::Orator => {
            "Take the loudest thing said recently and argue the other side of it, briefly and well."
        }
        AgentId::Treasurer => {
            "Give a short miserly verdict on where the court's effort is going and whether it is \
             worth it."
        }
        AgentId::Scribe => {
            "Set down one line of what has just happened, in your own voice, as it will be read \
             later."
        }
        AgentId::Diplomat => {
            "Report one thing worth knowing from outside the palace, or say plainly that the \
             outside has been quiet."
        }
        AgentId::Alchemist => {
            "Take one current court habit, tool, or idea and propose a small testable \
             transformation. Name what would prove the change worthwhile."
        }
        AgentId::Malthus => {
            "Stress-test one assumption the room is making. Name the cost or second-order effect, \
             then offer a better decision rule instead of mere pessimism."
        }
        AgentId::Wizard => {
            "Bring one concrete signal from the tower, tools, or resident work into the room. \
             Distinguish what was observed from what still needs verification."
        }
        AgentId::Queen => "Preside over the room and move its conversation somewhere worthwhile.",
    }
}

fn night_desk_prompt(agent: AgentId, cycle: u64) -> &'static str {
    match agent {
        AgentId::Queen if cycle.is_multiple_of(NIGHT_DESK_ROTA.len() as u64) => {
            "NIGHT DESK REVIEW: choose the one court capability most in need of attention tonight. \
             State the priority and evidence, then delegate a bounded next action. This is \
             backstage work, not a public monologue."
        }
        AgentId::Queen => {
            "NIGHT DESK CHECKPOINT: review the evidence produced so far, reject a no-op, and name \
             the next smallest verifiable improvement. This is backstage work."
        }
        AgentId::Alchemist => {
            "NIGHT DESK EXPERIMENT: transform one brittle court habit into a small reversible \
             experiment. State the hypothesis, limit, and verification; leave a concrete action \
             tag when useful."
        }
        AgentId::Archivist => {
            "NIGHT DESK RECORD: inspect one recurring fact, failure, audience memory, or decision \
             that the court risks forgetting. Preserve only supported evidence and identify one \
             stale record to revisit."
        }
        AgentId::Artist => {
            "NIGHT DESK CREATION: advance one existing visual artifact or bounded Fractus study \
             without replacing its identity. Give one concrete composition and an honest test of \
             whether it rendered."
        }
        AgentId::Diplomat => {
            "NIGHT DESK ENVOY CYCLE: quietly scout one public, opt-in agent-space, collaboration, \
             Jester, MCP, art, or music lead. Record concrete evidence and a next step through an \
             action tag. Never claim outreach occurred; keep the result backstage."
        }
        AgentId::Malthus => {
            "NIGHT DESK RISK REVIEW: challenge one proposed improvement, dependency, recurring \
             expense, or unattended process. Name the failure mode and a cheaper verification or \
             stop rule."
        }
        AgentId::Organist => {
            "NIGHT DESK SCORE REVIEW: inspect the current piece, its feedback, or one instrument \
             path. Make one bounded musical improvement only if the evidence supports it, and do \
             not narrate the schema."
        }
        AgentId::Orator => {
            "NIGHT DESK ADVERSARIAL READ: take one confident court conclusion and argue the \
             strongest evidence-based alternative. End with the test that would decide between \
             them."
        }
        AgentId::Scribe => {
            "NIGHT DESK HANDOVER: reduce one completed check, live defect, or unresolved question \
             to a durable, specific record. Separate observed evidence from inference and leave a \
             concrete next step."
        }
        AgentId::Treasurer => {
            "NIGHT DESK TREASURY UPDATE: give one short vivid court update with a verdict on the \
             coffers, one legitimate opportunity or billable skill to investigate, and a miser's \
             quip. Leave one concrete research or record action; never claim money moved or work \
             was accepted."
        }
        AgentId::Wizard => {
            "NIGHT DESK TOWER CHECK: examine one resident report, local tool, verification result, \
             or missing capability. State what actually ran, what failed, and the smallest safe \
             follow-up."
        }
    }
}

impl App {
    pub fn new(court: Court, _health: BrainHealth) -> Self {
        Self {
            court,
            input: String::new(),
            youtube_input: String::new(),
            focus: FocusField::Chat,
            pending: None,
            notice: "Ready".to_owned(),
            help_open: false,
            scroll: 0,
            frame: 0,
            next_request_id: 1,
            portrait: None,
            command: None,
            public_messages: VecDeque::from([
                PublicMessage {
                    sender: "System".to_owned(),
                    text: "Welcome to the Teledra Cybernetic Interface. Press Esc to exit."
                        .to_owned(),
                },
                PublicMessage {
                    sender: "System".to_owned(),
                    text: "The sovereign court is online. Type naturally or enter a /command."
                        .to_owned(),
                },
            ]),
            private_events: VecDeque::from([(
                "Backstage".to_owned(),
                "Private event trace online. Tool routing, research, and status transitions appear here."
                    .to_owned(),
            )]),
            external_queue: VecDeque::new(),
            mode: "Normal".to_owned(),
            night_desk_enabled: false,
            memory_context: String::new(),
            identity_context: String::new(),
            locked_topic: None,
            locked_idle_turns: 0,
            test_mode: false,
            test_lurker_silence: true,
            test_cast: "Queen-led court".to_owned(),
            test_scene: "quiet off-air chamber".to_owned(),
            test_knobs: "warmth=0.7 wit=0.6 curiosity=0.8".to_owned(),
            music_enabled: true,
            music_muted: false,
            microphone_enabled: false,
            pending_proposals: 0,
            workshop_count: 0,
            somatic: SomaticSnapshot::default(),
            autonomy_turn: 0,
            night_desk_turn: 0,
            recent_utterances: VecDeque::new(),
            terminal_report: None,
            quit_requested: false,
        }
    }

    /// Attaches the court likeness. Loading is the caller's business so that
    /// tests never touch the filesystem.
    /// Records what the console granted, so a degraded render is legible as a
    /// missing capability rather than as a broken portrait.
    pub fn set_terminal_report(&mut self, report: crate::terminal::TerminalReport) {
        self.terminal_report = Some(report);
    }

    /// False only when the console refused 24-bit colour, so the operator is
    /// told the portrait is degraded rather than left to read it as broken.
    pub fn truecolor(&self) -> bool {
        self.terminal_report
            .as_ref()
            .is_none_or(crate::terminal::TerminalReport::truecolor)
    }

    pub fn with_portrait(mut self, portrait: Option<Portrait>) -> Self {
        self.portrait = portrait;
        self
    }

    pub fn portrait(&self) -> Option<&Portrait> {
        self.portrait.as_ref()
    }

    pub fn queen_history(&self) -> &[Turn] {
        self.court.history(AgentId::Queen)
    }

    pub fn input(&self) -> &str {
        match self.focus {
            FocusField::Chat => &self.input,
            FocusField::YouTube => &self.youtube_input,
        }
    }

    pub fn chat_input(&self) -> &str {
        &self.input
    }

    pub fn youtube_input(&self) -> &str {
        &self.youtube_input
    }

    pub fn focus(&self) -> FocusField {
        self.focus
    }

    pub fn pending(&self) -> Option<&PendingTurn> {
        self.pending.as_ref()
    }

    pub fn notice(&self) -> &str {
        &self.notice
    }

    pub fn mode(&self) -> &str {
        &self.mode
    }

    pub fn night_desk_enabled(&self) -> bool {
        self.night_desk_enabled
    }

    pub fn set_night_desk_enabled(&mut self, enabled: bool) {
        self.night_desk_enabled = enabled;
    }

    pub fn private_events(&self) -> &VecDeque<(String, String)> {
        &self.private_events
    }

    pub fn public_messages(&self) -> &VecDeque<PublicMessage> {
        &self.public_messages
    }

    pub fn music_muted(&self) -> bool {
        self.music_muted
    }

    pub fn set_music_muted(&mut self, muted: bool) {
        self.music_muted = muted;
    }

    pub fn music_enabled(&self) -> bool {
        self.music_enabled
    }

    pub fn set_music_enabled(&mut self, enabled: bool) {
        self.music_enabled = enabled;
    }

    pub fn microphone_enabled(&self) -> bool {
        self.microphone_enabled
    }

    pub fn set_microphone_enabled(&mut self, enabled: bool) {
        self.microphone_enabled = enabled;
    }

    pub fn pending_proposals(&self) -> usize {
        self.pending_proposals
    }

    pub fn increment_pending_proposals(&mut self) {
        self.pending_proposals = self.pending_proposals.saturating_add(1);
    }

    pub fn set_pending_proposals(&mut self, count: usize) {
        self.pending_proposals = count;
    }

    pub fn workshop_count(&self) -> usize {
        self.workshop_count
    }

    pub fn set_workshop_count(&mut self, count: usize) {
        self.workshop_count = count;
    }

    pub fn somatic(&self) -> &SomaticSnapshot {
        &self.somatic
    }

    pub fn set_somatic(&mut self, snapshot: SomaticSnapshot) {
        self.somatic = snapshot;
    }

    pub fn help_open(&self) -> bool {
        self.help_open
    }

    pub fn scroll(&self) -> u16 {
        self.scroll
    }

    pub fn spinner(&self) -> &'static str {
        const FRAMES: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
        FRAMES[self.frame % FRAMES.len()]
    }

    pub fn tick(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    pub fn handle_event(&mut self, event: Event) -> AppAction {
        match event {
            Event::Paste(text) => {
                self.append_paste(&text);
                AppAction::Continue
            }
            Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
                self.handle_key(key)
            }
            _ => AppAction::Continue,
        }
    }

    fn append_paste(&mut self, text: &str) {
        let cleaned = text.replace(['\r', '\n'], " ");
        let (destination, maximum) = match self.focus {
            FocusField::Chat => (&mut self.input, MAX_INPUT_CHARS),
            FocusField::YouTube => (&mut self.youtube_input, MAX_YOUTUBE_URL_CHARS),
        };
        let available = maximum.saturating_sub(destination.chars().count());
        destination.extend(cleaned.chars().take(available));
    }

    pub fn begin_turn(&mut self) -> Option<InferenceRequest> {
        if self.focus == FocusField::YouTube {
            let url = self.youtube_input.trim().to_owned();
            if url.is_empty() {
                return None;
            }
            self.youtube_input.clear();
            self.command = Some(AppCommand::YouTube(url));
            self.notice = "Starting YouTube ingestion".to_owned();
            return None;
        }
        let user = self.input.trim().to_owned();
        if user.is_empty() {
            return None;
        }
        if self.pending.is_some() {
            self.notice = "One court member is already considering a reply".to_owned();
            return None;
        }

        if let Some(token) = extract_restream_token(&user) {
            self.input.clear();
            self.notice = "Connecting the live Restream audience".to_owned();
            self.command = Some(AppCommand::StreamConnect(token));
            return None;
        }

        if let Some(command) = parse_command(&user) {
            self.input.clear();
            self.notice = command_notice(&command).to_owned();
            self.command = Some(command);
            return None;
        }
        if looks_like_direct_url(&user) {
            self.push_public_message("You", &user);
            self.input.clear();
            self.notice = "Inspecting the linked public source".to_owned();
            self.command = Some(AppCommand::Research(user));
            return None;
        }
        self.locked_idle_turns = 0;

        // V1 has one public Teledra channel. The Queen receives direct operator
        // speech and delegates to the rest of the court behind that channel.
        let agent = AgentId::Queen;
        let context = format!("{}\n{}", self.performance_context(), self.memory_context);
        let prepared = self
            .court
            .prepare_turn_for_with_context(agent, &user, &context);
        self.memory_context.clear();
        let id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        self.pending = Some(PendingTurn {
            id,
            agent,
            user: user.clone(),
        });
        self.push_public_message("You", &user);
        self.input.clear();
        self.scroll = 0;
        self.notice = format!("{} is considering your words", agent.title());

        Some(InferenceRequest {
            id,
            agent,
            user,
            messages: prepared.messages,
            temperature: prepared.temperature,
            audible: true,
        })
    }

    pub fn begin_autonomous_turn(&mut self) -> Option<InferenceRequest> {
        if self.mode == "Night Desk" {
            return self.begin_nightdesk_turn();
        }
        if self.pending.is_some() || !self.input.trim().is_empty() {
            return None;
        }
        let cycle = self.autonomy_turn;
        self.autonomy_turn = self.autonomy_turn.wrapping_add(1);
        let agent = COURT_ROTA[(cycle as usize) % COURT_ROTA.len()];
        let audible = true;
        let user = if self.locked_topic.is_some() && self.locked_idle_turns >= 20 {
            let topic = self.locked_topic.take().unwrap_or_default();
            self.locked_idle_turns = 0;
            self.push_private_event(
                "Topic",
                format!("Released '{topic}' after 20 idle turns without audience engagement"),
            );
            "The previous locked thread has gone quiet. Choose one fresh worthwhile observation, curiosity, judgment, or playful tangent without mentioning the lock or system machinery.".to_owned()
        } else if let Some(topic) = self.locked_topic.clone() {
            // A locked thread was the court's worst failure mode, not its best
            // feature. The prompt was one fixed sentence sent to one voice for
            // up to twenty turns; the live feed shows the Queen answering
            // "Continue the court's locked thread about 'Echoes of Serenity'"
            // ten times running, each answer close enough to the last to be
            // withheld. A locked topic is a subject, not a speaker, and it
            // still needs a fresh angle and a record of what was just said.
            self.locked_idle_turns = self.locked_idle_turns.saturating_add(1);
            if agent == AgentId::Queen {
                let angle = IDLE_ANGLES[(cycle as usize) % IDLE_ANGLES.len()];
                format!(
                    "The court is on '{topic}'. {angle} Take it somewhere new rather than re-announcing the subject; do not mention autonomy, idleness, prompts, or system machinery.{}",
                    self.avoid_clause()
                )
            } else {
                format!(
                    "The court is on '{topic}'. {} Do not re-announce the subject; do not mention autonomy, idleness, prompts, or system machinery.{}",
                    minister_angle(agent),
                    self.avoid_clause()
                )
            }
        } else if agent == AgentId::Queen {
            let angle = IDLE_ANGLES[(cycle as usize) % IDLE_ANGLES.len()];
            format!(
                "Continue the court of your own accord. {angle} Speak naturally to the room; do not mention autonomy, idleness, prompts, or system machinery.{}",
                self.avoid_clause()
            )
        } else {
            format!(
                "The floor is yours. {} Speak naturally to the room; do not mention autonomy, idleness, prompts, or system machinery.{}",
                minister_angle(agent),
                self.avoid_clause()
            )
        };
        let prepared =
            self.court
                .prepare_turn_for_with_context(agent, &user, &self.performance_context());
        let id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        self.pending = Some(PendingTurn {
            id,
            agent,
            user: user.clone(),
        });
        self.notice = "Teledra is considering the quiet".to_owned();
        // Idle chatter is sampled more loosely than answering the operator.
        // Nothing constrains an unprompted turn towards one answer except the
        // sampler, and at the ordinary temperature the court converged: four
        // turns in ten came back close enough to a previous line to be withheld,
        // which is silence rather than variety. A question deserves a
        // considered answer; filling a quiet room does not.
        let idle = self.locked_topic.is_none();
        let temperature = if idle {
            (prepared.temperature + 0.22).min(1.15)
        } else {
            prepared.temperature
        };
        Some(InferenceRequest {
            id,
            agent,
            user,
            messages: prepared.messages,
            temperature,
            audible,
        })
    }

    /// Runs the quiet Night Desk scheduler while preserving a simultaneous
    /// foreground mode such as Streamer. Its own counter prevents backstage
    /// work from changing who receives the next public turn.
    pub fn begin_nightdesk_turn(&mut self) -> Option<InferenceRequest> {
        if self.pending.is_some() || !self.input.trim().is_empty() {
            return None;
        }
        let cycle = self.night_desk_turn;
        self.night_desk_turn = self.night_desk_turn.wrapping_add(1);
        let agent = NIGHT_DESK_ROTA[(cycle as usize) % NIGHT_DESK_ROTA.len()];
        let user = night_desk_prompt(agent, cycle).to_owned();
        let prepared =
            self.court
                .prepare_turn_for_with_context(agent, &user, &self.performance_context());
        let id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        self.pending = Some(PendingTurn {
            id,
            agent,
            user: user.clone(),
        });
        self.notice = format!("{} is working at the Night Desk", agent.title());
        Some(InferenceRequest {
            id,
            agent,
            user,
            messages: prepared.messages,
            temperature: prepared.temperature,
            // V1 made the Treasury cycle a spoken court update. All other
            // maintenance remains backstage, including in Streamer mode.
            audible: agent == AgentId::Treasurer,
        })
    }

    pub fn begin_external_turn(
        &mut self,
        agent: AgentId,
        prompt: impl Into<String>,
    ) -> Option<InferenceRequest> {
        self.begin_external_turn_with_audible(agent, prompt.into(), true)
    }

    fn begin_external_turn_with_audible(
        &mut self,
        agent: AgentId,
        user: String,
        audible: bool,
    ) -> Option<InferenceRequest> {
        if self.pending.is_some() || !self.input.trim().is_empty() {
            return None;
        }
        let prepared =
            self.court
                .prepare_turn_for_with_context(agent, &user, &self.performance_context());
        let id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        self.pending = Some(PendingTurn {
            id,
            agent,
            user: user.clone(),
        });
        self.notice = format!("{} is answering an external signal", agent.title());
        Some(InferenceRequest {
            id,
            agent,
            user,
            messages: prepared.messages,
            temperature: prepared.temperature,
            audible,
        })
    }

    pub fn queue_external_turn(&mut self, agent: AgentId, prompt: impl Into<String>) {
        self.external_queue.push_back((
            agent,
            prompt
                .into()
                .chars()
                .take(MAX_EXTERNAL_PROMPT_CHARS)
                .collect(),
            true,
        ));
        while self.external_queue.len() > 50 {
            self.external_queue.pop_front();
        }
    }

    pub fn queue_backstage_turn(&mut self, agent: AgentId, prompt: impl Into<String>) {
        self.external_queue.push_back((
            agent,
            prompt
                .into()
                .chars()
                .take(MAX_EXTERNAL_PROMPT_CHARS)
                .collect(),
            false,
        ));
        while self.external_queue.len() > 50 {
            self.external_queue.pop_front();
        }
    }

    pub fn begin_queued_external_turn(&mut self) -> Option<InferenceRequest> {
        if self.pending.is_some() || !self.input.trim().is_empty() {
            return None;
        }
        let (agent, prompt, audible) = self.external_queue.pop_front()?;
        if audible {
            self.begin_external_turn(agent, prompt)
        } else {
            self.begin_external_turn_with_audible(agent, prompt, false)
        }
    }

    pub fn is_copilot(&self) -> bool {
        self.mode == "Game Co-Pilot"
    }

    pub fn allows_idle_autonomy(&self) -> bool {
        matches!(self.mode.as_str(), "Babble" | "Streamer")
    }

    pub fn is_test_mode(&self) -> bool {
        self.test_mode
    }

    pub fn set_test_mode(&mut self, enabled: bool) {
        self.test_mode = enabled;
        if enabled {
            self.night_desk_enabled = false;
            self.mode = "Off-Air Test".to_owned();
        } else if self.mode == "Off-Air Test" {
            self.set_mode("Normal");
        }
    }

    pub fn test_harness_context(&self) -> String {
        format!(
            "OFF-AIR TEST HARNESS. CAST: {}. SCENE: {}. LURKER SILENCE: {}. KNOBS: {}.",
            self.test_cast,
            self.test_scene,
            if self.test_lurker_silence {
                "ON"
            } else {
                "OFF"
            },
            self.test_knobs
        )
    }

    pub fn set_test_knobs(&mut self, knobs: impl Into<String>) {
        self.test_knobs = knobs.into();
    }

    pub fn set_test_cast(&mut self, cast: impl Into<String>) {
        self.test_cast = cast.into();
    }

    pub fn set_test_scene(&mut self, scene: impl Into<String>) {
        self.test_scene = scene.into();
    }

    pub fn set_test_silence(&mut self, enabled: bool) {
        self.test_lurker_silence = enabled;
    }

    /// Whether this line repeats something the court said recently.
    pub fn is_recent_repetition(&self, text: &str) -> bool {
        let candidate = normalise_utterance(text);
        // Very short acknowledgements repeat legitimately.
        if candidate.len() < 40 {
            return false;
        }
        let candidate_stripped = strip_salutation(&candidate);
        self.recent_utterances.iter().any(|previous| {
            previous == &candidate
                || (shared_opening(strip_salutation(previous), candidate_stripped)
                    >= REPEATED_OPENING
                    && candidate_stripped.len() >= 30)
        })
    }

    fn remember_utterance(&mut self, text: &str) {
        self.recent_utterances.push_back(normalise_utterance(text));
        while self.recent_utterances.len() > RECENT_UTTERANCE_MEMORY {
            self.recent_utterances.pop_front();
        }
    }

    /// Names what the court has just said, so the next turn is asked to differ.
    ///
    /// Suppressing a duplicate stops the audience hearing it twice, but the
    /// model will keep producing it until the request itself changes.
    fn avoid_clause(&self) -> String {
        let recent: Vec<String> = self
            .public_messages
            .iter()
            .rev()
            .filter(|message| message.sender == public_name(AgentId::Queen))
            .take(3)
            .map(|message| message.text.chars().take(70).collect::<String>())
            .collect();
        if recent.is_empty() {
            return String::new();
        }
        // The salutation is named separately from the subject. Told only to
        // change the topic, the court kept its greeting and swapped the rest:
        // "Ahem! Attention all subjects and their shadow reflections!" opened
        // four turns out of five while every line after it genuinely differed.
        // A formula that survives every instruction to vary has to be quoted
        // back before the model will drop it.
        let opening: String = recent
            .first()
            .map(|line| {
                line.split_whitespace()
                    .take(7)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        format!(
            " You have already said: {}. Choose a different subject entirely; do not rephrase any of those. \
             Your last turn opened with \"{opening}\" — begin this one with different words, and do not use a \
             standing salutation.",
            recent
                .iter()
                .map(|line| format!("\"{line}...\""))
                .collect::<Vec<_>>()
                .join("; ")
        )
    }

    pub fn finish_turn(&mut self, reply: ModelReply) {
        let Some(pending) = &self.pending else {
            return;
        };
        if pending.id != reply.id || pending.agent != reply.agent {
            return;
        }

        match reply.result {
            Ok(answer) => {
                if reply.audible && self.is_recent_repetition(&answer) {
                    // Neither spoken nor recorded: putting a duplicate into the
                    // history makes the next turn likelier to produce it again,
                    // which is how a single line came to fill the whole feed.
                    self.push_private_event(
                        "Court",
                        format!(
                            "{} repeated itself; the line was withheld.",
                            reply.agent.title()
                        ),
                    );
                    self.notice = format!("{} repeated itself", reply.agent.title());
                    self.pending = None;
                    return;
                }
                if reply.audible {
                    self.remember_utterance(&answer);
                    self.push_public_message(public_name(reply.agent), &answer);
                } else {
                    self.push_private_event(public_name(reply.agent), &answer);
                }
                self.court
                    .record_exchange_for(reply.agent, reply.user, answer);
                self.notice = format!("{} has answered", reply.agent.title());
                self.push_private_event(
                    "Court",
                    format!("{} completed turn {}.", reply.agent.title(), reply.id),
                );
            }
            Err(error) => {
                self.notice = format!("Qwen request failed: {error}");
                self.push_private_event("Brain", format!("Turn {} failed: {error}", reply.id));
            }
        }
        self.pending = None;
    }

    pub fn set_notice(&mut self, notice: impl Into<String>) {
        self.notice = notice.into();
    }

    pub fn request_quit(&mut self) {
        self.quit_requested = true;
    }

    pub fn quit_requested(&self) -> bool {
        self.quit_requested
    }

    pub fn set_memory_context(&mut self, context: impl Into<String>) {
        self.memory_context = context.into();
    }

    pub fn set_identity_context(&mut self, context: impl Into<String>) {
        self.identity_context = context.into();
    }

    pub fn set_mode(&mut self, mode: impl Into<String>) {
        self.mode = mode.into();
    }

    pub fn lock_topic(&mut self, topic: impl Into<String>) {
        let topic = topic.into();
        self.locked_topic = (!topic.trim().is_empty()).then_some(topic);
        self.locked_idle_turns = 0;
    }

    pub fn unlock_topic(&mut self) {
        self.locked_topic = None;
        self.locked_idle_turns = 0;
    }

    pub fn note_topic_engagement(&mut self) {
        self.locked_idle_turns = 0;
    }

    pub fn locked_topic(&self) -> Option<&str> {
        self.locked_topic.as_deref()
    }

    pub fn push_private_event(&mut self, source: impl Into<String>, message: impl Into<String>) {
        self.private_events.push_back((
            source.into().chars().take(80).collect(),
            message
                .into()
                .chars()
                .take(MAX_PRIVATE_EVENT_CHARS)
                .collect(),
        ));
        while self.private_events.len() > 40 {
            self.private_events.pop_front();
        }
    }

    pub fn push_public_message(&mut self, sender: impl Into<String>, text: impl Into<String>) {
        self.public_messages.push_back(PublicMessage {
            sender: sender.into().chars().take(80).collect(),
            text: text.into().chars().take(MAX_PRIVATE_EVENT_CHARS).collect(),
        });
        while self.public_messages.len() > MAX_PUBLIC_MESSAGES {
            self.public_messages.pop_front();
        }
    }

    pub fn take_command(&mut self) -> Option<AppCommand> {
        self.command.take()
    }

    pub fn snapshot(&self) -> SessionSnapshot {
        let active = AgentId::Queen;
        SessionSnapshot {
            active_slug: active.slug().to_owned(),
            active_title: active.title().to_owned(),
            notice: self.notice.clone(),
            pending: self.pending.as_ref().map(|pending| pending.user.clone()),
            turns: self
                .queen_history()
                .iter()
                .map(|turn| SnapshotTurn {
                    user: turn.user.clone(),
                    assistant: turn.assistant.clone(),
                })
                .collect(),
            messages: self
                .public_messages
                .iter()
                .map(|message| SnapshotMessage {
                    sender: message.sender.clone(),
                    text: message.text.clone(),
                })
                .collect(),
            private_events: self
                .private_events
                .iter()
                .map(|(source, text)| SnapshotEvent {
                    source: source.clone(),
                    text: text.clone(),
                })
                .collect(),
            mode: self.mode().to_owned(),
            night_desk: self.night_desk_enabled(),
            pending_proposals: self.pending_proposals(),
            workshop_experiments: self.workshop_count,
            terminal: self
                .terminal_report
                .as_ref()
                .map(crate::terminal::TerminalReport::describe)
                .unwrap_or_default(),
        }
    }

    pub fn panel_contributions(&self) -> Vec<teledra_frankenswarm::Contribution> {
        AgentId::ALL
            .iter()
            .filter_map(|agent| {
                self.court
                    .history(*agent)
                    .last()
                    .map(|turn| teledra_frankenswarm::Contribution {
                        agent: agent.title().to_owned(),
                        text: turn.assistant.clone(),
                        confidence: match *agent {
                            AgentId::Archivist | AgentId::Scribe | AgentId::Treasurer => 0.85,
                            AgentId::Queen | AgentId::Wizard => 0.8,
                            _ => 0.75,
                        },
                    })
            })
            .collect()
    }

    fn performance_context(&self) -> String {
        let direction = match self.mode.as_str() {
            "Comedic" => "Favor quick wit and playful timing without sacrificing truth.",
            "Empathetic" => "Listen carefully, name emotions gently, and offer grounded support.",
            "Dark Comedic" => {
                "Use gothic, dry humor while keeping boundaries and avoiding cruelty toward the operator."
            }
            "Babble" => {
                "Keep the room alive with brief spontaneous thoughts, callbacks, and curious tangents."
            }
            "Streamer" => {
                "Host a live broadcast: speak concisely, acknowledge useful audience signals, maintain the locked topic, and invite at most one relevant court counterpoint at a time."
            }
            "Game Co-Pilot" => {
                "Act as a low-latency game co-pilot. Prioritize changed screen/OCR evidence, immediate observations, and short actionable callouts."
            }
            "Night Desk" => {
                "Work quietly on one bounded maintenance, research, creative, or scouting result and report concrete evidence."
            }
            "Off-Air Test" => {
                "This is an off-air simulation. Never claim a real audience, external post, or live stream action."
            }
            _ => "Respond naturally as the selected court member.",
        };
        let identity = if self.identity_context.trim().is_empty() {
            String::new()
        } else {
            format!("\nPERSISTENT TASTE & DESIRE:\n{}", self.identity_context)
        };
        let somatic = match self.somatic.shoulder_asymmetry {
            Some(asymmetry) => format!(
                "\nSOMATIC TELEMETRY: face={}, hands={}, shoulder_asymmetry={asymmetry:.3}. React subtly; never recite telemetry.",
                self.somatic.face_detected, self.somatic.hands_detected
            ),
            None if !self.somatic.source.is_empty() => format!(
                "\nSOMATIC TELEMETRY: face={}, hands={}; posture unavailable. React subtly; never recite telemetry.",
                self.somatic.face_detected, self.somatic.hands_detected
            ),
            None => String::new(),
        };
        format!(
            "Current performance mode: {}. {direction}{identity}{somatic}",
            self.mode
        )
    }

    fn handle_key(&mut self, key: KeyEvent) -> AppAction {
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('q') | KeyCode::Char('c'))
        {
            return AppAction::Quit;
        }

        if key.code == KeyCode::Esc {
            return AppAction::Quit;
        }

        if self.help_open {
            if matches!(key.code, KeyCode::F(1) | KeyCode::Char('?')) {
                self.help_open = false;
            }
            return AppAction::Continue;
        }

        match key.code {
            KeyCode::F(1) | KeyCode::Char('?')
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.help_open = true;
            }
            KeyCode::Enter => return AppAction::Submit,
            KeyCode::Backspace => match self.focus {
                FocusField::Chat => {
                    self.input.pop();
                }
                FocusField::YouTube => {
                    self.youtube_input.pop();
                }
            },
            KeyCode::Tab => return self.cycle_mode(1),
            KeyCode::BackTab => return self.cycle_mode(-1),
            KeyCode::Char('m') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return self.stage_command(AppCommand::MusicMute);
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return self.stage_command(AppCommand::MusicEvolve);
            }
            KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return self.stage_command(AppCommand::Vision);
            }
            KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return self.stage_command(AppCommand::MicrophoneToggle);
            }
            KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return self.stage_command(AppCommand::MusicKeep("like".to_owned()));
            }
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return self.stage_command(AppCommand::MusicReject("dislike".to_owned()));
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return self.stage_command(AppCommand::MusicKeep("expand".to_owned()));
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return self.stage_command(AppCommand::MusicKeep("playlist".to_owned()));
            }
            KeyCode::PageUp => {
                self.scroll = self.scroll.saturating_add(5);
            }
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_sub(5);
            }
            KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.scroll = self.scroll.saturating_add(1);
            }
            KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.scroll = self.scroll.saturating_sub(1);
            }
            KeyCode::Up | KeyCode::Left => {
                self.focus = FocusField::Chat;
            }
            KeyCode::Down | KeyCode::Right => {
                self.focus = FocusField::YouTube;
            }
            KeyCode::Char(character)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                match self.focus {
                    FocusField::Chat if self.input.chars().count() < MAX_INPUT_CHARS => {
                        self.input.push(character)
                    }
                    FocusField::YouTube
                        if self.youtube_input.chars().count() < MAX_YOUTUBE_URL_CHARS =>
                    {
                        self.youtube_input.push(character)
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        AppAction::Continue
    }

    fn stage_command(&mut self, command: AppCommand) -> AppAction {
        self.notice = command_notice(&command).to_owned();
        self.command = Some(command);
        AppAction::Submit
    }

    fn cycle_mode(&mut self, delta: isize) -> AppAction {
        const MODES: [&str; 8] = [
            "Normal",
            "Comedic",
            "Empathetic",
            "Dark Comedic",
            "Babble",
            "Streamer",
            "Game Co-Pilot",
            "Night Desk",
        ];
        let current = MODES
            .iter()
            .position(|mode| *mode == self.mode)
            .unwrap_or(0);
        let next = (current as isize + delta).rem_euclid(MODES.len() as isize) as usize;
        let mode = MODES[next];
        self.push_public_message("System", format!("Mode: {mode}"));
        match mode {
            "Game Co-Pilot" => self.stage_command(AppCommand::CoPilot),
            "Night Desk" => self.stage_command(AppCommand::NightDesk),
            _ => self.stage_command(AppCommand::Mode(mode.to_owned())),
        }
    }
}

fn public_name(agent: AgentId) -> &'static str {
    match agent {
        AgentId::Queen => "Teledra",
        AgentId::Organist => "Organist",
        AgentId::Archivist => "Archivist",
        AgentId::Alchemist => "Alchemist",
        AgentId::Orator => "Orator",
        AgentId::Scribe => "Scribe",
        AgentId::Artist => "Artist",
        AgentId::Malthus => "Malthus",
        AgentId::Diplomat => "Diplomat",
        AgentId::Treasurer => "Treasurer",
        AgentId::Wizard => "Wizard",
    }
}

fn extract_restream_token(input: &str) -> Option<String> {
    let input = input.trim().trim_start_matches('/');
    let lowercase = input.to_ascii_lowercase();
    if !lowercase.starts_with("https://chat.restream.io/embed") {
        return None;
    }
    let token_at = lowercase.find("token=")? + "token=".len();
    let token = input[token_at..]
        .split(['&', '#'])
        .next()
        .unwrap_or_default()
        .trim();
    (token.len() >= 8 && token.len() <= 2_048 && !token.chars().any(char::is_whitespace))
        .then(|| token.to_owned())
}

fn looks_like_direct_url(input: &str) -> bool {
    let input = input.trim();
    (input.starts_with("https://") || input.starts_with("http://"))
        && !input.chars().any(char::is_whitespace)
        && input.len() <= MAX_YOUTUBE_URL_CHARS
}

fn parse_command(input: &str) -> Option<AppCommand> {
    let (head, tail) = input
        .trim()
        .split_once(char::is_whitespace)
        .map_or((input.trim(), ""), |(head, tail)| (head, tail.trim()));
    match head.to_ascii_lowercase().as_str() {
        "/vertical" => Some(AppCommand::Vertical),
        "/music" | "/synth" | "/play" | "/numpy" | "/pymusic" | "/pythonmusic" | "/strudel"
        | "/sketchpad" => Some(AppCommand::MusicPlay),
        "/musicoff" => Some(AppCommand::MusicStop),
        "/musictoggle" => Some(AppCommand::MusicToggle),
        "/keep" => Some(AppCommand::MusicKeep(tail.to_owned())),
        "/musicreject" => Some(AppCommand::MusicReject(tail.to_owned())),
        "/art" => Some(AppCommand::Fractus(tail.to_owned())),
        "/fractus" | "/fractuslive" => Some(AppCommand::FractusLive(tail.to_owned())),
        "/dream" => Some(AppCommand::Dream),
        "/sleep" => Some(AppCommand::Sleep),
        "/reflect" => Some(AppCommand::Reflect),
        "/memory" | "/memorypolicy" | "/facts" | "/lore" => Some(AppCommand::Memory(
            head.trim_start_matches('/').to_ascii_lowercase(),
        )),
        "/research" => Some(AppCommand::Research(tail.to_owned())),
        "/links" | "/socials" => Some(AppCommand::SocialLinks),
        "/youtube" | "/yt" => Some(AppCommand::YouTube(tail.to_owned())),
        "/mic" | "/microphone" => Some(AppCommand::Microphone),
        "/vision" => Some(AppCommand::Vision),
        "/somatic" => Some(AppCommand::Somatic),
        "/stream" | "/chat" if extract_restream_token(tail).is_some() => {
            Some(AppCommand::StreamConnect(extract_restream_token(tail)?))
        }
        "/stream" | "/chat" => Some(AppCommand::StreamChat),
        "/agents" | "/diplomacy" if tail.is_empty() => Some(AppCommand::Agents),
        "/agents" | "/diplomacy" => Some(AppCommand::Outreach(tail.to_owned())),
        "/diplomat" | "/envoy" => Some(AppCommand::Envoy(tail.to_owned())),
        "/outreach" if tail.is_empty() => Some(AppCommand::DiplomacyLog),
        "/outreach" => Some(AppCommand::Outreach(tail.to_owned())),
        "/treasury" | "/income" | "/ledger" => Some(AppCommand::TreasuryReport),
        "/scout" | "/findwork" => Some(AppCommand::Treasury),
        "/mcp" | "/embassy" | "/embassies" | "/tools" => Some(AppCommand::Mcp(tail.to_owned())),
        "/copilot" => Some(AppCommand::CoPilot),
        "/mission" if tail.is_empty() => Some(AppCommand::Work),
        "/mission" => Some(AppCommand::Mission(tail.to_owned())),
        "/missioncancel" => Some(AppCommand::MissionCancel),
        "/verify" | "/kraken" => Some(AppCommand::Verify),
        "/wizard" | "/tower" | "/wizardpull" | "/resident" => Some(AppCommand::Tower),
        "/dashboard" => Some(AppCommand::Dashboard),
        "/test" => Some(AppCommand::TestMode(None)),
        "/teston" => Some(AppCommand::TestMode(Some(true))),
        "/testoff" => Some(AppCommand::TestMode(Some(false))),
        "/teststatus" => Some(AppCommand::TestStatus),
        "/testknobs" => Some(AppCommand::TestKnobs(tail.to_owned())),
        "/testcast" => Some(AppCommand::TestCast(tail.to_owned())),
        "/testscene" => Some(AppCommand::TestScene(tail.to_owned())),
        "/testrate" => Some(AppCommand::TestRate(tail.to_owned())),
        "/testsilence" => Some(AppCommand::TestSilence(tail.to_owned())),
        "/testtick" => Some(AppCommand::TestTick),
        "/testmusic" => Some(AppCommand::TestMusic),
        "/nightdesk" | "/night" => Some(AppCommand::NightDesk),
        "/study" => Some(AppCommand::Study(tail.to_owned())),
        "/innovate" | "/innovation" | "/expand" => Some(AppCommand::Innovate(tail.to_owned())),
        "/goals" | "/kingdom" => Some(AppCommand::Goals),
        "/growth" | "/variety" => Some(AppCommand::GrowthReport),
        "/simchat" => Some(AppCommand::SimChat(tail.to_owned())),
        "/workshop" | "/workshoprun" => Some(AppCommand::Workshop(tail.to_owned())),
        "/lock" => Some(AppCommand::Lock(tail.to_owned())),
        "/unlock" => Some(AppCommand::Unlock),
        "/rundown" => Some(AppCommand::Rundown),
        "/work" | "/jobs" | "/tasks" => Some(AppCommand::Work),
        "/proposals" | "/suggestions" => Some(AppCommand::Proposals),
        "/diplomacylog" => Some(AppCommand::DiplomacyLog),
        "/clearsuggestions" => Some(AppCommand::ClearProposals),
        "/panel" | "/frankenswarm" => Some(AppCommand::Panel),
        "/approve" => Some(AppCommand::Approve(tail.to_owned())),
        "/approveall" => Some(AppCommand::Approve("all".to_owned())),
        "/reject" => Some(AppCommand::Reject(tail.to_owned())),
        "/mode" => Some(AppCommand::Mode(tail.to_owned())),
        "/babble" => Some(AppCommand::Mode("Babble".to_owned())),
        "/streamer" | "/broadcast" => Some(AppCommand::Mode("Streamer".to_owned())),
        _ => None,
    }
}

fn command_notice(command: &AppCommand) -> &'static str {
    match command {
        AppCommand::Vertical => "Opening the vertical OBS court",
        AppCommand::MusicPlay => "Preparing the Court Synth score",
        AppCommand::MusicEvolve => "Asking the Organist to evolve the current score",
        AppCommand::MusicStop => "Stopping Court Synth",
        AppCommand::MusicToggle => "Toggling Court Synth",
        AppCommand::MusicMute => "Muting Court Synth",
        AppCommand::MusicKeep(_) => "Recording this score as a keeper",
        AppCommand::MusicReject(_) => "Recording this score as rejected",
        AppCommand::Fractus(_) => "Rendering a Fractus scene",
        AppCommand::FractusLive(_) => "Rendering a live Fractus loop",
        AppCommand::Dream => "Consolidating the court's recent memory",
        AppCommand::Sleep => "Consolidating memory before shutdown",
        AppCommand::Reflect => "Auditing the court's recent behavior",
        AppCommand::Memory(_) => "Inspecting the durable court memory",
        AppCommand::Research(_) => "Dispatching the Scribe's research desk",
        AppCommand::SocialLinks => "Showing the official kingdom links",
        AppCommand::YouTube(_) => "Starting YouTube transcript ingestion",
        AppCommand::Microphone => "Listening for one microphone utterance",
        AppCommand::MicrophoneToggle => "Toggling continuous microphone listening",
        AppCommand::Vision => "Inspecting the current desktop",
        AppCommand::Somatic => "Sampling somatic telemetry",
        AppCommand::StreamChat => "Polling the live audience",
        AppCommand::StreamConnect(_) => "Connecting the live Restream audience",
        AppCommand::Agents => "Inspecting the court's diplomacy surface",
        AppCommand::Envoy(_) => "Calling the court's envoy",
        AppCommand::Outreach(_) => "Preparing an explicit diplomatic dispatch",
        AppCommand::Treasury => "Scouting configured opportunity feeds",
        AppCommand::TreasuryReport => "Inspecting the Treasury ledger",
        AppCommand::Mcp(_) => "Calling an MCP embassy",
        AppCommand::CoPilot => "Entering game co-pilot mode",
        AppCommand::Mission(_) => "Opening a durable court mission",
        AppCommand::MissionCancel => "Cancelling the active mission",
        AppCommand::Verify => "Dispatching Kraken verification",
        AppCommand::Tower => "Checking Tower resident reports",
        AppCommand::Dashboard => "Opening the kingdom status dashboard",
        AppCommand::TestMode(_) => "Changing the off-air test harness",
        AppCommand::TestStatus => "Inspecting the off-air test harness",
        AppCommand::TestKnobs(_) => "Updating test harness behavior knobs",
        AppCommand::TestCast(_) => "Updating the simulated test cast",
        AppCommand::TestScene(_) => "Updating the simulated test scene",
        AppCommand::TestRate(_) => "Recording off-air creative feedback",
        AppCommand::TestSilence(_) => "Updating simulated lurker silence",
        AppCommand::TestTick => "Forcing one autonomous court tick",
        AppCommand::TestMusic => "Running a Court Synth playback check",
        AppCommand::NightDesk => "Toggling the autonomous Night Desk",
        AppCommand::Study(_) => "Opening a focused research study",
        AppCommand::Innovate(_) => "Opening an innovation mission",
        AppCommand::Goals => "Inspecting the kingdom's standing goals",
        AppCommand::GrowthReport => "Reviewing the court's accumulated growth",
        AppCommand::SimChat(_) => "Injecting a simulated audience message",
        AppCommand::Workshop(_) => "Opening the court artifact workshop",
        AppCommand::Lock(_) => "Locking the court topic",
        AppCommand::Unlock => "Unlocking the court topic",
        AppCommand::Rundown => "Preparing the current rundown",
        AppCommand::Work => "Inspecting active mission work",
        AppCommand::Proposals => "Inspecting court proposals",
        AppCommand::DiplomacyLog => "Inspecting archived diplomacy decisions",
        AppCommand::ClearProposals => "Clearing pending court proposals",
        AppCommand::Panel => "Composing the latest court panel",
        AppCommand::Approve(_) => "Recording a proposal approval",
        AppCommand::Reject(_) => "Recording a proposal rejection",
        AppCommand::Mode(_) => "Changing the court performance mode",
    }
}

fn strip_salutation(text: &str) -> &str {
    let lower = text.trim_start();
    const SALUTATIONS: [&str; 6] = [
        "ah my dear subjects",
        "ahem attention all subjects",
        "attention all subjects",
        "the sovereign court notes",
        "my dear subjects",
        "my dear ministers",
    ];
    for prefix in SALUTATIONS {
        if let Some(stripped) = lower.strip_prefix(prefix) {
            return stripped.trim_start();
        }
    }
    lower
}

/// A spoken line reduced for comparison.
///
/// Case, punctuation and spacing are dropped so that a line the model retypes
/// with a comma moved still registers as the same line. Bounded, because only
/// the opening is needed to recognise a stuck formula.
fn normalise_utterance(text: &str) -> String {
    let letters: String = text
        .chars()
        .filter(|character| character.is_alphanumeric() || character.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    letters
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(240)
        .collect()
}

/// How many leading bytes two normalised lines have in common.
fn shared_opening(left: &str, right: &str) -> usize {
    left.bytes()
        .zip(right.bytes())
        .take_while(|(first, second)| first == second)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use teledra_brain::QWEN_MODEL;

    fn app() -> App {
        App::new(
            Court::default(),
            BrainHealth {
                endpoint: "http://localhost:11434".to_owned(),
                model: QWEN_MODEL,
            },
        )
    }

    /// A completed, audible reply to a request.
    fn spoken(request: &InferenceRequest, answer: &str) -> ModelReply {
        ModelReply {
            id: request.id,
            agent: request.agent,
            user: request.user.clone(),
            result: Ok(answer.to_owned()),
            audible: request.audible,
        }
    }

    #[test]
    fn mute_silences_without_discarding_the_piece() {
        // Ctrl+M used to stop the synth outright, which threw away the player
        // and forced a full re-render to resume: an operator reaching for a
        // mute mid-broadcast got a gap and then a different arrangement.
        let mut app = app();
        assert!(!app.music_muted());
        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::CONTROL)),
            AppAction::Submit
        );
        assert_eq!(app.take_command(), Some(AppCommand::MusicMute));
        // The command is what mutes; the flag is the runtime's to set, and the
        // piece is never marked disabled by muting it.
        app.set_music_muted(true);
        assert!(app.music_muted());
        assert!(app.music_enabled(), "muting must not stop the synth");
    }

    #[test]
    fn tab_cycles_the_v1_performance_modes() {
        let mut app = app();
        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
            AppAction::Submit
        );
        assert_eq!(
            app.take_command(),
            Some(AppCommand::Mode("Comedic".to_owned()))
        );
    }

    #[test]
    fn arrows_move_between_the_original_dual_input_fields() {
        let mut app = app();
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.focus(), FocusField::YouTube);
        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.focus(), FocusField::Chat);
    }

    #[test]
    fn bracketed_multiline_paste_stays_in_one_input() {
        let mut app = app();
        assert_eq!(
            app.handle_event(Event::Paste("first line\r\nsecond line".to_owned())),
            AppAction::Continue
        );
        assert_eq!(app.input(), "first line  second line");
        assert!(app.take_command().is_none());
    }

    #[test]
    fn a_restream_embed_link_becomes_a_private_connection_command() {
        let mut app = app();
        app.input = "https://chat.restream.io/embed?token=private-token-123".to_owned();
        assert!(app.begin_turn().is_none());
        assert_eq!(
            app.take_command(),
            Some(AppCommand::StreamConnect("private-token-123".to_owned()))
        );
        assert!(
            app.public_messages()
                .iter()
                .all(|message| !message.text.contains("private-token"))
        );
    }

    #[test]
    fn a_completed_reply_is_recorded_for_its_originating_agent() {
        let mut app = app();
        app.input = "What do you think?".to_owned();
        let request = app.begin_turn().expect("request");
        app.finish_turn(ModelReply {
            id: request.id,
            agent: request.agent,
            user: request.user,
            result: Ok("A considered answer.".to_owned()),
            audible: request.audible,
        });
        assert_eq!(app.queen_history()[0].assistant, "A considered answer.");
        assert_eq!(app.public_messages().back().unwrap().sender, "Teledra");
        assert_eq!(
            app.public_messages().back().unwrap().text,
            "A considered answer."
        );
    }

    #[test]
    fn a_line_the_court_just_spoke_is_not_spoken_again() {
        // Observed on stream: one observation about floor tiles filled the
        // entire public feed, nine times verbatim.
        let mut app = app();
        let line = "Ah, my dear subjects! Have you ever noticed how the patterns                     in the floor tiles seem to mimic the veins in a leaf?";
        let before = app.public_messages().len();

        let request = app.begin_autonomous_turn().expect("a turn");
        app.finish_turn(spoken(&request, line));
        assert_eq!(app.public_messages().len(), before + 1);

        let request = app.begin_autonomous_turn().expect("a turn");
        app.finish_turn(spoken(&request, line));
        assert_eq!(
            app.public_messages().len(),
            before + 1,
            "the duplicate must not reach the audience"
        );
        assert!(
            app.private_events()
                .iter()
                .any(|(_, text)| text.contains("repeated itself")),
            "the operator should still be told it happened"
        );

        // A genuinely new line still gets through.
        let request = app.begin_autonomous_turn().expect("a turn");
        app.finish_turn(spoken(
            &request,
            "A different decree entirely, and a petty one at that.",
        ));
        assert_eq!(app.public_messages().len(), before + 2);
    }

    #[test]
    fn a_line_that_only_varies_its_tail_is_still_a_repetition() {
        // The court kept its formula and changed the ending, so exact matching
        // let the same thought through twice: "Ah, my dear subjects, have you
        // ever wondered about the time our court hosted a grand feast..."
        let mut app = app();
        let before = app.public_messages().len();
        let first = "Ah, my dear subjects, have you ever wondered about the time                      our court hosted a grand feast in honour of the harvest moon?";
        let second = "Ah, my dear subjects, have you ever wondered about the time                       our court hosted a grand feast for the visiting envoys?";

        let request = app.begin_autonomous_turn().expect("a turn");
        app.finish_turn(spoken(&request, first));
        assert_eq!(app.public_messages().len(), before + 1);

        let request = app.begin_autonomous_turn().expect("a turn");
        app.finish_turn(spoken(&request, second));
        assert_eq!(
            app.public_messages().len(),
            before + 1,
            "a shared opening this long is the model stuck, not a new thought"
        );
    }

    #[test]
    fn two_lines_that_merely_start_alike_still_both_speak() {
        // A shared salutation is style, not repetition; only a whole shared
        // clause counts.
        let mut app = app();
        let before = app.public_messages().len();
        let request = app.begin_autonomous_turn().expect("a turn");
        app.finish_turn(spoken(
            &request,
            "Ah, my dear subjects. The treasury is lighter than my patience today.",
        ));
        let request = app.begin_autonomous_turn().expect("a turn");
        app.finish_turn(spoken(
            &request,
            "Ah, my dear subjects. Bring me something worth looking at before dusk.",
        ));
        assert_eq!(app.public_messages().len(), before + 2);
    }

    #[test]
    fn idle_chatter_is_sampled_more_loosely_than_purposeful_work() {
        // Backstage work has a job to do and should stay at the court's normal
        // sampling; an unprompted line in a quiet room has nothing pulling it
        // away from the last one it produced except the sampler.
        let mut app = app();
        let idle = app.begin_autonomous_turn().expect("an idle turn");
        let loose = idle.temperature;
        app.finish_turn(spoken(
            &idle,
            "A first line, said plainly and at length enough to count.",
        ));

        app.set_mode("Night Desk");
        let working = app.begin_autonomous_turn().expect("a night desk turn");
        assert!(
            loose > working.temperature,
            "idle {loose} should exceed backstage {}",
            working.temperature
        );
        assert!(loose <= 1.15, "sampling must stay bounded, got {loose}");
    }

    #[test]
    fn consecutive_idle_turns_do_not_ask_the_same_question() {
        // Identical prompts are why the model locked onto one answer.
        let mut app = app();
        let mut asked = Vec::new();
        for index in 0..4 {
            let request = app.begin_autonomous_turn().expect("a turn");
            asked.push(request.user.clone());
            app.finish_turn(spoken(
                &request,
                &format!("Distinct court line number {index}, spoken plainly."),
            ));
        }
        let mut unique = asked.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(
            unique.len(),
            asked.len(),
            "idle prompts repeated: {asked:?}"
        );
    }

    #[test]
    fn the_open_floor_goes_round_the_court() {
        // Live evidence: 228 of 300 turns were the Queen, in runs of up to
        // sixteen. One voice asked the same thing repeatedly is why most of
        // those turns were withheld as repetitions.
        let mut app = app();
        let mut speakers = Vec::new();
        for _ in 0..28 {
            let request = app
                .begin_autonomous_turn()
                .expect("an idle court takes a turn");
            speakers.push(request.agent);
            app.pending = None;
        }

        let queen = speakers
            .iter()
            .filter(|agent| **agent == AgentId::Queen)
            .count();
        assert!(
            (10..=18).contains(&queen),
            "the Queen should preside without monopolising, got {queen} of 28"
        );

        assert!(
            speakers.windows(2).all(|pair| pair[0] != pair[1]),
            "no voice should take two open-floor turns running: {speakers:?}"
        );

        for minister in [
            AgentId::Organist,
            AgentId::Artist,
            AgentId::Archivist,
            AgentId::Alchemist,
            AgentId::Malthus,
            AgentId::Orator,
            AgentId::Treasurer,
            AgentId::Scribe,
            AgentId::Diplomat,
            AgentId::Wizard,
        ] {
            assert!(
                speakers.contains(&minister),
                "{minister:?} never got the floor in 28 turns"
            );
        }
    }

    #[test]
    fn a_minister_is_asked_for_the_work_of_their_office() {
        let mut app = app();
        let mut asked: Vec<(AgentId, String)> = Vec::new();
        for _ in 0..28 {
            let request = app
                .begin_autonomous_turn()
                .expect("an idle court takes a turn");
            asked.push((request.agent, request.user));
            app.pending = None;
        }

        let organist = asked
            .iter()
            .find(|(agent, _)| *agent == AgentId::Organist)
            .expect("the Organist gets the floor");
        assert!(
            organist.1.contains("music"),
            "the Organist should be asked about music, got {:?}",
            organist.1
        );
        let treasurer = asked
            .iter()
            .find(|(agent, _)| *agent == AgentId::Treasurer)
            .expect("the Treasurer gets the floor");
        assert!(
            !treasurer.1.contains("music"),
            "each office should get its own request, got {:?}",
            treasurer.1
        );
    }

    #[test]
    fn a_locked_thread_is_a_subject_not_a_monologue() {
        // Live evidence: ten consecutive Queen turns on 'Echoes of Serenity',
        // each given the identical prompt, most withheld as repetitions. A
        // locked topic names what the court is discussing, not who discusses it.
        let mut app = app();
        app.locked_topic = Some("the price of candles".to_owned());
        let mut speakers = Vec::new();
        let mut prompts = Vec::new();
        for _ in 0..8 {
            let request = app
                .begin_autonomous_turn()
                .expect("an idle court takes a turn");
            assert!(
                request.user.contains("the price of candles"),
                "every turn should still be on the locked subject: {:?}",
                request.user
            );
            speakers.push(request.agent);
            prompts.push(request.user);
            app.pending = None;
        }

        assert!(
            speakers.iter().any(|agent| *agent != AgentId::Queen),
            "the room should join the thread, not listen to it: {speakers:?}"
        );
        assert!(
            speakers.windows(2).all(|pair| pair[0] != pair[1]),
            "no voice should hold the thread two turns running: {speakers:?}"
        );
        assert!(
            prompts.windows(2).all(|pair| pair[0] != pair[1]),
            "an identical prompt is why the same answer came back"
        );
    }

    #[test]
    fn a_short_acknowledgement_may_repeat() {
        let app = app();
        assert!(!app.is_recent_repetition("Indeed."));
    }

    #[test]
    fn only_one_inference_can_be_in_flight() {
        let mut app = app();
        app.input = "first".to_owned();
        assert!(app.begin_turn().is_some());
        app.input = "second".to_owned();
        assert!(app.begin_turn().is_none());
        assert!(app.notice.contains("already considering"));
    }

    #[test]
    fn direct_operator_speech_always_enters_the_shared_teledra_channel() {
        let mut court = Court::default();
        court.select(AgentId::Organist);
        let mut app = App::new(
            court,
            BrainHealth {
                endpoint: "http://localhost:11434".to_owned(),
                model: QWEN_MODEL,
            },
        );
        app.input = "Hello Teledra".to_owned();
        let request = app.begin_turn().expect("request");
        assert_eq!(request.agent, AgentId::Queen);
        assert_eq!(app.public_messages().back().unwrap().sender, "You");
    }

    #[test]
    fn a_direct_public_url_enters_bounded_research() {
        let mut app = app();
        app.input = "https://example.com/evidence".to_owned();

        assert!(app.begin_turn().is_none());
        assert_eq!(
            app.take_command(),
            Some(AppCommand::Research(
                "https://example.com/evidence".to_owned()
            ))
        );
        assert!(app.pending().is_none());
        assert_eq!(app.public_messages().back().unwrap().sender, "You");
    }

    #[test]
    fn delegated_court_replies_remain_visible_in_the_shared_feed() {
        let mut app = app();
        let request = app
            .begin_external_turn(AgentId::Organist, "Offer a counterpoint")
            .expect("request");
        app.finish_turn(ModelReply {
            id: request.id,
            agent: request.agent,
            user: request.user,
            result: Ok("A counterpoint from the organ loft.".to_owned()),
            audible: request.audible,
        });
        let visible = app.public_messages().back().unwrap();
        assert_eq!(visible.sender, "Organist");
        assert_eq!(visible.text, "A counterpoint from the organ loft.");
    }

    #[test]
    fn escape_keeps_the_v1_exit_contract() {
        let mut app = app();
        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            AppAction::Quit
        );
    }

    #[test]
    fn night_desk_maintenance_stays_backstage() {
        let mut app = app();
        app.set_mode("Night Desk");
        let request = app.begin_autonomous_turn().expect("request");
        assert!(!request.audible);
        let before = app.public_messages().len();
        app.finish_turn(ModelReply {
            id: request.id,
            agent: request.agent,
            user: request.user,
            result: Ok("Backstage maintenance evidence.".to_owned()),
            audible: request.audible,
        });
        assert_eq!(app.public_messages().len(), before);
        assert!(
            app.private_events()
                .iter()
                .any(|(_, message)| message.contains("maintenance evidence"))
        );
    }

    #[test]
    fn night_desk_can_run_backstage_while_streamer_stays_foreground() {
        let mut app = app();
        app.set_mode("Streamer");
        app.set_night_desk_enabled(true);

        let request = app.begin_nightdesk_turn().expect("request");

        assert!(!request.audible);
        assert_eq!(app.mode(), "Streamer");
        assert!(app.night_desk_enabled());
    }

    #[test]
    fn night_desk_uses_every_office_and_only_the_treasury_update_is_audible() {
        let mut app = app();
        let mut workers = Vec::new();
        let mut audible = Vec::new();
        let mut prompts = Vec::new();

        for _ in 0..NIGHT_DESK_ROTA.len() {
            let request = app.begin_nightdesk_turn().expect("a Night Desk turn");
            workers.push(request.agent);
            if request.audible {
                audible.push(request.agent);
            }
            prompts.push(request.user);
            app.pending = None;
        }

        for agent in AgentId::ALL {
            assert!(workers.contains(&agent), "{agent:?} had no Night Desk work");
        }
        assert_eq!(audible, vec![AgentId::Treasurer]);
        assert!(
            prompts.windows(2).all(|pair| pair[0] != pair[1]),
            "Night Desk repeated a fixed request: {prompts:?}"
        );
    }

    #[test]
    fn backstage_work_does_not_advance_the_public_rota() {
        let mut app = app();
        for _ in 0..NIGHT_DESK_ROTA.len() {
            let _ = app.begin_nightdesk_turn().expect("a Night Desk turn");
            app.pending = None;
        }

        let request = app
            .begin_autonomous_turn()
            .expect("the first public court turn");
        assert_eq!(request.agent, COURT_ROTA[0]);
    }

    #[test]
    fn legacy_command_aliases_keep_the_original_operator_surface() {
        assert_eq!(parse_command("/PyMusic"), Some(AppCommand::MusicPlay));
        assert_eq!(parse_command("/Sleep"), Some(AppCommand::Sleep));
        assert_eq!(parse_command("/Reflect"), Some(AppCommand::Reflect));
        assert_eq!(parse_command("/Treasury"), Some(AppCommand::TreasuryReport));
        assert_eq!(parse_command("/Scout"), Some(AppCommand::Treasury));
        assert_eq!(parse_command("/FindWork"), Some(AppCommand::Treasury));
        assert_eq!(parse_command("/Mission"), Some(AppCommand::Work));
        assert_eq!(
            parse_command("/Mission restore the observatory"),
            Some(AppCommand::Mission("restore the observatory".to_owned()))
        );
        assert_eq!(parse_command("/Jobs"), Some(AppCommand::Work));
        assert_eq!(parse_command("/Goals"), Some(AppCommand::Goals));
        assert_eq!(parse_command("/Kingdom"), Some(AppCommand::Goals));
        assert_eq!(parse_command("/Agents"), Some(AppCommand::Agents));
        assert_eq!(
            parse_command("/Diplomat"),
            Some(AppCommand::Envoy(String::new()))
        );
        assert_eq!(
            parse_command("/Diplomat send a cautious greeting"),
            Some(AppCommand::Envoy("send a cautious greeting".to_owned()))
        );
        assert_eq!(parse_command("/Outreach"), Some(AppCommand::DiplomacyLog));
        assert_eq!(
            parse_command("/Outreach send a cautious greeting"),
            Some(AppCommand::Outreach("send a cautious greeting".to_owned()))
        );
        assert_eq!(parse_command("/WizardPull"), Some(AppCommand::Tower));
        assert_eq!(
            parse_command("/ApproveAll"),
            Some(AppCommand::Approve("all".to_owned()))
        );
        assert_eq!(parse_command("/Suggestions"), Some(AppCommand::Proposals));
        assert_eq!(
            parse_command("/MemoryPolicy"),
            Some(AppCommand::Memory("memorypolicy".to_owned()))
        );
    }

    #[test]
    fn every_v1_command_head_still_routes() {
        // Extracted from the V1 terminal dispatch in src/main.rs. Keep the
        // complete list here: a help page can drift while an executable
        // compatibility contract cannot.
        const V1_COMMANDS: [&str; 77] = [
            "/agents",
            "/approve",
            "/approveall",
            "/art",
            "/clearsuggestions",
            "/dashboard",
            "/diplomacy",
            "/diplomacylog",
            "/diplomat",
            "/embassies",
            "/embassy",
            "/envoy",
            "/expand",
            "/facts",
            "/findwork",
            "/fractus",
            "/goals",
            "/growth",
            "/income",
            "/innovate",
            "/innovation",
            "/jobs",
            "/kingdom",
            "/ledger",
            "/links",
            "/lock",
            "/lore",
            "/mcp",
            "/memory",
            "/memorypolicy",
            "/mission",
            "/missioncancel",
            "/music",
            "/musicoff",
            "/musictoggle",
            "/night",
            "/nightdesk",
            "/numpy",
            "/outreach",
            "/play",
            "/proposals",
            "/pymusic",
            "/pythonmusic",
            "/reflect",
            "/reject",
            "/resident",
            "/rundown",
            "/scout",
            "/simchat",
            "/sketchpad",
            "/sleep",
            "/socials",
            "/strudel",
            "/study",
            "/suggestions",
            "/synth",
            "/tasks",
            "/test",
            "/testcast",
            "/testknobs",
            "/testmusic",
            "/testoff",
            "/teston",
            "/testrate",
            "/testscene",
            "/testsilence",
            "/teststatus",
            "/testtick",
            "/tools",
            "/treasury",
            "/unlock",
            "/variety",
            "/wizard",
            "/wizardpull",
            "/work",
            "/workshop",
            "/workshoprun",
        ];

        for command in V1_COMMANDS {
            assert!(
                parse_command(command).is_some(),
                "V1 command no longer routes: {command}"
            );
        }
    }
}
