use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use chrono::{DateTime, Datelike, Local, NaiveDate, Timelike};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc;

use crate::api::models::{Issue, User, WorklogRecord};
use crate::api::{detect_org_header, ApiResult, TrackerClient};
use crate::calendar::{bucket_by_day, fetch_window, MonthData};
use crate::config::{Config, OrgHeader};
use crate::input::TextInput;
use crate::store::{ActiveTimer, PersistedState, RunningTimer, Tokens};
use crate::timer;

pub enum AppEvent {
    Key(KeyEvent),
    Tick,
    Api(ApiMsg),
}

pub enum ApiMsg {
    Detected(ApiResult<(OrgHeader, User)>),
    MyIssues(ApiResult<Vec<Issue>>),
    Search(String, ApiResult<Vec<Issue>>),
    FavSearch(String, ApiResult<Vec<Issue>>),
    Worklog {
        key: String,
        date: NaiveDate,
        result: ApiResult<()>,
    },
    WorklogSearch {
        month: (i32, u32),
        result: ApiResult<Vec<WorklogRecord>>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Main,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tile {
    Favorites,
    Tasks,
    Calendar,
}

pub struct TasksTile {
    pub issues: Vec<Issue>,
    pub filtered: Vec<Issue>,
    pub selected: usize,
    /// First visible row of the list window, kept in step with `selected`
    /// at render time (see `ui::list_scroll`).
    pub scroll: usize,
    pub filter: TextInput,
    pub filter_active: bool,
    /// Active issue-type filter (type key, e.g. `bug`).
    pub type_filter: Option<String>,
    /// Mirrors `Config::hide_closed_tasks`; the display filters honor it so
    /// the setting can flip without a refetch.
    pub hide_closed: bool,
    pub loading: bool,
    pub error: Option<String>,
}

impl TasksTile {
    pub(crate) fn new() -> Self {
        Self {
            issues: Vec::new(),
            filtered: Vec::new(),
            selected: 0,
            scroll: 0,
            filter: TextInput::new(""),
            filter_active: false,
            type_filter: None,
            hide_closed: true,
            loading: false,
            error: None,
        }
    }

    /// Distinct issue types among the loaded issues, most frequent first.
    pub fn available_types(&self) -> Vec<(String, String, usize)> {
        let mut counts: HashMap<String, (String, usize)> = HashMap::new();
        for i in self
            .issues
            .iter()
            .filter(|i| !(self.hide_closed && i.is_closed()))
        {
            if let Some(t) = &i.issue_type {
                if t.key.is_empty() {
                    continue;
                }
                let entry = counts
                    .entry(t.key.clone())
                    .or_insert_with(|| (t.display.clone().unwrap_or_else(|| t.key.clone()), 0));
                entry.1 += 1;
            }
        }
        let mut out: Vec<(String, String, usize)> =
            counts.into_iter().map(|(k, (d, c))| (k, d, c)).collect();
        out.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
        out
    }

    pub fn type_filter_display(&self) -> Option<String> {
        let tf = self.type_filter.as_deref()?;
        self.available_types()
            .into_iter()
            .find(|(k, _, _)| k == tf)
            .map(|(_, d, _)| d)
    }

    pub fn refresh_filter(&mut self) {
        use fuzzy_matcher::skim::SkimMatcherV2;
        use fuzzy_matcher::FuzzyMatcher;
        let matcher = SkimMatcherV2::default();
        let pat = self.filter.value.trim().to_string();
        let type_filter = self.type_filter.clone();
        let hide_closed = self.hide_closed;
        let visible = |i: &Issue| {
            !(hide_closed && i.is_closed())
                && match &type_filter {
                    Some(tf) => i.issue_type.as_ref().is_some_and(|t| t.key == *tf),
                    None => true,
                }
        };
        if pat.is_empty() {
            self.filtered = self.issues.iter().filter(|i| visible(i)).cloned().collect();
        } else {
            let mut scored: Vec<(i64, Issue)> = self
                .issues
                .iter()
                .filter(|i| visible(i))
                .filter_map(|i| {
                    matcher
                        .fuzzy_match(&format!("{} {}", i.key, i.summary), &pat)
                        .map(|s| (s, i.clone()))
                })
                .collect();
            scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
            self.filtered = scored.into_iter().map(|(_, i)| i).collect();
        }
        if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len().saturating_sub(1);
        }
    }
}

pub struct Dropdown {
    pub items: Vec<Issue>,
    pub selected: usize,
    pub loading: bool,
    /// Text at the time of the last remote search completion.
    pub query: String,
}

pub struct LogTile {
    pub task: TextInput,
    /// The day the entry logs (`YYYY-MM-DD`), set when the modal opens from
    /// the calendar.
    pub date: String,
    pub duration: TextInput,
    pub comment: TextInput,
    /// 0 = task, 1 = duration, 2 = comment
    pub focus: usize,
    pub issue: Option<Issue>,
    pub dropdown: Option<Dropdown>,
    pub error: Option<String>,
    pub busy: bool,
    pub last_edit: Option<Instant>,
    pub dirty_search: bool,
}

impl LogTile {
    fn new() -> Self {
        Self {
            task: TextInput::new(""),
            date: String::new(),
            duration: TextInput::new(""),
            comment: TextInput::new(""),
            focus: 0,
            issue: None,
            dropdown: None,
            error: None,
            busy: false,
            last_edit: None,
            dirty_search: false,
        }
    }
}

pub struct CalendarTile {
    pub view: (i32, u32),
    pub cursor: NaiveDate,
    pub months: HashMap<(i32, u32), MonthData>,
    pub loading: HashSet<(i32, u32)>,
    pub error: Option<String>,
}

impl CalendarTile {
    fn new() -> Self {
        let today = Local::now().date_naive();
        Self {
            view: (today.year(), today.month()),
            cursor: today,
            months: HashMap::new(),
            loading: HashSet::new(),
            error: None,
        }
    }
}

pub struct StopReview {
    pub key: String,
    pub summary: String,
    pub description: String,
    pub started_at: DateTime<Local>,
    pub posting: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputTarget {
    ClientId,
    OrgId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmAction {
    Logout,
}

pub enum Modal {
    Help,
    QuitConfirm,
    Resume(ActiveTimer),
    StopReview(StopReview),
    Describe {
        issue: Issue,
        input: TextInput,
    },
    Input {
        title: String,
        input: TextInput,
        target: InputTarget,
    },
    DayDetail {
        date: NaiveDate,
    },
    /// Retrospective log form, opened from the calendar; state lives in
    /// `App::log`.
    LogEntry,
    /// Add-a-favorite search; dropdown state lives in `App::fav_dd`.
    FavSearch {
        input: TextInput,
    },
    TypeFilter {
        selected: usize,
    },
    Confirm {
        msg: String,
        action: ConfirmAction,
    },
    Error {
        title: String,
        msg: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ModalKind {
    Help,
    QuitConfirm,
    Resume,
    StopReview,
    Describe,
    Input,
    DayDetail,
    LogEntry,
    FavSearch,
    TypeFilter,
    Confirm,
    Error,
}

fn modal_kind(m: &Modal) -> ModalKind {
    match m {
        Modal::Help => ModalKind::Help,
        Modal::QuitConfirm => ModalKind::QuitConfirm,
        Modal::Resume(_) => ModalKind::Resume,
        Modal::StopReview(_) => ModalKind::StopReview,
        Modal::Describe { .. } => ModalKind::Describe,
        Modal::Input { .. } => ModalKind::Input,
        Modal::DayDetail { .. } => ModalKind::DayDetail,
        Modal::LogEntry => ModalKind::LogEntry,
        Modal::FavSearch { .. } => ModalKind::FavSearch,
        Modal::TypeFilter { .. } => ModalKind::TypeFilter,
        Modal::Confirm { .. } => ModalKind::Confirm,
        Modal::Error { .. } => ModalKind::Error,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardStep {
    ClientId,
    OrgId,
    Token,
}

pub struct OnboardState {
    pub step: OnboardStep,
    pub input: TextInput,
    pub error: Option<String>,
}

pub struct App {
    pub cfg: Config,
    pub state: PersistedState,
    pub client: Option<TrackerClient>,
    pub me: Option<User>,
    pub tab: Tab,
    pub tile: Tile,
    pub tasks: TasksTile,
    pub cal: CalendarTile,
    pub log: LogTile,
    pub settings_sel: usize,
    /// Selection in the favorites tile (the list itself is `state.favorites`).
    pub fav_sel: usize,
    /// Scroll offset of the favorites list window (see `ui::list_scroll`).
    pub fav_scroll: usize,
    /// Dropdown for the add-favorite search modal.
    pub fav_dd: Option<Dropdown>,
    pub fav_dirty: bool,
    pub fav_last_edit: Option<Instant>,
    pub timer: Option<RunningTimer>,
    pub modals: Vec<Modal>,
    pub toast: Option<(ToastKind, String, Instant)>,
    pub quit: bool,
    pub quit_after_log: bool,
    pub tick: usize,
    pub auth: Option<OnboardState>,
    pub detecting: bool,
    pub api_tx: mpsc::Sender<AppEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

const TOAST_TTL: Duration = Duration::from_secs(5);

/// How many local fuzzy matches the task dropdowns show before remote
/// (Tracker) results — enough for a quick pick, short enough that remote
/// results stay within the first screen.
const LOCAL_MATCH_CAP: usize = 6;

impl App {
    pub fn new(cfg: Config, state: PersistedState, api_tx: mpsc::Sender<AppEvent>) -> Self {
        let mut tasks = TasksTile::new();
        tasks.hide_closed = cfg.hide_closed_tasks;
        Self {
            cfg,
            state,
            client: None,
            me: None,
            tab: Tab::Main,
            tile: Tile::Tasks,
            tasks,
            cal: CalendarTile::new(),
            log: LogTile::new(),
            settings_sel: 0,
            fav_sel: 0,
            fav_scroll: 0,
            fav_dd: None,
            fav_dirty: false,
            fav_last_edit: None,
            timer: None,
            modals: Vec::new(),
            toast: None,
            quit: false,
            quit_after_log: false,
            tick: 0,
            auth: None,
            detecting: false,
            api_tx,
        }
    }

    /// Initial data kicks: resume prompt, onboarding or org detection.
    pub fn boot(&mut self) {
        if let Some(t) = self.state.active_timer.clone() {
            self.modals.push(Modal::Resume(t));
        }
        let has_token = self
            .state
            .tokens
            .as_ref()
            .is_some_and(|t| !t.access_token.is_empty());
        if !has_token || self.cfg.org_id.as_deref().is_none_or(str::is_empty) {
            self.open_onboarding();
        } else {
            self.spawn_detect();
        }
    }

    fn open_onboarding(&mut self) {
        let step = if self.cfg.client_id.as_deref().is_none_or(str::is_empty) {
            OnboardStep::ClientId
        } else if self.cfg.org_id.as_deref().is_none_or(str::is_empty) {
            OnboardStep::OrgId
        } else {
            OnboardStep::Token
        };
        self.auth = Some(OnboardState {
            step,
            input: TextInput::new(""),
            error: None,
        });
    }

    fn spawn_detect(&mut self) {
        let Some(tokens) = self.state.tokens.clone() else {
            return;
        };
        let Some(org_id) = self.cfg.org_id.clone() else {
            return;
        };
        if org_id.is_empty() || tokens.access_token.is_empty() {
            return;
        }
        self.detecting = true;
        let tx = self.api_tx.clone();
        let cfg = self.cfg.clone();
        tokio::spawn(async move {
            let res = detect_org_header(&cfg, tokens.access_token).await;
            let _ = tx.send(AppEvent::Api(ApiMsg::Detected(res))).await;
        });
    }

    fn spawn_my_issues(&mut self) {
        let Some(client) = self.client.clone() else {
            return;
        };
        self.tasks.loading = true;
        let tx = self.api_tx.clone();
        tokio::spawn(async move {
            let res = client.issues("assignee:me()", 5).await;
            let _ = tx.send(AppEvent::Api(ApiMsg::MyIssues(res))).await;
        });
    }

    fn fetch_month(&mut self, year: i32, month: u32, force: bool) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let Some(me) = self.me.clone() else {
            return;
        };
        let key = (year, month);
        if self.cal.loading.contains(&key) {
            return;
        }
        if self.cal.months.contains_key(&key) && !force {
            return;
        }
        if force {
            self.cal.months.remove(&key);
        }
        self.cal.loading.insert(key);
        let (from, to) = fetch_window(year, month);
        let tx = self.api_tx.clone();
        tokio::spawn(async move {
            let res = client.worklog_search(&me.login, from, to).await;
            let _ = tx
                .send(AppEvent::Api(ApiMsg::WorklogSearch {
                    month: key,
                    result: res,
                }))
                .await;
        });
    }

    fn ensure_month(&mut self) {
        let (y, m) = self.cal.view;
        self.fetch_month(y, m, false);
    }

    /// Contextual key hints for the bottom legend, most useful first (the
    /// tail is what gets dropped on narrow terminals). The onboarding
    /// overlay and the topmost modal win; then the focused tile/tab.
    pub fn key_hints(&self) -> Vec<(&'static str, &'static str)> {
        if self.auth.is_some() {
            return vec![("Enter", "continue"), ("Esc", "skip")];
        }
        if let Some(modal) = self.modals.last() {
            return match modal {
                Modal::Help | Modal::Error { .. } => vec![("Esc", "close"), ("q", "close")],
                Modal::QuitConfirm => vec![
                    ("Enter", "log & quit"),
                    ("d", "discard timer"),
                    ("Esc", "stay"),
                ],
                Modal::Resume(_) => vec![
                    ("r", "resume"),
                    ("Enter", "stop & log"),
                    ("d", "discard"),
                    ("Esc", "later"),
                ],
                Modal::StopReview(r) if r.posting => vec![("…", "posting")],
                Modal::StopReview(_) => {
                    vec![("Enter", "log"), ("d", "discard"), ("Esc", "keep running")]
                }
                Modal::Describe { .. } => vec![("Enter", "start timer"), ("Esc", "cancel")],
                Modal::Input { .. } => vec![("Enter", "save"), ("Esc", "cancel")],
                Modal::DayDetail { .. } => vec![("n", "log time here"), ("Esc", "close")],
                Modal::TypeFilter { .. } => {
                    vec![("↑/↓", "move"), ("Enter", "apply"), ("Esc", "cancel")]
                }
                Modal::LogEntry => {
                    if self.log.dropdown.is_some() {
                        vec![("↑/↓", "pick"), ("Enter", "select"), ("Esc", "close list")]
                    } else {
                        vec![
                            ("↑/↓", "fields"),
                            ("Enter", "submit on Note"),
                            ("Esc", "close"),
                        ]
                    }
                }
                Modal::FavSearch { .. } => {
                    vec![("↑/↓", "pick"), ("Enter", "add"), ("Esc", "close")]
                }
                Modal::Confirm { .. } => vec![("Enter", "yes"), ("Esc", "no")],
            };
        }
        let mut hints: Vec<(&'static str, &'static str)> = match (self.tab, self.tile) {
            (Tab::Main, Tile::Favorites) => vec![
                ("⏎", "track"),
                ("l", "log retro"),
                ("a", "add favorite"),
                ("d", "remove"),
            ],
            (Tab::Main, Tile::Tasks) if self.tasks.filter_active => {
                vec![("Enter", "apply filter"), ("Esc", "close filter")]
            }
            (Tab::Main, Tile::Tasks) => vec![
                ("⏎", "start timer"),
                ("/", "filter"),
                ("t", "type"),
                ("r", "refresh"),
            ],
            (Tab::Main, Tile::Calendar) => vec![
                ("hjkl", "day"),
                ("[/]", "month"),
                ("t", "today"),
                ("n", "log time"),
                ("⏎", "day details"),
                ("r", "reload"),
            ],
            (Tab::Settings, _) => vec![("⏎", "edit"), ("j/k", "move")],
        };
        if self.timer.is_some() {
            hints.push(("s", "stop timer"));
        }
        hints.push(("1/2/3", "tile"));
        hints.push(("Shift+1/2", "tab"));
        hints.push(("?", "keys"));
        hints.push(("q", "quit"));
        hints
    }

    pub fn toast(&mut self, kind: ToastKind, msg: impl Into<String>) {
        self.toast = Some((kind, msg.into(), Instant::now()));
    }

    fn save_state(&mut self) {
        if let Err(e) = self.state.save() {
            let msg = format!("state save failed: {e}");
            self.toast = Some((ToastKind::Error, msg, Instant::now()));
        }
    }

    fn save_cfg(&mut self) {
        if let Err(e) = self.cfg.save() {
            let msg = format!("config save failed: {e}");
            self.toast = Some((ToastKind::Error, msg, Instant::now()));
        }
    }

    /// Rounded amount the current rounding config would log for `elapsed`.
    pub fn rounded_for(&self, elapsed: Duration) -> Duration {
        timer::round_duration(elapsed, self.cfg.rounding.minutes, self.cfg.rounding.mode)
    }

    fn input_context(&self) -> bool {
        match (self.tab, self.tile) {
            (Tab::Main, Tile::Tasks) => self.tasks.filter_active,
            _ => false,
        }
    }

    fn stop_timer_modal(&mut self) {
        let Some(t) = &self.timer else {
            return;
        };
        self.modals.push(Modal::StopReview(StopReview {
            key: t.issue.key.clone(),
            summary: t.issue.summary.clone(),
            description: t.description.clone(),
            started_at: t.started_at,
            posting: false,
        }));
    }

    fn clear_timer(&mut self) {
        self.timer = None;
        self.state.active_timer = None;
        self.save_state();
    }

    pub async fn run(
        mut self,
        mut rx: mpsc::Receiver<AppEvent>,
        mut terminal: ratatui::DefaultTerminal,
    ) -> anyhow::Result<()> {
        self.boot();
        while !self.quit {
            terminal.draw(|f| crate::ui::draw(&mut self, f))?;
            match rx.recv().await {
                Some(AppEvent::Key(k)) => self.on_key(k),
                Some(AppEvent::Tick) => self.on_tick(),
                Some(AppEvent::Api(m)) => self.on_api(m).await,
                None => break,
            }
        }
        Ok(())
    }

    fn on_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        if let Some((_, _, at)) = self.toast {
            if at.elapsed() > TOAST_TTL {
                self.toast = None;
            }
        }
        // debounced remote search for the log task dropdown
        let fire = self.log.dirty_search
            && self
                .log
                .last_edit
                .is_some_and(|t| t.elapsed() > Duration::from_millis(400))
            && self.log.task.value.trim().len() >= 3;
        if fire {
            self.log.dirty_search = false;
            self.spawn_issue_search();
        }
        // debounced remote search for the add-favorite dropdown
        let fav_fire = self.fav_dirty
            && self
                .fav_last_edit
                .is_some_and(|t| t.elapsed() > Duration::from_millis(400));
        if fav_fire {
            let q = match self.modals.last() {
                Some(Modal::FavSearch { input }) => input.value.trim().to_string(),
                _ => String::new(),
            };
            if q.len() >= 3 {
                self.fav_dirty = false;
                self.spawn_fav_search(&q);
            }
        }
    }

    /// Local half of the add-favorite dropdown: fuzzy matches over the
    /// favorites themselves plus the loaded issues, deduplicated by key.
    fn rebuild_fav_dropdown(&mut self) {
        use fuzzy_matcher::skim::SkimMatcherV2;
        use fuzzy_matcher::FuzzyMatcher;
        let q = match self.modals.last() {
            Some(Modal::FavSearch { input }) => input.value.trim().to_string(),
            _ => return,
        };
        if q.is_empty() {
            self.fav_dd = None;
            return;
        }
        let matcher = SkimMatcherV2::default();
        let mut pool: Vec<Issue> = self.state.favorites.clone();
        for i in &self.tasks.issues {
            if !pool.iter().any(|p| p.key == i.key) {
                pool.push(i.clone());
            }
        }
        let mut items: Vec<Issue> = pool
            .iter()
            .filter_map(|i| {
                matcher
                    .fuzzy_match(&format!("{} {}", i.key, i.summary), &q)
                    .map(|_| i.clone())
            })
            .collect();
        // local matches are a fast preview — keep them few so remote results
        // surface within the first screen of the dropdown
        items.truncate(LOCAL_MATCH_CAP);
        if let Some(dd) = &mut self.fav_dd {
            let selected_key = dd.items.get(dd.selected).map(|i| i.key.clone());
            if dd.query == q {
                for remote in dd.items.clone() {
                    if !items.iter().any(|i| i.key == remote.key) {
                        items.push(remote);
                    }
                }
            }
            dd.items = items;
            dd.selected = selected_key
                .and_then(|k| dd.items.iter().position(|i| i.key == k))
                .unwrap_or(0);
            if dd.selected >= dd.items.len() {
                dd.selected = dd.items.len().saturating_sub(1);
            }
        } else {
            self.fav_dd = Some(Dropdown {
                items,
                selected: 0,
                loading: false,
                query: String::new(),
            });
        }
    }

    fn spawn_fav_search(&mut self, q: &str) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let q = q.to_string();
        if q.is_empty() {
            return;
        }
        if let Some(dd) = &mut self.fav_dd {
            dd.loading = true;
            dd.query = q.clone();
        }
        let tx = self.api_tx.clone();
        tokio::spawn(async move {
            let res = client.issues(&q, 1).await;
            let _ = tx.send(AppEvent::Api(ApiMsg::FavSearch(q, res))).await;
        });
    }

    fn spawn_issue_search(&mut self) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let q = self.log.task.value.trim().to_string();
        if q.is_empty() {
            return;
        }
        if let Some(dd) = &mut self.log.dropdown {
            dd.loading = true;
            dd.query = q.clone();
        }
        let tx = self.api_tx.clone();
        tokio::spawn(async move {
            let res = client.issues(&q, 1).await;
            let _ = tx.send(AppEvent::Api(ApiMsg::Search(q, res))).await;
        });
    }

    fn rebuild_dropdown(&mut self) {
        use fuzzy_matcher::skim::SkimMatcherV2;
        use fuzzy_matcher::FuzzyMatcher;
        let matcher = SkimMatcherV2::default();
        let q = self.log.task.value.trim().to_string();
        if q.is_empty() {
            self.log.dropdown = None;
            return;
        }
        let mut items: Vec<Issue> = {
            let mut scored: Vec<(i64, Issue)> = self
                .tasks
                .issues
                .iter()
                .filter_map(|i| {
                    matcher
                        .fuzzy_match(&format!("{} {}", i.key, i.summary), &q)
                        .map(|s| (s, i.clone()))
                })
                .collect();
            scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
            let mut local: Vec<Issue> = scored.into_iter().map(|(_, i)| i).collect();
            // keep the local preview short so remote results are reachable
            local.truncate(LOCAL_MATCH_CAP);
            local
        };
        if let Some(dd) = &mut self.log.dropdown {
            let selected_key = dd.items.get(dd.selected).map(|i| i.key.clone());
            // keep remote results only while the query still matches them
            if dd.query == q {
                for remote in dd.items.clone() {
                    if !items.iter().any(|i| i.key == remote.key) {
                        items.push(remote);
                    }
                }
            }
            dd.items = items;
            dd.selected = selected_key
                .and_then(|k| dd.items.iter().position(|i| i.key == k))
                .unwrap_or(0);
            if dd.selected >= dd.items.len() {
                dd.selected = dd.items.len().saturating_sub(1);
            }
        } else {
            self.log.dropdown = Some(Dropdown {
                items,
                selected: 0,
                loading: false,
                query: String::new(),
            });
        }
    }

    // ----- key handling -----

    fn on_key(&mut self, key: KeyEvent) {
        // last-resort exit, works above any modal or onboarding state
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c' | 'C'))
        {
            self.quit = true;
            return;
        }
        // the onboarding overlay is drawn on top of modals, so it also
        // receives keys first
        if self.auth.is_some() {
            self.auth_key(key);
            return;
        }
        if !self.modals.is_empty() {
            self.modal_key(key);
            return;
        }
        // always-safe globals
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('s' | 'S') => return self.stop_timer_modal(),
                KeyCode::Char('q' | 'Q') => return self.request_quit(),
                _ => {}
            }
        }
        match key.code {
            KeyCode::Tab => return self.cycle_focus(1),
            KeyCode::BackTab => return self.cycle_focus(-1),
            _ => {}
        }
        if !self.input_context() {
            let shifted = key.modifiers.contains(KeyModifiers::SHIFT);
            match key.code {
                KeyCode::Char('s') => return self.stop_timer_modal(),
                KeyCode::Char('q') => return self.request_quit(),
                KeyCode::Char('?') => return self.modals.push(Modal::Help),
                // a digit selects a tile, Shift+digit selects a tab
                KeyCode::Char(c) if c.is_ascii_digit() => {
                    let idx = c.to_digit(10).unwrap_or(0) as usize;
                    if shifted {
                        match idx {
                            1 => self.tab = Tab::Main,
                            2 => self.tab = Tab::Settings,
                            _ => {}
                        }
                    } else if let Some(tile) = match idx {
                        1 => Some(Tile::Favorites),
                        2 => Some(Tile::Tasks),
                        3 => Some(Tile::Calendar),
                        _ => None,
                    } {
                        self.tab = Tab::Main;
                        self.tile = tile;
                    }
                    return;
                }
                // latin layouts send these for Shift+1 / Shift+2
                KeyCode::Char('!') => {
                    self.tab = Tab::Main;
                    return;
                }
                KeyCode::Char('@') => {
                    self.tab = Tab::Settings;
                    return;
                }
                KeyCode::Esc => {
                    self.tasks.filter_active = false;
                    self.log.dropdown = None;
                    return;
                }
                _ => {}
            }
        }
        match self.tab {
            Tab::Main => self.main_key(key),
            Tab::Settings => self.settings_key(key),
        }
    }

    fn request_quit(&mut self) {
        if self.timer.is_some() {
            self.modals.push(Modal::QuitConfirm);
        } else {
            self.quit = true;
        }
    }

    fn cycle_focus(&mut self, dir: i32) {
        // linear cycle: Favorites → Tasks → Calendar → Settings
        let idx = match (self.tab, self.tile) {
            (Tab::Main, Tile::Favorites) => 0,
            (Tab::Main, Tile::Tasks) => 1,
            (Tab::Main, Tile::Calendar) => 2,
            (Tab::Settings, _) => 3,
        };
        let next = (idx + dir).rem_euclid(4) as usize;
        match next {
            0 => {
                self.tab = Tab::Main;
                self.tile = Tile::Favorites;
            }
            1 => {
                self.tab = Tab::Main;
                self.tile = Tile::Tasks;
            }
            2 => {
                self.tab = Tab::Main;
                self.tile = Tile::Calendar;
            }
            _ => self.tab = Tab::Settings,
        }
    }

    fn main_key(&mut self, key: KeyEvent) {
        match self.tile {
            Tile::Favorites => self.favs_key(key),
            Tile::Tasks => self.tasks_key(key),
            Tile::Calendar => self.cal_key(key),
        }
    }

    fn favs_key(&mut self, key: KeyEvent) {
        let len = self.state.favorites.len();
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.fav_sel = (self.fav_sel + 1).min(len.saturating_sub(1));
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.fav_sel = self.fav_sel.saturating_sub(1);
            }
            KeyCode::Char('g') | KeyCode::Home => self.fav_sel = 0,
            KeyCode::Char('G') | KeyCode::End => {
                self.fav_sel = len.saturating_sub(1);
            }
            // live timer, same flow as the tasks tile
            KeyCode::Enter => {
                if let Some(issue) = self.state.favorites.get(self.fav_sel).cloned() {
                    if let Some(t) = &self.timer {
                        self.modals.push(Modal::Error {
                            title: "Timer already running".into(),
                            msg: format!("Stop the current timer for {} first (s).", t.issue.key),
                        });
                    } else {
                        self.modals.push(Modal::Describe {
                            issue,
                            input: TextInput::new(""),
                        });
                    }
                }
            }
            // retrospective entry on the calendar's selected day
            KeyCode::Char('l') => {
                if let Some(issue) = self.state.favorites.get(self.fav_sel).cloned() {
                    let date = self.cal.cursor;
                    self.open_log_modal(date);
                    self.log.issue = Some(issue);
                    self.log.focus = 1; // duration — the task is already set
                }
            }
            KeyCode::Char('a') => {
                self.fav_dd = None;
                self.fav_dirty = false;
                self.modals.push(Modal::FavSearch {
                    input: TextInput::new(""),
                });
            }
            KeyCode::Char('d') if self.fav_sel < len => {
                let removed = self.state.favorites.remove(self.fav_sel);
                if self.fav_sel >= self.state.favorites.len() {
                    self.fav_sel = self.state.favorites.len().saturating_sub(1);
                }
                self.save_state();
                self.toast(
                    ToastKind::Info,
                    format!("removed {} from favorites", removed.key),
                );
            }
            _ => {}
        }
    }

