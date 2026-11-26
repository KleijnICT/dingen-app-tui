//! Runs the conformance test set of the Dingen specification
//! (`spec/conformance`, a copy of the one in the main repository).

use dingen_core::actions::{self, Action, MoveTarget, Write};
use dingen_core::model::{Account, ChecklistItem, Obj, RepeatRule, Task};
use dingen_core::record::{apply_changes, Change, Record};
use dingen_core::{
    dates, entities, hlc, misc, order, repeat, review, search, tag_tree, today, views, CONFORMANCE_VERSION,
    SYNC_PROTOCOL,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../spec/conformance")
}

fn load(file: &str) -> Value {
    let text = std::fs::read_to_string(dir().join(file)).expect("the conformance file is present");
    serde_json::from_str(&text).expect("the conformance file is JSON")
}

fn s(v: &Value) -> &str {
    v.as_str().expect("a string")
}

fn opt_s(v: &Value) -> Option<&str> {
    v.as_str()
}

fn int(v: &Value) -> i64 {
    v.as_i64().expect("an integer")
}

fn object(v: &Value) -> Obj {
    v.as_object().cloned().unwrap_or_default()
}

fn account(input: &Value) -> Account {
    let mut account = Account::default();
    for (key, entity_type) in [
        ("areas", "area"),
        ("projects", "project"),
        ("headings", "heading"),
        ("tasks", "task"),
        ("checklist", "checklistItem"),
        ("tags", "tag"),
    ] {
        for entity in input[key].as_array().into_iter().flatten() {
            account.add(entity_type, &object(entity));
        }
    }
    account
}

fn rule(v: &Value) -> RepeatRule {
    RepeatRule::read(v).expect("a repeat rule")
}

fn items(v: &Value) -> Vec<ChecklistItem> {
    v.as_array()
        .into_iter()
        .flatten()
        .map(|i| ChecklistItem::read(&object(i)))
        .collect()
}

/// JSON values are equal when numbers are, whatever their notation.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => {
            (a.as_f64().unwrap_or(f64::NAN) - b.as_f64().unwrap_or(f64::NAN)).abs() < 1e-12
        }
        (Value::Array(a), Value::Array(b)) => a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same(a, b)),
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len() && a.iter().all(|(k, v)| b.get(k).is_some_and(|w| same(v, w)))
        }
        _ => a == b,
    }
}

fn action(v: &Value) -> Action {
    let task_id = v["taskId"].as_str().unwrap_or_default().to_string();
    match s(&v["kind"]) {
        "toggleTask" => Action::ToggleTask { task_id },
        "deleteTask" => Action::DeleteTask { task_id },
        "changeTask" => Action::ChangeTask {
            task_id,
            fields: object(&v["fields"]),
        },
        "moveTask" => {
            let t = &v["target"];
            let target = match s(&t["kind"]) {
                "when" => MoveTarget::When {
                    when: Task::read(&object(&json!({ "when": t["when"] }))).when,
                    daypart: t.get("daypart").map(|d| d.as_str().map(str::to_string)),
                },
                "project" => MoveTarget::Project {
                    project_id: s(&t["projectId"]).to_string(),
                    heading_id: t["headingId"].as_str().map(str::to_string),
                },
                "area" => MoveTarget::Area {
                    area_id: s(&t["areaId"]).to_string(),
                },
                _ => MoveTarget::None,
            };
            Action::MoveTask { task_id, target }
        }
        "setRepeat" => Action::SetRepeat {
            task_id,
            rule: v["rule"].clone(),
        },
        "stopRepeat" => Action::StopRepeat { task_id },
        "duplicateTask" => Action::DuplicateTask { task_id },
        "convertTaskToProject" => Action::ConvertTaskToProject { task_id },
        "changeChecklist" => Action::ChangeChecklist {
            task_id,
            items: items(&v["items"]),
        },
        "toggleProject" => Action::ToggleProject {
            project_id: s(&v["projectId"]).to_string(),
        },
        "deleteProject" => Action::DeleteProject {
            project_id: s(&v["projectId"]).to_string(),
        },
        "deleteArea" => Action::DeleteArea {
            area_id: s(&v["areaId"]).to_string(),
        },
        other => panic!("unknown action {other}"),
    }
}

