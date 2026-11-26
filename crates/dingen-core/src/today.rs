//! Today (conformance "today"): next week, tasks and calendar events per
//! daypart, and which tasks are new in Today.

use crate::dates;
use crate::misc;
use crate::model::Task;
use std::collections::HashMap;

/// "Next week" in the When picker: the next Monday; on a Monday a week later.
pub fn next_week(today: &str) -> String {
    let weekday = dates::weekday(today).unwrap_or(1);
    let days = match (8 - weekday) % 7 {
        0 => 7,
        n => n,
    };
    dates::add_days(today, days).unwrap_or_else(|| today.to_string())
}

/// Ids per group of Today, in the order given.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ByDaypart {
    pub none: Vec<String>,
    pub morning: Vec<String>,
    pub afternoon: Vec<String>,
    pub evening: Vec<String>,
}

impl ByDaypart {
    fn push(&mut self, daypart: Option<&str>, id: String) {
        match daypart {
            Some("morning") => self.morning.push(id),
            Some("afternoon") => self.afternoon.push(id),
            Some("evening") => self.evening.push(id),
            _ => self.none.push(id),
        }
    }

    /// The group of a daypart (`None`: the group without one).
    pub fn of(&self, daypart: Option<&str>) -> &[String] {
        match daypart {
            Some("morning") => &self.morning,
            Some("afternoon") => &self.afternoon,
            Some("evening") => &self.evening,
            _ => &self.none,
        }
    }
}

/// Tasks (`(id, daypart)`) per daypart, in the order given.
pub fn tasks_by_daypart<'a>(tasks: impl IntoIterator<Item = (&'a str, Option<&'a str>)>) -> ByDaypart {
    let mut out = ByDaypart::default();
    for (id, daypart) in tasks {
        out.push(daypart, id.to_string());
    }
    out
}

/// A calendar event (api.md §2.2).
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub id: String,
    pub calendar_id: String,
    pub title: String,
    pub location: Option<String>,
    pub all_day: bool,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub start_date: String,
    pub end_date: String,
}

impl Event {
    pub fn read(v: &serde_json::Value) -> Option<Event> {
        let text = |k: &str| v.get(k).and_then(serde_json::Value::as_str).map(str::to_string);
        Some(Event {
            id: text("id")?,
            calendar_id: text("calendarId").unwrap_or_default(),
            title: text("title").unwrap_or_default(),
            location: text("location"),
            all_day: v
                .get("allDay")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            start_time: text("startTime"),
            end_time: text("endTime"),
            start_date: text("startDate")?,
            end_date: text("endDate")?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Span {
    AllDay,
    Single,
    First,
    Middle,
    Last,
}

impl Span {
    pub fn name(self) -> &'static str {
        match self {
            Span::AllDay => "allDay",
            Span::Single => "single",
            Span::First => "first",
            Span::Middle => "middle",
            Span::Last => "last",
        }
    }
}

/// How an event shows on a day.
pub fn event_span(event: &Event, date: &str) -> Span {
    if event.all_day {
        Span::AllDay
    } else if event.start_date == event.end_date {
        Span::Single
    } else if date == event.start_date {
        Span::First
    } else if date == event.end_date {
        Span::Last
    } else {
        Span::Middle
    }
}

/// The events on a day: all-day events and middle days first, then by time
/// (`00:00` on the last day), then by title, then by id.
pub fn events_on_day<'a>(events: &'a [Event], date: &str) -> Vec<&'a Event> {
    let mut on: Vec<&Event> = events
        .iter()
        .filter(|e| e.start_date.as_str() <= date && date <= e.end_date.as_str())
        .collect();
    let key = |e: &Event| {
        let span = event_span(e, date);
        let top = matches!(span, Span::AllDay | Span::Middle);
        let time = match span {
            Span::AllDay | Span::Middle => String::new(),
            Span::Last => "00:00".to_string(),
            _ => e.start_time.clone().unwrap_or_default(),
        };
        (!top, time)
    };
    on.sort_by(|a, b| {
        key(a)
            .cmp(&key(b))
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.id.cmp(&b.id))
    });
    on
}

/// The events of a day per daypart: under the daypart of their start time
/// when they start that day, else without one.
pub fn events_by_daypart(events: &[Event], date: &str) -> ByDaypart {
    let mut out = ByDaypart::default();
    for event in events_on_day(events, date) {
        let start = match event_span(event, date) {
            Span::Single | Span::First => event.start_time.as_deref(),
            _ => None,
        };
        out.push(start.map(misc::daypart_of), event.id.clone());
    }
    out
}

/// The tasks new in Today: open, with a date `d` where `seen < d <= today`
/// (without `seen`, `d == today`), and whose `when` was set on a day before
/// `d` (`set_on`, per task id; unknown is not new).
pub fn new_in_today(
    tasks: &[Task],
    set_on: &HashMap<String, Option<String>>,
    today: &str,
    seen: Option<&str>,
) -> Vec<String> {
    tasks
        .iter()
        .filter(|t| !t.deleted && t.is_open())
        .filter(|t| {
            let Some(d) = t.when.date() else { return false };
            let in_range = match seen {
                Some(seen) => seen < d && d <= today,
                None => d == today,
            };
            let set = set_on.get(&t.id).cloned().flatten();
            in_range && set.is_some_and(|s| s.as_str() < d)
        })
        .map(|t| t.id.clone())
        .collect()
}
