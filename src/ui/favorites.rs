use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;

use super::{centered, dim, focus_border, input_spans, spinner, truncate};
use crate::app::App;

pub fn draw(app: &App, f: &mut Frame, area: Rect, focused: bool) {
    let favs = &app.state.favorites;
    let block = Block::bordered()
        .title(Span::styled(
            format!(" 1· Favorites ({}) ", favs.len()),
            Style::default().fg(Color::Cyan),
        ))
        .border_style(focus_border(focused));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let width = usize::from(inner.width.saturating_sub(1));
    let lines: Vec<Line> = if favs.is_empty() {
        vec![Line::from(Span::styled(
            "no favorites yet — press a to search and add",
            dim(),
        ))]
    } else {
        favs.iter()
            .enumerate()
            .map(|(i, issue)| {
                let selected = i == app.fav_sel.min(favs.len().saturating_sub(1));
                let base = if selected {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                Line::from(vec![
                    Span::styled(format!("{:<kw$}", issue.key, kw = 12), {
                        let mut s = base;
                        if !selected {
                            s = s.fg(Color::Cyan);
                        }
                        s
                    }),
                    Span::styled(
                        format!("{:<tw$}", truncate(issue.type_display(), 8), tw = 8),
                        if selected {
                            base
                        } else {
                            Style::default().fg(super::tasks::type_color(issue.type_key()))
                        },
                    ),
                    Span::styled(truncate(&issue.summary, width.saturating_sub(22)), base),
                ])
            })
            .collect()
    };
    f.render_widget(Paragraph::new(lines), inner);
}

/// The add-a-favorite search modal: an input plus a dropdown of matches
/// (local fuzzy over favorites + loaded issues, remote search from 3 chars).
pub fn draw_search_modal(app: &App, f: &mut Frame) {
    let area = centered(64, 6, f.area());
    f.render_widget(Clear, area);
    let block = Block::bordered()
        .title(Span::styled(
            " Add favorite ",
            Style::default().fg(Color::Cyan),
        ))
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let input = match app.modals.last() {
        Some(crate::app::Modal::FavSearch { input }) => input,
        _ => return,
    };
    let mut line = vec![Span::styled("Search ", Style::default().fg(Color::Yellow))];
    line.extend(input_spans(
        input,
        Style::default().fg(Color::White),
        Style::default().bg(Color::Yellow).fg(Color::Black),
    ));
    let rows = vec![
        Line::from(line),
        Line::from(Span::styled("", Style::default())),
        Line::from(Span::styled(
            "↑/↓ Enter add · Esc close · ≥3 chars searches Tracker",
            dim(),
        )),
    ];
    for (i, row) in rows.into_iter().enumerate() {
        let r = Rect {
            x: inner.x + 1,
            y: inner.y + u16::try_from(i).unwrap_or(u16::MAX),
            width: inner.width.saturating_sub(2),
            height: 1,
        };
        f.render_widget(Paragraph::new(row), r);
    }

    if let Some(dd) = &app.fav_dd {
        let w = area.width.min(64);
        let h = 9u16.min(area.y);
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
            format!(
                " Matches ({}){} ",
                dd.items.len(),
                if dd.items.is_empty() {
                    ""
                } else {
                    " · ★ = favorite"
                }
            )
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
                    "no matches — keep typing".to_string()
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
                    let is_fav = app.state.favorites.iter().any(|fv| fv.key == issue.key);
                    let base = if selected {
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    Line::from(vec![
                        Span::styled(if is_fav { "★ " } else { "  " }, {
                            let mut s = base;
                            if !selected {
                                s = s.fg(Color::Green);
                            }
                            s
                        }),
                        Span::styled(format!("{:<12}", issue.key), {
                            let mut s = base;
                            if !selected {
                                s = s.fg(Color::Cyan);
                            }
                            s
                        }),
                        Span::styled(truncate(&issue.summary, list_w.saturating_sub(15)), base),
                    ])
                })
                .collect()
        };
        f.render_widget(Paragraph::new(lines), dd_inner);
    }
}
