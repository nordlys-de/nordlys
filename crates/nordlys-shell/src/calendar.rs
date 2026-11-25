//! The month that drops down under the clock.

use chrono::{Datelike, Months, NaiveDate};

/// The first day of the month `date` is in.
pub fn first_of(date: NaiveDate) -> NaiveDate {
    date.with_day(1).unwrap_or(date)
}

/// `month` moved by `by` months, staying put at chrono's limits.
pub fn shift(month: NaiveDate, by: i32) -> NaiveDate {
    let n = Months::new(by.unsigned_abs());
    let moved = if by < 0 {
        month.checked_sub_months(n)
    } else {
        month.checked_add_months(n)
    };
    moved.unwrap_or(month)
}

/// Six weeks, Monday first, covering the month that starts on `month`.
/// Always six, so the panel never changes height.
pub fn weeks(month: NaiveDate) -> [[NaiveDate; 7]; 6] {
    let back = month.weekday().num_days_from_monday() as u64;
    let mut day = month
        .checked_sub_days(chrono::Days::new(back))
        .unwrap_or(month);
    let mut grid = [[month; 7]; 6];
    for week in &mut grid {
        for cell in week {
            *cell = day;
            day = day.succ_opt().unwrap_or(day);
        }
    }
    grid
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn weeks_start_on_monday_and_cover_the_month() {
        // November 2026 starts on a Sunday.
        let grid = weeks(date(2026, 11, 1));
        assert_eq!(grid[0][0], date(2026, 10, 26));
        assert_eq!(grid[0][6], date(2026, 11, 1));
        assert_eq!(grid[5][0], date(2026, 11, 30));
        // A month that starts on Monday starts the grid.
        assert_eq!(weeks(date(2027, 2, 1))[0][0], date(2027, 2, 1));
    }

    #[test]
    fn months_step_across_years() {
        assert_eq!(shift(date(2026, 12, 1), 1), date(2027, 1, 1));
        assert_eq!(shift(date(2026, 1, 1), -1), date(2025, 12, 1));
        assert_eq!(first_of(date(2026, 11, 25)), date(2026, 11, 1));
    }
}
