use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::app::{App, FocusField};
use crate::portrait::Portrait;
use crate::session::SessionSnapshot;

const NEON: Color = Color::Rgb(0, 255, 66);
const PURPLE: Color = Color::Rgb(147, 51, 234);
const DARK_PURPLE: Color = Color::Rgb(80, 0, 120);
const LABEL: Color = Color::Rgb(80, 80, 80);
const MUTED: Color = Color::Rgb(128, 128, 128);
const TRACE: Color = Color::Rgb(170, 170, 170);
const GOLD: Color = Color::Rgb(255, 215, 0);

/// The V1 control room, with the original geometry and visual language.
pub fn render(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(3),
        ])
        .split(area);

    render_header(frame, rows[0], app);
    render_court(frame, rows[1], app);
    render_input(frame, rows[2], app);

    if app.help_open() {
        render_help(frame, centered_rect(68, 68, area));
    }
}

/// Read-only portrait view opened by `/Vertical` in a second terminal.
pub fn render_vertical(frame: &mut Frame<'_>, snapshot: &SessionSnapshot, portrait: &Portrait) {
    let area = frame.area();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(4),
            Constraint::Percentage(52),
            Constraint::Min(10),
            Constraint::Length(3),
        ])
        .split(area);

    let header = Paragraph::new(vec![
        Line::from(Span::styled(
            "SIBELIUM COGNITIVE INTERFACE",
            Style::default().fg(NEON).add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(
                "TELEDRA",
                Style::default().fg(GOLD).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" // VERTICAL BROADCAST", Style::default().fg(PURPLE)),
        ]),
    ])
    .alignment(Alignment::Center)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(PURPLE)),
    );
    frame.render_widget(header, rows[0]);

    render_portrait_source(frame, rows[1], portrait);

    let mut speech = Vec::new();
    let start = snapshot.messages.len().saturating_sub(8);
    for message in &snapshot.messages[start..] {
        push_labeled_message(
            &mut speech,
            format!("[{}] ", message.sender),
            &message.text,
            Style::default()
                .fg(sender_color(&message.sender))
                .add_modifier(Modifier::BOLD),
            Style::default().fg(NEON),
        );
    }
    if speech.is_empty() {
        speech.push(Line::from(Span::styled(
            "The sovereign court is gathering its thoughts...",
            Style::default().fg(MUTED),
        )));
    }
    if snapshot.pending.is_some() {
        speech.push(Line::from(vec![
            Span::styled(
                "[Teledra] ",
                Style::default().fg(GOLD).add_modifier(Modifier::BOLD),
            ),
            Span::styled("Considering...", Style::default().fg(NEON)),
        ]));
    }
    let bottom = scroll_bottom(&speech, rows[2]);
    frame.render_widget(
        Paragraph::new(speech)
            .wrap(Wrap { trim: false })
            .scroll((bottom, 0))
            .block(
                Block::default()
                    .title(" LIVE NEURAL CHANNEL ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(PURPLE)),
            ),
        rows[2],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" STATUS: ", Style::default().fg(PURPLE)),
            Span::styled(&snapshot.notice, Style::default().fg(NEON)),
        ]))
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(DARK_PURPLE)),
        ),
        rows[3],
    );
}

