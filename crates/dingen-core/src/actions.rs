//! What a client writes when the user does something (conformance "actions").
//!
//! Every client must write the same changes, so that a task completed here
//! looks the same as one completed in the browser. An action is computed
//! against the whole account and gives a list of writes; each write becomes
//! one change in the sync log.

use crate::model::{Account, ChecklistItem, Obj, RepeatRule, Task, When};
use crate::order::{compare, key_between};
use crate::{dates, entities, repeat};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq)]
pub enum Write {
    /// A new entity, complete.
    Create { entity_type: String, entity: Obj },
    /// Changed fields of an existing entity.
    Change {
        entity_type: String,
        entity_id: String,
        fields: Obj,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum MoveTarget {
    /// To a list. `daypart` absent means: keep the task's own.
    When {
        when: When,
        daypart: Option<Option<String>>,
    },
    Project {
        project_id: String,
        heading_id: Option<String>,
    },
    Area {
        area_id: String,
    },
    /// Out of every container.
    None,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    ToggleTask {
        task_id: String,
    },
    DeleteTask {
        task_id: String,
    },
    ChangeTask {
        task_id: String,
        fields: Obj,
    },
    MoveTask {
        task_id: String,
        target: MoveTarget,
    },
    SetRepeat {
        task_id: String,
        rule: Value,
    },
    StopRepeat {
        task_id: String,
    },
    DuplicateTask {
        task_id: String,
    },
    ConvertTaskToProject {
        task_id: String,
    },
    /// The checklist of the task is now `items`.
    ChangeChecklist {
        task_id: String,
        items: Vec<ChecklistItem>,
    },
    ToggleProject {
        project_id: String,
    },
    DeleteProject {
        project_id: String,
    },
    DeleteArea {
        area_id: String,
    },
}

/// Fields of an open instance that its template follows.
const MIRRORED: [&str; 9] = [
    "title",
    "notes",
    "daypart",
    "duration",
    "reminder",
    "tagIds",
    "projectId",
    "headingId",
    "areaId",
];

/// The content a template and its instances share.
const CONTENT: [&str; 10] = [
    "title",
    "notes",
    "tagIds",
    "projectId",
    "headingId",
    "areaId",
    "daypart",
    "duration",
    "reminder",
    "assigneeId",
];

struct Run<'a> {
    account: &'a Account,
    today: &'a str,
    now: &'a str,
    new_id: &'a mut dyn FnMut() -> String,
    writes: Vec<Write>,
}

fn obj(value: Value) -> Obj {
    value.as_object().cloned().unwrap_or_default()
}

fn copy_fields(from: &Obj, names: &[&str]) -> Obj {
    names
        .iter()
        .map(|n| (n.to_string(), from.get(*n).cloned().unwrap_or(Value::Null)))
        .collect()
}

impl Run<'_> {
    fn change(&mut self, entity_type: &str, entity_id: &str, fields: Obj) {
        if !fields.is_empty() {
            self.writes.push(Write::Change {
                entity_type: entity_type.into(),
                entity_id: entity_id.into(),
                fields,
            });
        }
    }

    fn create(&mut self, entity_type: &str, fields: Obj) -> String {
        let id = (self.new_id)();
        if let Some(entity) = entities::new_entity(entity_type, &id, self.now, &fields) {
            self.writes.push(Write::Create {
                entity_type: entity_type.into(),
                entity,
            });
        }
        id
    }

    /// The live template of an open instance.
    fn template_of(&self, task: &Task) -> Option<(&Task, RepeatRule)> {
        if !task.is_open() {
            return None;
        }
        let template = self.account.task(task.template_id.as_deref()?)?;
        if template.deleted {
            return None;
        }
        Some((template, template.repeat.clone()?))
    }

    /// A key below the other tasks in a container.
    fn end_of_tasks(
        &self,
        project_id: Option<&str>,
        heading_id: Option<&str>,
        area_id: Option<&str>,
        except: &str,
    ) -> String {
        let last = self
            .account
            .tasks
            .iter()
            .filter(|t| {
                !t.deleted
                    && !t.is_template()
                    && t.id != except
                    && t.project_id.as_deref() == project_id
                    && t.heading_id.as_deref() == heading_id
                    && t.area_id.as_deref() == area_id
            })
            .max_by(|a, b| compare((&a.order, &a.id), (&b.order, &b.id)));
        key_between(last.map(|t| t.order.as_str()), None)
    }