    fn tasks_key(&mut self, key: KeyEvent) {
        if self.tasks.filter_active {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => self.tasks.filter_active = false,
                _ => {
                    self.tasks.filter.handle_key(key);
                    self.tasks.refresh_filter();
                }
            }
            return;
        }
        match key.code {
            KeyCode::Char('/') => {
                self.tasks.filter_active = true;
            }
            KeyCode::Char('t') => {
                let selected = self
                    .tasks
                    .type_filter
                    .as_deref()
                    .and_then(|tf| {
                        self.tasks
                            .available_types()
                            .iter()
                            .position(|(k, _, _)| k == tf)
                    })
                    .map_or(0, |p| p + 1); // +1: "all types" is row 0
                self.modals.push(Modal::TypeFilter { selected });
            }
            KeyCode::Char('r') => self.spawn_my_issues(),
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.tasks.filtered.is_empty() {
                    self.tasks.selected =
                        (self.tasks.selected + 1).min(self.tasks.filtered.len() - 1);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.tasks.selected = self.tasks.selected.saturating_sub(1);
            }
            KeyCode::Char('g') | KeyCode::Home => self.tasks.selected = 0,
            KeyCode::Char('G') | KeyCode::End => {
                self.tasks.selected = self.tasks.filtered.len().saturating_sub(1);
            }
            KeyCode::Enter => {
                if let Some(issue) = self.tasks.filtered.get(self.tasks.selected).cloned() {
                    if let Some(t) = &self.timer {
                        self.modals.push(Modal::Error {
                            title: "Timer already running".into(),
                            msg: format!("Stop the current timer for {} first (s).", t.issue.key),
                        });
                    } else {
                        self.modals.push(Modal::Describe {
                            issue,
                            input: TextInput::new(""),
                        });
                    }
                }
            }
            _ => {}
        }
    }

    fn move_cursor_days(&mut self, days: i64) {
        let Some(delta) = chrono::Duration::try_days(days) else {
            return;
        };
        if let Some(next) = self.cal.cursor.checked_add_signed(delta) {
            self.cal.cursor = next;
            self.cal.view = (next.year(), next.month());
            self.ensure_month();
        }
    }

    fn cal_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('h') | KeyCode::Left => self.move_cursor_days(-1),
            KeyCode::Char('l') | KeyCode::Right => self.move_cursor_days(1),
            KeyCode::Char('j') | KeyCode::Down => self.move_cursor_days(7),
            KeyCode::Char('k') | KeyCode::Up => self.move_cursor_days(-7),
            KeyCode::Char('[') => self.shift_month(-1),
            KeyCode::Char(']') => self.shift_month(1),
            KeyCode::Char('t') => {
                let today = Local::now().date_naive();
                self.cal.cursor = today;
                self.cal.view = (today.year(), today.month());
                self.ensure_month();
            }
            KeyCode::Char('r') => {
                let v = self.cal.view;
                self.fetch_month(v.0, v.1, true);
            }
            KeyCode::Char('n') => {
                let date = self.cal.cursor;
                self.open_log_modal(date);
            }
            KeyCode::Enter => {
                let date = self.cal.cursor;
                self.modals.push(Modal::DayDetail { date });
            }
            _ => {}
        }
    }

    /// Opens the retrospective log-entry form for `date`; field values from
    /// a previous entry are kept to make logging several days in a row easy.
    fn open_log_modal(&mut self, date: NaiveDate) {
        self.log.date = date.format("%Y-%m-%d").to_string();
        if self.log.focus > 2 {
            self.log.focus = 0;
        }
        self.log.dropdown = None;
        self.log.error = None;
        self.modals.push(Modal::LogEntry);
    }

    fn shift_month(&mut self, delta: i32) {
        let (y, m) = self.cal.view;
        let mut year = y;
        let mut month = m as i32 + delta;
        while month < 1 {
            month += 12;
            year -= 1;
        }
        while month > 12 {
            month -= 12;
            year += 1;
        }
        self.cal.view = (year, month as u32);
        let day = self
            .cal
            .cursor
            .day()
            .min(crate::calendar::last_of_month(year, month as u32).day());
        if let Some(d) = NaiveDate::from_ymd_opt(year, month as u32, day) {
            self.cal.cursor = d;
        }
        self.ensure_month();
    }

    fn log_key(&mut self, key: KeyEvent) {
        // dropdown navigation takes precedence while it is open
        if self.log.dropdown.is_some() {
            let handled = match key.code {
                KeyCode::Down => {
                    if let Some(d) = self.log.dropdown.as_mut() {
                        d.selected = (d.selected + 1).min(d.items.len().saturating_sub(1));
                    }
                    true
                }
                KeyCode::Up => {
                    if let Some(d) = self.log.dropdown.as_mut() {
                        d.selected = d.selected.saturating_sub(1);
                    }
                    true
                }
                KeyCode::Enter => {
                    let picked = self
                        .log
                        .dropdown
                        .as_ref()
                        .and_then(|d| d.items.get(d.selected).cloned());
                    if let Some(issue) = picked {
                        self.log.issue = Some(issue.clone());
                        self.log.task = TextInput::new(issue.title());
                        self.log.dropdown = None;
                        self.log.focus = 1;
                    }
                    true
                }
                KeyCode::Esc => {
                    self.log.dropdown = None;
                    true
                }
                _ => false,
            };
            if handled {
                return;
            }
        }
        match key.code {
            KeyCode::Esc => {
                // close the modal, keep field values for the next entry
                self.modals.pop();
            }
            KeyCode::Up => {
                self.log.focus = (self.log.focus + 2) % 3;
            }
            KeyCode::Down | KeyCode::Enter => {
                if self.log.focus == 2 {
                    self.try_submit_log();
                } else {
                    self.log.focus += 1;
                }
            }
            _ => {
                let focus = self.log.focus;
                if let Some(input) = match focus {
                    0 => Some(&mut self.log.task),
                    1 => Some(&mut self.log.duration),
                    _ => Some(&mut self.log.comment),
                } {
                    let is_char = matches!(key.code, KeyCode::Char(_));
                    input.handle_key(key);
                    if focus == 0 && is_char {
                        self.log.issue = None;
                        self.log.dirty_search = true;
                        self.log.last_edit = Some(Instant::now());
                        self.rebuild_dropdown();
                    }
                }
            }
        }
    }

    fn try_submit_log(&mut self) {
        if self.log.busy {
            return;
        }
        let Some(issue) = self.log.issue.clone() else {
            self.log.error = Some("pick a task from the dropdown".into());
            self.log.focus = 0;
            return;
        };
        // the date is set by the calendar when the modal opens
        let (date, hh, mm) = match parse_date_field(&self.log.date) {
            Some(v) => v,
            None => {
                self.log.error = Some("bad date — reopen from the calendar".into());
                return;
            }
        };
        let Some(dur) = timer::parse_duration_input(&self.log.duration.value) else {
            self.log.error = Some("bad duration (e.g. 1h30m, 45m, 1.5h, 1:30)".into());
            self.log.focus = 1;
            return;
        };
        if dur.is_zero() {
            self.log.error = Some("duration must be positive".into());
            self.log.focus = 1;
            return;
        }
        if dur > timer::MAX_ENTRY {
            self.log.error = Some("duration over 31 days looks like a typo".into());
            self.log.focus = 1;
            return;
        }
        let Some(naive_start) = date.and_hms_opt(hh, mm, 0) else {
            self.log.error = Some("bad time".into());
            return;
        };
        let Some(start) = naive_start.and_local_timezone(Local).single() else {
            self.log.error = Some("bad time".into());
            return;
        };
        let comment = self.log.comment.value.trim().to_string();
        let Some(client) = self.client.clone() else {
            self.log.error = Some("not connected".into());
            return;
        };
        self.log.error = None;
        self.log.busy = true;
        let key = issue.key.clone();
        let tx = self.api_tx.clone();
        tokio::spawn(async move {
            let res = client.add_worklog(&key, start, dur, &comment).await;
            let _ = tx
                .send(AppEvent::Api(ApiMsg::Worklog {
                    key,
                    date: start.date_naive(),
                    result: res,
                }))
                .await;
        });
    }

    fn settings_key(&mut self, key: KeyEvent) {
        const ROWS: usize = 8;
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.settings_sel = (self.settings_sel + 1).min(ROWS - 1);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.settings_sel = self.settings_sel.saturating_sub(1);
            }
            KeyCode::Enter => self.activate_setting(),
            _ => {}
        }
    }

    fn activate_setting(&mut self) {
        match self.settings_sel {
            0 => self.modals.push(Modal::Input {
                title: "OAuth client_id".into(),
                input: TextInput::new(self.cfg.client_id.clone().unwrap_or_default()),
                target: InputTarget::ClientId,
            }),
            1 => self.modals.push(Modal::Input {
                title: "Organization id".into(),
                input: TextInput::new(self.cfg.org_id.clone().unwrap_or_default()),
                target: InputTarget::OrgId,
            }),
            2 => {
                self.cfg.org_header = match self.cfg.org_header {
                    None => Some(OrgHeader::XOrgId),
                    Some(OrgHeader::XOrgId) => Some(OrgHeader::XCloudOrgId),
                    Some(OrgHeader::XCloudOrgId) => None,
                };
                self.save_cfg();
                self.reconnect();
            }
            3 => {
                self.cfg.rounding.cycle_unit();
                self.save_cfg();
            }
            4 => {
                self.cfg.rounding.mode = self.cfg.rounding.mode.cycle();
                self.save_cfg();
            }
            5 => {
                self.cfg.hide_closed_tasks = !self.cfg.hide_closed_tasks;
                self.tasks.hide_closed = self.cfg.hide_closed_tasks;
                self.tasks.refresh_filter();
                self.save_cfg();
            }
            6 => self.open_onboarding(),
            _ => self.modals.push(Modal::Confirm {
                msg: "Delete the stored OAuth token and disconnect?".into(),
                action: ConfirmAction::Logout,
            }),
        }
    }

    fn reconnect(&mut self) {
        if self.state.tokens.is_some() && self.cfg.org_id.as_deref().is_some_and(|s| !s.is_empty())
        {
            self.client = None;
            self.me = None;
            self.tasks.issues.clear();
            self.tasks.filtered.clear();
            self.cal.months.clear();
            self.spawn_detect();
        }
    }

    // ----- modal keys -----

    fn modal_key(&mut self, key: KeyEvent) {
        let Some(modal) = self.modals.last() else {
            return;
        };
        match modal_kind(modal) {
            ModalKind::Help | ModalKind::Error => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q')) {
                    self.modals.pop();
                }
            }
            ModalKind::QuitConfirm => match key.code {
                KeyCode::Enter => {
                    self.modals.pop();
                    if self.timer.is_some() {
                        self.quit_after_log = true;
                        self.stop_timer_modal();
                    } else {
                        self.quit = true;
                    }
                }
                KeyCode::Char('d') => {
                    self.clear_timer();
                    self.quit = true;
                }
                _ => {
                    self.modals.pop();
                }
            },
            ModalKind::Resume => {
                let Some(Modal::Resume(t)) = self.modals.last() else {
                    return;
                };
                let t = t.clone();
                match key.code {
                    KeyCode::Char('r') => {
                        self.modals.pop();
                        self.timer = Some(RunningTimer {
                            issue: Issue {
                                key: t.issue_key.clone(),
                                summary: t.summary.clone(),
                                ..Default::default()
                            },
                            description: t.description.clone(),
                            started_at: t.started_at,
                        });
                    }
                    KeyCode::Enter => {
                        self.modals.pop();
                        self.modals.push(Modal::StopReview(StopReview {
                            key: t.issue_key,
                            summary: t.summary,
                            description: t.description,
                            started_at: t.started_at,
                            posting: false,
                        }));
                    }
                    KeyCode::Char('d') => {
                        self.clear_timer();
                        self.modals.pop();
                    }
                    KeyCode::Esc => {
                        self.modals.pop();
                        self.toast(
                            ToastKind::Info,
                            "timer kept — it will be offered again on the next launch",
                        );
                    }
                    _ => {}
                }
            }
            ModalKind::StopReview => {
                let posting = match self.modals.last() {
                    Some(Modal::StopReview(r)) => r.posting,
                    _ => false,
                };
                if posting {
                    return;
                }
                match key.code {
                    KeyCode::Enter => self.post_timer_worklog(),
                    KeyCode::Char('d') => {
                        self.clear_timer();
                        self.modals.pop();
                        if self.quit_after_log {
                            self.quit = true;
                        }
                    }
                    _ => {
                        self.modals.pop();
                        self.quit_after_log = false;
                    }
                }
            }
            ModalKind::Describe => match key.code {
                KeyCode::Enter => {
                    let (issue, description) = match self.modals.last() {
                        Some(Modal::Describe { issue, input }) => {
                            (issue.clone(), input.value.trim().to_string())
                        }
                        _ => return,
                    };
                    if description.is_empty() {
                        return;
                    }
                    self.modals.pop();
                    self.start_timer(issue, description);
                }
                KeyCode::Esc => {
                    self.modals.pop();
                }
                _ => {
                    if let Some(Modal::Describe { input, .. }) = self.modals.last_mut() {
                        input.handle_key(key);
                    }
                }
            },
            ModalKind::Input => match key.code {
                KeyCode::Enter => {
                    let (target, value) = match self.modals.last() {
                        Some(Modal::Input { input, target, .. }) => {
                            (*target, input.value.trim().to_string())
                        }
                        _ => return,
                    };
                    self.modals.pop();
                    self.apply_input(target, value);
                }
                KeyCode::Esc => {
                    self.modals.pop();
                }
                _ => {
                    if let Some(Modal::Input { input, .. }) = self.modals.last_mut() {
                        input.handle_key(key);
                    }
                }
            },
            ModalKind::DayDetail => match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.modals.pop();
                }
                KeyCode::Char('n') => {
                    let date = match self.modals.last() {
                        Some(Modal::DayDetail { date }) => *date,
                        _ => return,
                    };
                    self.modals.pop();
                    self.open_log_modal(date);
                }
                _ => {}
            },
            ModalKind::LogEntry => self.log_key(key),
            ModalKind::FavSearch => match key.code {
                KeyCode::Esc => {
                    self.fav_dd = None;
                    self.modals.pop();
                }
                KeyCode::Down => {
                    if let Some(d) = self.fav_dd.as_mut() {
                        d.selected = (d.selected + 1).min(d.items.len().saturating_sub(1));
                    }
                }
                KeyCode::Up => {
                    if let Some(d) = self.fav_dd.as_mut() {
                        d.selected = d.selected.saturating_sub(1);
                    }
                }
                KeyCode::Enter => {
                    let picked = self
                        .fav_dd
                        .as_ref()
                        .and_then(|d| d.items.get(d.selected).cloned());
                    if let Some(issue) = picked {
                        if self.state.favorites.iter().any(|f| f.key == issue.key) {
                            self.toast(
                                ToastKind::Info,
                                format!("{} is already in favorites", issue.key),
                            );
                        } else {
                            self.state.favorites.push(issue.clone());
                            self.save_state();
                            self.toast(ToastKind::Success, format!("added {}", issue.key));
                        }
                    }
                }
                _ => {
                    let is_char = matches!(key.code, KeyCode::Char(_));
                    if let Some(Modal::FavSearch { input }) = self.modals.last_mut() {
                        input.handle_key(key);
                    }
                    if is_char {
                        self.fav_dirty = true;
                        self.fav_last_edit = Some(Instant::now());
                        self.rebuild_fav_dropdown();
                    }
                }
            },
            ModalKind::TypeFilter => {
                let total = self.tasks.available_types().len() + 1;
                match key.code {
                    KeyCode::Char('j') | KeyCode::Down => {
                        if let Some(Modal::TypeFilter { selected }) = self.modals.last_mut() {
                            *selected = (*selected + 1).min(total.saturating_sub(1));
                        }
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        if let Some(Modal::TypeFilter { selected }) = self.modals.last_mut() {
                            *selected = selected.saturating_sub(1);
                        }
                    }
                    KeyCode::Enter => {
                        let sel = match self.modals.last() {
                            Some(Modal::TypeFilter { selected }) => *selected,
                            _ => 0,
                        };
                        self.tasks.type_filter = if sel == 0 {
                            None
                        } else {
                            self.tasks
                                .available_types()
                                .get(sel - 1)
                                .map(|(k, _, _)| k.clone())
                        };
                        self.tasks.refresh_filter();
                        self.modals.pop();
                    }
                    _ => {
                        self.modals.pop();
                    }
                }
            }
            ModalKind::Confirm => match key.code {
                KeyCode::Enter => {
                    let action = match self.modals.last() {
                        Some(Modal::Confirm { action, .. }) => *action,
                        _ => return,
                    };
                    self.modals.pop();
                    if action == ConfirmAction::Logout {
                        self.logout();
                    }
                }
                _ => {
                    self.modals.pop();
                }
            },
        }
    }

    fn apply_input(&mut self, target: InputTarget, value: String) {
        if value.is_empty() {
            return;
        }
        match target {
            InputTarget::ClientId => {
                self.cfg.client_id = Some(value);
                self.save_cfg();
            }
            InputTarget::OrgId => {
                self.cfg.org_id = Some(value);
                self.save_cfg();
                self.reconnect();
            }
        }
    }

    fn logout(&mut self) {
        self.state.tokens = None;
        self.save_state();
        self.client = None;
        self.me = None;
        self.tasks.issues.clear();
        self.tasks.filtered.clear();
        self.cal.months.clear();
        self.toast(ToastKind::Info, "logged out");
        self.open_onboarding();
    }

    fn start_timer(&mut self, issue: Issue, description: String) {
        let started_at = Local::now();
        self.timer = Some(RunningTimer {
            issue: issue.clone(),
            description: description.clone(),
            started_at,
        });
        self.state.active_timer = Some(ActiveTimer {
            issue_key: issue.key.clone(),
            summary: issue.summary,
            description,
            started_at,
        });
        self.save_state();
        self.toast(
            ToastKind::Success,
            format!("tracking {} — press s to stop and log", issue.key),
        );
    }

    fn post_timer_worklog(&mut self) {
        let (key, started, description) = match self.modals.last() {
            Some(Modal::StopReview(r)) => (r.key.clone(), r.started_at, r.description.clone()),
            _ => return,
        };
        let elapsed = (Local::now() - started).to_std().unwrap_or_default();
        let rounded = self.rounded_for(elapsed);
        if rounded.is_zero() {
            self.clear_timer();
            self.modals.pop();
            self.toast(
                ToastKind::Info,
                "rounded to zero with current settings — nothing logged",
            );
            if self.quit_after_log {
                self.quit = true;
            }
            return;
        }
        let Some(client) = self.client.clone() else {
            self.modals.pop();
            self.modals.push(Modal::Error {
                title: "Not connected".into(),
                msg: "The timer keeps running; connect and stop again to log.".into(),
            });
            return;
        };
        if let Some(Modal::StopReview(r)) = self.modals.last_mut() {
            r.posting = true;
        }
        let tx = self.api_tx.clone();
        tokio::spawn(async move {
            let res = client
                .add_worklog(&key, started, rounded, &description)
                .await;
            let _ = tx
                .send(AppEvent::Api(ApiMsg::Worklog {
                    key,
                    date: started.date_naive(),
                    result: res,
                }))
                .await;
        });
    }

    // ----- onboarding keys -----

    fn auth_key(&mut self, key: KeyEvent) {
        let Some(step) = self.auth.as_ref().map(|a| a.step) else {
            return;
        };
        match key.code {
            KeyCode::Enter => {
                let value = self
                    .auth
                    .as_ref()
                    .map(|a| a.input.value.trim().to_string())
                    .unwrap_or_default();
                if value.is_empty() {
                    return;
                }
                match step {
                    OnboardStep::ClientId => {
                        self.cfg.client_id = Some(value);
                        self.save_cfg();
                        if let Some(a) = &mut self.auth {
                            a.step = OnboardStep::OrgId;
                            a.input.clear();
                            a.error = None;
                        }
                    }
                    OnboardStep::OrgId => {
                        self.cfg.org_id = Some(value);
                        self.save_cfg();
                        if let Some(a) = &mut self.auth {
                            a.step = OnboardStep::Token;
                            a.input.clear();
                            a.error = None;
                        }
                    }
                    OnboardStep::Token => {
                        self.state.tokens = Some(Tokens {
                            access_token: value,
                        });
                        self.save_state();
                        self.cfg.org_header = None;
                        self.auth = None;
                        self.spawn_detect();
                        self.toast(ToastKind::Success, "token saved, connecting…");
                    }
                }
            }
            KeyCode::Esc => {
                self.auth = None;
            }
            _ => {
                if let Some(a) = &mut self.auth {
                    a.input.handle_key(key);
                }
            }
        }
    }

    // ----- api results -----

    fn session_expired(&mut self) {
        self.toast(
            ToastKind::Error,
            "session expired — please re-login (Settings → Authorize)",
        );
        self.open_onboarding();
    }

    async fn on_api(&mut self, msg: ApiMsg) {
        match msg {
            ApiMsg::Detected(res) => {
                self.detecting = false;
                match res {
                    Ok((header, user)) => {
                        let token = self
                            .state
                            .tokens
                            .as_ref()
                            .map(|t| t.access_token.clone())
                            .unwrap_or_default();
                        self.cfg.org_header = Some(header);
                        self.save_cfg();
                        let org_id = self.cfg.org_id.clone().unwrap_or_default();
                        self.client = Some(TrackerClient::new(
                            token,
                            org_id,
                            header,
                            self.cfg.api_base.clone(),
                        ));
                        self.me = Some(user);
                        self.spawn_my_issues();
                        self.ensure_month();
                    }
                    Err(e) => {
                        if e.is_unauthorized() {
                            self.session_expired();
                        } else {
                            self.modals.push(Modal::Error {
                                title: "Connection failed".into(),
                                msg: format!(
                                    "{e}\n\nCheck the org id in Settings; also try switching \
                                     the org header (X-Org-Id ↔ X-Cloud-Org-Id)."
                                ),
                            });
                        }
                    }
                }
            }
            ApiMsg::MyIssues(res) => match res {
                Ok(mut issues) => {
                    // closed issues stay in `issues` — hiding is a display
                    // concern (`hide_closed`), so the setting can flip
                    // without a refetch, and the dropdowns' local-match
                    // pool keeps covering them
                    issues.sort_by(|a, b| {
                        let ta = a
                            .updated_at
                            .as_deref()
                            .and_then(crate::api::models::parse_dt);
                        let tb = b
                            .updated_at
                            .as_deref()
                            .and_then(crate::api::models::parse_dt);
                        tb.cmp(&ta)
                    });
                    self.tasks.loading = false;
                    self.tasks.error = None;
                    self.tasks.issues = issues;
                    self.tasks.refresh_filter();
                }
                Err(e) => {
                    self.tasks.loading = false;
                    self.tasks.error = Some(e.to_string());
                    if e.is_unauthorized() {
                        self.session_expired();
                    }
                }
            },
            ApiMsg::Search(q, res) => {
                let still_current = self.log.task.value.trim() == q;
                if let Some(dd) = &mut self.log.dropdown {
                    if still_current && dd.query == q {
                        if let Ok(remote) = res {
                            for issue in remote {
                                if !dd.items.iter().any(|i| i.key == issue.key) {
                                    dd.items.push(issue);
                                }
                            }
                        }
                    }
                    dd.loading = false;
                }
            }
            ApiMsg::FavSearch(q, res) => {
                let still_current = matches!(self.modals.last(),
                    Some(Modal::FavSearch { input }) if input.value.trim() == q);
                if let Some(dd) = &mut self.fav_dd {
                    if still_current && dd.query == q {
                        if let Ok(remote) = res {
                            for issue in remote {
                                if !dd.items.iter().any(|i| i.key == issue.key) {
                                    dd.items.push(issue);
                                }
                            }
                        }
                    }
                    dd.loading = false;
                }
            }
            ApiMsg::Worklog { key, date, result } => {
                let from_stop_modal = matches!(self.modals.last(), Some(Modal::StopReview(_)));
                match result {
                    Ok(()) => {
                        self.log.busy = false;
                        if from_stop_modal {
                            self.modals.pop();
                            self.clear_timer();
                            if self.quit_after_log {
                                self.quit = true;
                            }
                        } else {
                            // manual entry: close the form and clear it
                            if matches!(self.modals.last(), Some(Modal::LogEntry)) {
                                self.modals.pop();
                            }
                            self.log.task = TextInput::new("");
                            self.log.date = String::new();
                            self.log.duration = TextInput::new("");
                            self.log.comment = TextInput::new("");
                            self.log.issue = None;
                            self.log.focus = 0;
                            self.log.error = None;
                        }
                        self.toast(ToastKind::Success, format!("logged to {key}"));
                        self.fetch_month(date.year(), date.month(), true);
                    }
                    Err(e) => {
                        self.log.busy = false;
                        if from_stop_modal {
                            if let Some(Modal::StopReview(r)) = self.modals.last_mut() {
                                r.posting = false;
                            }
                        }
                        if e.is_unauthorized() {
                            self.session_expired();
                        } else {
                            self.modals.push(Modal::Error {
                                title: "Failed to log time".into(),
                                msg: e.to_string(),
                            });
                        }
                    }
                }
            }
            ApiMsg::WorklogSearch { month, result } => {
                self.cal.loading.remove(&month);
                match result {
                    Ok(records) => {
                        self.cal.error = None;
                        let data = bucket_by_day(records);
                        self.cal.months.insert(month, data);
                    }
                    Err(e) => {
                        self.cal.error = Some(e.to_string());
                        if e.is_unauthorized() {
                            self.session_expired();
                        }
                    }
                }
            }
        }
    }
}

