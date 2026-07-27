use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use teledra_agents::AgentId;

use crate::app::App;

const GOLD: Color = Color::Rgb(218, 179, 92);
const INK: Color = Color::Rgb(220, 220, 226);
const MUTED: Color = Color::Rgb(125, 128, 142);
const QUEEN: Color = Color::Rgb(205, 124, 220);
const USER: Color = Color::Rgb(106, 182, 255);
const PANEL: Color = Color::Rgb(31, 33, 43);

pub fn render(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(area);

    render_header(frame, rows[0], app);
    render_court(frame, rows[1], app);
    render_input(frame, rows[2], app);
    render_footer(frame, rows[3], app);

    if app.help_open() {
        render_help(frame, centered_rect(62, 62, area));
    }
}

fn render_header(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let title = Line::from(vec![
        Span::styled(
            " TELEDRA V2 ",
            Style::default()
                .fg(Color::Black)
                .bg(GOLD)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  Sovereign Court  ", Style::default().fg(INK)),
        Span::styled("Qwen 2.5 only", Style::default().fg(GOLD)),
    ]);
    let status = Line::from(vec![
        Span::styled(" ", Style::default()),
        Span::styled(app.notice(), Style::default().fg(MUTED)),
    ]);
    let header = Paragraph::new(vec![title, status]).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(GOLD)),
    );
    frame.render_widget(header, area);
}

/// Widest the left column is allowed to grow, and the narrowest terminal that
/// still earns one: below this the conversation needs every column it can get.
const CHROME_WIDTH: u16 = 28;
const CHROME_MINIMUM_TERMINAL: u16 = 72;

fn render_court(frame: &mut Frame<'_>, area: Rect, app: &App) {
    if area.width < CHROME_MINIMUM_TERMINAL {
        // A narrow terminal spends its width on the court's words.
        render_conversation(frame, area, app);
        return;
    }

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(CHROME_WIDTH), Constraint::Min(30)])
        .split(area);
    render_chrome(frame, columns[0], app);
    render_conversation(frame, columns[1], app);
}

/// The left column: the court's likeness above, its roster below.
fn render_chrome(frame: &mut Frame<'_>, area: Rect, app: &App) {
    // The roster is the fixed obligation; the portrait takes what is left, and
    // is skipped entirely when that would be too little to recognise.
    let roster_height = AgentId::ALL.len() as u16;
    let portrait_height = area.height.saturating_sub(roster_height);

    if app.portrait().is_none() || portrait_height < 6 {
        render_roster(frame, area, app);
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(portrait_height),
            Constraint::Length(roster_height),
        ])
        .split(area);
    render_portrait(frame, rows[0], app);
    render_roster(frame, rows[1], app);
}

