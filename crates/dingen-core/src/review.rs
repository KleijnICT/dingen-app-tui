//! The weekly review (conformance "review"): when an area or project is due,
//! the queue in sidebar order, what to select next, how long ago a review
//! was, and the open tasks of an entry.

use crate::dates;
use crate::model::{Account, Area, Obj, Project, Task};
use crate::order::compare;
use serde_json::Value;

/// How often an item is reviewed; `None` in the data means every week.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Every {
    pub n: i64,
    /// `day`, `week`, `month` or `year`.
    pub unit: &'static str,
}

pub const WEEKLY: Every = Every { n: 1, unit: "week" };

impl Every {
    pub fn read(raw: &Obj) -> Every {
        let Some(o) = raw.get("reviewEvery").and_then(Value::as_object) else {
            return WEEKLY;
        };
        let n = o.get("n").and_then(Value::as_i64).unwrap_or(1).max(1);
        let unit = match o.get("unit").and_then(Value::as_str) {
            Some("day") => "day",
            Some("month") => "month",
            Some("year") => "year",
            _ => "week",
        };
        Every { n, unit }
    }
}

/// The calendar date of `reviewedAt`, by `to_date` (the device's clock in
/// an app; the UTC date in the conformance cases).
pub fn reviewed_on(raw: &Obj, to_date: &dyn Fn(&str) -> String) -> Option<String> {
    raw.get("reviewedAt").and_then(Value::as_str).map(to_date)
}

/// The date from which an item is due: its review date plus its frequency,
/// or today when it was never reviewed.
pub fn next_review_on(reviewed_on: Option<&str>, every: Every, today: &str) -> String {
    let Some(date) = reviewed_on else {
        return today.to_string();
    };
    let next = match every.unit {
        "day" => dates::add_days(date, every.n),
        "month" => dates::add_months(date, every.n, None),
        "year" => dates::add_years(date, every.n, None),
        _ => dates::add_days(date, every.n * 7),
    };
    next.unwrap_or_else(|| today.to_string())
}

/// What can be reviewed: an area that is not deleted, or an open project.
pub fn reviewable_area(area: &Area) -> bool {
    !area.deleted
}

pub fn reviewable_project(project: &Project) -> bool {
    !project.deleted && project.is_open()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// `area` or `project`.
    pub kind: &'static str,
    pub id: String,
    pub next_review_on: String,
}

/// The due items in sidebar order: projects without an area, then each area
/// followed by its projects, each by order then id.
pub fn queue(account: &Account, today: &str, to_date: &dyn Fn(&str) -> String) -> Vec<Entry> {
    let due = |raw: &Obj| {
        let next = next_review_on(reviewed_on(raw, to_date).as_deref(), Every::read(raw), today);
        (next.as_str() <= today).then_some(next)
    };
    let mut projects: Vec<&Project> = account
        .projects
        .iter()
        .filter(|p| reviewable_project(p))
        .collect();
    projects.sort_by(|a, b| compare((&a.order, &a.id), (&b.order, &b.id)));
    let mut areas: Vec<&Area> = account.areas.iter().filter(|a| reviewable_area(a)).collect();
    areas.sort_by(|a, b| compare((&a.order, &a.id), (&b.order, &b.id)));
    let mut out = vec![];
    let push_projects = |area: Option<&str>, out: &mut Vec<Entry>| {
        for project in projects.iter().filter(|p| p.area_id.as_deref() == area) {
            if let Some(next) = due(&project.raw) {
                out.push(Entry {
                    kind: "project",
                    id: project.id.clone(),
                    next_review_on: next,
                });
            }
        }
    };
    push_projects(None, &mut out);
    for area in areas {
        if let Some(next) = due(&area.raw) {
            out.push(Entry {
                kind: "area",
                id: area.id.clone(),
                next_review_on: next,
            });
        }
        push_projects(Some(&area.id), &mut out);
    }
    out
}

/// The id to select after `id` was marked reviewed: the next, else the one
/// before; when `id` is not in the queue, the first.
pub fn select_after(queue: &[String], id: &str) -> Option<String> {
    match queue.iter().position(|q| q == id) {
        Some(at) => queue
            .get(at + 1)
            .or_else(|| at.checked_sub(1).and_then(|i| queue.get(i)))
            .cloned(),
        None => queue.first().cloned(),
    }
}

/// How long ago a review was, in the largest whole unit: `(n, unit)`.
pub fn age(reviewed_on: Option<&str>, today: &str) -> Option<(i64, &'static str)> {
    let days = dates::days_between(reviewed_on?, today)?.max(0);
    Some(match days {
        d if d < 7 => (d, "day"),
        d if d < 30 => (d / 7, "week"),
        d if d < 365 => (d / 30, "month"),
        d => (d / 365, "year"),
    })
}

/// The open tasks of a project, or of an area without a project.
pub fn open_count(kind: &str, id: &str, tasks: &[Task]) -> usize {
    tasks
        .iter()
        .filter(|t| !t.deleted && !t.is_template() && t.is_open())
        .filter(|t| match kind {
            "project" => t.project_id.as_deref() == Some(id),
            _ => t.area_id.as_deref() == Some(id) && t.project_id.is_none(),
        })
        .count()
}
