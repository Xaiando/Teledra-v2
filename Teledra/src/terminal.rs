use std::io::{self, Stdout};

use crossterm::{
    cursor,
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

type NativeTerminal = Terminal<CrosstermBackend<Stdout>>;

pub struct TerminalSession {
    terminal: NativeTerminal,
    report: TerminalReport,
}

/// What the console actually granted, so a degraded session says so.
///
/// Losing 24-bit colour is silent and total: the court still runs, still
/// speaks, still renders — it simply renders wrong, and the portrait turns into
/// a field of blocks that reads as a broken image rather than a missing
/// capability. Recording the answer at startup means the operator can see which
/// one they are looking at instead of guessing from the pixels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalReport {
    /// Standard output is a console whose mode could be read.
    pub console: bool,
    /// Virtual terminal processing was already on when we arrived.
    pub was_enabled: bool,
    /// Virtual terminal processing is on now.
    pub enabled: bool,
    /// Crossterm's explicit colour opt-out, independent of console capability.
    pub no_color: bool,
}

impl TerminalReport {
    /// True when `Color::Rgb` will reach the terminal intact.
    ///
    /// Crossterm probes once and caches: its probe returns early and succeeds
    /// whenever the bit is already set, so setting it before anything asks is
    /// what makes 24-bit colour available for the rest of the session.
    pub const fn truecolor(&self) -> bool {
        self.enabled && !self.no_color
    }

    pub fn describe(&self) -> String {
        if self.no_color {
            return "colour disabled by NO_COLOR; remove that setting when launching to display the portrait in colour".to_owned();
        }
        match (self.console, self.enabled, self.was_enabled) {
            (false, _, _) => {
                "colour degraded: standard output is not a console, so 24-bit colour is unavailable and the portrait will render as blocks".to_owned()
            }
            (true, false, _) => {
                "colour degraded: the console refused virtual terminal processing, so every colour falls back to the 16-colour console API and the portrait will render as blocks".to_owned()
            }
            (true, true, true) => "24-bit colour: virtual terminal processing was already on".to_owned(),
            (true, true, false) => {
                "24-bit colour: virtual terminal processing enabled at startup".to_owned()
            }
        }
    }
}

impl TerminalSession {
    pub fn enter() -> io::Result<Self> {
        Self::enter_profile(None, None)
    }

    pub fn enter_vertical() -> io::Result<Self> {
        Self::enter_profile(Some((54, 48)), Some("Teledra Vertical // OBS"))
    }

    pub fn enter_dashboard() -> io::Result<Self> {
        Self::enter_profile(Some((120, 42)), Some("Teledra Kingdom Observatory"))
    }

    pub fn enter_work_board() -> io::Result<Self> {
        Self::enter_profile(Some((132, 42)), Some("Teledra Work Board"))
    }

    fn enter_profile(size: Option<(u16, u16)>, title: Option<&str>) -> io::Result<Self> {
        let mut report = enable_ansi();
        report.no_color = std::env::var("NO_COLOR").is_ok_and(|value| !value.is_empty());
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        if let Some(title) = title {
            let _ = execute!(stdout, crossterm::terminal::SetTitle(title));
        }
        if let Some((width, height)) = size {
            let _ = execute!(stdout, crossterm::terminal::SetSize(width, height));
        }
        if let Err(error) = execute!(
            stdout,
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture,
            cursor::Hide
        ) {
            let _ = disable_raw_mode();
            return Err(error);
        }
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal.clear()?;
        Ok(Self { terminal, report })
    }

    /// What this session was granted by the console.
    pub const fn report(&self) -> TerminalReport {
        self.report
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
            DisableBracketedPaste,
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

/// Turns on virtual terminal processing before anything asks whether it exists.
///
/// The backend probes for ANSI support once and caches the answer for the life
/// of the process. When the court is started by something other than a person
/// double-clicking it — a launcher, a scheduler, a scripted restart — that probe
/// can run before the console is fully attached, conclude there is no ANSI
/// support, and fall back to the Win32 console API for the whole session. That
/// API has no 24-bit colour, so every `Color::Rgb` collapses to the terminal
/// default: the court renders in white on black and the portrait becomes an
/// unreadable field of blocks. Setting the mode here removes the race.
#[cfg(windows)]
fn enable_ansi() -> TerminalReport {
    use windows_sys::Win32::System::Console::{
        ENABLE_PROCESSED_OUTPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, GetStdHandle,
        STD_OUTPUT_HANDLE, SetConsoleMode,
    };

    // Safety: the handle is owned by the console subsystem and both calls only
    // read or replace this process's own output mode.
    let unavailable = TerminalReport {
        console: false,
        was_enabled: false,
        enabled: false,
        no_color: false,
    };

    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        if handle.is_null() {
            return unavailable;
        }
        let mut mode = 0;
        if GetConsoleMode(handle, &mut mode) == 0 {
            return unavailable;
        }
        let was_enabled = mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0;
        SetConsoleMode(
            handle,
            mode | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
        );
        // Read the mode back rather than trusting the write. `SetConsoleMode`
        // reports success for a mode the console then declines to keep, and the
        // whole point of this report is to know what actually holds.
        let mut applied = 0;
        let enabled = GetConsoleMode(handle, &mut applied) != 0
            && applied & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0;
        TerminalReport {
            console: true,
            was_enabled,
            enabled,
            no_color: false,
        }
    }
}

#[cfg(not(windows))]
fn enable_ansi() -> TerminalReport {
    TerminalReport {
        console: true,
        was_enabled: true,
        enabled: true,
        no_color: false,
    }
}

#[cfg(test)]
mod tests {
    use super::TerminalReport;

    #[test]
    fn explicit_colour_opt_out_is_not_reported_as_a_colourful_terminal() {
        let mut report = TerminalReport {
            console: true,
            was_enabled: true,
            enabled: true,
            no_color: true,
        };
        assert!(!report.truecolor());
        assert!(report.describe().contains("NO_COLOR"));
        report.no_color = false;
        assert!(report.truecolor());
        assert!(report.describe().contains("24-bit colour"));
    }
}
