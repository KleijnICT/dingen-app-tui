//! The strict shape of every entity type: what the server accepts
//! (entities.md). Used to check a change before it is queued, and to make new
//! entities with the defaults of Dingen.

use crate::model::Obj;
use crate::{dates, ids, misc};
use serde_json::{json, Value};

#[derive(Clone, Copy)]
enum Kind {
    Id,
    IdOrNull,
    IdList,
    Text,
    TextOrNull,
    Bool,
    Instant,
    InstantOrNull,
    DateOrNull,
    Order,
    OrderOrNull,
    Status,
    When,
    DaypartOrNull,
    DurationOrNull,
    ReminderOrNull,
    RepeatOrNull,
    ReviewEveryOrNull,
    MemberOrNull,
}

struct Field {
    name: &'static str,
    kind: Kind,
    /// A field with a default may be absent; it then reads as `null`.
    has_default: bool,
}

const fn req(name: &'static str, kind: Kind) -> Field {
    Field {
        name,
        kind,
        has_default: false,
    }
}

const fn opt(name: &'static str, kind: Kind) -> Field {
    Field {
        name,
        kind,
        has_default: true,
    }
}

const AREA: &[Field] = &[
    req("id", Kind::Id),
    req("createdAt", Kind::Instant),
    req("deleted", Kind::Bool),
    req("title", Kind::Text),
    req("order", Kind::Order),
    opt("icon", Kind::TextOrNull),
    opt("color", Kind::TextOrNull),
    opt("reviewedAt", Kind::InstantOrNull),
    opt("reviewEvery", Kind::ReviewEveryOrNull),
];

const PROJECT: &[Field] = &[
    req("id", Kind::Id),
    req("createdAt", Kind::Instant),
    req("deleted", Kind::Bool),
    req("title", Kind::Text),
    req("notes", Kind::Text),
    req("areaId", Kind::IdOrNull),
    req("status", Kind::Status),
    req("when", Kind::When),
    req("deadline", Kind::DateOrNull),
    req("tagIds", Kind::IdList),
    req("order", Kind::Order),
    req("completedAt", Kind::InstantOrNull),
    opt("reviewedAt", Kind::InstantOrNull),
    opt("reviewEvery", Kind::ReviewEveryOrNull),
];

const HEADING: &[Field] = &[
    req("id", Kind::Id),
    req("createdAt", Kind::Instant),
    req("deleted", Kind::Bool),
    req("projectId", Kind::Id),
    req("title", Kind::Text),
    req("order", Kind::Order),
];

const TASK: &[Field] = &[
    req("id", Kind::Id),
    req("createdAt", Kind::Instant),
    req("deleted", Kind::Bool),
    req("title", Kind::Text),
    req("notes", Kind::Text),
    req("status", Kind::Status),
    req("projectId", Kind::IdOrNull),
    req("headingId", Kind::IdOrNull),
    req("areaId", Kind::IdOrNull),
    req("when", Kind::When),
    opt("daypart", Kind::DaypartOrNull),
    opt("duration", Kind::DurationOrNull),
    opt("reminder", Kind::ReminderOrNull),
    req("deadline", Kind::DateOrNull),
    req("tagIds", Kind::IdList),
    req("repeat", Kind::RepeatOrNull),
    opt("templateId", Kind::IdOrNull),
    req("order", Kind::Order),
    opt("todayOrder", Kind::OrderOrNull),
    req("completedAt", Kind::InstantOrNull),
    opt("assigneeId", Kind::MemberOrNull),
];

const CHECKLIST_ITEM: &[Field] = &[
    req("id", Kind::Id),
    req("createdAt", Kind::Instant),
    req("deleted", Kind::Bool),
    req("taskId", Kind::Id),
    req("title", Kind::Text),
    req("done", Kind::Bool),
    req("order", Kind::Order),
];

const TAG: &[Field] = &[
    req("id", Kind::Id),
    req("createdAt", Kind::Instant),
    req("deleted", Kind::Bool),
    req("title", Kind::Text),
    req("parentId", Kind::IdOrNull),
    req("order", Kind::Order),
];

