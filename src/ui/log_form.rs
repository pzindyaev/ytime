use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;

use super::{centered, dim, input_spans, spinner, truncate};
use crate::app::App;
use crate::input::TextInput;

/// The retrospective log-entry modal. State lives in `App::log`; the date is
/// chosen on the calendar before opening (`n` on a day).
pub fn draw_modal(app: &App, f: &mut Frame) {
    let log = &app.log;
    let area = centered(64, 10, f.area());
    f.render_widget(Clear, area);
    let title = if log.busy {
        format!(" Log time {} ", spinner(app.tick))
    } else {
        format!(" Log time — {} ", date_title(&log.date))
    };
    let block = Block::bordered()
        .title(Span::styled(title, Style::default().fg(Color::Cyan)))
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let row_w = usize::from(inner.width.saturating_sub(2));
    let label = |i: usize, text: &str| {
        Span::styled(
            format!("{text} "),
            Style::default().fg(if log.focus == i {
                Color::Yellow
            } else {
                Color::DarkGray
            }),
        )
    };

    // row 1: task (chosen issue or search input)
    let mut row1 = vec![label(0, "Task")];
    if let Some(issue) = &log.issue {
        row1.push(Span::styled(
            truncate(&issue.title(), row_w.saturating_sub(6)),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        row1.extend(field(&log.task, "search KEY or text…", log.focus == 0));
    }

    // row 2: duration
    let mut row2 = vec![label(1, "Time")];
    row2.extend(field(
        &log.duration,
        "1h30m · 45m · 1.5h · 1:30",
        log.focus == 1,
    ));

    // row 3: comment
    let mut row3 = vec![label(2, "Note")];
    row3.extend(field(&log.comment, "what was done", log.focus == 2));

    // row 4: validation / status (the bottom legend carries the keys)
    let row4 = if let Some(err) = &log.error {
        Line::from(Span::styled(
            format!("! {err}"),
            Style::default().fg(Color::Red),
        ))
    } else if log.busy {
        Line::from(Span::styled("logging…", dim()))
    } else {
        Line::from("")
    };

    let rows = [row1, row2, row3];
    let mut y = inner.y + 1;
    for row in &rows {
        let r = Rect {
            x: inner.x + 1,
            y,
            width: inner.width.saturating_sub(2),
            height: 1,
        };
        f.render_widget(Paragraph::new(Line::from(row.clone())), r);
        y += 1;
    }
    let r = Rect {
        x: inner.x + 1,
        y: y + 1,
        width: inner.width.saturating_sub(2),
        height: 1,
    };
    f.render_widget(Paragraph::new(row4), r);

    // dropdown with task matches, floating above the modal
    if let Some(dd) = &log.dropdown {
        let w = area.width.min(64);
        let h = 8u16.min(area.y);
        let popup = Rect {
            x: area.x,
            y: area.y - h,
            width: w,
            height: h,
        };
        let list_w = usize::from(popup.width.saturating_sub(4));
        let title = if dd.loading {
            format!(" Matches {} ", spinner(app.tick))
        } else {
            format!(" Matches ({}) ", dd.items.len())
        };
        let block = Block::bordered()
            .title(Span::styled(title, Style::default().fg(Color::Cyan)))
            .border_style(Style::default().fg(Color::Cyan));
        let dd_inner = block.inner(popup);
        f.render_widget(Clear, popup);
        f.render_widget(block, popup);
        let lines: Vec<Line> = if dd.items.is_empty() {
            vec![Line::from(Span::styled(
                if dd.loading {
                    "searching…".to_string()
                } else {
                    "no matches — keep typing (≥3 chars for remote search)".to_string()
                },
                dim(),
            ))]
        } else {
            let visible = usize::from(dd_inner.height);
            let start = super::dropdown_window(dd.selected, visible);
            dd.items
                .iter()
                .skip(start)
                .take(visible)
                .enumerate()
                .map(|(i, issue)| {
                    let selected = start + i == dd.selected;
                    let base = if selected {
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    Line::from(vec![
                        Span::styled(format!("{:<12}", issue.key), {
                            let mut s = base;
                            if !selected {
                                s = s.fg(Color::Cyan);
                            }
                            s
                        }),
                        Span::styled(truncate(&issue.summary, list_w.saturating_sub(13)), base),
                    ])
                })
                .collect()
        };
        f.render_widget(Paragraph::new(lines), dd_inner);
    }
}

fn field(input: &TextInput, placeholder: &str, active: bool) -> Vec<Span<'static>> {
    if input.value.is_empty() {
        let color = if active {
            Color::Yellow
        } else {
            Color::DarkGray
        };
        return vec![
            Span::styled("▏".to_string(), Style::default().fg(Color::Yellow)),
            Span::styled(placeholder.to_string(), Style::default().fg(color)),
        ];
    }
    let base = Style::default().fg(Color::White);
    let cursor = if active {
        Style::default()
            .bg(Color::Yellow)
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD)
    } else {
        base
    };
    input_spans(input, base, cursor)
}

fn date_title(date: &str) -> String {
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map(|d| d.format("%a %d %b %Y").to_string())
        .unwrap_or_else(|_| date.to_string())
}
