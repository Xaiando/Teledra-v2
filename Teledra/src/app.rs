use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use teledra_agents::AgentId;
use teledra_brain::{BrainHealth, Message};
use teledra_court::{Court, Turn};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppAction {
    Continue,
    Submit,
    Quit,
}

#[derive(Debug)]
pub struct InferenceRequest {
    pub id: u64,
    pub agent: AgentId,
    pub user: String,
    pub messages: Vec<Message>,
    pub temperature: f32,
}

#[derive(Debug)]
pub struct ModelReply {
    pub id: u64,
    pub agent: AgentId,
    pub user: String,
    pub result: Result<String, String>,
}

#[derive(Clone, Debug)]
pub struct PendingTurn {
    pub id: u64,
    pub agent: AgentId,
    pub user: String,
}

#[derive(Debug)]
pub struct App {
    court: Court,
    input: String,
    pending: Option<PendingTurn>,
    notice: String,
    endpoint: String,
    help_open: bool,
    scroll: u16,
    frame: usize,
    next_request_id: u64,
}

impl App {
    pub fn new(court: Court, health: BrainHealth) -> Self {
        Self {
            court,
            input: String::new(),
            pending: None,
            notice: "Court ready".to_owned(),
            endpoint: health.endpoint,
            help_open: false,
            scroll: 0,
            frame: 0,
            next_request_id: 1,
        }
    }

    pub fn active_agent(&self) -> AgentId {
        self.court.active()
    }

    pub fn active_history(&self) -> &[Turn] {
        self.court.active_history()
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn pending(&self) -> Option<&PendingTurn> {
        self.pending.as_ref()
    }

    pub fn notice(&self) -> &str {
        &self.notice
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
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
        let Event::Key(key) = event else {
            return AppAction::Continue;
        };
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return AppAction::Continue;
        }
        self.handle_key(key)
    }

    pub fn begin_turn(&mut self) -> Option<InferenceRequest> {
        let user = self.input.trim().to_owned();
        if user.is_empty() {
            return None;
        }
        if self.pending.is_some() {
            self.notice = "One court member is already considering a reply".to_owned();
            return None;
        }

        let agent = self.court.active();
        let prepared = self.court.prepare_turn_for(agent, &user);
        let id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        self.pending = Some(PendingTurn {
            id,
            agent,
            user: user.clone(),
        });
        self.input.clear();
        self.scroll = 0;
        self.notice = format!("{} is considering your words", agent.title());

        Some(InferenceRequest {
            id,
            agent,
            user,
            messages: prepared.messages,
            temperature: prepared.temperature,
        })
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
                self.court
                    .record_exchange_for(reply.agent, reply.user, answer);
                self.notice = format!("{} has answered", reply.agent.title());
            }
            Err(error) => {
                self.notice = format!("Qwen request failed: {error}");
            }
        }
        self.pending = None;
    }

    fn handle_key(&mut self, key: KeyEvent) -> AppAction {
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('q') | KeyCode::Char('c'))
        {
            return AppAction::Quit;
        }

        if self.help_open {
            if matches!(key.code, KeyCode::Esc | KeyCode::F(1) | KeyCode::Char('?')) {
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
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Esc => {
                self.input.clear();
                self.notice = "Input cleared".to_owned();
            }
            KeyCode::Tab => self.select_relative(1),
            KeyCode::BackTab => self.select_relative(-1),
            KeyCode::PageUp => {
                self.scroll = self.scroll.saturating_add(3);
            }
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_sub(3);
            }
            KeyCode::Char(character)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.input.push(character);
            }
            _ => {}
        }
        AppAction::Continue
    }

    fn select_relative(&mut self, delta: isize) {
        let current = AgentId::ALL
            .iter()
            .position(|agent| *agent == self.court.active())
            .unwrap_or_default();
        let count = AgentId::ALL.len() as isize;
        let next = (current as isize + delta).rem_euclid(count) as usize;
        let agent = AgentId::ALL[next];
        self.court.select(agent);
        self.scroll = 0;
        self.notice = format!("Entered {}'s chamber", agent.title());
    }
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

    #[test]
    fn tab_moves_between_independent_court_sessions() {
        let mut app = app();
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.active_agent(), AgentId::Organist);
        assert!(app.active_history().is_empty());
    }

    #[test]
    fn a_completed_reply_is_recorded_for_its_originating_agent() {
        let mut app = app();
        app.input = "What do you think?".to_owned();
        let request = app.begin_turn().expect("request");
        app.select_relative(1);
        app.finish_turn(ModelReply {
            id: request.id,
            agent: request.agent,
            user: request.user,
            result: Ok("A considered answer.".to_owned()),
        });
        assert!(app.active_history().is_empty());
        app.select_relative(-1);
        assert_eq!(app.active_history()[0].assistant, "A considered answer.");
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
}