/// Fields of protocol 2 that stored tasks can still carry.
const RETIRED_TASK_FIELDS: [&str; 2] = ["evening", "time"];

fn schema(entity_type: &str) -> Option<&'static [Field]> {
    Some(match entity_type {
        "area" => AREA,
        "project" => PROJECT,
        "heading" => HEADING,
        "task" => TASK,
        "checklistItem" => CHECKLIST_ITEM,
        "tag" => TAG,
        _ => return None,
    })
}

/// Is this an entity type this client knows?
pub fn is_known_type(entity_type: &str) -> bool {
    schema(entity_type).is_some()
}

fn whole(v: &Value, min: i64, max: i64) -> bool {
    let n = match v.as_i64() {
        Some(n) => n,
        None => match v.as_f64() {
            Some(f) if f.fract() == 0.0 => f as i64,
            _ => return false,
        },
    };
    (min..=max).contains(&n)
}

fn one_of(v: &Value, options: &[&str]) -> bool {
    v.as_str().is_some_and(|s| options.contains(&s))
}

fn date(v: &Value) -> bool {
    v.as_str().is_some_and(dates::is_date)
}

type Check<'a> = &'a dyn Fn(&Value) -> bool;

/// An object with exactly these keys, each passing its check.
fn object(v: &Value, fields: &[(&str, Check)]) -> bool {
    let Some(o) = v.as_object() else { return false };
    o.keys().all(|k| fields.iter().any(|(name, _)| name == k))
        && fields.iter().all(|(name, check)| o.get(*name).is_some_and(check))
}

fn or_null(v: &Value, check: impl Fn(&Value) -> bool) -> bool {
    v.is_null() || check(v)
}

fn is_id(v: &Value) -> bool {
    v.as_str().is_some_and(ids::is_ulid)
}

fn is_when(v: &Value) -> bool {
    match v.get("kind").and_then(Value::as_str) {
        Some("inbox" | "anytime" | "someday") => v.as_object().is_some_and(|o| o.len() == 1),
        Some("date") => object(v, &[("kind", &|_| true), ("date", &date)]),
        _ => false,
    }
}

fn is_repeat(v: &Value) -> bool {
    object(
        v,
        &[
            ("unit", &|v| one_of(v, &["day", "week", "month", "year"])),
            ("every", &|v| whole(v, 1, 999)),
            ("basis", &|v| one_of(v, &["schedule", "completion"])),
            ("weekdays", &|v| {
                v.as_array().is_some_and(|a| a.iter().all(|d| whole(d, 0, 6)))
            }),
            ("start", &date),
            ("until", &|v| or_null(v, date)),
            ("deadlineDays", &|v| or_null(v, |v| whole(v, 0, 3650))),
        ],
    )
}

fn check(kind: Kind, v: &Value) -> bool {
    match kind {
        Kind::Id => is_id(v),
        Kind::IdOrNull => or_null(v, is_id),
        Kind::IdList => v.as_array().is_some_and(|a| a.iter().all(is_id)),
        Kind::Text => v.is_string(),
        Kind::TextOrNull => or_null(v, Value::is_string),
        Kind::Bool => v.is_boolean(),
        Kind::Instant => v.as_str().is_some_and(dates::is_instant),
        Kind::InstantOrNull => or_null(v, |v| v.as_str().is_some_and(dates::is_instant)),
        Kind::DateOrNull => or_null(v, date),
        Kind::Order => v.as_str().is_some_and(|s| !s.is_empty()),
        Kind::OrderOrNull => or_null(v, |v| v.as_str().is_some_and(|s| !s.is_empty())),
        Kind::Status => one_of(v, &["open", "completed", "canceled"]),
        Kind::When => is_when(v),
        Kind::DaypartOrNull => or_null(v, |v| one_of(v, &["morning", "afternoon", "evening"])),
        Kind::DurationOrNull => or_null(v, |v| whole(v, 1, 1440)),
        Kind::ReminderOrNull => or_null(v, |v| {
            object(
                v,
                &[
                    ("daysBefore", &|v| whole(v, 0, 365)),
                    ("time", &|v| v.as_str().is_some_and(dates::is_time_of_day)),
                ],
            )
        }),
        Kind::RepeatOrNull => or_null(v, is_repeat),
        Kind::ReviewEveryOrNull => or_null(v, |v| {
            object(
                v,
                &[
                    ("n", &|v| whole(v, 1, 999)),
                    ("unit", &|v| one_of(v, &["day", "week", "month", "year"])),
                ],
            )
        }),
        Kind::MemberOrNull => or_null(v, |v| {
            v.as_str().is_some_and(|s| {
                ["user:", "agent:"]
                    .iter()
                    .any(|p| s.strip_prefix(p).is_some_and(|id| !id.is_empty()))
            })
        }),
    }
}

