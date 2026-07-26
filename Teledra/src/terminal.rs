use std::io::{self, Stdout};

use crossterm::{
    cursor,
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

type NativeTerminal = Terminal<CrosstermBackend<Stdout>>;

pub struct TerminalSession {
    terminal: NativeTerminal,
}

impl TerminalSession {
    pub fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(
            stdout,
            EnterAlternateScreen,
            EnableMouseCapture,
            cursor::Hide
        ) {
            let _ = disable_raw_mode();
            return Err(error);
        }
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal.clear()?;
        Ok(Self { terminal })
    }

    pub fn draw(
        &mut self,
        render: impl FnOnce(&mut ratatui::Frame<'_>),
    ) -> io::Result<ratatui::CompletedFrame<'_>> {
        self.terminal.draw(render)
    }

    fn restore(&mut self) -> io::Result<()> {
        disable_raw_mode()?;
        execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture,
            cursor::Show
        )?;
        self.terminal.show_cursor()
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}