/// Read-only kingdom observability view opened by `/Dashboard`.
pub fn render_dashboard(frame: &mut Frame<'_>, snapshot: &SessionSnapshot) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(5),
            Constraint::Min(10),
            Constraint::Length(3),
        ])
        .split(frame.area());
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "TELEDRA KINGDOM OBSERVATORY",
                Style::default().fg(GOLD).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" // READ-ONLY", Style::default().fg(PURPLE)),
        ]))
        .alignment(Alignment::Center)
        .block(neon_block(" LIVE DASHBOARD ")),
        rows[0],
    );
    let status = vec![
        Line::from(vec![
            Span::styled("Mode: ", Style::default().fg(PURPLE)),
            Span::styled(&snapshot.mode, Style::default().fg(NEON)),
            Span::styled("   Night Desk: ", Style::default().fg(PURPLE)),
            Span::styled(
                if snapshot.night_desk { "ON" } else { "OFF" },
                Style::default().fg(if snapshot.night_desk {
                    Color::Yellow
                } else {
                    MUTED
                }),
            ),
        ]),
        Line::from(format!(
            "Pending proposals: {}   Workshop experiments: {}   Court messages: {}",
            snapshot.pending_proposals,
            snapshot.workshop_experiments,
            snapshot.messages.len()
        )),
        Line::from(format!("Status: {}", snapshot.notice)),
    ];
    frame.render_widget(
        Paragraph::new(status).block(purple_block(" KINGDOM STATUS ")),
        rows[1],
    );

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[2]);
    let left = snapshot
        .private_events
        .iter()
        .rev()
        .take(18)
        .rev()
        .map(|event| {
            Line::from(vec![
                Span::styled(
                    format!("[{}] ", event.source),
                    Style::default()
                        .fg(sender_color(&event.source))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(&event.text, Style::default().fg(TRACE)),
            ])
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(if left.is_empty() {
            vec![Line::from("No backstage events yet")]
        } else {
            left
        })
        .wrap(Wrap { trim: false })
        .block(purple_block(" OPERATIONS / DIAGNOSTICS ")),
        columns[0],
    );
    let right = snapshot
        .messages
        .iter()
        .rev()
        .take(16)
        .rev()
        .map(|message| {
            Line::from(vec![
                Span::styled(
                    format!("[{}] ", message.sender),
                    Style::default()
                        .fg(sender_color(&message.sender))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(&message.text, Style::default().fg(NEON)),
            ])
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(if right.is_empty() {
            vec![Line::from("The public court is quiet")]
        } else {
            right
        })
        .wrap(Wrap { trim: false })
        .block(purple_block(" COURT ACTIVITY ")),
        columns[1],
    );
    render_view_footer(frame, rows[3], snapshot);
}

/// Read-only mission and treasury board opened by `/Work`.
pub fn render_work_board(frame: &mut Frame<'_>, snapshot: &SessionSnapshot) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(4),
            Constraint::Min(12),
            Constraint::Length(3),
        ])
        .split(frame.area());
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "TELEDRA WORK BOARD",
                Style::default().fg(GOLD).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" // MISSIONS & INCOME LEADS", Style::default().fg(PURPLE)),
        ]))
        .alignment(Alignment::Center)
        .block(neon_block(" READ-ONLY BOARD ")),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(snapshot.notice.as_str()),
            Line::from("YOU = operator action   AGENT = court deliverable   BOUNTY = bug bounty   IDEA = one-off product"),
        ])
        .block(purple_block(" STANDING POLICY ")),
        rows[1],
    );

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(rows[2]);
    for (index, lane) in ["you", "agent", "bounty", "idea"].iter().enumerate() {
        let matching = snapshot
            .private_events
            .iter()
            .filter(|event| {
                (event.source.eq_ignore_ascii_case("Treasury")
                    && event
                        .text
                        .to_ascii_lowercase()
                        .contains(&format!("[{lane}]")))
                    || (index == 1 && event.source.eq_ignore_ascii_case("Work"))
            })
            .map(|event| Line::from(event.text.as_str()))
            .collect::<Vec<_>>();
        let skip = matching.len().saturating_sub(16);
        let mut lines = matching.into_iter().skip(skip).collect::<Vec<_>>();
        if lines.is_empty() {
            lines.push(Line::from(Span::styled(
                "No entries",
                Style::default().fg(MUTED),
            )));
        }
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(purple_block(&format!(" {} ", lane.to_ascii_uppercase()))),
            columns[index],
        );
    }
    render_view_footer(frame, rows[3], snapshot);
}

fn neon_block(title: &str) -> Block<'_> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(NEON))
}

fn purple_block(title: &str) -> Block<'_> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(PURPLE))
}

