//! Typed views of the entities.
//!
//! The replica stores every entity as an open JSON object, so fields from a
//! newer server survive. These types read the fields this client understands
//! and fill in the defaults for the ones that are absent (entities.md). Reading
//! never fails: a value this client cannot read counts as its default.

use serde_json::{Map, Value};

pub type Obj = Map<String, Value>;

pub const ENTITY_TYPES: [&str; 6] = ["area", "project", "heading", "task", "checklistItem", "tag"];

fn text(o: &Obj, key: &str) -> String {
    o.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn opt_text(o: &Obj, key: &str) -> Option<String> {
    o.get(key).and_then(Value::as_str).map(str::to_string)
}

fn flag(o: &Obj, key: &str) -> bool {
    o.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn id_list(o: &Obj, key: &str) -> Vec<String> {
    o.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq)]
pub enum When {
    Inbox,
    Anytime,
    Someday,
    Date(String),
}

impl When {
    fn read(value: Option<&Value>, fallback: When) -> When {
        let Some(o) = value.and_then(Value::as_object) else {
            return fallback;
        };
        match o.get("kind").and_then(Value::as_str) {
            Some("inbox") => When::Inbox,
            Some("anytime") => When::Anytime,
            Some("someday") => When::Someday,
            Some("date") => match o.get("date").and_then(Value::as_str) {
                Some(date) => When::Date(date.to_string()),
                None => fallback,
            },
            _ => fallback,
        }
    }

    pub fn date(&self) -> Option<&str> {
        match self {
            When::Date(date) => Some(date),
            _ => None,
        }
    }

    pub fn to_json(&self) -> Value {
        match self {
            When::Inbox => serde_json::json!({ "kind": "inbox" }),
            When::Anytime => serde_json::json!({ "kind": "anytime" }),
            When::Someday => serde_json::json!({ "kind": "someday" }),
            When::Date(date) => serde_json::json!({ "kind": "date", "date": date }),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reminder {
    pub days_before: i64,
    pub time: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RepeatRule {
    pub unit: String,
    pub every: i64,
    /// `schedule` or `completion`.
    pub basis: String,
    pub weekdays: Vec<i64>,
    pub start: String,
    pub until: Option<String>,
    pub deadline_days: Option<i64>,
}

impl RepeatRule {
    pub fn read(value: &Value) -> Option<RepeatRule> {
        let o = value.as_object()?;
        Some(RepeatRule {
            unit: o.get("unit")?.as_str()?.to_string(),
            every: o.get("every")?.as_i64()?.max(1),
            basis: o
                .get("basis")
                .and_then(Value::as_str)
                .unwrap_or("schedule")
                .to_string(),
            weekdays: o
                .get("weekdays")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_i64).collect())
                .unwrap_or_default(),
            start: o.get("start")?.as_str()?.to_string(),
            until: opt_text(o, "until"),
            deadline_days: o.get("deadlineDays").and_then(Value::as_i64),
        })
    }
}

macro_rules! entity {
    ($name:ident { $($field:ident : $ty:ty),* $(,)? }) => {
        #[derive(Debug, Clone, PartialEq)]
        pub struct $name {
            pub id: String,
            pub created_at: String,
            pub deleted: bool,
            $(pub $field: $ty,)*
            /// The entity as stored, with the fields this client does not know.
            pub raw: Obj,
        }
    };
}

entity!(Area { title: String, order: String, icon: Option<String>, color: Option<String> });
entity!(Project {
    title: String,
    notes: String,
    area_id: Option<String>,
    status: String,
    when: When,
    deadline: Option<String>,
    tag_ids: Vec<String>,
    order: String,
    completed_at: Option<String>,
});
entity!(Heading {
    project_id: String,
    title: String,
    order: String
});
entity!(Task {
    title: String,
    notes: String,
    status: String,
    project_id: Option<String>,
    heading_id: Option<String>,
    area_id: Option<String>,
    when: When,
    daypart: Option<String>,
    duration: Option<i64>,
    reminder: Option<Reminder>,
    deadline: Option<String>,
    tag_ids: Vec<String>,
    repeat: Option<RepeatRule>,
    template_id: Option<String>,
    order: String,
    today_order: Option<String>,
    completed_at: Option<String>,
    assignee_id: Option<String>,
});
entity!(ChecklistItem {
    task_id: String,
    title: String,
    done: bool,
    order: String
});
entity!(Tag { title: String, parent_id: Option<String>, order: String });

impl Area {
    pub fn read(o: &Obj) -> Area {
        Area {
            id: text(o, "id"),
            created_at: text(o, "createdAt"),
            deleted: flag(o, "deleted"),
            title: text(o, "title"),
            order: text(o, "order"),
            icon: opt_text(o, "icon"),
            color: opt_text(o, "color"),
            raw: o.clone(),
        }
    }
}

impl Project {
    pub fn read(o: &Obj) -> Project {
        Project {
            id: text(o, "id"),
            created_at: text(o, "createdAt"),
            deleted: flag(o, "deleted"),
            title: text(o, "title"),
            notes: text(o, "notes"),
            area_id: opt_text(o, "areaId"),
            status: opt_text(o, "status").unwrap_or_else(|| "open".into()),
            when: When::read(o.get("when"), When::Anytime),
            deadline: opt_text(o, "deadline"),
            tag_ids: id_list(o, "tagIds"),
            order: text(o, "order"),
            completed_at: opt_text(o, "completedAt"),
            raw: o.clone(),
        }
    }

    pub fn is_open(&self) -> bool {
        self.status == "open"
    }
}

impl Heading {
    pub fn read(o: &Obj) -> Heading {
        Heading {
            id: text(o, "id"),
            created_at: text(o, "createdAt"),
            deleted: flag(o, "deleted"),
            project_id: text(o, "projectId"),
            title: text(o, "title"),
            order: text(o, "order"),
            raw: o.clone(),
        }
    }
}

impl Task {
    pub fn read(o: &Obj) -> Task {
        let reminder = o.get("reminder").and_then(Value::as_object).and_then(|r| {
            Some(Reminder {
                days_before: r.get("daysBefore")?.as_i64()?,
                time: r.get("time")?.as_str()?.to_string(),
            })
        });
        Task {
            id: text(o, "id"),
            created_at: text(o, "createdAt"),
            deleted: flag(o, "deleted"),
            title: text(o, "title"),
            notes: text(o, "notes"),
            status: opt_text(o, "status").unwrap_or_else(|| "open".into()),
            project_id: opt_text(o, "projectId"),
            heading_id: opt_text(o, "headingId"),
            area_id: opt_text(o, "areaId"),
            when: When::read(o.get("when"), When::Inbox),
            daypart: opt_text(o, "daypart"),
            duration: o.get("duration").and_then(Value::as_i64),
            reminder,
            deadline: opt_text(o, "deadline"),
            tag_ids: id_list(o, "tagIds"),
            repeat: o.get("repeat").and_then(RepeatRule::read),
            template_id: opt_text(o, "templateId"),
            order: text(o, "order"),
            today_order: opt_text(o, "todayOrder"),
            completed_at: opt_text(o, "completedAt"),
            assignee_id: opt_text(o, "assigneeId"),
            raw: o.clone(),
        }
    }

    pub fn is_open(&self) -> bool {
        self.status == "open"
    }

    /// A template is a task with a repeat rule; it is in no list.
    pub fn is_template(&self) -> bool {
        self.raw.get("repeat").is_some_and(|r| !r.is_null())
    }

    /// Its place in Today.
    pub fn today_key(&self) -> &str {
        self.today_order.as_deref().unwrap_or(&self.order)
    }
}

impl ChecklistItem {
    pub fn read(o: &Obj) -> ChecklistItem {
        ChecklistItem {
            id: text(o, "id"),
            created_at: text(o, "createdAt"),
            deleted: flag(o, "deleted"),
            task_id: text(o, "taskId"),
            title: text(o, "title"),
            done: flag(o, "done"),
            order: text(o, "order"),
            raw: o.clone(),
        }
    }
}

impl Tag {
    pub fn read(o: &Obj) -> Tag {
        Tag {
            id: text(o, "id"),
            created_at: text(o, "createdAt"),
            deleted: flag(o, "deleted"),
            title: text(o, "title"),
            parent_id: opt_text(o, "parentId"),
            order: text(o, "order"),
            raw: o.clone(),
        }
    }
}

/// Everything the user can see, deleted entities included.
#[derive(Debug, Clone, Default)]
pub struct Account {
    pub areas: Vec<Area>,
    pub projects: Vec<Project>,
    pub headings: Vec<Heading>,
    pub tasks: Vec<Task>,
    pub checklist: Vec<ChecklistItem>,
    pub tags: Vec<Tag>,
}

impl Account {
    /// Adds one entity; entity types this client does not know are ignored.
    pub fn add(&mut self, entity_type: &str, data: &Obj) {
        match entity_type {
            "area" => self.areas.push(Area::read(data)),
            "project" => self.projects.push(Project::read(data)),
            "heading" => self.headings.push(Heading::read(data)),
            "task" => self.tasks.push(Task::read(data)),
            "checklistItem" => self.checklist.push(ChecklistItem::read(data)),
            "tag" => self.tags.push(Tag::read(data)),
            _ => {}
        }
    }

    pub fn task(&self, id: &str) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == id)
    }

    pub fn project(&self, id: &str) -> Option<&Project> {
        self.projects.iter().find(|p| p.id == id)
    }

    pub fn area(&self, id: &str) -> Option<&Area> {
        self.areas.iter().find(|a| a.id == id)
    }

    pub fn tag(&self, id: &str) -> Option<&Tag> {
        self.tags.iter().find(|t| t.id == id)
    }

    /// The live checklist of a task, in order.
    pub fn checklist_of(&self, task_id: &str) -> Vec<&ChecklistItem> {
        let mut items: Vec<_> = self
            .checklist
            .iter()
            .filter(|c| !c.deleted && c.task_id == task_id)
            .collect();
        items.sort_by(|a, b| crate::order::compare((&a.order, &a.id), (&b.order, &b.id)));
        items
    }
}
