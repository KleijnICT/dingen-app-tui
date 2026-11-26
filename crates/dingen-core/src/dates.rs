//! Calendar dates (`YYYY-MM-DD`) without time zones, and instants.
//!
//! A date is handled as a count of days since 1970-01-01, so arithmetic never
//! touches a clock or a time zone.

pub type Days = i64;

fn days_from_civil(y: i64, m: i64, d: i64) -> Days {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: Days) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m, d)
}

pub fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        _ => 28,
    }
}

/// Year, month and day of a `YYYY-MM-DD` string; `None` when it is not a real date.
pub fn split(date: &str) -> Option<(i64, i64, i64)> {
    let b = date.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let digits = |r: std::ops::Range<usize>| -> Option<i64> {
        if b[r.clone()].iter().all(u8::is_ascii_digit) {
            date[r].parse().ok()
        } else {
            None
        }
    };
    let (y, m, d) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
    if !(1..=12).contains(&m) || d < 1 || d > days_in_month(y, m) {
        return None;
    }
    Some((y, m, d))
}

pub fn is_date(date: &str) -> bool {
    split(date).is_some()
}

pub fn to_days(date: &str) -> Option<Days> {
    split(date).map(|(y, m, d)| days_from_civil(y, m, d))
}

pub fn from_days(days: Days) -> String {
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

pub fn add_days(date: &str, days: i64) -> Option<String> {
    Some(from_days(to_days(date)? + days))
}

/// The same day of the month `months` later, or `day_of_month` when given;
/// shortened to the last day when the month is shorter.
pub fn add_months(date: &str, months: i64, day_of_month: Option<i64>) -> Option<String> {
    let (y, m, d) = split(date)?;
    let index = y * 12 + (m - 1) + months;
    let (ny, nm) = (index.div_euclid(12), index.rem_euclid(12) + 1);
    let day = day_of_month.unwrap_or(d).min(days_in_month(ny, nm));
    Some(format!("{ny:04}-{nm:02}-{day:02}"))
}

pub fn add_years(date: &str, years: i64, day_of_month: Option<i64>) -> Option<String> {
    add_months(date, years * 12, day_of_month)
}

/// 0 (Sunday) to 6 (Saturday).
pub fn weekday(date: &str) -> Option<i64> {
    Some((to_days(date)? + 4).rem_euclid(7))
}

pub fn day_of_month(date: &str) -> Option<i64> {
    split(date).map(|(_, _, d)| d)
}

pub fn days_between(from: &str, to: &str) -> Option<i64> {
    Some(to_days(to)? - to_days(from)?)
}

/// An instant as clients write it: UTC with milliseconds.
pub fn instant_from_ms(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let (y, m, d) = civil_from_days(secs.div_euclid(86400));
    let rem = secs.rem_euclid(86400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        ms % 1000
    )
}

/// `HH:MM`, 00:00 to 23:59.
pub fn is_time_of_day(time: &str) -> bool {
    let b = time.as_bytes();
    b.len() == 5
        && b[2] == b':'
        && [0, 1, 3, 4].iter().all(|&i| b[i].is_ascii_digit())
        && &time[0..2] <= "23"
        && &time[3..5] <= "59"
}

/// ISO 8601 date and time with `Z` or an offset.
pub fn is_instant(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() < 20 || !s.is_ascii() || !is_date(&s[0..10]) || b[10] != b'T' {
        return false;
    }
    let time_ok = |t: &str| -> bool {
        let t = t.as_bytes();
        t.len() == 8
            && t[2] == b':'
            && t[5] == b':'
            && [0, 1, 3, 4, 6, 7].iter().all(|&i| t[i].is_ascii_digit())
    };
    if !time_ok(&s[11..19]) || &s[11..13] > "23" || &s[14..16] > "59" || &s[17..19] > "59" {
        return false;
    }
    let mut rest = &s[19..];
    if let Some(fraction) = rest.strip_prefix('.') {
        let n = fraction.bytes().take_while(u8::is_ascii_digit).count();
        if n == 0 {
            return false;
        }
        rest = &fraction[n..];
    }
    if rest == "Z" {
        return true;
    }
    let r = rest.as_bytes();
    r.len() == 6
        && (r[0] == b'+' || r[0] == b'-')
        && r[3] == b':'
        && [1, 2, 4, 5].iter().all(|&i| r[i].is_ascii_digit())
}