/// The writes as the test set lists them. The order of the writes is free, so
/// they are sorted; new ids are already `$1`, `$2`, … in order of creation.
fn writes_json(writes: &[Write]) -> Value {
    let mut list = writes_in_order(writes);
    if let Value::Array(list) = &mut list {
        list.sort_by_key(|w| serde_json::to_string(w).unwrap_or_default());
    }
    list
}

/// The writes as the cases write them, in the order they were made.
fn writes_in_order(writes: &[Write]) -> Value {
    let list: Vec<Value> = writes
        .iter()
        .map(|w| match w {
            Write::Create { entity_type, entity } => {
                json!({ "write": "create", "entityType": entity_type, "entity": entity })
            }
            Write::Change { entity_type, entity_id, fields } => {
                json!({ "write": "change", "entityType": entity_type, "entityId": entity_id, "fields": fields })
            }
        })
        .collect();
    Value::Array(list)
}

fn sorted(v: &Value) -> Value {
    let mut list: Vec<Value> = v
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|w| serde_json::from_str(&w.to_string()).unwrap())
        .collect();
    list.sort_by_key(|w| serde_json::to_string(w).unwrap_or_default());
    Value::Array(list)
}

fn by_daypart(groups: &today::ByDaypart) -> Value {
    json!({ "none": groups.none, "morning": groups.morning, "afternoon": groups.afternoon, "evening": groups.evening })
}

fn events(v: &Value) -> Vec<today::Event> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|e| today::Event::read(e).unwrap())
        .collect()
}