fn render_portrait(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let Some(portrait) = app.portrait() else {
        return;
    };
    let lines = portrait.lines(area.width, area.height);
    if lines.is_empty() {
        return;
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn render_roster(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let items: Vec<ListItem<'_>> = AgentId::ALL
        .iter()
        .map(|agent| {
            let pending = app.pending().is_some_and(|pending| pending.agent == *agent);
            let suffix = if pending {
                format!(" {}", app.spinner())
            } else {
                String::new()
            };
            ListItem::new(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::raw(format!("{}{}", agent.slug(), suffix)),
            ]))
        })
        .collect();
    let mut state = ListState::default().with_selected(
        AgentId::ALL
            .iter()
            .position(|agent| *agent == app.active_agent()),
    );
    let list = List::new(items)
        .block(
            Block::default()
                .title(" COURT ")
                .borders(Borders::RIGHT)
                .border_style(Style::default().fg(PANEL)),
        )
        .highlight_symbol("◆ ")
        .highlight_style(Style::default().fg(GOLD).add_modifier(Modifier::BOLD));
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_conversation(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let agent = app.active_agent();
    let mut lines = vec![
        Line::from(Span::styled(
            agent.title(),
            Style::default()
                .fg(if agent == AgentId::Queen { QUEEN } else { GOLD })
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "Role-local court session",
            Style::default().fg(MUTED),
        )),
        Line::default(),
    ];

    if app.active_history().is_empty()
        && !app.pending().is_some_and(|pending| pending.agent == agent)
    {
        lines.push(Line::from(Span::styled(
            "The chamber is quiet. Address this member of the court below.",
            Style::default().fg(MUTED),
        )));
    }

    for turn in app.active_history() {
        push_speech(
            &mut lines,
            "YOU  ",
            Style::default().fg(USER).add_modifier(Modifier::BOLD),
            &turn.user,
        );
        lines.push(Line::default());
        push_speech(
            &mut lines,
            &format!("{:<5}", agent.slug().to_ascii_uppercase()),
            Style::default()
                .fg(if agent == AgentId::Queen { QUEEN } else { GOLD })
                .add_modifier(Modifier::BOLD),
            &turn.assistant,
        );
        lines.push(Line::default());
    }

    if let Some(pending) = app.pending().filter(|pending| pending.agent == agent) {
        push_speech(
            &mut lines,
            "YOU  ",
            Style::default().fg(USER).add_modifier(Modifier::BOLD),
            &pending.user,
        );
        lines.push(Line::default());
        lines.push(Line::from(vec![
            Span::styled(
                format!("{}  ", app.spinner()),
                Style::default().fg(GOLD).add_modifier(Modifier::BOLD),
            ),
            Span::styled("Considering…", Style::default().fg(MUTED)),
        ]));
    }

    let content_width = area.width.saturating_sub(1).max(1) as usize;
    let rendered_line_count: usize = lines
        .iter()
        .map(|line| line.width().max(1).div_ceil(content_width))
        .sum();
    let bottom = rendered_line_count.saturating_sub(area.height as usize) as u16;
    let mut conversation = Paragraph::new(Text::from(lines))
        .block(
            Block::default()
                .title(format!(" {} ", agent.title()))
                .borders(Borders::NONE),
        )
        .wrap(Wrap { trim: false });
    conversation = conversation.scroll((bottom.saturating_sub(app.scroll()), 0));
    frame.render_widget(conversation, area);
}

fn push_speech<'a>(lines: &mut Vec<Line<'a>>, label: &str, label_style: Style, content: &'a str) {
    let mut content_lines = content.lines();
    let first = content_lines.next().unwrap_or_default();
    lines.push(Line::from(vec![
        Span::styled(label.to_owned(), label_style),
        Span::styled(first, Style::default().fg(INK)),
    ]));
    for continuation in content_lines {
        lines.push(Line::from(vec![
            Span::raw("     "),
            Span::styled(continuation, Style::default().fg(INK)),
        ]));
    }
}

