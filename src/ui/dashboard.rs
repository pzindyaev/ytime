use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::Frame;

use super::{calendar, favorites, tasks};
use crate::app::{App, Tile};

pub fn draw(app: &App, f: &mut Frame, area: Rect) {
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(area);
    let [fav_area, tasks_area] =
        Layout::vertical([Constraint::Percentage(32), Constraint::Min(3)]).areas(left);
    let [cal_area, today_area] =
        Layout::vertical([Constraint::Percentage(62), Constraint::Min(3)]).areas(right);

    favorites::draw(app, f, fav_area, app.tile == Tile::Favorites);
    tasks::draw(app, f, tasks_area, app.tile == Tile::Tasks);
    calendar::draw(app, f, cal_area, app.tile == Tile::Calendar);
    calendar::draw_today(app, f, today_area);
}
