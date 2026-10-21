use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use super::{dim, focus_border, input_spans, list_scroll, scroll_title, spinner, truncate};
use crate::app::TasksTile;

pub(crate) fn type_color(key: &str) -> Color {
    match key {
        "bug" => Color::Red,
        "epic" => Color::Magenta,
        "story" => Color::LightGreen,
        "task" => Color::Cyan,
        "support" => Color::Yellow,
        _ => Color::Gray,
    }
}

pub fn draw(t: &mut TasksTile, tick: usize, f: &mut Frame, area: Rect, focused: bool) {
    let mut title = String::from(" 2· My tasks");
    if let Some(d) = t.type_filter_display() {
        title.push_str(&format!(" · {d}"));
    }
    if !t.filter.value.trim().is_empty() {
        title.push_str(&format!(" /{}", t.filter.value.trim()));
    }
    title.push_str(&format!(" ({})", t.filtered.len()));
    let title = if t.loading {
        format!(" 2· My tasks {} ", spinner(tick))
    } else {
        format!("{title} ")
    };
    let block = Block::bordered().border_style(focus_border(focused));
    let inner = block.inner(area);
    let [list_area, filter_area] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(inner);

    let height = usize::from(list_area.height);
    let len = t.filtered.len();
    t.scroll = list_scroll(t.scroll, t.selected, len, height);
    let block = block.title(scroll_title(&title, t.scroll, t.scroll + height < len));
    f.render_widget(block, area);

    let width = usize::from(list_area.width.saturating_sub(1));
    let key_w = t
        .filtered
        .iter()
        .map(|i| i.key.len())
        .max()
        .unwrap_or(8)
        .clamp(8, 14);
    let type_w = t
        .filtered
        .iter()
        .map(|i| i.type_display().len())
        .max()
        .unwrap_or(4)
        .clamp(4, 10);
    let status_w = t
        .filtered
        .iter()
        .map(|i| i.status_display().len())
        .max()
        .unwrap_or(8)
        .clamp(6, 18);
    let summary_w = width.saturating_sub(key_w + type_w + status_w + 5).max(10);

    let lines: Vec<Line> = if t.loading && t.issues.is_empty() {
        vec![Line::from(Span::styled(
            format!("{} loading issues…", spinner(tick)),
            dim(),
        ))]
    } else if let Some(err) = &t.error {
        vec![Line::from(Span::styled(
            truncate(err, width),
            Style::default().fg(Color::Red),
        ))]
    } else if t.filtered.is_empty() {
        vec![Line::from(Span::styled(
            if t.issues.is_empty() {
                "no issues assigned to you — press r to refresh".to_string()
            } else if t.hide_closed && t.issues.iter().all(|i| i.is_closed()) {
                "every assigned issue is closed — toggle “Hide closed tasks” in Settings"
                    .to_string()
            } else {
                "no matches for the filter".to_string()
            },
            dim(),
        ))]
    } else {
        t.filtered
            .iter()
            .enumerate()
            .skip(t.scroll)
            .take(height)
            .map(|(i, issue)| {
                let selected = i == t.selected;
                let base = if selected {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                Line::from(vec![
                    Span::styled(format!("{:<kw$}", issue.key, kw = key_w), {
                        let mut s = base;
                        if !selected {
                            s = s.fg(Color::Cyan);
                        }
                        s
                    }),
                    Span::styled(" ", base),
                    Span::styled(
                        truncate(issue.type_display(), type_w),
                        if selected {
                            base
                        } else {
                            Style::default().fg(type_color(issue.type_key()))
                        },
                    ),
                    Span::styled(" ", base),
                    Span::styled(
                        truncate(issue.status_display(), status_w),
                        if selected { base } else { dim() },
                    ),
                    Span::styled(" ", base),
                    Span::styled(truncate(&issue.summary, summary_w), base),
                ])
            })
            .collect()
    };
    f.render_widget(Paragraph::new(lines), list_area);

    // filter line: the input while filtering, otherwise blank (the bottom
    // legend carries the keys)
    let filter_line = if t.filter_active {
        let spans = input_spans(
            &t.filter,
            Style::default().fg(Color::Yellow),
            Style::default().bg(Color::Yellow).fg(Color::Black),
        );
        let mut out = vec![Span::styled("/ ", Style::default().fg(Color::Yellow))];
        out.extend(spans);
        Line::from(out)
    } else {
        Line::from("")
    };
    f.render_widget(Paragraph::new(filter_line), filter_area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::models::Issue;
    use ratatui::backend::TestBackend;

    fn tile(n: usize) -> TasksTile {
        let mut t = TasksTile::new();
        t.issues = (0..n)
            .map(|i| Issue {
                key: format!("K-{i:03}"),
                summary: format!("sum {i}"),
                ..Default::default()
            })
            .collect();
        t.refresh_filter();
        t
    }

    fn rendered(t: &mut TasksTile, w: u16, h: u16) -> String {
        let mut term = ratatui::Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| draw(t, 0, f, f.area(), true)).unwrap();
        let buf = term.backend().buffer();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn selection_below_the_fold_scrolls_into_view() {
        // 12 rows → 9 list rows; 30 issues, cursor on the last one
        let mut t = tile(30);
        t.selected = 29;
        let screen = rendered(&mut t, 60, 12);
        assert!(screen.contains("K-029"), "selected row must be on screen");
        assert!(screen.contains("↑"), "more-above marker expected");
        assert!(
            !screen.contains("K-000"),
            "window should have scrolled off the top"
        );
    }

    #[test]
    fn short_list_never_scrolls_and_marks_below() {
        let mut t = tile(3);
        let screen = rendered(&mut t, 60, 12);
        assert!(screen.contains("K-000") && screen.contains("K-002"));
        assert!(!screen.contains("↑") && !screen.contains("↓"));
    }
}
