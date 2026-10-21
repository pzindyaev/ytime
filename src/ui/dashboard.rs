use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::Frame;

use super::{calendar, favorites, tasks};
use crate::app::{App, Tile};

pub fn draw(app: &mut App, f: &mut Frame, area: Rect) {
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(area);
    let [fav_area, tasks_area] =
        Layout::vertical([Constraint::Percentage(32), Constraint::Min(3)]).areas(left);
    let [cal_area, today_area] =
        Layout::vertical([Constraint::Percentage(62), Constraint::Min(3)]).areas(right);

    let fav_focused = app.tile == Tile::Favorites;
    favorites::draw(app, f, fav_area, fav_focused);
    let tasks_focused = app.tile == Tile::Tasks;
    tasks::draw(&mut app.tasks, app.tick, f, tasks_area, tasks_focused);
    calendar::draw(app, f, cal_area, app.tile == Tile::Calendar);
    calendar::draw_today(app, f, today_area);
}