fn run(suite: &str, op: &str, i: &Value) -> Value {
    match (suite, op) {
        ("hlc", "format") => json!(hlc::format(
            i["wall"].as_u64().unwrap(),
            int(&i["counter"]) as u32,
            s(&i["clientId"])
        )),
        ("hlc", "parse") => {
            let (wall, counter, client_id) = hlc::parse(s(&i["ts"])).unwrap();
            json!({ "wall": wall, "counter": counter, "clientId": client_id })
        }
        ("hlc", "compare") => json!(hlc::compare(s(&i["a"]), s(&i["b"])) as i8),
        ("hlc", "clock") => {
            let mut clock = hlc::Clock::new(s(&i["clientId"]));
            let steps = i["steps"].as_array().unwrap();
            json!(steps
                .iter()
                .map(|step| {
                    let wall = step["wall"].as_u64().unwrap();
                    if s(&step["do"]) == "now" {
                        json!(clock.now_at(wall))
                    } else {
                        clock.receive_at(wall, s(&step["remote"]));
                        Value::Null
                    }
                })
                .collect::<Vec<_>>())
        }
        ("apply-change", "apply") => {
            let existing: Option<Record> = serde_json::from_value(i["existing"].clone()).unwrap();
            let changes: Vec<Change> = serde_json::from_value(i["changes"].clone()).unwrap();
            json!(apply_changes(existing, &changes))
        }
        ("order", "between") => json!(order::key_between(opt_s(&i["before"]), opt_s(&i["after"]))),
        ("order", "nBetween") => {
            json!(order::n_keys_between(
                opt_s(&i["before"]),
                opt_s(&i["after"]),
                int(&i["n"]) as usize
            ))
        }
        ("order", "place") => {
            let siblings: Vec<(String, String)> = i["siblings"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| (s(&x["id"]).to_string(), s(&x["order"]).to_string()))
                .collect();
            let placed = order::place(&siblings, int(&i["index"]) as usize, int(&i["count"]) as usize);
            json!({ "orders": placed.orders, "renumber": placed.renumber })
        }
        ("order", "sort") => {
            let mut list: Vec<(&str, &str)> = i["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| (s(&x["order"]), s(&x["id"])))
                .collect();
            list.sort_by(|a, b| order::compare(*a, *b));
            json!(list.iter().map(|x| x.1).collect::<Vec<_>>())
        }
        ("dates", "addDays") => json!(dates::add_days(s(&i["date"]), int(&i["days"]))),
        ("dates", "addMonths") => json!(dates::add_months(
            s(&i["date"]),
            int(&i["months"]),
            i["dayOfMonth"].as_i64()
        )),
        ("dates", "addYears") => json!(dates::add_years(
            s(&i["date"]),
            int(&i["years"]),
            i["dayOfMonth"].as_i64()
        )),
        ("dates", "weekday") => json!(dates::weekday(s(&i["date"]))),
        ("dates", "dayOfMonth") => json!(dates::day_of_month(s(&i["date"]))),
        ("dates", "daysBetween") => json!(dates::days_between(s(&i["from"]), s(&i["to"]))),
        ("repeat", "matches") => {
            let rule = rule(&i["rule"]);
            json!(i["dates"]
                .as_array()
                .unwrap()
                .iter()
                .map(s)
                .filter(|d| repeat::matches(&rule, d))
                .collect::<Vec<_>>())
        }
        ("repeat", "next") => json!(repeat::next(
            &rule(&i["rule"]),
            s(&i["after"]),
            s(&i["notBefore"])
        )),
        ("repeat", "first") => json!(repeat::first(&rule(&i["rule"]), s(&i["today"]))),
        ("repeat", "addInterval") => json!(repeat::add_interval(&rule(&i["rule"]), s(&i["date"]))),
        ("repeat", "checklistMirror") => {
            let (template, instance) = (items(&i["templateItems"]), items(&i["instanceItems"]));
            repeat::checklist_mirror(
                &template.iter().collect::<Vec<_>>(),
                &instance.iter().collect::<Vec<_>>(),
            )
            .to_json()
        }
        ("views", "lists") => {
            let account = account(i);
            let dated = |list: &[views::Dated]| {
                json!(list
                    .iter()
                    .map(|d| json!({ "id": d.id, "date": d.date }))
                    .collect::<Vec<_>>())
            };
            let t = views::task_lists(&account, s(&i["today"]));
            let p = views::project_lists(&account, s(&i["today"]));
            json!({
                "tasks": {
                    "inbox": t.inbox, "today": t.today, "upcoming": dated(&t.upcoming),
                    "anytime": t.anytime, "someday": t.someday, "logbook": t.logbook,
                },
                "projects": {
                    "today": p.today, "upcoming": dated(&p.upcoming), "someday": p.someday, "logbook": p.logbook,
                },
            })
        }
        ("views", "containers") => {
            let c = views::containers(&account(i));
            let checklists: serde_json::Map<String, Value> = c
                .checklist_by_task
                .iter()
                .map(|(id, l)| {
                    (
                        id.clone(),
                        json!({ "items": l.items, "done": l.done, "total": l.total }),
                    )
                })
                .collect();
            json!({
                "areas": c.areas,
                "looseProjects": c.loose_projects,
                "projectsByArea": c.projects_by_area,
                "tasksByArea": c.tasks_by_area,
                "tasksByProject": c.tasks_by_project,
                "headingsByProject": c.headings_by_project,
                "progressByProject": c.progress_by_project,
                "checklistByTask": checklists,
            })
        }
        ("dayparts", "daypartOf") => json!(misc::daypart_of(s(&i["time"]))),
        ("dayparts", "startTime") => json!(misc::daypart_start(opt_s(&i["daypart"]))),
        ("duration", "format") => json!(misc::format_duration(int(&i["minutes"]), s(&i["hourUnit"]))),
        ("duration", "total") => {
            json!(misc::total_duration(
                i["durations"].as_array().unwrap().iter().map(Value::as_i64)
            ))
        }
        ("duration", "fits") => json!(misc::duration_fits(i["duration"].as_i64(), i["max"].as_i64())),
        ("duration", "parse") => json!(misc::parse_duration(s(&i["input"]))),
        ("tags", "normalize") => json!(misc::normalize_tag(s(&i["title"]))),
        ("tags", "same") => json!(misc::same_tag(s(&i["a"]), s(&i["b"]))),
        ("reminders", "moment") => match misc::reminder_moment(&Task::read(&object(&i["task"]))) {
            Some((date, time)) => json!({ "date": date, "time": time }),
            None => Value::Null,
        },
        ("reminders", "fields") => {
            let reminder = Some(&i["reminder"]).filter(|r| !r.is_null());
            json!(misc::reminder_fields(&Task::read(&object(&i["task"])), reminder))
        }
        ("protocol", "mergeCursor") => {
            let base: misc::Cursor = serde_json::from_value(i["base"].clone()).unwrap();
            let update: Option<misc::Cursor> = serde_json::from_value(i["update"].clone()).unwrap();
            json!(misc::merge_cursor(&base, update.as_ref()))
        }
        ("protocol", "parseCursor") => json!(misc::parse_cursor(s(&i["raw"]))),
        ("protocol", "requestProtocol") => json!(misc::request_protocol(opt_s(&i["header"]))),
        ("entities", "parse") => match entities::parse(s(&i["entityType"]), &object(&i["data"])) {
            Some(entity) => json!({ "ok": true, "entity": entity }),
            None => json!({ "ok": false }),
        },
        ("entities", "validateChange") => {
            let change = &i["change"];
            let existing = i["existing"].as_object();
            json!({ "ok": entities::validate_change(
                s(&change["entityType"]), s(&change["entityId"]), &object(&change["fields"]), existing,
            ) })
        }
        ("entities", "new") => {
            json!(entities::new_entity(
                s(&i["entityType"]),
                "$1",
                s(&i["now"]),
                &object(&i["fields"])
            ))
        }
        ("actions", _) => {
            let mut count = 0;
            let mut new_id = || {
                count += 1;
                format!("${count}")
            };
            let writes = actions::perform(
                &account(i),
                s(&i["today"]),
                s(&i["now"]),
                &action(&i["action"]),
                &mut new_id,
            );
            writes_json(&writes)
        }
        ("review", op) => {
            // The cases put `reviewedAt` at 12:00 UTC: its UTC date is the date everywhere.
            let to_date = |instant: &str| instant.chars().take(10).collect::<String>();
            let today = || s(&i["today"]);
            match op {
                "nextReviewOn" | "isDue" => {
                    let item = object(&i["item"]);
                    let next = review::next_review_on(
                        review::reviewed_on(&item, &to_date).as_deref(),
                        review::Every::read(&item),
                        today(),
                    );
                    if op == "nextReviewOn" {
                        json!(next)
                    } else {
                        let reviewable = if item.contains_key("status") {
                            review::reviewable_project(&dingen_core::model::Project::read(&item))
                        } else {
                            review::reviewable_area(&dingen_core::model::Area::read(&item))
                        };
                        json!(reviewable && next.as_str() <= today())
                    }
                }
                "queue" => json!(review::queue(&account(i), today(), &to_date)
                    .iter()
                    .map(|e| json!({ "kind": e.kind, "id": e.id, "nextReviewOn": e.next_review_on }))
                    .collect::<Vec<_>>()),
                "selectAfter" => {
                    let queue: Vec<String> = serde_json::from_value(i["queue"].clone()).unwrap();
                    json!(review::select_after(&queue, s(&i["id"])))
                }
                "age" => match review::age(opt_s(&i["reviewedAt"]).map(to_date).as_deref(), today()) {
                    Some((n, unit)) => json!({ "n": n, "unit": unit }),
                    None => Value::Null,
                },
                "openCount" => json!(review::open_count(
                    s(&i["entry"]["kind"]),
                    s(&i["entry"]["id"]),
                    &account(i).tasks
                )),
                _ => panic!("unknown operation review/{op}"),
            }
        }
        ("today", "nextWeek") => json!(today::next_week(s(&i["today"]))),
        ("today", "tasksByDaypart") => {
            let tasks = i["tasks"].as_array().unwrap();
            let groups = today::tasks_by_daypart(tasks.iter().map(|t| (s(&t["id"]), opt_s(&t["daypart"]))));
            by_daypart(&groups)
        }
        ("today", "eventSpan") => {
            json!(today::event_span(&today::Event::read(&i["event"]).unwrap(), s(&i["date"])).name())
        }
        ("today", "eventsOnDay") => {
            let events = events(&i["events"]);
            json!(today::events_on_day(&events, s(&i["date"]))
                .iter()
                .map(|e| e.id.clone())
                .collect::<Vec<_>>())
        }
        ("today", "eventsByDaypart") => {
            by_daypart(&today::events_by_daypart(&events(&i["events"]), s(&i["date"])))
        }
        ("today", "newInToday") => {
            let set_on: HashMap<String, Option<String>> = serde_json::from_value(i["setOn"].clone()).unwrap();
            json!(today::new_in_today(
                &account(i).tasks,
                &set_on,
                s(&i["today"]),
                opt_s(&i["seen"])
            ))
        }
        ("tag-tree", op) => {
            let account = account(i);
            let spaces: tag_tree::Spaces = serde_json::from_value(i["spaces"].clone()).unwrap_or_default();
            let tree = |t: Vec<(String, usize)>| {
                json!(t
                    .iter()
                    .map(|(id, depth)| json!({ "id": id, "depth": depth }))
                    .collect::<Vec<_>>())
            };
            match op {
                "tree" => tree(tag_tree::tree(&account.tags)),
                "validParents" => tree(tag_tree::valid_parents(&account.tags, s(&i["tagId"]), &spaces)),
                "nameIssue" => json!(tag_tree::name_issue(
                    &account.tags,
                    opt_s(&i["tagId"]),
                    s(&i["typed"]),
                    &spaces
                )),
                "openTaskCounts" => json!(tag_tree::open_task_counts(&account.tasks)),
                "deleteTag" => writes_in_order(&tag_tree::delete_tag(
                    &account.tasks,
                    &account.projects,
                    &account.tags,
                    s(&i["tagId"]),
                )),
                _ => panic!("unknown operation tag-tree/{op}"),
            }
        }
        ("search", "search") => json!(
            search::search(&account(i), s(&i["query"]), int(&i["limit"]) as usize)
                .iter()
                .map(|h| json!({ "kind": h.kind(), "id": h.id() }))
                .collect::<Vec<_>>()
        ),
        _ => panic!("unknown operation {suite}/{op}"),
    }
}

#[test]
fn the_manifest_is_the_version_this_crate_records() {
    let manifest = load("manifest.json");
    assert_eq!(manifest["version"], json!(CONFORMANCE_VERSION));
    assert_eq!(manifest["syncProtocol"], json!(SYNC_PROTOCOL));
}

#[test]
fn every_case_of_every_suite_passes() {
    let manifest = load("manifest.json");
    let mut failures = vec![];
    let mut total = 0;
    for entry in manifest["suites"].as_array().unwrap() {
        let suite = load(s(&entry["file"]));
        let name = s(&suite["suite"]);
        let cases = suite["cases"].as_array().unwrap();
        assert_eq!(
            cases.len() as i64,
            int(&entry["cases"]),
            "the case count of {name}"
        );
        let mut passed = 0;
        for case in cases {
            total += 1;
            let got = run(name, s(&case["op"]), &case["input"]);
            let expected = if name == "actions" {
                sorted(&case["expected"])
            } else {
                case["expected"].clone()
            };
            if same(&got, &expected) {
                passed += 1;
            } else {
                failures.push(format!(
                    "{name} / {}: {}\n  expected {expected}\n  got      {got}",
                    s(&case["op"]),
                    s(&case["name"])
                ));
            }
        }
        println!("{name}: {passed}/{}", cases.len());
    }
    assert!(
        failures.is_empty(),
        "{} of {total} cases fail:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