fn render_input(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let input = if app.input().is_empty() {
        Line::from(Span::styled(
            "Speak to the court…",
            Style::default().fg(MUTED),
        ))
    } else {
        let width = area.width.saturating_sub(2) as usize;
        let visible: String = app
            .input()
            .chars()
            .rev()
            .take(width)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        Line::from(Span::styled(visible, Style::default().fg(INK)))
    };
    let input = Paragraph::new(input).block(
        Block::default()
            .title(format!(" Address {} ", app.active_agent().title()))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if app.pending().is_some() { MUTED } else { GOLD })),
    );
    frame.render_widget(input, area);
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let footer = Line::from(vec![
        Span::styled(
            " Enter ",
            Style::default().fg(GOLD).add_modifier(Modifier::BOLD),
        ),
        Span::styled("send   ", Style::default().fg(MUTED)),
        Span::styled(
            "Tab ",
            Style::default().fg(GOLD).add_modifier(Modifier::BOLD),
        ),
        Span::styled("change court member   ", Style::default().fg(MUTED)),
        Span::styled("PgUp/PgDn ", Style::default().fg(GOLD)),
        Span::styled("scroll   ", Style::default().fg(MUTED)),
        Span::styled("F1 ", Style::default().fg(GOLD)),
        Span::styled("help   ", Style::default().fg(MUTED)),
        Span::styled("Ctrl+Q ", Style::default().fg(GOLD)),
        Span::styled("exit", Style::default().fg(MUTED)),
        Span::styled(
            format!("    {}", app.endpoint()),
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    frame.render_widget(Paragraph::new(footer), area);
}

fn render_help(frame: &mut Frame<'_>, area: Rect) {
    frame.render_widget(Clear, area);
    let help = vec![
        Line::from(Span::styled(
            "TELEDRA V2 COURT CONTROLS",
            Style::default().fg(GOLD).add_modifier(Modifier::BOLD),
        )),
        Line::default(),
        Line::from("Type naturally, then press Enter to address the selected court member."),
        Line::default(),
        Line::from("Tab / Shift+Tab   Move through the court"),
        Line::from("PageUp / PageDown Scroll the current chamber"),
        Line::from("Escape            Clear the input"),
        Line::from("F1 or ?            Close this help"),
        Line::from("Ctrl+Q or Ctrl+C   Leave Teledra"),
        Line::default(),
        Line::from(Span::styled(
            "Each member keeps a separate conversation. Qwen requests run in the",
            Style::default().fg(MUTED),
        )),
        Line::from(Span::styled(
            "background so the court remains responsive while an answer is formed.",
            Style::default().fg(MUTED),
        )),
    ];
    let popup = Paragraph::new(help)
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: true })
        .block(
            Block::default()
                .title(" HELP ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(GOLD)),
        );
    frame.render_widget(popup, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};
    use teledra_brain::{BrainHealth, QWEN_MODEL};
    use teledra_court::Court;

    use super::*;

    #[test]
    fn court_renders_into_a_terminal_buffer() {
        let backend = TestBackend::new(110, 32);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let app = App::new(
            Court::default(),
            BrainHealth {
                endpoint: "http://localhost:11434".to_owned(),
                model: QWEN_MODEL,
            },
        );
        terminal
            .draw(|frame| render(frame, &app))
            .expect("TUI should render");
        let rendered =
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .fold(String::new(), |mut text, cell| {
                    text.push_str(cell.symbol());
                    text
                });
        assert!(rendered.contains("TELEDRA V2"));
        assert!(rendered.contains("queen"));
        assert!(rendered.contains("Address Teledra"));
    }
}

#[cfg(test)]
mod visual_proof {
    use ratatui::{Terminal, backend::TestBackend};
    use teledra_brain::{BrainHealth, QWEN_MODEL};
    use teledra_court::Court;

    use super::*;
    use crate::portrait::Portrait;

    /// Renders the court at a realistic terminal size and writes it out as an
    /// image, so the composition can be judged by eye rather than asserted at.
    /// Ignored by default; run explicitly when the layout changes.
    #[test]
    #[ignore]
    fn write_layout_png() {
        const COLUMNS: u16 = 120;
        const ROWS: u16 = 38;
        // A terminal cell is about twice as tall as it is wide.
        const CELL_W: u32 = 8;
        const CELL_H: u32 = 16;

        let backend = TestBackend::new(COLUMNS, ROWS);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut court = Court::default();
        court.record_exchange(
            "What stirs in the court tonight?",
            "Strange science, and a portrait that finally has a face. Ha!",
        );
        let app = App::new(
            court,
            BrainHealth {
                endpoint: "http://localhost:11434".to_owned(),
                model: QWEN_MODEL,
            },
        )
        .with_portrait(Some(Portrait::embedded()));

        terminal
            .draw(|frame| render(frame, &app))
            .expect("the court should render");

        let buffer = terminal.backend().buffer();
        let mut canvas =
            image::RgbImage::new(u32::from(COLUMNS) * CELL_W, u32::from(ROWS) * CELL_H);
        for row in 0..ROWS {
            for column in 0..COLUMNS {
                let cell = &buffer[(column, row)];
                let symbol = cell.symbol();
                let fg = to_rgb(cell.style().fg, [220, 220, 226]);
                let bg = to_rgb(cell.style().bg, [16, 17, 24]);
                for y in 0..CELL_H {
                    for x in 0..CELL_W {
                        // Half blocks paint their lower half with the
                        // foreground; anything else is approximated by ink
                        // coverage so text remains legible in the dump.
                        let colour = if symbol == "\u{2584}" {
                            if y >= CELL_H / 2 { fg } else { bg }
                        } else if symbol == " " {
                            bg
                        } else if (CELL_H / 4..CELL_H * 3 / 4).contains(&y)
                            && (CELL_W / 4..CELL_W * 3 / 4).contains(&x)
                        {
                            fg
                        } else {
                            bg
                        };
                        canvas.put_pixel(
                            u32::from(column) * CELL_W + x,
                            u32::from(row) * CELL_H + y,
                            image::Rgb(colour),
                        );
                    }
                }
            }
        }
        let out = std::env::temp_dir().join("teledra_v2_layout.png");
        canvas.save(&out).expect("layout image should save");
        println!("LAYOUT_PNG={}", out.display());
    }

    fn to_rgb(colour: Option<ratatui::style::Color>, fallback: [u8; 3]) -> [u8; 3] {
        match colour {
            Some(ratatui::style::Color::Rgb(r, g, b)) => [r, g, b],
            Some(ratatui::style::Color::Black) => [0, 0, 0],
            Some(ratatui::style::Color::DarkGray) => [90, 92, 100],
            _ => fallback,
        }
    }
}