fn render_view_footer(frame: &mut Frame<'_>, area: Rect, snapshot: &SessionSnapshot) {
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" LIVE: ", Style::default().fg(PURPLE)),
            Span::styled(&snapshot.notice, Style::default().fg(NEON)),
            Span::styled("   Esc/Ctrl+Q closes this view", Style::default().fg(MUTED)),
        ]))
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL)),
        area,
    );
}

fn render_header(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let night = app.night_desk_enabled();
    let suggestions = app.pending_proposals();
    let header = Line::from(vec![
        Span::styled(
            "SIBELIUM COGNITIVE INTERFACE v0.1.0 // HOST: TELEDRA",
            Style::default().fg(NEON).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" | STATUS: ", Style::default().fg(PURPLE)),
        Span::styled(app.notice().to_uppercase(), Style::default().fg(NEON)),
        Span::styled(" | SUGGESTIONS: ", Style::default().fg(PURPLE)),
        Span::styled(
            suggestions.to_string(),
            if suggestions > 0 {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(MUTED)
            },
        ),
        Span::styled(" | NIGHT: ", Style::default().fg(PURPLE)),
        Span::styled(
            if night { "ON" } else { "OFF" },
            if night {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(MUTED)
            },
        ),
    ]);
    frame.render_widget(
        Paragraph::new(header).alignment(Alignment::Center).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(PURPLE)),
        ),
        area,
    );
}

fn render_court(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(area);
    render_left(frame, columns[0], app);
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(12), Constraint::Length(16)])
        .split(columns[1]);
    render_conversation(frame, right[0], app);
    render_private_trace(frame, right[1], app);
}

