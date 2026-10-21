# ytime

A terminal time tracker for [Yandex Tracker](https://tracker.yandex.ru) — start
a timer on one of your issues, or log time retrospectively, and see your logged
hours per day in a month calendar.

Built with Rust + [ratatui](https://ratatui.rs). Keyboard only. The bottom
line is a contextual key legend — bold keys, dim descriptions — that always
shows what the focused tile or dialog can do (`?` opens the full help).

```
+--------------------------------+------------------------+
| My tasks (12)                  | October 2026 · 142h    |
|  KEY-123  Epic  In Prog  summ… |  Mo Tu We Th Fr Sa Su  |
|  KEY-124  Bug   Open     summ… |     1  2  3 ...        |
|  ...                           |  6½ 8h     4h ...      |
|                                +------------------------+
|                                | Today · 6h 30m         |
|                                |  KEY-123  1h30m note…  |
+--------------------------------+------------------------+
| ▶ KEY-123 fixing the flaky test 1:23:45  → will log 1h 30m|
+---------------------------------------------------------+
```

## Features

- **Live timer** — pick one of your assigned issues, describe what you are
  doing, press Enter. A status-bar timer shows elapsed time and the amount
  that will be logged. The timer survives app restarts and crashes (resume is
  offered on the next launch).
- **Rounding** — tracked time is rounded before posting (default: **round up
  to 30m**, so any tracked minute logs as 30m). Unit (1/5/15/30/60m) and mode
  (up / nearest / down) are configurable in Settings.
- **Retrospective logging** — move to a day on the calendar and press `n`:
  search any task (local fuzzy + remote search), enter a duration (`1h30m`,
  `45m`, `1.5h`, `1:30`, bare number = hours) and a note. Field values are
  kept between entries, so logging several days in a row is quick.
- **Calendar** — hours logged per day with a per-day breakdown popup, a
  "today" summary, and the log form right on the selected day.
- **OAuth 2.0** — Yandex device authorization flow in-app (plus manual token
  paste and automatic token refresh).

## Setup

### 1. Register an OAuth app (once)

1. Open <https://oauth.yandex.ru/client/new> and log in.
2. Name the app (e.g. `ytime`).
3. Platform: **Устройства / Devices** (device authorization code flow).
   No redirect URI is needed.
4. Submit — you'll get an application ID (`client_id`) and a **secret**
   (`client_secret`, shown as «Секрет»). Yandex requires the secret when
   exchanging the device code for a token.

The first run of `ytime` walks you through this and asks for the `client_id`,
the `client_secret`, your organization id (Tracker → **Administration →
Organizations → ID**), and then opens the device flow: open the shown URL,
enter the shown code, allow access. That's it.

> **Note for org admins**: access to the Tracker API via OAuth may require
> the app to be allowed by your organization's policy. If the connection
> fails with a permissions error, check the org's OAuth app restrictions.

### 2. Build and run

```sh
cargo run            # or: cargo build --release && ./target/release/ytime
```

Headless alternatives:

```sh
ytime auth     # run the device flow in the console, without the TUI
ytime logout   # forget the stored token
```

## Keys

| Keys              | Action                                             |
|-------------------|----------------------------------------------------|
| `Tab` / `Shift+Tab` | cycle Favorites → Tasks → Calendar → Settings   |
| `1` / `2` / `3`   | select tile: Favorites / Tasks / Calendar         |
| `Shift+1` / `Shift+2` | select tab: dashboard / Settings              |
| `s` (or `Ctrl+S`) | stop the running timer → review → log              |
| `?`               | help · `q` quit (guarded while a timer runs)       |

**Favorites tile**: `j/k` select · `Enter` start timer · `l` log retroactively
to the calendar's selected day · `a` search Tracker to add (`↑/↓ Enter`,
`Esc` closes — the search covers your loaded issues and Tracker at large) ·
`d` remove. Favorites are stored in the state file.
**Tasks tile**: `j/k` select · `t` filter by issue type · `/` filter text ·
`r` refresh · `Enter` start timer.
**Calendar tile**: `h/l` `j/k` move day · `[` `]` change month · `t` today ·
`n` log time on the selected day · `r` reload · `Enter` day breakdown
(`n` works from the breakdown too).
**Log form** (opened with `n`): `↑/↓` move between fields · `Enter` submit
from the note field · in the task field type ≥ 3 chars for remote search,
`↑/↓ Enter` to pick · `Esc` closes.

## Files

| Path                                | Contents                                    |
|-------------------------------------|---------------------------------------------|
| `~/.config/ytime/config.toml`       | client_id, org_id, rounding, org header     |
| `~/.local/share/ytime/state.json`   | OAuth tokens (0600), running timer state    |

## Configuration reference

```toml
client_id = "…"                  # OAuth application id
client_secret = "…"              # app secret («Секрет») — required by Yandex
                                 # for the device-flow token exchange
org_id = "…"                     # Tracker organization id
# org_header = "x-org-id"        # or "x-cloud-org-id"; omit for auto-detect

[rounding]
minutes = 30                     # 1 | 5 | 15 | 30 | 60
mode = "up"                      # up | nearest | down

# api_base = "https://api.tracker.yandex.net"
# oauth_base = "https://oauth.yandex.ru"
```

Use `X-Org-Id` for organizations linked to Yandex 360 for Business, and
`X-Cloud-Org-Id` for Yandex Identity Hub orgs. `ytime` probes both and
remembers the one that works.

## Development

```sh
cargo test     # unit tests: rounding, duration parsing, calendar bucketing
cargo clippy
cargo fmt
```

## Caveats

- Keyboard-only; no mouse support.
- Worklog "days" follow Tracker's convention: `1d` counts as one 8-hour
  workday (and `1w` as five workdays) — both for entries the calendar reads
  and for durations you type. ytime posts its own entries in unambiguous
  hours/minutes (`PT8H`), so nothing it logs is affected by day conventions.
- The Tracker API's worklog search filters by record *creation* time, not by
  the day the work happened, so for past months ytime fetches everything
  created from two weeks before the month up to today. Histories longer than
  1000 records since the viewed month may be truncated (10 pages × 100).
- Manual time entries with no note are allowed; the timer always requires a
  description.