/// Checks an entity and fills in the defaults for absent fields. Fields this
/// client does not know are kept. `None` when the data is not valid.
pub fn parse(entity_type: &str, data: &Obj) -> Option<Obj> {
    let mut entity = data.clone();
    for field in schema(entity_type)? {
        match data.get(field.name) {
            Some(value) if check(field.kind, value) => {}
            Some(_) => return None,
            None if field.has_default => {
                entity.insert(field.name.to_string(), Value::Null);
            }
            None => return None,
        }
    }
    Some(entity)
}

/// Would the server accept these fields as a change to `existing` (or, without
/// it, as a create)? Unknown fields are refused.
pub fn validate_change(entity_type: &str, entity_id: &str, fields: &Obj, existing: Option<&Obj>) -> bool {
    let Some(schema) = schema(entity_type) else {
        return false;
    };
    let mut merged = existing.cloned().unwrap_or_default();
    merged.insert("id".into(), json!(entity_id));
    for (name, value) in fields {
        match name.as_str() {
            "id" if value.as_str() != Some(entity_id) => return false,
            // Not an entity field: it names the space the entity goes to.
            "spaceId" => {
                if !value.as_str().is_some_and(|s| !s.is_empty()) {
                    return false;
                }
            }
            _ if schema.iter().any(|f| f.name == name) => {
                merged.insert(name.clone(), value.clone());
            }
            _ => return false,
        }
    }
    if entity_type == "task" {
        for retired in RETIRED_TASK_FIELDS {
            merged.remove(retired);
        }
    }
    schema.iter().all(|field| match merged.get(field.name) {
        Some(value) => check(field.kind, value),
        None => field.has_default,
    })
}

/// A new entity with the defaults of Dingen, `fields` laid over them.
pub fn new_entity(entity_type: &str, id: &str, now: &str, fields: &Obj) -> Option<Obj> {
    let defaults = match entity_type {
        "area" => {
            json!({ "order": "a0", "icon": null, "color": null, "reviewedAt": null, "reviewEvery": null })
        }
        "project" => json!({
            "notes": "", "areaId": null, "status": "open", "when": { "kind": "anytime" },
            "deadline": null, "tagIds": [], "order": "a0", "completedAt": null,
            "reviewedAt": null, "reviewEvery": null,
        }),
        "heading" => json!({ "order": "a0" }),
        "task" => json!({
            "notes": "", "status": "open", "projectId": null, "headingId": null, "areaId": null,
            "when": { "kind": "inbox" }, "daypart": null, "duration": null, "reminder": null,
            "deadline": null, "tagIds": [], "repeat": null, "templateId": null, "order": "a0",
            "todayOrder": null, "completedAt": null, "assigneeId": null,
        }),
        "checklistItem" => json!({ "done": false, "order": "a0" }),
        "tag" => json!({ "parentId": null, "order": "a0" }),
        _ => return None,
    };
    let mut entity = Obj::new();
    entity.insert("id".into(), json!(id));
    entity.insert("createdAt".into(), json!(now));
    entity.insert("deleted".into(), json!(false));
    entity.extend(defaults.as_object().cloned().unwrap_or_default());
    entity.extend(fields.clone());
    if entity_type == "tag" {
        if let Some(title) = entity.get("title").and_then(Value::as_str) {
            let title = misc::normalize_tag(title);
            entity.insert("title".into(), json!(title));
        }
    }
    Some(entity)
}
