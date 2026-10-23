use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use super::{dim, truncate};
use crate::app::App;

pub fn draw(app: &App, f: &mut Frame, area: Rect) {
    use crate::config::{OrgHeader, RoundingMode};

    let block = Block::bordered()
        .title(Span::styled(
            " Settings · Shift+2 ",
            Style::default().fg(Color::Cyan),
        ))
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let [list_area, foot_area] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(3)]).areas(inner);
    let width = usize::from(list_area.width.saturating_sub(2));

    let mask = |v: &Option<String>| match v {
        Some(s) if !s.is_empty() => s.clone(),
        _ => "—".to_string(),
    };
    let rows: Vec<(&str, String, bool)> = vec![
        ("OAuth client_id", mask(&app.cfg.client_id), true),
        ("Organization id", mask(&app.cfg.org_id), true),
        (
            "Org header",
            match app.cfg.org_header {
                None => "auto".to_string(),
                Some(OrgHeader::XOrgId) => "X-Org-Id".to_string(),
                Some(OrgHeader::XCloudOrgId) => "X-Cloud-Org-Id".to_string(),
            },
            true,
        ),
        (
            "Rounding unit",
            format!("{}m", app.cfg.rounding.minutes),
            true,
        ),
        ("Rounding mode", app.cfg.rounding.mode.to_string(), true),
        (
            "Hide closed tasks",
            if app.cfg.hide_closed_tasks {
                "on"
            } else {
                "off"
            }
            .to_string(),
            true,
        ),
        ("Authorize (paste token)…", String::new(), true),
        ("Logout", String::new(), true),
    ];

    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .map(|(i, (label, value, action))| {
            let selected = i == app.settings_sel;
            let marker = if selected { "▸ " } else { "  " };
            let label_style = if selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else if *action {
                Style::default().fg(Color::White)
            } else {
                Style::default()
            };
            let mut spans = vec![Span::styled(marker, label_style)];
            spans.push(Span::styled(format!("{:<22}", label), label_style));
            if !value.is_empty() {
                spans.push(Span::styled(
                    truncate(value, width.saturating_sub(26)),
                    if selected { label_style } else { dim() },
                ));
            }
            Line::from(spans)
        })
        .collect();
    f.render_widget(Paragraph::new(lines), list_area);

    let mode_hint = match app.cfg.rounding.mode {
        RoundingMode::Up => "any tracked time ≥ 1s rounds up to the unit",
        RoundingMode::Nearest => "half rounds up; may round to zero (nothing logged)",
        RoundingMode::Down => "remainder discarded; may round to zero (nothing logged)",
    };
    let auth_line = match (&app.me, &app.cfg.org_id) {
        (Some(me), Some(org)) => format!("signed in as {} · org {}", me.login, org),
        _ => "not connected".to_string(),
    };
    f.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                format!(
                    "{} · rounding {} {} — {mode_hint}",
                    auth_line, app.cfg.rounding.minutes, app.cfg.rounding.mode
                ),
                dim(),
            )),
            Line::from(Span::styled(
                "j/k move · ⏎ edit or toggle · Tab back to the dashboard",
                dim(),
            )),
        ]),
        foot_area,
    );
}
