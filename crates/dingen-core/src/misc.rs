//! Small rules: dayparts, durations, tag titles, reminders, cursors.

use crate::dates;
use crate::model::Task;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// The part of the day a time falls in.
pub fn daypart_of(time: &str) -> &'static str {
    if time < "12:00" {
        "morning"
    } else if time < "18:00" {
        "afternoon"
    } else {
        "evening"
    }
}

/// The suggested reminder time for a daypart.
pub fn daypart_start(daypart: Option<&str>) -> &'static str {
    match daypart {
        Some("afternoon") => "13:00",
        Some("evening") => "19:00",
        _ => "09:00",
    }
}

/// The short form of a duration: `15m`, `1h`, `1h30`.
pub fn format_duration(minutes: i64, hour_unit: &str) -> String {
    let (h, m) = (minutes / 60, minutes % 60);
    match (h, m) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}{hour_unit}"),
        (h, m) => format!("{h}{hour_unit}{m:02}"),
    }
}

/// The longest duration a task can have, in minutes.
pub const MAX_DURATION: i64 = 1440;

/// A typed duration in minutes (conformance "duration", `parse`): `45`,
/// `45m`, `45min`, `1:30`, `1u30`, `1h30`, `2u`, `1,5u`. `None` when it is
/// not a duration from 1 to 1440 minutes.
pub fn parse_duration(input: &str) -> Option<i64> {
    let text: String = input
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let text = text.replacen(',', ".", 1);
    if text.is_empty() {
        return None;
    }
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    let minutes: f64 = if let Some(n) = [
        text.strip_suffix("min"),
        text.strip_suffix('m'),
        Some(text.as_str()),
    ]
    .into_iter()
    .flatten()
    .find(|n| digits(n))
    {
        n.parse().ok()?
    } else if let Some((h, m)) = text.split_once(':') {
        if !digits(h) || !digits(m) || m.len() > 2 {
            return None;
        }
        h.parse::<f64>().ok()? * 60.0 + m.parse::<f64>().ok()?
    } else {
        let at = text.find(['u', 'h'])?;
        let (hours, rest) = (&text[..at], &text[at + 1..]);
        let mut parts = hours.splitn(2, '.');
        let whole = parts.next().unwrap_or("");
        if !digits(whole) || !parts.next().is_none_or(digits) {
            return None;
        }
        let rest = rest.strip_suffix('m').unwrap_or(rest);
        let extra = if rest.is_empty() {
            0.0
        } else if digits(rest) && rest.len() <= 2 {
            rest.parse().ok()?
        } else {
            return None;
        };
        hours.parse::<f64>().ok()? * 60.0 + extra
    };
    let rounded = minutes.round() as i64;
    (1..=MAX_DURATION).contains(&rounded).then_some(rounded)
}

pub fn total_duration(durations: impl IntoIterator<Item = Option<i64>>) -> i64 {
    durations.into_iter().flatten().sum()
}

/// Does a task with this duration pass the filter "at most `max` minutes"?
pub fn duration_fits(duration: Option<i64>, max: Option<i64>) -> bool {
    match (duration, max) {
        (_, None) => true,
        (Some(d), Some(max)) => d <= max,
        (None, Some(_)) => false,
    }
}

/// Trimmed, inner whitespace collapsed to one space, lowercase.
pub fn normalize_tag(title: &str) -> String {
    title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub fn same_tag(a: &str, b: &str) -> bool {
    normalize_tag(a) == normalize_tag(b)
}

/// The date and time of a task's reminder in local clock time.
pub fn reminder_moment(task: &Task) -> Option<(String, String)> {
    if task.deleted || task.status != "open" || task.repeat.is_some() {
        return None;
    }
    let reminder = task.reminder.as_ref()?;
    let date = dates::add_days(task.when.date()?, -reminder.days_before)?;
    Some((date, reminder.time.clone()))
}

/// The fields to write to set (or, with `None`, remove) a reminder.
pub fn reminder_fields(task: &Task, reminder: Option<&Value>) -> Map<String, Value> {
    let mut fields = Map::new();
    let current = task.raw.get("reminder").cloned().unwrap_or(Value::Null);
    let wanted = reminder.cloned().unwrap_or(Value::Null);
    let has_date = task.when.date().is_some();
    if current != wanted && (has_date || wanted.is_null()) {
        fields.insert("reminder".into(), wanted);
    }
    fields
}

/// Per space the highest `seq` the client has applied.
pub type Cursor = BTreeMap<String, u64>;

/// The cursor with the highest `seq` per space.
pub fn merge_cursor(base: &Cursor, update: Option<&Cursor>) -> Cursor {
    let mut merged = base.clone();
    for (space, seq) in update.into_iter().flatten() {
        let entry = merged.entry(space.clone()).or_insert(0);
        *entry = (*entry).max(*seq);
    }
    merged
}

pub fn parse_cursor(raw: &str) -> Option<Cursor> {
    let Value::Object(map) = serde_json::from_str(raw).ok()? else {
        return None;
    };
    let mut cursor = Cursor::new();
    for (space, seq) in map {
        if space.is_empty() || space.chars().count() > 64 {
            return None;
        }
        cursor.insert(space, seq.as_u64()?);
    }
    Some(cursor)
}

pub fn cursor_json(cursor: &Cursor) -> Value {
    json!(cursor)
}

/// The protocol number in the `x-dingen-protocol` header; 0 when it names none.
pub fn request_protocol(header: Option<&str>) -> u64 {
    header
        .filter(|h| !h.is_empty() && h.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|h| h.parse().ok())
        .unwrap_or(0)
}
