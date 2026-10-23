mod calendar;
mod dashboard;
mod favorites;
mod log_form;
mod modals;
mod settings;
mod tasks;

use ratatui::layout::{Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::timer;

pub fn draw(app: &mut App, f: &mut Frame) {
    let [content, status, hints] = Layout::vertical([
        ratatui::layout::Constraint::Min(3),
        ratatui::layout::Constraint::Length(1),
        ratatui::layout::Constraint::Length(1),
    ])
    .areas(f.area());
    match app.tab {
        crate::app::Tab::Main => dashboard::draw(app, f, content),
        crate::app::Tab::Settings => settings::draw(app, f, content),
    }
    draw_status(app, f, status);
    modals::draw(app, f);
    if let Some(auth) = &app.auth {
        let client_id = app.cfg.client_id.clone().unwrap_or_default();
        let oauth_base = app.cfg.oauth_base.clone();
        modals::draw_onboarding(auth, &client_id, &oauth_base, f);
    }
    draw_toast(app, f);
    draw_hints(app, f, hints);
}

pub fn focus_border(focused: bool) -> Style {
    if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

pub fn spinner(tick: usize) -> &'static str {
    const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    FRAMES[tick % FRAMES.len()]
}

/// Cuts `s` to `width` display cells, appending `…` when truncated.
pub fn truncate(s: &str, width: usize) -> String {
    use unicode_width::UnicodeWidthStr;
    if s.width() <= width {
        return s.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in s.chars() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w > width.saturating_sub(1) {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    out
}

pub fn centered(width: u16, height: u16, area: Rect) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

/// Renders an editable input with a visible cursor block.
pub fn input_spans(
    input: &crate::input::TextInput,
    style: Style,
    cursor_style: Style,
) -> Vec<Span<'static>> {
    let (before, at, after) = input.parts();
    vec![
        Span::styled(before.to_string(), style),
        Span::styled(at.to_string(), cursor_style),
        Span::styled(after.to_string(), style),
    ]
}

pub fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

/// First visible row of a dropdown window of `visible` rows that keeps the
/// `selected` row on screen (windows scroll one line at a time past the fold).
pub fn dropdown_window(selected: usize, visible: usize) -> usize {
    if visible == 0 {
        return 0;
    }
    selected.saturating_sub(visible - 1)
}

/// Scroll offset of a `visible`-row window over `len` rows that keeps
/// `selected` on screen: unchanged while the selection stays inside the
/// window, follows one line at a time past either edge, and clamps when
/// the list or the terminal shrinks.
pub fn list_scroll(scroll: usize, selected: usize, len: usize, visible: usize) -> usize {
    if visible == 0 || len == 0 {
        return 0;
    }
    let max = len.saturating_sub(visible);
    let s = scroll.min(max);
    if selected < s {
        selected.min(max)
    } else if selected >= s + visible {
        selected + 1 - visible
    } else {
        s
    }
}

/// Tile border title: `text` in cyan plus muted ↑/↓ markers when the scroll
/// window doesn't cover the whole list.
pub fn scroll_title(text: &str, scroll: usize, more_below: bool) -> Line<'static> {
    let mut spans = vec![Span::styled(
        text.to_string(),
        Style::default().fg(Color::Cyan),
    )];
    if scroll > 0 {
        spans.push(Span::styled(" ↑", dim()));
    }
    if more_below {
        spans.push(Span::styled(" ↓", dim()));
    }
    Line::from(spans)
}

/// Display width of a rendered hints line.
pub fn hints_width(pairs: &[(&str, &str)]) -> usize {
    pairs
        .iter()
        .map(|(k, d)| k.width() + 1 + d.width())
        .sum::<usize>()
        + 2 * pairs.len().saturating_sub(1)
}

/// A line of key hints: `key desc  key desc …`, keys bold and bright so
/// they stand out from the muted descriptions.
pub fn hints_line(pairs: &[(&str, &str)]) -> Line<'static> {
    let mut spans = Vec::with_capacity(pairs.len() * 3);
    for (i, (k, d)) in pairs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(
            (*k).to_string(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(format!(" {d}"), dim()));
    }
    Line::from(spans)
}

/// Whole hints are dropped from the tail until the line fits `width`; in a
/// modal the `Esc` way out is pinned, on the dashboard `? keys` and `q quit`.
pub fn fit_hint_pairs<'a>(
    pairs: &[(&'a str, &'a str)],
    width: usize,
    modal_ctx: bool,
) -> Vec<(&'a str, &'a str)> {
    let pin = |k: &str| {
        if modal_ctx {
            k.contains("Esc")
        } else {
            k == "?" || k == "q"
        }
    };
    let mut kept: Vec<(&'a str, &'a str)> = pairs.to_vec();
    while hints_width(&kept) > width {
        match kept.iter().rposition(|(k, _)| !pin(k)) {
            Some(pos) => {
                kept.remove(pos);
            }
            None => break,
        }
    }
    kept
}

