use chrono::{Datelike, Local};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use super::{dim, focus_border, spinner, truncate};
use crate::app::App;
use crate::calendar::{month_grid, MonthData};
use crate::timer;

const CELL: usize = 4;
const WEEKDAYS: [&str; 7] = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];

/// Day-hours cell text: `8h`, `6h30`, `6½`… compact for a 4-wide cell.
fn hours_cell(minutes: i64) -> String {
    if minutes <= 0 {
        return String::new();
    }
    let h = minutes / 60;
    let m = minutes % 60;
    match m {
        0 => format!("{h}h"),
        30 => format!("{h}½"),
        _ => format!("{h}h{m}"),
    }
}

fn hours_style(minutes: i64) -> Style {
    if minutes >= 9 * 60 {
        Style::default().fg(Color::Yellow)
    } else if minutes >= 4 * 60 {
        Style::default().fg(Color::Green)
    } else if minutes > 0 {
        Style::default().fg(Color::LightGreen)
    } else {
        dim()
    }
}

fn month_name(month: u32) -> &'static str {
    [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ]
    .get((month as usize).saturating_sub(1))
    .copied()
    .unwrap_or("?")
}

pub fn draw(app: &App, f: &mut Frame, area: Rect, focused: bool) {
    let cal = &app.cal;
    let (y, m) = cal.view;
    let loading = cal.loading.contains(&(y, m));
    let data = cal.months.get(&(y, m));

    let total = data.map(MonthData::total_minutes).unwrap_or(0);
    let title = format!(
        " 3· {} {} · {} ",
        month_name(m),
        y,
        if loading {
            spinner(app.tick).to_string()
        } else {
            timer::format_human(std::time::Duration::from_secs(
                u64::try_from(total).unwrap_or(0) * 60,
            ))
        }
    );
    let block = Block::bordered()
        .title(Span::styled(title, Style::default().fg(Color::Cyan)))
        .border_style(focus_border(focused));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let grid_area = inner;

    let today = Local::now().date_naive();
    let mut lines: Vec<Line> = Vec::new();

    // a fetch failure replaces the grid — an easy-to-miss trailing line
    // hid real problems before
    if let Some(err) = &cal.error {
        lines.push(line_err("failed to load this month:"));
        for l in wrap_err(err, usize::from(inner.width)) {
            lines.push(line_err(&l));
        }
        lines.push(Line::from(Span::styled(
            "r: retry",
            Style::default().fg(Color::DarkGray),
        )));
        f.render_widget(Paragraph::new(lines), grid_area);
        return;
    }

    // weekday header
    let spans: Vec<Span> = WEEKDAYS
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let style = if i >= 5 { dim() } else { Style::default() };
            Span::styled(format!("{d:^CELL$}"), style)
        })
        .collect();
    lines.push(Line::from(spans));

    for week in month_grid(y, m) {
        // day numbers
        let spans: Vec<Span> = week
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let text = match d {
                    Some(d) => format!("{:>CELL$}", d.day()),
                    None => " ".repeat(CELL),
                };
                let style = match d {
                    Some(d) if *d == today => Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                    Some(d) if *d == cal.cursor => {
                        Style::default().bg(Color::Cyan).fg(Color::Black)
                    }
                    Some(_) if i >= 5 => dim(),
                    Some(_) => Style::default(),
                    None => Style::default(),
                };
                Span::styled(text, style)
            })
            .collect();
        lines.push(Line::from(spans));
        // logged hours
        let spans: Vec<Span> = week
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let mins = d
                    .map(|d| data.map(|md| md.day_minutes(d)).unwrap_or(0))
                    .unwrap_or(0);
                let text = if mins > 0 {
                    format!("{:>CELL$}", hours_cell(mins))
                } else {
                    " ".repeat(CELL)
                };
                let mut style = hours_style(mins);
                if let Some(d) = d {
                    if *d == cal.cursor {
                        style = style.bg(Color::Cyan).fg(Color::Black);
                    } else if i >= 5 {
                        style = dim();
                    }
                }
                Span::styled(text, style)
            })
            .collect();
        lines.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(lines), grid_area);
}

fn line_err(s: &str) -> Line<'static> {
    Line::from(Span::styled(s.to_string(), Style::default().fg(Color::Red)))
}

fn wrap_err(s: &str, width: usize) -> Vec<String> {
    let width = width.max(10);
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
    out
}

pub fn draw_today(app: &App, f: &mut Frame, area: Rect) {
    let today = Local::now().date_naive();
    let data = app.cal.months.get(&(today.year(), today.month()));
    let records = data.map(|md| md.records_on(today)).unwrap_or_default();
    let total = data.map(|md| md.day_minutes(today)).unwrap_or(0);

    let title = format!(
        " Today · {} ",
        timer::format_human(std::time::Duration::from_secs(
            u64::try_from(total).unwrap_or(0) * 60
        ))
    );
    let block = Block::bordered()
        .title(Span::styled(title, Style::default().fg(Color::Cyan)))
        .border_style(focus_border(false));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let width = usize::from(inner.width.saturating_sub(1));
    let lines: Vec<Line> = if records.is_empty() {
        vec![Line::from(Span::styled("nothing logged today", dim()))]
    } else {
        records
            .iter()
            .map(|r| {
                let Some(dur) = timer::parse_iso_duration(&r.duration) else {
                    return Line::from(Span::styled(r.issue_key(), dim()));
                };
                Line::from(vec![
                    Span::styled(
                        format!("{:<10}", r.issue_key()),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::styled(
                        format!("{:>7}", timer::format_human(dur)),
                        Style::default().fg(Color::Green),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        truncate(r.comment_or_empty(), width.saturating_sub(22)),
                        dim(),
                    ),
                ])
            })
            .collect()
    };
    f.render_widget(Paragraph::new(lines), inner);
}