    /// Copies a checklist onto another task, unticked.
    fn copy_checklist(&mut self, from_task: &str, to_task: &str) {
        let items: Vec<(String, String)> = self
            .account
            .checklist_of(from_task)
            .iter()
            .map(|c| (c.title.clone(), c.order.clone()))
            .collect();
        for (title, order) in items {
            self.create(
                "checklistItem",
                obj(json!({ "title": title, "order": order, "taskId": to_task })),
            );
        }
    }

    /// The instance leaves its list: the next one appears, if the rule goes on.
    fn next_instance(&mut self, instance: &Task) {
        let Some((template, rule)) = self.template_of(instance) else {
            return;
        };
        let template = template.clone();
        let after = if rule.basis == "completion" {
            self.today
        } else {
            instance.when.date().unwrap_or(self.today)
        };
        let Some(date) = repeat::next(&rule, after, self.today) else {
            return;
        };
        let mut fields = copy_fields(&template.raw, &CONTENT);
        fields.insert("when".into(), json!({ "kind": "date", "date": date }));
        fields.insert("deadline".into(), deadline_for(&rule, &date));
        fields.insert("templateId".into(), json!(template.id));
        let order = self.end_of_tasks(
            template.project_id.as_deref(),
            template.heading_id.as_deref(),
            template.area_id.as_deref(),
            &instance.id,
        );
        fields.insert("order".into(), json!(order));
        let id = self.create("task", fields);
        self.copy_checklist(&template.id, &id);
    }

    fn toggle_task(&mut self, task: &Task) {
        if task.is_open() {
            self.change(
                "task",
                &task.id,
                obj(json!({ "status": "completed", "completedAt": self.now })),
            );
            self.next_instance(task);
        } else {
            self.change(
                "task",
                &task.id,
                obj(json!({ "status": "open", "completedAt": null })),
            );
        }
    }

    fn delete_task(&mut self, task: &Task) {
        self.change("task", &task.id, obj(json!({ "deleted": true })));
        self.next_instance(task);
    }

    fn change_task(&mut self, task: &Task, fields: &Obj) {
        let mut own = fields.clone();
        // Only a task with a date has a daypart and a reminder.
        let leaves_date = fields
            .get("when")
            .is_some_and(|w| w.get("kind") != Some(&json!("date")));
        if leaves_date {
            for name in ["daypart", "reminder"] {
                let set = task.raw.get(name).is_some_and(|v| !v.is_null());
                if set && !own.contains_key(name) {
                    own.insert(name.into(), Value::Null);
                }
            }
        }
        self.change("task", &task.id, own);
        self.mirror(task, fields);
    }

