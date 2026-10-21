use std::collections::BTreeMap;

use chrono::{Datelike, Duration as ChronoDuration, Local, NaiveDate};

use crate::api::models::{parse_dt, WorklogRecord};
use crate::timer;

#[derive(Debug, Clone, Default)]
pub struct MonthData {
    pub by_day: BTreeMap<NaiveDate, i64>,
    pub records: Vec<WorklogRecord>,
}

impl MonthData {
    pub fn total_minutes(&self) -> i64 {
        self.by_day.values().sum()
    }

    pub fn day_minutes(&self, date: NaiveDate) -> i64 {
        self.by_day.get(&date).copied().unwrap_or(0)
    }

    pub fn records_on(&self, date: NaiveDate) -> Vec<&WorklogRecord> {
        self.records
            .iter()
            .filter(|r| {
                parse_dt(&r.start)
                    .map(|dt| dt.with_timezone(&Local).date_naive() == date)
                    .unwrap_or(false)
            })
            .collect()
    }
}

/// Buckets worklog records by the local date of their `start` field.
/// `from`/`to` bound the records' *creation* time, so a generous buffer is
/// applied by the caller to catch retrospective entries.
pub fn bucket_by_day(records: Vec<WorklogRecord>) -> MonthData {
    let mut data = MonthData {
        records,
        by_day: BTreeMap::new(),
    };
    let recs = std::mem::take(&mut data.records);
    for r in recs {
        if let (Some(start), Some(dur)) =
            (parse_dt(&r.start), timer::parse_iso_duration(&r.duration))
        {
            let day = start.with_timezone(&Local).date_naive();
            *data.by_day.entry(day).or_insert(0) += (dur.as_secs() / 60) as i64;
        }
        data.records.push(r);
    }
    data
}

pub fn first_of_month(year: i32, month: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, 1).expect("valid month")
}

pub fn last_of_month(year: i32, month: u32) -> NaiveDate {
    first_of_month(year, month)
        .checked_add_months(chrono::Months::new(1))
        .expect("next month")
        .pred_opt()
        .expect("last day")
}

/// Monday-first week grid of the month; leading/trailing days from adjacent
/// months are included as `None`.
pub fn month_grid(year: i32, month: u32) -> Vec<Vec<Option<NaiveDate>>> {
    let first = first_of_month(year, month);
    let lead = first.weekday().num_days_from_monday() as i64;
    let grid_start = first - ChronoDuration::days(lead);
    let mut weeks = Vec::new();
    let mut day = grid_start;
    while day.month() == month || day < first {
        let mut week = Vec::with_capacity(7);
        for _ in 0..7 {
            week.push(if day.month() == month {
                Some(day)
            } else {
                None
            });
            day += ChronoDuration::try_days(1).expect("one day");
        }
        weeks.push(week);
        if day.month() != month && day > last_of_month(year, month) {
            break;
        }
    }
    weeks
}

/// Fetch window: records *created* within this range can cover work started
/// in the target month. Users routinely log in batches weeks (or months)
/// after the work happened, so for past months the window extends all the
/// way to today — records cannot be created in the future.
pub fn fetch_window(year: i32, month: u32) -> (NaiveDate, NaiveDate) {
    let one = ChronoDuration::try_days(1).expect("1 day");
    let back = first_of_month(year, month) - ChronoDuration::try_days(14).expect("14 days");
    let natural_end = last_of_month(year, month) + one;
    let today_end = Local::now().date_naive() + one;
    (back, natural_end.max(today_end))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(start: &str, duration: &str) -> WorklogRecord {
        WorklogRecord {
            issue: None,
            start: start.to_string(),
            duration: duration.to_string(),
            comment: None,
        }
    }

    #[test]
    fn buckets_by_local_date() {
        let data = bucket_by_day(vec![
            rec("2026-10-01T10:00:00.000+0300", "PT1H30M"),
            rec("2026-10-01T12:00:00.000+0300", "PT30M"),
            rec("2026-10-03T09:00:00.000+0300", "P1DT1H"),
            rec("not-a-date", "PT1H"),
        ]);
        assert_eq!(data.by_day.len(), 2);
        assert_eq!(
            data.by_day[&NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()],
            120
        );
        // P1D is one 8h workday; +1h → 9h = 540 min
        assert_eq!(
            data.by_day[&NaiveDate::from_ymd_opt(2026, 10, 3).unwrap()],
            540
        );
        assert_eq!(data.total_minutes(), 660);
        assert_eq!(data.records.len(), 4);
    }

    #[test]
    fn grid_is_monday_first_and_covers_month() {
        // October 2026 starts on Thursday
        let weeks = month_grid(2026, 10);
        assert_eq!(weeks[0][0], None);
        assert_eq!(
            weeks[0][3],
            Some(NaiveDate::from_ymd_opt(2026, 10, 1).unwrap())
        );
        let all: Vec<NaiveDate> = weeks.into_iter().flatten().flatten().collect();
        assert_eq!(all.len(), 31);
        assert_eq!(
            *all.last().unwrap(),
            NaiveDate::from_ymd_opt(2026, 10, 31).unwrap()
        );
        // November 2026 starts on Sunday — one leading None
        let nov = month_grid(2026, 11);
        assert_eq!(nov[0].iter().filter(|d| d.is_none()).count(), 6);
    }

    #[test]
    fn fetch_window_buffers_month() {
        let today = Local::now().date_naive();
        let one = ChronoDuration::try_days(1).unwrap();
        // a month far in the past: the window must reach "now + 1" so that
        // retrospective entries created long after the work are included
        let (from, to) = fetch_window(2020, 6);
        assert_eq!(from, NaiveDate::from_ymd_opt(2020, 5, 18).unwrap());
        assert_eq!(to, today + one);
        // a month far in the future: just the month plus the back-buffer
        let far = today + ChronoDuration::try_days(400).unwrap();
        let (from, to) = fetch_window(far.year(), far.month());
        assert_eq!(
            from,
            first_of_month(far.year(), far.month()) - ChronoDuration::try_days(14).unwrap()
        );
        assert_eq!(to, last_of_month(far.year(), far.month()) + one);
    }
}
