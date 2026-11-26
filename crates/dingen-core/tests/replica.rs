//! The client algorithm of the sync protocol, against a scripted server.

use dingen_core::actions::Write;
use dingen_core::api::{ApiError, Transport};
use dingen_core::misc::Cursor;
use dingen_core::model::Obj;
use dingen_core::replica::Replica;
use dingen_core::{entities, ids};
use serde_json::{json, Value};
use std::cell::RefCell;

/// Answers pulls from a script and records what is pushed.
struct Script {
    pulls: RefCell<Vec<Value>>,
    pushes: RefCell<Vec<Value>>,
    push_answers: RefCell<Vec<Result<Value, ApiError>>>,
}

impl Script {
    fn new(pulls: Vec<Value>) -> Script {
        Script {
            pulls: RefCell::new(pulls),
            pushes: RefCell::new(vec![]),
            push_answers: RefCell::new(vec![]),
        }
    }
}

impl Transport for Script {
    fn pull(&self, _cursor: &Cursor) -> Result<Value, ApiError> {
        Ok(self.pulls.borrow_mut().remove(0))
    }

    fn push(&self, changes: &Value, _cursor: &Cursor) -> Result<Value, ApiError> {
        self.pushes.borrow_mut().push(changes.clone());
        let mut answers = self.push_answers.borrow_mut();
        if answers.is_empty() {
            Ok(json!({ "applied": changes.as_array().map_or(0, Vec::len), "duplicates": 0 }))
        } else {
            answers.remove(0)
        }
    }
}

const SPACE: &str = "01M83XJ8801W82E44THBXM8WSS";
const TASK: &str = "01M83XJA6GPT2FAVXC6RP7S4X3";

fn fields(value: Value) -> Obj {
    value.as_object().cloned().unwrap()
}

fn task_record(title: &str, ts: &str) -> Value {
    let data = entities::new_entity(
        "task",
        TASK,
        "2026-03-10T09:00:00.000Z",
        &fields(json!({ "title": title })),
    )
    .unwrap();
    let field_ts: serde_json::Map<String, Value> = data
        .keys()
        .filter(|k| *k != "id")
        .map(|k| (k.clone(), json!(ts)))
        .collect();
    json!({ "entityType": "task", "id": TASK, "data": data, "fieldTs": field_ts })
}

fn pull(space: Value, seq: u64) -> Value {
    json!({ "protocol": 4, "spaces": [space], "removed": [], "cursor": { SPACE: seq }, "hasMore": false })
}

fn snapshot(records: Vec<Value>, seq: u64) -> Value {
    pull(
        json!({ "id": SPACE, "kind": "personal", "level": "manage", "seq": seq, "changes": [], "entities": records, "hasMore": false }),
        seq,
    )
}

fn title(replica: &Replica) -> Option<String> {
    replica.account().unwrap().task(TASK).map(|t| t.title.clone())
}

#[test]
fn a_first_pull_fills_the_replica_and_stores_the_cursor() {
    let mut replica = Replica::in_memory().unwrap();
    assert!(!replica.has_synced().unwrap());
    let server = Script::new(vec![snapshot(
        vec![task_record("Buy milk", "000000000001000:00000:REMOTE")],
        7,
    )]);
    let report = replica.sync(&server).unwrap();
    assert!(report.changed);
    assert_eq!(title(&replica).as_deref(), Some("Buy milk"));
    assert_eq!(replica.cursor().unwrap().get(SPACE), Some(&7));
}

#[test]
fn a_change_is_applied_per_field_and_an_older_one_loses() {
    let mut replica = Replica::in_memory().unwrap();
    let change = |id: &str, title: &str, ts: &str| json!({ "id": id, "entityType": "task", "entityId": TASK, "fields": { "title": title }, "ts": ts, "clientId": "REMOTE" });
    let space = |changes: Value| json!({ "id": SPACE, "kind": "personal", "level": "manage", "seq": 9, "changes": changes, "hasMore": false });
    let server = Script::new(vec![
        snapshot(vec![task_record("Buy milk", "000000000001000:00000:REMOTE")], 7),
        pull(
            space(json!([change(
                "01M83XJB5RA3VVDZTFTHGWSEBT",
                "Buy oat milk",
                "000000000003000:00000:REMOTE"
            )])),
            8,
        ),
        pull(
            space(json!([change(
                "01M83XJC50D9YKVDF8Z7QH9FTG",
                "Too late",
                "000000000002000:00000:REMOTE"
            )])),
            9,
        ),
    ]);
    replica.sync(&server).unwrap();
    replica.sync(&server).unwrap();
    assert_eq!(title(&replica).as_deref(), Some("Buy oat milk"));
    replica.sync(&server).unwrap();
    assert_eq!(title(&replica).as_deref(), Some("Buy oat milk"));
}

