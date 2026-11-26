//! Repeat rules: which dates fit and the next occurrence (entities.md, section 9).

use crate::dates::{self, Days};
use crate::model::{ChecklistItem, RepeatRule};
use serde_json::{json, Map, Value};

/// A schedule never looks further ahead than this: 999 years of days.
const HORIZON: Days = 999 * 366 + 366;

fn monday_of(days: Days) -> Days {
    // 1970-01-01 was a Thursday; weeks run from Monday.
    days - (days + 3).rem_euclid(7)
}

fn in_schedule(rule: &RepeatRule, start: Days, day: Days) -> bool {
    if day < start {
        return false;
    }
    if let Some(until) = rule.until.as_deref().and_then(dates::to_days) {
        if day > until {
            return false;
        }
    }
    match rule.unit.as_str() {
        "day" => (day - start) % rule.every == 0,
        "week" => {
            let weeks = (monday_of(day) - monday_of(start)) / 7;
            let weekday = (day + 4).rem_euclid(7);
            let on_day = if rule.weekdays.is_empty() {
                weekday == (start + 4).rem_euclid(7)
            } else {
                rule.weekdays.contains(&weekday)
            };
            weeks % rule.every == 0 && on_day
        }
        unit @ ("month" | "year") => {
            let date = dates::from_days(day);
            let (Some((y, m, d)), Some((sy, sm, sd))) = (dates::split(&date), dates::split(&rule.start))
            else {
                return false;
            };
            let months = (y * 12 + m) - (sy * 12 + sm);
            let step = if unit == "year" {
                rule.every * 12
            } else {
                rule.every
            };
            months % step == 0 && d == sd.min(dates::days_in_month(y, m))
        }
        _ => false,
    }
}

/// Is `date` in the rule's schedule? Only for rules with basis `schedule`.
pub fn matches(rule: &RepeatRule, date: &str) -> bool {
    match (dates::to_days(&rule.start), dates::to_days(date)) {
        (Some(start), Some(day)) => in_schedule(rule, start, day),
        _ => false,
    }
}

/// The first date in the schedule on or after `from`.
fn first_from(rule: &RepeatRule, from: Days) -> Option<String> {
    let start = dates::to_days(&rule.start)?;
    let until = rule.until.as_deref().and_then(dates::to_days);
    let from = from.max(start);
    let end = until.unwrap_or(Days::MAX).min(from.saturating_add(HORIZON));
    (from..=end)
        .find(|&day| in_schedule(rule, start, day))
        .map(dates::from_days)
}

fn within(rule: &RepeatRule, date: String) -> Option<String> {
    match rule.until.as_deref() {
        Some(until) if date.as_str() > until => None,
        _ => Some(date),
    }
}

/// The date one interval later. Months and years take the day of the month of
/// the rule's start.
pub fn add_interval(rule: &RepeatRule, date: &str) -> Option<String> {
    let day = dates::day_of_month(&rule.start);
    match rule.unit.as_str() {
        "day" => dates::add_days(date, rule.every),
        "week" => dates::add_days(date, rule.every * 7),
        "month" => dates::add_months(date, rule.every, day),
        "year" => dates::add_years(date, rule.every, day),
        _ => None,
    }
}

/// The occurrence after `after`. On a schedule: the first date after `after`
/// that is not before `not_before`. After completion: one interval after
/// `after`. `None` when the rule has ended.
pub fn next(rule: &RepeatRule, after: &str, not_before: &str) -> Option<String> {
    if rule.basis == "completion" {
        return within(rule, add_interval(rule, after)?);
    }
    let from = (dates::to_days(after)? + 1).max(dates::to_days(not_before)?);
    first_from(rule, from)
}

/// The date of the first instance of a new rule.
pub fn first(rule: &RepeatRule, today: &str) -> Option<String> {
    if rule.basis == "completion" {
        let date = if today > rule.start.as_str() {
            today
        } else {
            rule.start.as_str()
        };
        return within(rule, date.to_string());
    }
    if matches(rule, &rule.start) {
        return Some(rule.start.clone());
    }
    first_from(rule, dates::to_days(today)?)
}

/// What to do to a template's checklist so it matches an instance's.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mirror {
    /// Template item id and the fields to change.
    pub update: Vec<(String, Map<String, Value>)>,
    /// Title and order of items to add.
    pub create: Vec<(String, String)>,
    /// Template item ids to delete.
    pub remove: Vec<String>,
}

impl Mirror {
    pub fn to_json(&self) -> Value {
        json!({
            "update": self.update.iter().map(|(id, fields)| json!({ "id": id, "fields": fields })).collect::<Vec<_>>(),
            "create": self.create.iter().map(|(title, order)| json!({ "title": title, "order": order })).collect::<Vec<_>>(),
            "remove": self.remove,
        })
    }
}

/// Matches the live items of both checklists by position. Ticks are never copied.
pub fn checklist_mirror(template: &[&ChecklistItem], instance: &[&ChecklistItem]) -> Mirror {
    let live = |items: &[&ChecklistItem]| -> Vec<ChecklistItem> {
        let mut items: Vec<ChecklistItem> = items
            .iter()
            .filter(|c| !c.deleted)
            .map(|c| (*c).clone())
            .collect();
        items.sort_by(|a, b| crate::order::compare((&a.order, &a.id), (&b.order, &b.id)));
        items
    };
    let (template, instance) = (live(template), live(instance));
    let mut mirror = Mirror::default();
    for (i, item) in instance.iter().enumerate() {
        match template.get(i) {
            Some(target) => {
                let mut fields = Map::new();
                if target.title != item.title {
                    fields.insert("title".into(), json!(item.title));
                }
                if target.order != item.order {
                    fields.insert("order".into(), json!(item.order));
                }
                if !fields.is_empty() {
                    mirror.update.push((target.id.clone(), fields));
                }
            }
            None => mirror.create.push((item.title.clone(), item.order.clone())),
        }
    }
    mirror.remove = template
        .iter()
        .skip(instance.len())
        .map(|c| c.id.clone())
        .collect();
    mirror
}