fn render_left(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(6),
            Constraint::Length(10),
            Constraint::Length(5),
        ])
        .split(area);
    if let Some(portrait) = app.portrait() {
        render_portrait_source(frame, sections[0], portrait);
    } else {
        frame.render_widget(
            Paragraph::new("").block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(DARK_PURPLE)),
            ),
            sections[0],
        );
    }

    let emotion = match app.mode() {
        "Comedic" => "Teasing / Playful",
        "Empathetic" => "Protective / Gentle",
        "Dark Comedic" => "Cynical / Deadpan",
        "Babble" => "Excited / Curious",
        "Streamer" => "Regal thoughts / Live broadcast",
        "Game Co-Pilot" => "Game Co-Pilot / Couch companion",
        "Night Desk" => "Quiet / Autonomous",
        _ => "Proud / Imperial",
    };
    let suggestions = app.pending_proposals();
    let proposals_text = if suggestions > 0 {
        format!("{suggestions} PENDING")
    } else {
        "CLEAR".to_owned()
    };
    let workshop_text = format!("{} EXP", app.workshop_count());
    let protocols = vec![
        protocol_line("Behavior  ", app.mode(), PURPLE, true),
        protocol_line("Emotion   ", emotion, NEON, false),
        protocol_line("Voice     ", teledra_tts::queen_voice_mode(), PURPLE, false),
        // A console without 24-bit colour still runs the court perfectly and
        // renders the portrait as an unreadable field of blocks. Naming it here
        // is the difference between a known limitation and a bug report.
        protocol_line(
            "Display   ",
            if app.truecolor() {
                "24-BIT"
            } else {
                "NO COLOUR"
            },
            if app.truecolor() { PURPLE } else { Color::Red },
            false,
        ),
        protocol_line(
            "Music     ",
            // Muted is neither on nor off: the piece is still running and one
            // key brings it straight back, which the operator needs to see.
            match (app.music_enabled(), app.music_muted()) {
                (_, true) => "MUTED",
                (true, false) => "ON",
                (false, false) => "OFF",
            },
            match (app.music_enabled(), app.music_muted()) {
                (_, true) => Color::Yellow,
                (true, false) => NEON,
                (false, false) => Color::Red,
            },
            true,
        ),
        protocol_line(
            "Proposals ",
            &proposals_text,
            if suggestions > 0 {
                Color::Yellow
            } else {
                LABEL
            },
            suggestions > 0,
        ),
        protocol_line("Workshop  ", &workshop_text, PURPLE, false),
        protocol_line(
            "NightDesk ",
            if app.night_desk_enabled() {
                "ACTIVE"
            } else {
                "OFF"
            },
            if app.night_desk_enabled() {
                Color::Yellow
            } else {
                LABEL
            },
            app.night_desk_enabled(),
        ),
        Line::default(),
        Line::from(Span::styled(
            "Tab:Mode  Ctrl+M:Music  Ctrl+L/K/E/P:Like/Dislike/Expand/Playlist",
            Style::default().fg(Color::Rgb(60, 60, 60)),
        )),
    ];
    frame.render_widget(
        Paragraph::new(protocols).block(
            Block::default()
                .title(" PROTOCOLS ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(DARK_PURPLE)),
        ),
        sections[1],
    );

    let state = app.somatic();
    let face = if state.face_detected {
        "PRESENT"
    } else if state.source.is_empty() {
        "UNKNOWN"
    } else {
        "ABSENT"
    };
    let face_color = if state.face_detected {
        NEON
    } else if state.source.is_empty() {
        LABEL
    } else {
        Color::Red
    };
    let asymmetry = state.shoulder_asymmetry.unwrap_or_default();
    let posture = if state.shoulder_asymmetry.is_none() {
        "UNKNOWN"
    } else if asymmetry > 0.08 {
        "POOR"
    } else if asymmetry > 0.04 {
        "UNEVEN"
    } else {
        "EXCELLENT"
    };
    let posture_color = if state.shoulder_asymmetry.is_none() {
        LABEL
    } else if asymmetry > 0.08 {
        Color::Red
    } else if asymmetry > 0.04 {
        Color::Yellow
    } else {
        NEON
    };
    let filled = ((asymmetry.clamp(0.0, 0.16) / 0.16) * 16.0).round() as usize;
    let meter = format!("[{}{}]", "#".repeat(filled), ".".repeat(16 - filled));
    let somatic = vec![
        protocol_line("Face    ", face, face_color, false),
        protocol_line("Posture ", posture, posture_color, false),
        Line::from(vec![
            Span::styled("Asym    ", Style::default().fg(LABEL)),
            Span::styled(format!("{asymmetry:.3}"), Style::default().fg(PURPLE)),
            Span::styled(format!(" {meter}"), Style::default().fg(DARK_PURPLE)),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(somatic).block(
            Block::default()
                .title(" SOMATIC ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(DARK_PURPLE)),
        ),
        sections[2],
    );
}

fn protocol_line<'a>(label: &'a str, value: &'a str, color: Color, bold: bool) -> Line<'a> {
    let mut style = Style::default().fg(color);
    if bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    Line::from(vec![
        Span::styled(label, Style::default().fg(LABEL)),
        Span::styled(value, style),
    ])
}

fn render_portrait_source(frame: &mut Frame<'_>, area: Rect, portrait: &Portrait) {
    let width = area.width.saturating_sub(2);
    let height = area.height.saturating_sub(2);
    let lines = portrait.lines(width, height);
    frame.render_widget(
        Paragraph::new(Text::from(lines)).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(DARK_PURPLE)),
        ),
        area,
    );
}

fn render_conversation(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let mut lines = Vec::new();
    for message in app.public_messages() {
        push_labeled_message(
            &mut lines,
            format!("[{}] ", message.sender),
            &message.text,
            Style::default()
                .fg(sender_color(&message.sender))
                .add_modifier(Modifier::BOLD),
            Style::default().fg(NEON),
        );
        // V1 deliberately leaves air between public turns. Without this the
        // stream-facing court reads as one dense diagnostic log.
        lines.push(Line::default());
    }
    if app.pending().is_some() {
        lines.push(Line::from(vec![
            Span::styled(
                "[Teledra] ",
                Style::default().fg(GOLD).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{} Considering...", app.spinner()),
                Style::default().fg(NEON),
            ),
        ]));
    }
    let bottom = scroll_bottom(&lines, area);
    let position = bottom.saturating_sub(app.scroll());
    let title = if app.scroll() > 0 {
        format!(
            " NEURAL COGNITIVE CHANNEL [SCROLL: {position}/{bottom}] (Shift+Up/Down, PageUp/Down) "
        )
    } else {
        " NEURAL COGNITIVE CHANNEL (Shift+Up/Down, PageUp/Down to Scroll) ".to_owned()
    };
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .scroll((position, 0))
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(PURPLE)),
            ),
        area,
    );
}