    fn mirror(&mut self, task: &Task, fields: &Obj) {
        let Some((template, _)) = self.template_of(task) else {
            return;
        };
        let id = template.id.clone();
        let shared: Obj = fields
            .iter()
            .filter(|(k, _)| MIRRORED.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        self.change("task", &id, shared);
    }

    fn move_task(&mut self, task: &Task, target: &MoveTarget) {
        let leaves_inbox = task.when == When::Inbox;
        let mut fields = Obj::new();
        match target {
            MoveTarget::Project {
                project_id,
                heading_id,
            } => {
                if task.project_id.as_deref() == Some(project_id) && task.heading_id == *heading_id {
                    return;
                }
                let order = self.end_of_tasks(Some(project_id), heading_id.as_deref(), None, &task.id);
                fields = obj(
                    json!({ "projectId": project_id, "headingId": heading_id, "areaId": null, "order": order }),
                );
                if leaves_inbox {
                    fields.insert("when".into(), json!({ "kind": "anytime" }));
                }
            }
            MoveTarget::Area { area_id } => {
                if task.area_id.as_deref() == Some(area_id) && task.project_id.is_none() {
                    return;
                }
                let order = self.end_of_tasks(None, None, Some(area_id), &task.id);
                fields =
                    obj(json!({ "areaId": area_id, "projectId": null, "headingId": null, "order": order }));
                if leaves_inbox {
                    fields.insert("when".into(), json!({ "kind": "anytime" }));
                }
            }
            MoveTarget::When { when, daypart } => {
                let is_date = when.date().is_some();
                let daypart = match daypart {
                    _ if !is_date => None,
                    Some(daypart) => daypart.clone(),
                    None => task.daypart.clone(),
                };
                let contained =
                    task.project_id.is_some() || task.area_id.is_some() || task.heading_id.is_some();
                if task.when == *when && task.daypart == daypart && !(*when == When::Inbox && contained) {
                    return;
                }
                fields.insert("when".into(), when.to_json());
                fields.insert("daypart".into(), json!(daypart));
                if *when == When::Inbox {
                    fields.extend(obj(
                        json!({ "projectId": null, "areaId": null, "headingId": null }),
                    ));
                }
                if !is_date && task.reminder.is_some() {
                    fields.insert("reminder".into(), Value::Null);
                }
                self.change("task", &task.id, fields);
                return;
            }
            MoveTarget::None => {
                if task.project_id.is_some() || task.area_id.is_some() || task.heading_id.is_some() {
                    fields = obj(json!({ "projectId": null, "areaId": null, "headingId": null }));
                }
            }
        }
        self.change("task", &task.id, fields.clone());
        // The template takes the new place; its own order and list stay.
        self.mirror(task, &fields);
    }

    fn set_repeat(&mut self, task: &Task, rule_json: &Value) {
        let Some(rule) = RepeatRule::read(rule_json) else {
            return;
        };
        let date = repeat::first(&rule, self.today).unwrap_or_else(|| rule.start.clone());
        let mut own = obj(json!({ "when": { "kind": "date", "date": date } }));
        own.insert("deadline".into(), deadline_for(&rule, &date));
        if let Some((template, _)) = self.template_of(task) {
            let id = template.id.clone();
            self.change("task", &id, obj(json!({ "repeat": rule_json })));
            self.change("task", &task.id, own);
            return;
        }
        let mut fields = copy_fields(&task.raw, &CONTENT);
        fields.insert("when".into(), json!({ "kind": "anytime" }));
        fields.insert("repeat".into(), rule_json.clone());
        fields.insert("order".into(), json!(task.order));
        let template_id = self.create("task", fields);
        self.copy_checklist(&task.id, &template_id);
        let mut with_template = obj(json!({ "templateId": template_id }));
        with_template.extend(own);
        self.change("task", &task.id, with_template);
    }

    fn stop_repeat(&mut self, task: &Task) {
        if let Some((template, _)) = self.template_of(task) {
            let id = template.id.clone();
            self.change("task", &id, obj(json!({ "deleted": true })));
        }
        if task.template_id.is_some() {
            self.change("task", &task.id, obj(json!({ "templateId": null })));
        }
    }

    fn change_checklist(&mut self, task: &Task, items: &[ChecklistItem]) {
        let Some((template, _)) = self.template_of(task) else {
            return;
        };
        let template_id = template.id.clone();
        let template_items = self.account.checklist_of(&template_id);
        let instance_items: Vec<&ChecklistItem> = items.iter().collect();
        let mirror = repeat::checklist_mirror(&template_items, &instance_items);
        for (id, fields) in mirror.update {
            self.change("checklistItem", &id, fields);
        }
        for (title, order) in mirror.create {
            self.create(
                "checklistItem",
                obj(json!({ "title": title, "order": order, "taskId": template_id })),
            );
        }
        for id in mirror.remove {
            self.change("checklistItem", &id, obj(json!({ "deleted": true })));
        }
    }

    fn duplicate_task(&mut self, task: &Task) {
        // The copy sorts right below the task in every list, so the next key
        // is looked for among all tasks, not only its siblings.
        let others = || {
            self.account
                .tasks
                .iter()
                .filter(|t| !t.deleted && t.id != task.id)
        };
        let next_order = others()
            .map(|t| t.order.as_str())
            .filter(|o| *o > task.order.as_str())
            .min();
        let mut fields = copy_fields(
            &task.raw,
            &[
                "title",
                "notes",
                "projectId",
                "headingId",
                "areaId",
                "when",
                "daypart",
                "duration",
                "reminder",
                "deadline",
                "tagIds",
            ],
        );
        fields.insert("order".into(), json!(key_between(Some(&task.order), next_order)));
        if let Some(today_order) = task.today_order.as_deref() {
            let next = others().map(Task::today_key).filter(|o| *o > today_order).min();
            fields.insert("todayOrder".into(), json!(key_between(Some(today_order), next)));
        }
        let id = self.create("task", fields);
        self.copy_checklist(&task.id, &id);
    }

    fn convert_to_project(&mut self, task: &Task) {
        let area_id = task.area_id.clone().or_else(|| {
            self.account
                .project(task.project_id.as_deref()?)
                .and_then(|p| p.area_id.clone())
        });
        let last = self
            .account
            .projects
            .iter()
            .filter(|p| !p.deleted && p.area_id == area_id)
            .max_by(|a, b| compare((&a.order, &a.id), (&b.order, &b.id)));
        let when = if task.when == When::Inbox {
            When::Anytime
        } else {
            task.when.clone()
        };
        let mut fields = copy_fields(&task.raw, &["title", "notes", "deadline", "tagIds"]);
        fields.insert("areaId".into(), json!(area_id));
        fields.insert("when".into(), when.to_json());
        fields.insert(
            "order".into(),
            json!(key_between(last.map(|p| p.order.as_str()), None)),
        );
        let project_id = self.create("project", fields);
        let items: Vec<ChecklistItem> = self.account.checklist_of(&task.id).into_iter().cloned().collect();
        for item in items {
            let mut step = obj(json!({
                "title": item.title, "projectId": project_id, "when": { "kind": "anytime" }, "order": item.order,
            }));
            if item.done {
                step.extend(obj(json!({ "status": "completed", "completedAt": self.now })));
            }
            self.create("task", step);
        }
        self.change("task", &task.id, obj(json!({ "deleted": true })));
        self.next_instance(task);
    }

    fn delete_project(&mut self, project_id: &str) {
        let deleted = || obj(json!({ "deleted": true }));
        let tasks: Vec<String> = self
            .account
            .tasks
            .iter()
            .filter(|t| !t.deleted && t.project_id.as_deref() == Some(project_id))
            .map(|t| t.id.clone())
            .collect();
        for id in tasks {
            self.change("task", &id, deleted());
        }
        let headings: Vec<String> = self
            .account
            .headings
            .iter()
            .filter(|h| !h.deleted && h.project_id == project_id)
            .map(|h| h.id.clone())
            .collect();
        for id in headings {
            self.change("heading", &id, deleted());
        }
        self.change("project", project_id, deleted());
    }

    fn delete_area(&mut self, area_id: &str) {
        let projects: Vec<String> = self
            .account
            .projects
            .iter()
            .filter(|p| !p.deleted && p.area_id.as_deref() == Some(area_id))
            .map(|p| p.id.clone())
            .collect();
        for id in projects {
            self.delete_project(&id);
        }
        let tasks: Vec<String> = self
            .account
            .tasks
            .iter()
            .filter(|t| !t.deleted && t.area_id.as_deref() == Some(area_id))
            .map(|t| t.id.clone())
            .collect();
        for id in tasks {
            self.change("task", &id, obj(json!({ "deleted": true })));
        }
        self.change("area", area_id, obj(json!({ "deleted": true })));
    }
}

fn deadline_for(rule: &RepeatRule, date: &str) -> Value {
    json!(rule.deadline_days.and_then(|days| dates::add_days(date, days)))
}

/// The writes for one user action. `new_id` gives the id of each new entity.
pub fn perform(
    account: &Account,
    today: &str,
    now: &str,
    action: &Action,
    new_id: &mut dyn FnMut() -> String,
) -> Vec<Write> {
    let mut run = Run {
        account,
        today,
        now,
        new_id,
        writes: vec![],
    };
    let task = |id: &str| account.task(id).filter(|t| !t.deleted);
    match action {
        Action::ToggleTask { task_id } => task(task_id).into_iter().for_each(|t| run.toggle_task(t)),
        Action::DeleteTask { task_id } => task(task_id).into_iter().for_each(|t| run.delete_task(t)),
        Action::ChangeTask { task_id, fields } => {
            task(task_id).into_iter().for_each(|t| run.change_task(t, fields))
        }
        Action::MoveTask { task_id, target } => {
            task(task_id).into_iter().for_each(|t| run.move_task(t, target))
        }
        Action::SetRepeat { task_id, rule } => {
            task(task_id).into_iter().for_each(|t| run.set_repeat(t, rule))
        }
        Action::StopRepeat { task_id } => task(task_id).into_iter().for_each(|t| run.stop_repeat(t)),
        Action::DuplicateTask { task_id } => task(task_id).into_iter().for_each(|t| run.duplicate_task(t)),
        Action::ConvertTaskToProject { task_id } => {
            task(task_id).into_iter().for_each(|t| run.convert_to_project(t))
        }
        Action::ChangeChecklist { task_id, items } => task(task_id)
            .into_iter()
            .for_each(|t| run.change_checklist(t, items)),
        Action::ToggleProject { project_id } => {
            if let Some(project) = account.project(project_id).filter(|p| !p.deleted) {
                let fields = if project.is_open() {
                    json!({ "status": "completed", "completedAt": now })
                } else {
                    json!({ "status": "open", "completedAt": null })
                };
                run.change("project", project_id, obj(fields));
            }
        }
        Action::DeleteProject { project_id } => run.delete_project(project_id),
        Action::DeleteArea { area_id } => run.delete_area(area_id),
    }
    run.writes
}