/// The contextual key legend on the last line — always visible (ostrich-style).
fn draw_hints(app: &App, f: &mut Frame, area: Rect) {
    if area.width < 2 || area.height == 0 {
        return;
    }
    let width = usize::from(area.width);
    let modal_ctx = app.auth.is_some() || !app.modals.is_empty();
    let pairs = app.key_hints();
    let fitted = fit_hint_pairs(&pairs, width, modal_ctx);
    let line = if hints_width(&fitted) > width {
        // a single over-wide hint: plain truncation as the last resort
        let flat = fitted
            .iter()
            .map(|(k, d)| format!("{k} {d}"))
            .collect::<Vec<_>>()
            .join("  ");
        Line::from(Span::styled(truncate(&flat, width), dim()))
    } else {
        hints_line(&fitted)
    };
    f.render_widget(Paragraph::new(line), area);
}

fn draw_status(app: &App, f: &mut Frame, area: Rect) {
    use ratatui::layout::Constraint;

    let busy = app.tasks.loading || !app.cal.loading.is_empty() || app.log.busy || app.detecting;

    let right = {
        let mut parts = Vec::new();
        if busy {
            parts.push(format!("{} ", spinner(app.tick)));
        }
        match (&app.me, &app.cfg.org_id) {
            (Some(me), Some(org)) => parts.push(format!("{} · org {}", me.login, org)),
            _ => parts.push("not connected".to_string()),
        }
        parts.join("")
    };

    let right_w = right.width().min(usize::from(area.width)) as u16;
    let [left_area, right_area] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(right_w)])
            .flex(ratatui::layout::Flex::Start)
            .areas(area);

    let left = if let Some(t) = &app.timer {
        let elapsed = timer::format_hms(t.elapsed());
        let rounded = timer::format_human(app.rounded_for(t.elapsed()));
        Line::from(vec![
            Span::styled(" ▶ ", Style::default().fg(Color::Green)),
            Span::styled(
                t.issue.key.clone(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                truncate(&t.description, 40),
                Style::default().fg(Color::Green),
            ),
            Span::raw(" "),
            Span::styled(
                elapsed,
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  → will log {rounded}"), dim()),
        ])
    } else {
        Line::from(Span::styled(" ytime · Yandex Tracker", dim()))
    };
    f.render_widget(
        Paragraph::new(left).style(Style::default().bg(Color::Black)),
        left_area,
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(right, dim()))),
        right_area,
    );
}

fn draw_toast(app: &App, f: &mut Frame) {
    use ratatui::widgets::{Block, Borders};
    let Some((kind, msg, _)) = &app.toast else {
        return;
    };
    let color = match kind {
        crate::app::ToastKind::Success => Color::Green,
        crate::app::ToastKind::Error => Color::Red,
        crate::app::ToastKind::Info => Color::Cyan,
    };
    let msg = truncate(msg, 60);
    let w = (msg.width() + 4).min(usize::from(f.area().width.saturating_sub(2))) as u16;
    let area = Rect {
        x: f.area().x + f.area().width.saturating_sub(w + 1),
        y: f.area().y + f.area().height.saturating_sub(2),
        width: w,
        height: 3.min(f.area().height),
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(msg, Style::default().fg(color)))).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(color)),
        ),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_fit_by_dropping_tail_but_keep_pinned() {
        let pairs = [
            ("j/k", "move"),
            ("1/2/3", "tile"),
            ("?", "keys"),
            ("q", "quit"),
        ];
        // everything fits
        let kept = fit_hint_pairs(&pairs, 80, false);
        assert_eq!(kept.len(), 4);
        // drops from the tail, "?" and "q" are pinned
        let kept = fit_hint_pairs(&pairs, 20, false);
        assert!(hints_width(&kept) <= 20);
        assert!(kept.contains(&("?", "keys")));
        assert!(kept.contains(&("q", "quit")));
        assert!(!kept.contains(&("1/2/3", "tile")));
        // in a modal the Esc way out is pinned instead
        let modal_pairs = [("Enter", "log"), ("d", "discard"), ("Esc", "keep")];
        let kept = fit_hint_pairs(&modal_pairs, 12, true);
        assert!(kept.contains(&("Esc", "keep")));
        assert!(hints_width(&kept) <= 12 || kept.len() == 1);
    }

    #[test]
    fn hints_width_counts_separators() {
        assert_eq!(hints_width(&[]), 0);
        assert_eq!(hints_width(&[("s", "stop")]), 6);
        assert_eq!(hints_width(&[("s", "stop"), ("q", "quit")]), 14);
    }

    #[test]
    fn list_scroll_follows_selection_and_clamps() {
        // everything fits: never scrolls
        assert_eq!(list_scroll(0, 9, 10, 10), 0);
        // selection inside the window: offset stays put
        assert_eq!(list_scroll(5, 7, 20, 10), 5);
        // past the bottom fold: follows one line at a time
        assert_eq!(list_scroll(0, 10, 20, 10), 1);
        assert_eq!(list_scroll(10, 19, 20, 10), 10);
        // above the window: pulled up to the selection
        assert_eq!(list_scroll(10, 9, 20, 10), 9);
        // stale offset clamps when the list or terminal shrinks
        assert_eq!(list_scroll(8, 0, 12, 10), 0);
        assert_eq!(list_scroll(8, 11, 12, 10), 2);
        assert_eq!(list_scroll(3, 1, 5, 0), 0);
    }
}