fn render_private_trace(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let mut lines = Vec::new();
    for (source, message) in app.private_events() {
        push_labeled_message(
            &mut lines,
            format!("[{source}] "),
            message,
            Style::default()
                .fg(private_color(source))
                .add_modifier(Modifier::BOLD),
            Style::default().fg(TRACE),
        );
    }
    let bottom = scroll_bottom(&lines, area);
    let title = app.private_events().back().map_or_else(
        || " BACKSTAGE EVENT TRACE ".to_owned(),
        |(source, _)| {
            format!(
                " BACKSTAGE EVENT TRACE [{} events, latest: {source}] ",
                app.private_events().len()
            )
        },
    );
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((bottom, 0))
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(DARK_PURPLE)),
            ),
        area,
    );
}

fn push_labeled_message<'a>(
    lines: &mut Vec<Line<'a>>,
    label: String,
    content: &'a str,
    label_style: Style,
    content_style: Style,
) {
    let indent = " ".repeat(label.chars().count());
    let mut parts = content.lines();
    lines.push(Line::from(vec![
        Span::styled(label, label_style),
        Span::styled(parts.next().unwrap_or_default(), content_style),
    ]));
    for continuation in parts {
        lines.push(Line::from(vec![
            Span::raw(indent.clone()),
            Span::styled(continuation, content_style),
        ]));
    }
}

fn scroll_bottom(lines: &[Line<'_>], area: Rect) -> u16 {
    let width = area.width.saturating_sub(2).max(1) as usize;
    let height = area.height.saturating_sub(2) as usize;
    let rendered: usize = lines
        .iter()
        .map(|line| line.width().max(1).div_ceil(width))
        .sum();
    rendered.saturating_sub(height).min(u16::MAX as usize) as u16
}

fn sender_color(sender: &str) -> Color {
    match sender {
        "System" => MUTED,
        "You" => NEON,
        "Teledra" | "Queen" => GOLD,
        "Organist" => Color::Rgb(255, 0, 255),
        "Archivist" => Color::Rgb(0, 255, 255),
        "Alchemist" => Color::Rgb(0, 255, 0),
        "Orator" => Color::Rgb(255, 69, 0),
        "Scribe" => Color::Rgb(169, 169, 169),
        "Artist" => Color::Rgb(255, 165, 0),
        "Malthus" => Color::Rgb(255, 90, 90),
        "Broadcast" => Color::Rgb(210, 255, 80),
        _ => PURPLE,
    }
}

fn private_color(source: &str) -> Color {
    match source {
        "NightDesk" | "Night Desk" => Color::Yellow,
        "Diplomat" => Color::Rgb(120, 200, 255),
        "Diplomacy" => Color::Rgb(80, 220, 180),
        "Research" => Color::Rgb(0, 255, 255),
        "Innovation" => Color::Rgb(255, 0, 255),
        "Workshop" => NEON,
        "Restream" => PURPLE,
        "Status" => MUTED,
        "Broadcast" => Color::Rgb(210, 255, 80),
        _ => Color::Rgb(180, 180, 180),
    }
}

fn render_input(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    let chat_active = app.focus() == FocusField::Chat;
    let chat_width = columns[0].width.saturating_sub(5) as usize;
    let chat_text = tail(app.chat_input(), chat_width);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" > ", Style::default().fg(NEON)),
            Span::styled(&chat_text, Style::default().fg(NEON)),
        ]))
        .block(
            Block::default()
                .title(if chat_active {
                    " TRANSMIT (ACTIVE) "
                } else {
                    " TRANSMIT "
                })
                .borders(Borders::ALL)
                .border_style(Style::default().fg(if chat_active { NEON } else { PURPLE })),
        ),
        columns[0],
    );

    let youtube_active = app.focus() == FocusField::YouTube;
    let youtube_width = columns[1].width.saturating_sub(9) as usize;
    let youtube_text = tail(app.youtube_input(), youtube_width);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" URL > ", Style::default().fg(NEON)),
            Span::styled(&youtube_text, Style::default().fg(NEON)),
        ]))
        .block(
            Block::default()
                .title(if youtube_active {
                    " YOUTUBE INGEST (ACTIVE) "
                } else {
                    " YOUTUBE INGEST "
                })
                .borders(Borders::ALL)
                .border_style(Style::default().fg(if youtube_active { NEON } else { PURPLE })),
        ),
        columns[1],
    );

    if chat_active {
        frame.set_cursor_position((
            (columns[0].x + 4 + chat_text.chars().count() as u16)
                .min(columns[0].right().saturating_sub(2)),
            columns[0].y + 1,
        ));
    } else {
        frame.set_cursor_position((
            (columns[1].x + 7 + youtube_text.chars().count() as u16)
                .min(columns[1].right().saturating_sub(2)),
            columns[1].y + 1,
        ));
    }
}