#[test]
fn a_local_edit_is_queued_pushed_and_survives_a_resync() {
    let mut replica = Replica::in_memory().unwrap();
    let server = Script::new(vec![snapshot(
        vec![task_record("Buy milk", "000000000001000:00000:REMOTE")],
        7,
    )]);
    replica.sync(&server).unwrap();

    let edit = Write::Change {
        entity_type: "task".into(),
        entity_id: TASK.into(),
        fields: fields(json!({ "title": "Mine" })),
    };
    replica.write(&[edit]).unwrap();
    assert_eq!(replica.queue_len().unwrap(), 1);
    assert_eq!(title(&replica).as_deref(), Some("Mine"));

    // The server is unreachable for the push: the queue stays.
    let down = Script::new(vec![]);
    down.push_answers
        .borrow_mut()
        .push(Err(ApiError::Unreachable("offline".into())));
    assert!(replica.sync(&down).is_err());
    assert_eq!(replica.queue_len().unwrap(), 1);

    // A resync snapshot replaces the record; the unsent edit goes back on top.
    let mut resync = snapshot(vec![task_record("Theirs", "000000000002000:00000:REMOTE")], 12);
    resync["spaces"][0]["resync"] = json!(true);
    let refusing = Script::new(vec![resync]);
    refusing.push_answers.borrow_mut().push(Err(ApiError::Refused {
        status: 400,
        error: "invalid_changes".into(),
        body: json!({ "error": "invalid_changes", "problems": [] }),
    }));
    assert!(
        replica.sync(&refusing).is_err(),
        "a batch refused without named changes is not dropped"
    );
    assert_eq!(replica.queue_len().unwrap(), 1);

    let up = Script::new(vec![snapshot(
        vec![task_record("Theirs", "000000000002000:00000:REMOTE")],
        12,
    )]);
    let report = replica.sync(&up).unwrap();
    assert_eq!(report.pushed, 1);
    assert_eq!(replica.queue_len().unwrap(), 0);
    let pushed = up.pushes.borrow()[0].clone();
    assert_eq!(pushed[0]["fields"], json!({ "title": "Mine" }));
    assert_eq!(pushed[0]["clientId"], json!(replica.client_id()));
    assert!(ids::is_ulid(pushed[0]["id"].as_str().unwrap()));
}

#[test]
fn a_refused_change_leaves_the_queue_so_the_rest_is_sent() {
    let mut replica = Replica::in_memory().unwrap();
    let write = |title: &str| Write::Change {
        entity_type: "task".into(),
        entity_id: TASK.into(),
        fields: fields(json!({ "title": title })),
    };
    replica.write(&[write("One"), write("Two")]).unwrap();
    let server = Script::new(vec![snapshot(vec![], 3)]);
    // Learn the id of the first change from the first push, then refuse it.
    struct RefuseFirst<'a>(&'a Script, RefCell<bool>);
    impl Transport for RefuseFirst<'_> {
        fn pull(&self, cursor: &Cursor) -> Result<Value, ApiError> {
            self.0.pull(cursor)
        }
        fn push(&self, changes: &Value, cursor: &Cursor) -> Result<Value, ApiError> {
            if !self.1.replace(true) {
                let id = changes[0]["id"].clone();
                return Err(ApiError::Refused {
                    status: 400,
                    error: "invalid_changes".into(),
                    body: json!({ "error": "invalid_changes", "problems": [{ "changeId": id, "issues": ["title: no"] }] }),
                });
            }
            self.0.push(changes, cursor)
        }
    }
    let report = replica.sync(&RefuseFirst(&server, RefCell::new(false))).unwrap();
    assert_eq!(report.refused.len(), 1);
    assert_eq!(report.pushed, 1);
    assert_eq!(server.pushes.borrow()[0][0]["fields"], json!({ "title": "Two" }));
    assert_eq!(replica.queue_len().unwrap(), 0);
}

#[test]
fn unknown_entity_types_are_ignored_and_the_cursor_moves_on() {
    let mut replica = Replica::in_memory().unwrap();
    let unknown = json!({ "entityType": "note", "id": TASK, "data": { "id": TASK }, "fieldTs": {} });
    let server = Script::new(vec![snapshot(vec![unknown], 5)]);
    replica.sync(&server).unwrap();
    assert!(replica.account().unwrap().tasks.is_empty());
    assert_eq!(replica.cursor().unwrap().get(SPACE), Some(&5));
}

#[test]
fn an_arrival_and_a_departure() {
    let mut replica = Replica::in_memory().unwrap();
    let space = |extra: Value| {
        let mut space = json!({ "id": SPACE, "kind": "personal", "level": "manage", "seq": 4, "changes": [], "hasMore": false });
        space
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().cloned().unwrap());
        space
    };
    let server = Script::new(vec![
        snapshot(vec![], 3),
        pull(
            space(json!({ "arrived": [task_record("Arrived", "000000000001000:00000:REMOTE")] })),
            4,
        ),
        pull(
            space(json!({ "left": [{ "entityType": "task", "id": TASK }] })),
            5,
        ),
    ]);
    replica.sync(&server).unwrap();
    replica.sync(&server).unwrap();
    assert_eq!(title(&replica).as_deref(), Some("Arrived"));
    replica.sync(&server).unwrap();
    assert_eq!(title(&replica), None);
}
