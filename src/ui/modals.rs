use chrono::{Datelike, Local};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;

use super::{centered, dim, input_spans, spinner, truncate};
use crate::app::{App, Modal, OnboardState, OnboardStep};
use crate::timer;

pub fn draw(app: &App, f: &mut Frame) {
    for modal in &app.modals {
        match modal {
            Modal::Help => draw_help(f),
            Modal::QuitConfirm => popup(
                f,
                52,
                5,
                " Quit ",
                vec![
                    line("A timer is still running.", Color::Yellow),
                    line(
                        "Enter: stop & log · d: discard timer · Esc: stay",
                        Color::DarkGray,
                    ),
                ],
            ),
            Modal::Resume(t) => popup(
                f,
                62,
                7,
                " Resume timer ",
                vec![
                    Line::from(vec![
                        Span::styled(
                            t.issue_key.to_string(),
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(" "),
                        Span::raw(truncate(&t.summary, 40)),
                    ]),
                    Line::from(format!(
                        "started {} — {} elapsed",
                        t.started_at.format("%H:%M"),
                        timer::format_hms(
                            (Local::now() - t.started_at).to_std().unwrap_or_default()
                        ),
                    )),
                    line("", Color::Reset),
                    line(
                        "r: resume · Enter: stop & log · d: discard · Esc: ignore",
                        Color::DarkGray,
                    ),
                ],
            ),
            Modal::StopReview(review) => {
                let elapsed = (Local::now() - review.started_at)
                    .to_std()
                    .unwrap_or_default();
                let rounded = app.rounded_for(elapsed);
                let mut body = vec![
                    Line::from(vec![
                        Span::styled(
                            review.key.clone(),
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(" "),
                        Span::raw(truncate(&review.summary, 40)),
                    ]),
                    Line::from(Span::styled(truncate(&review.description, 54), dim())),
                    line("", Color::Reset),
                    Line::from(vec![
                        Span::styled("elapsed   ", dim()),
                        Span::styled(
                            timer::format_hms(elapsed),
                            Style::default().fg(Color::White),
                        ),
                    ]),
                    Line::from(vec![
                        Span::styled("will log  ", dim()),
                        Span::styled(
                            timer::format_human(rounded),
                            Style::default()
                                .fg(Color::Green)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!(
                                "  ({} {} {}m)",
                                match app.cfg.rounding.mode {
                                    crate::config::RoundingMode::Up => "rounded up,",
                                    crate::config::RoundingMode::Nearest => "nearest,",
                                    crate::config::RoundingMode::Down => "rounded down,",
                                },
                                "unit",
                                app.cfg.rounding.minutes
                            ),
                            dim(),
                        ),
                    ]),
                ];
                body.push(line("", Color::Reset));
                body.push(if review.posting {
                    line(format!("{} posting…", spinner(app.tick)), Color::Cyan)
                } else {
                    line(
                        "Enter: log · d: discard · Esc: keep running",
                        Color::DarkGray,
                    )
                });
                popup(f, 62, 10, " Stop & log ", body);
            }
            Modal::Describe { issue, input } => {
                let mut header = Vec::new();
                if !issue.type_key().is_empty() {
                    header.push(Span::styled(
                        format!("{} ", truncate(issue.type_display(), 12)),
                        Style::default().fg(super::tasks::type_color(issue.type_key())),
                    ));
                }
                header.push(Span::styled(
                    issue.key.clone(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));
                header.push(Span::raw(" "));
                header.push(Span::raw(truncate(&issue.summary, 44)));
                let body = vec![
                    Line::from(header),
                    line("", Color::Reset),
                    Line::from(input_spans(
                        input,
                        Style::default().fg(Color::White),
                        Style::default().bg(Color::Yellow).fg(Color::Black),
                    )),
                    line("", Color::Reset),
                    line("Enter: start · Esc: cancel", Color::DarkGray),
                ];
                popup(f, 60, 8, " What are you working on? ", body);
            }
            Modal::Input { title, input, .. } => popup(
                f,
                60,
                7,
                &format!(" {title} "),
                vec![
                    Line::from(input_spans(
                        input,
                        Style::default().fg(Color::White),
                        Style::default().bg(Color::Yellow).fg(Color::Black),
                    )),
                    line("", Color::Reset),
                    line("Enter: save · Esc: cancel", Color::DarkGray),
                ],
            ),
            Modal::DayDetail { date } => {
                let data = app.cal.months.get(&(date.year(), date.month()));
                let records = data.map(|md| md.records_on(*date)).unwrap_or_default();
                let total = data.map(|md| md.day_minutes(*date)).unwrap_or(0);
                let mut body = Vec::new();
                if data.is_none() {
                    body.push(line(
                        "month not loaded yet — press ] [ r on the calendar tile",
                        Color::DarkGray,
                    ));
                } else if records.is_empty() {
                    body.push(line("no time logged on this day", Color::DarkGray));
                } else {
                    for r in &records {
                        let dur = timer::parse_iso_duration(&r.duration)
                            .map(timer::format_human)
                            .unwrap_or_else(|| r.duration.clone());
                        body.push(Line::from(vec![
                            Span::styled(
                                format!("{:<12}", r.issue_key()),
                                Style::default().fg(Color::Cyan),
                            ),
                            Span::styled(format!("{:>8}", dur), Style::default().fg(Color::Green)),
                            Span::raw("  "),
                            Span::styled(truncate(r.comment_or_empty(), 40), dim()),
                        ]));
                    }
                    body.push(line("", Color::Reset));
                    body.push(line(
                        format!(
                            "total {}",
                            timer::format_human(std::time::Duration::from_secs(
                                u64::try_from(total).unwrap_or(0) * 60
                            ))
                        ),
                        Color::White,
                    ));
                }
                body.push(line("", Color::Reset));
                body.push(line(
                    "n: log time on this day · Esc: close",
                    Color::DarkGray,
                ));
                popup(
                    f,
                    64,
                    8 + u16::try_from(records.len()).unwrap_or(0).min(8),
                    &format!(" {} ", date.format("%a %d %B %Y")),
                    body,
                );
            }
            Modal::LogEntry => super::log_form::draw_modal(app, f),
            Modal::FavSearch { .. } => super::favorites::draw_search_modal(app, f),
            Modal::TypeFilter { selected } => {
                let types = app.tasks.available_types();
                let mut body = Vec::new();
                let row = |label: &str, count: usize, sel: bool| {
                    let style = if sel {
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    Line::from(vec![
                        Span::styled(if sel { "▸ " } else { "  " }, style),
                        Span::styled(label.to_string(), style),
                        Span::styled(format!("  ({count})"), dim()),
                    ])
                };
                body.push(row("all types", app.tasks.issues.len(), *selected == 0));
                for (i, (_, display, count)) in types.iter().enumerate() {
                    body.push(row(display, *count, *selected == i + 1));
                }
                body.push(line("", Color::Reset));
                body.push(line("j/k move · ⏎ apply · Esc cancel", Color::DarkGray));
                popup(f, 40, body.len() as u16 + 2, " Filter by type ", body);
            }
            Modal::Confirm { msg, .. } => popup(
                f,
                54,
                5,
                " Confirm ",
                vec![
                    line(msg.as_str(), Color::Yellow),
                    line("Enter: yes · Esc: no", Color::DarkGray),
                ],
            ),
            Modal::Error { title, msg } => {
                let wrapped = wrap(msg, 54);
                let h = (wrapped.len() as u16 + 4).min(20);
                let body = wrapped
                    .iter()
                    .map(|l| line(l.as_str(), Color::Red))
                    .chain(std::iter::once(line("", Color::Reset)))
                    .chain(std::iter::once(line("Esc/Enter: close", Color::DarkGray)))
                    .collect();
                popup(f, 60, h, &format!(" {title} "), body);
            }
        }
    }
}

pub fn draw_onboarding(auth: &OnboardState, f: &mut Frame, tick: usize) {
    match auth.step {
        OnboardStep::ClientId => {
            let body = vec![
                line("ytime needs a Yandex OAuth application.", Color::White),
                Line::from(vec![
                    Span::styled("1. open ", dim()),
                    Span::styled(
                        "https://oauth.yandex.ru/client/new",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::UNDERLINED),
                    ),
                ]),
                line(
                    "2. name it, choose the platform «Устройства / Devices»",
                    Color::DarkGray,
                ),
                line(
                    "3. no redirect URI or access checkboxes are needed",
                    Color::DarkGray,
                ),
                line(
                    "4. copy the application id (client_id) below:",
                    Color::DarkGray,
                ),
                line("", Color::Reset),
                Line::from(input_spans(
                    &auth.input,
                    Style::default().fg(Color::White),
                    Style::default().bg(Color::Yellow).fg(Color::Black),
                )),
                line("", Color::Reset),
                line(
                    "Enter: continue · Esc: skip (set it later in Settings)",
                    Color::DarkGray,
                ),
            ];
            popup(
                f,
                64,
                body.len() as u16 + 2,
                " Welcome — OAuth setup (1/4) ",
                body,
            );
        }
        OnboardStep::ClientSecret => {
            let body = vec![
                line("The app's client secret (Секрет).", Color::White),
                Line::from(Span::styled(
                    "On oauth.yandex.ru open your app — the secret is shown next to the ID.",
                    dim(),
                )),
                line(
                    "Yandex requires it when exchanging the device code for a token.",
                    Color::DarkGray,
                ),
                line(
                    "Leave empty if your app was registered without one.",
                    Color::DarkGray,
                ),
                line("", Color::Reset),
                Line::from(input_spans(
                    &auth.input,
                    Style::default().fg(Color::White),
                    Style::default().bg(Color::Yellow).fg(Color::Black),
                )),
                line("", Color::Reset),
                line(
                    "Enter: continue (empty = skip) · Esc: skip",
                    Color::DarkGray,
                ),
            ];
            popup(
                f,
                64,
                body.len() as u16 + 2,
                " Welcome — client secret (2/4) ",
                body,
            );
        }
        OnboardStep::OrgId => {
            let body = vec![
                line("Your Tracker organization id.", Color::White),
                Line::from(Span::styled(
                    "In Tracker: Administration → Organizations → the ID field.",
                    dim(),
                )),
                line("If in doubt, ask your Tracker admin.", Color::DarkGray),
                line("", Color::Reset),
                Line::from(input_spans(
                    &auth.input,
                    Style::default().fg(Color::White),
                    Style::default().bg(Color::Yellow).fg(Color::Black),
                )),
                line("", Color::Reset),
                line("Enter: continue · Esc: skip", Color::DarkGray),
            ];
            popup(
                f,
                64,
                body.len() as u16 + 2,
                " Welcome — organization (3/4) ",
                body,
            );
        }
        OnboardStep::WaitingCode => {
            let mut body = Vec::new();
            match &auth.code {
                Some(code) => {
                    body.push(Line::from(vec![
                        Span::styled("open  ", dim()),
                        Span::styled(
                            code.verification_url.clone(),
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::UNDERLINED),
                        ),
                    ]));
                    body.push(line("and enter this code:", Color::DarkGray));
                    body.push(line("", Color::Reset));
                    body.push(Line::from(Span::styled(
                        code.user_code.clone(),
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    )));
                    body.push(line("", Color::Reset));
                    body.push(line(
                        format!("{} waiting for authorization…", spinner(tick)),
                        Color::Cyan,
                    ));
                }
                None => {
                    // nothing is in flight when an error is on screen
                    if auth.error.is_none() {
                        body.push(line(
                            format!("{} requesting a device code…", spinner(tick)),
                            Color::Cyan,
                        ));
                    }
                }
            }
            if let Some(err) = &auth.error {
                body.push(line("", Color::Reset));
                body.push(line(format!("error: {err}"), Color::Red));
            }
            body.push(line("", Color::Reset));
            body.push(line("Esc: cancel · r: retry", Color::DarkGray));
            popup(f, 60, body.len() as u16 + 2, " Authorization (4/4) ", body);
        }
    }
}

fn draw_help(f: &mut Frame) {
    // key chords rendered bright and bold, descriptions plain — the same
    // look as the always-visible legend on the bottom line
    let key = |k: &str| {
        Span::styled(
            format!("{k:<18}"),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
    };
    let row =
        |k: &str, d: &str| Line::from(vec![key(k), Span::styled(d.to_string(), Style::default())]);
    let body = vec![
        line("Global", Color::Cyan),
        row("Tab / Shift+Tab", "cycle tiles and Settings"),
        row("1 / 2 / 3", "select tile (Favorites/Tasks/Calendar)"),
        row("Shift+1 / Shift+2", "select tab (dashboard/Settings)"),
        row("s", "stop the running timer (log it)"),
        row("?", "this help · q quit (Ctrl+Q anywhere)"),
        line("Favorites", Color::Cyan),
        row("j/k ↑/↓", "select"),
        row("⏎", "start timer · l log retro (calendar day)"),
        row("a", "add favorite (search) · d remove"),
        line("Tasks", Color::Cyan),
        row("j/k ↑/↓", "select · t type · / filter · r refresh"),
        row("⏎", "start timer"),
        line("Calendar", Color::Cyan),
        row("h/l j/k ←→↑↓", "move day · [ ] change month"),
        row("t", "today · r reload"),
        row("n", "log time on the day · ⏎ day details"),
        line("Log time (from the calendar)", Color::Cyan),
        row("↑/↓", "fields · Enter submit on Note"),
        row("Esc", "close · task field: ≥3 chars searches remotely"),
        line("Rounding", Color::Cyan),
        row(
            "",
            "time is rounded per Settings (default: up to 30m — 1m+ tracked = 30m)",
        ),
    ];
    popup(f, 70, body.len() as u16 + 2, " Help — Esc to close ", body);
}

fn line(s: impl Into<String>, color: Color) -> Line<'static> {
    Line::from(Span::styled(s.into(), Style::default().fg(color)))
}

fn popup(f: &mut Frame, w: u16, h: u16, title: &str, body: Vec<Line<'static>>) {
    let area = centered(w, h, f.area());
    f.render_widget(Clear, area);
    let block = Block::bordered()
        .title(Span::styled(
            title.to_string(),
            Style::default().fg(Color::Cyan),
        ))
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let pad = Rect {
        x: inner.x + 1,
        y: inner.y,
        width: inner.width.saturating_sub(2),
        height: inner.height,
    };
    let shown: Vec<Line> = body.into_iter().take(usize::from(pad.height)).collect();
    f.render_widget(Paragraph::new(shown), pad);
}

/// Naive word wrap for popup messages.
fn wrap(s: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        if cur.is_empty() {
            cur = word.to_string();
        } else if cur.chars().count() + 1 + word.chars().count() <= width {
            cur.push(' ');
            cur.push_str(word);
        } else {
            out.push(std::mem::take(&mut cur));
            cur = word.to_string();
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}