fn tail(value: &str, width: usize) -> String {
    value
        .chars()
        .rev()
        .take(width)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn render_help(frame: &mut Frame<'_>, area: Rect) {
    frame.render_widget(Clear, area);
    let help = vec![
        Line::from(Span::styled(
            "TELEDRA CONTROL SURFACE",
            Style::default().fg(NEON).add_modifier(Modifier::BOLD),
        )),
        Line::default(),
        Line::from("Enter              Transmit to Teledra"),
        Line::from("Left / Up          Focus chat input"),
        Line::from("Right / Down       Focus YouTube ingest"),
        Line::from("Tab / Shift+Tab    Cycle performance mode"),
        Line::from("Shift+Up / Down    Scroll one line"),
        Line::from("PageUp / PageDown  Scroll five lines"),
        Line::from("Ctrl+M             Toggle Court Synth"),
        Line::from("Ctrl+U             Evolve/play the current music"),
        Line::from("Ctrl+L/K/E/P        Like/dislike/expand/playlist"),
        Line::from("Ctrl+G / Ctrl+J    Refresh vision / microphone"),
        Line::from("Esc                Exit Teledra"),
        Line::default(),
        Line::from("/Vertical opens the simultaneous OBS portrait TUI."),
    ];
    frame.render_widget(
        Paragraph::new(help).wrap(Wrap { trim: true }).block(
            Block::default()
                .title(" HELP ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(PURPLE)),
        ),
        area,
    );
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
    use crate::session::{SnapshotEvent, SnapshotMessage};

    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .fold(String::new(), |mut text, cell| {
                text.push_str(cell.symbol());
                text
            })
    }

    #[test]
    fn wide_tui_preserves_the_v1_contract() {
        let backend = TestBackend::new(120, 38);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let app = App::new(
            Court::default(),
            BrainHealth {
                endpoint: "http://localhost:11434".to_owned(),
                model: QWEN_MODEL,
            },
        );
        terminal.draw(|frame| render(frame, &app)).expect("render");
        let rendered = buffer_text(&terminal);
        assert!(rendered.contains("SIBELIUM COGNITIVE INTERFACE v0.1.0"));
        assert!(rendered.contains("NEURAL COGNITIVE CHANNEL"));
        assert!(rendered.contains("BACKSTAGE EVENT TRACE"));
        assert!(rendered.contains("TRANSMIT (ACTIVE)"));
        assert!(rendered.contains("YOUTUBE INGEST"));
        assert!(!rendered.contains("Address Teledra"));
        // Every protocol row has to survive the panel height. Adding one
        // silently pushed nothing out on paper and showed nothing on screen,
        // which is the kind of thing only a rendered buffer catches.
        for row in [
            "Behavior",
            "Emotion",
            "Voice",
            "Display",
            "Music",
            "Proposals",
            "Workshop",
            "NightDesk",
        ] {
            assert!(rendered.contains(row), "protocol row {row} was not drawn");
        }
    }

    #[test]
    fn the_protocol_panel_holds_every_row_on_a_real_terminal() {
        // The wide test runs at 120x38. A maximised window is closer to 250x66,
        // and a panel with a fixed height behaves differently when the space
        // around it grows. Check the size the court is actually streamed at.
        for (width, height) in [(120, 38), (200, 50), (250, 66), (140, 42)] {
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).expect("test terminal");
            let app = App::new(
                Court::default(),
                BrainHealth {
                    endpoint: "http://localhost:11434".to_owned(),
                    model: QWEN_MODEL,
                },
            );
            terminal.draw(|frame| render(frame, &app)).expect("render");
            let rendered = buffer_text(&terminal);
            for row in ["Behavior", "Voice", "Display", "Music", "NightDesk"] {
                assert!(
                    rendered.contains(row),
                    "protocol row {row} missing at {width}x{height}"
                );
            }
        }
    }

    #[test]
    fn vertical_tui_uses_the_shared_public_feed() {
        let backend = TestBackend::new(54, 64);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let snapshot = SessionSnapshot {
            active_slug: "queen".to_owned(),
            active_title: "Teledra".to_owned(),
            notice: "Court ready".to_owned(),
            pending: None,
            turns: Vec::new(),
            messages: vec![SnapshotMessage {
                sender: "Organist".to_owned(),
                text: "The portrait court is live.".to_owned(),
            }],
            terminal: String::new(),
            ..SessionSnapshot::default()
        };
        let portrait = Portrait::embedded();
        terminal
            .draw(|frame| render_vertical(frame, &snapshot, &portrait))
            .expect("render");
        let rendered = buffer_text(&terminal);
        assert!(rendered.contains("VERTICAL BROADCAST"));
        assert!(rendered.contains("portrait court is live"));
    }

    #[test]
    fn dashboard_renders_operational_and_court_state() {
        let backend = TestBackend::new(120, 42);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let snapshot = SessionSnapshot {
            notice: "Court ready".to_owned(),
            mode: "Streamer".to_owned(),
            private_events: vec![SnapshotEvent {
                source: "Mission".to_owned(),
                text: "Task one is running".to_owned(),
            }],
            messages: vec![SnapshotMessage {
                sender: "Teledra".to_owned(),
                text: "Welcome to the court".to_owned(),
            }],
            ..SessionSnapshot::default()
        };
        terminal
            .draw(|frame| render_dashboard(frame, &snapshot))
            .expect("render");
        let rendered = buffer_text(&terminal);
        assert!(rendered.contains("KINGDOM OBSERVATORY"));
        assert!(rendered.contains("Task one is running"));
        assert!(rendered.contains("Welcome to the court"));
    }

    #[test]
    fn work_board_sorts_missions_and_leads_into_lanes() {
        let backend = TestBackend::new(132, 42);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let snapshot = SessionSnapshot {
            notice: "Mission m1 has 1 task".to_owned(),
            private_events: vec![
                SnapshotEvent {
                    source: "Work".to_owned(),
                    text: "#1 Running: build the verifier".to_owned(),
                },
                SnapshotEvent {
                    source: "Treasury".to_owned(),
                    text: "[bounty] Repair a parser".to_owned(),
                },
            ],
            ..SessionSnapshot::default()
        };
        terminal
            .draw(|frame| render_work_board(frame, &snapshot))
            .expect("render");
        let rendered = buffer_text(&terminal);
        assert!(rendered.contains("WORK BOARD"));
        assert!(rendered.contains("build the verifier"));
        assert!(rendered.contains("Repair a parser"));
    }
}