/// Accepts `YYYY-MM-DD` / `DD.MM.YYYY`, optional ` HH:MM` suffix.
fn parse_date_field(s: &str) -> Option<(NaiveDate, u32, u32)> {
    use chrono::NaiveTime;
    let s = s.trim();
    let (date_part, time_part) = match s.split_once(' ') {
        Some((d, t)) => (d, Some(t)),
        None => (s, None),
    };
    let date = chrono::NaiveDate::parse_from_str(date_part, "%Y-%m-%d")
        .or_else(|_| chrono::NaiveDate::parse_from_str(date_part, "%d.%m.%Y"))
        .ok()?;
    let (hh, mm) = match time_part {
        Some(t) => {
            let time = NaiveTime::parse_from_str(t, "%H:%M").ok()?;
            (time.hour(), time.minute())
        }
        None => (12, 0),
    };
    Some((date, hh, mm))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let (tx, _rx) = mpsc::channel(8);
        App::new(Config::default(), PersistedState::default(), tx)
    }

    fn key(code: KeyCode, shift: bool) -> KeyEvent {
        let mods = if shift {
            KeyModifiers::SHIFT
        } else {
            KeyModifiers::NONE
        };
        KeyEvent::new(code, mods)
    }

    #[test]
    fn esc_closes_describe_modal_and_typing_keeps_it_open() {
        let mut a = app();
        a.modals.push(Modal::Describe {
            issue: Issue::default(),
            input: TextInput::new(""),
        });
        a.on_key(key(KeyCode::Char('w'), false));
        assert_eq!(a.modals.len(), 1);
        let typed = match a.modals.last() {
            Some(Modal::Describe { input, .. }) => input.value.clone(),
            _ => String::new(),
        };
        assert_eq!(typed, "w");
        a.on_key(key(KeyCode::Esc, false));
        assert!(a.modals.is_empty());
    }

    #[test]
    fn digits_select_tiles_and_shifted_digits_select_tabs() {
        let mut a = app();
        a.on_key(key(KeyCode::Char('2'), false));
        assert_eq!((a.tab, a.tile), (Tab::Main, Tile::Tasks));
        a.on_key(key(KeyCode::Char('1'), false));
        assert_eq!((a.tab, a.tile), (Tab::Main, Tile::Favorites));
        a.on_key(key(KeyCode::Char('3'), false));
        assert_eq!((a.tab, a.tile), (Tab::Main, Tile::Calendar));

        // Shift+digit reports the digit char with SHIFT on some terminals
        a.on_key(key(KeyCode::Char('2'), true));
        assert_eq!(a.tab, Tab::Settings);
        a.on_key(key(KeyCode::Char('1'), true));
        assert_eq!(a.tab, Tab::Main);
        assert_eq!(a.tile, Tile::Calendar); // tile focus kept

        // latin layouts send the shifted symbol instead
        a.on_key(key(KeyCode::Char('@'), false));
        assert_eq!(a.tab, Tab::Settings);
        a.on_key(key(KeyCode::Char('!'), false));
        assert_eq!(a.tab, Tab::Main);

        // out-of-range digits do nothing
        let (tab, tile) = (a.tab, a.tile);
        a.on_key(key(KeyCode::Char('9'), false));
        a.on_key(key(KeyCode::Char('9'), true));
        assert_eq!((a.tab, a.tile), (tab, tile));
    }

    #[test]
    fn date_field_parsing() {
        assert_eq!(
            parse_date_field("2026-10-20"),
            Some((NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(), 12, 0))
        );
        assert_eq!(
            parse_date_field("20.10.2026 09:30"),
            Some((NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(), 9, 30))
        );
        assert_eq!(parse_date_field("garbage"), None);
        assert_eq!(parse_date_field("2026-13-01"), None);
    }

    #[test]
    fn type_filter_combines_with_text_filter() {
        use crate::api::models::StatusRef;
        let mk = |key: &str, type_key: Option<&str>| Issue {
            key: key.to_string(),
            summary: format!("sum {key}"),
            issue_type: type_key.map(|k| StatusRef {
                key: k.to_string(),
                display: Some(k.to_string()),
            }),
            ..Default::default()
        };
        let mut t = TasksTile::new();
        t.issues = vec![
            mk("A", Some("bug")),
            mk("B", Some("task")),
            mk("C", Some("bug")),
            mk("D", None),
        ];

        let types = t.available_types();
        assert_eq!(types.len(), 2);
        assert_eq!(types[0].0, "bug"); // most frequent first
        assert_eq!(types[0].2, 2);

        t.refresh_filter();
        assert_eq!(t.filtered.len(), 4);

        t.type_filter = Some("bug".into());
        t.refresh_filter();
        assert_eq!(
            t.filtered.iter().map(|i| i.key.clone()).collect::<Vec<_>>(),
            ["A", "C"]
        );

        // both filters apply together (AND)
        t.filter = TextInput::new("C");
        t.refresh_filter();
        assert_eq!(t.filtered.len(), 1);
        assert_eq!(t.filtered[0].key, "C");

        t.type_filter = None;
        t.filter = TextInput::new("");
        t.refresh_filter();
        assert_eq!(t.filtered.len(), 4);
    }

    #[test]
    fn hide_closed_setting_controls_visibility() {
        use crate::api::models::StatusRef;
        let mk = |key: &str, status: Option<&str>| Issue {
            key: key.to_string(),
            summary: format!("sum {key}"),
            status: status.map(|s| StatusRef {
                key: s.to_string(),
                display: Some(s.to_string()),
            }),
            ..Default::default()
        };
        let mut t = TasksTile::new();
        t.issues = vec![
            mk("A", Some("open")),
            mk("B", Some("done")),
            mk("C", Some("canceled")),
            mk("D", Some("devDone")),
            mk("E", None),
        ];

        // default: closed hidden from both the list and the type picker
        t.refresh_filter();
        assert_eq!(
            t.filtered.iter().map(|i| i.key.clone()).collect::<Vec<_>>(),
            ["A", "D", "E"]
        );

        // toggle off — everything is back, no refetch needed
        t.hide_closed = false;
        t.refresh_filter();
        assert_eq!(t.filtered.len(), 5);

        // closed issues never take over the selection index
        t.hide_closed = true;
        t.selected = 4;
        t.refresh_filter();
        assert_eq!(t.selected, 2);
    }
}
