//! The local replica: records, the queue of unsent changes and the cursor, in
//! one SQLite file, and the client algorithm of the sync protocol
//! (protocol.md, section 8).
//!
//! A replica belongs to one user on one server. Keep a file per user.

use crate::actions::Write;
use crate::api::{ApiError, Transport};
use crate::hlc::Clock;
use crate::misc::{merge_cursor, Cursor};
use crate::model::{Account, Obj};
use crate::record::{Change, Record};
use crate::{entities, ids};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

/// The most changes one push may carry.
const PUSH_BATCH: usize = 1000;

#[derive(Debug)]
pub enum ReplicaError {
    Storage(rusqlite::Error),
    Api(ApiError),
    /// The server refused a batch as malformed; it is not sent again unchanged.
    InvalidBatch(Value),
}

impl std::fmt::Display for ReplicaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplicaError::Storage(e) => write!(f, "local storage: {e}"),
            ReplicaError::Api(e) => write!(f, "{e}"),
            ReplicaError::InvalidBatch(_) => write!(f, "the server refused the changes as malformed"),
        }
    }
}

impl std::error::Error for ReplicaError {}

impl From<rusqlite::Error> for ReplicaError {
    fn from(e: rusqlite::Error) -> Self {
        ReplicaError::Storage(e)
    }
}

impl From<ApiError> for ReplicaError {
    fn from(e: ApiError) -> Self {
        ReplicaError::Api(e)
    }
}

type Result<T> = std::result::Result<T, ReplicaError>;

/// What a sync round did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SyncReport {
    pub pushed: usize,
    /// Changes the server will never accept; they were dropped from the queue.
    pub refused: Vec<String>,
    /// Did the pull change anything in the replica?
    pub changed: bool,
}

pub struct Replica {
    db: Connection,
    clock: Clock,
}

fn json_text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".into())
}

fn get_record(
    tx: &Connection,
    entity_type: &str,
    id: &str,
) -> rusqlite::Result<Option<(Record, Option<String>)>> {
    tx.query_row(
        "SELECT data, field_ts, space_id FROM records WHERE entity_type = ?1 AND id = ?2",
        params![entity_type, id],
        |row| {
            let data: String = row.get(0)?;
            let field_ts: String = row.get(1)?;
            Ok((
                Record {
                    entity_type: entity_type.to_string(),
                    id: id.to_string(),
                    data: serde_json::from_str(&data).unwrap_or_default(),
                    field_ts: serde_json::from_str(&field_ts).unwrap_or_default(),
                },
                row.get(2)?,
            ))
        },
    )
    .optional()
}

fn put_record(tx: &Connection, record: &Record, space_id: Option<&str>) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO records (entity_type, id, space_id, data, field_ts) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (entity_type, id) DO UPDATE SET space_id = ?3, data = ?4, field_ts = ?5",
        params![
            record.entity_type,
            record.id,
            space_id,
            json_text(&record.data),
            json_text(&record.field_ts)
        ],
    )?;
    Ok(())
}

fn get_meta(db: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    db.query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| row.get(0))
        .optional()
}

fn set_meta(db: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    db.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = ?2",
        params![key, value],
    )?;
    Ok(())
}

fn queued(db: &Connection, limit: usize) -> rusqlite::Result<Vec<Change>> {
    let mut statement = db.prepare("SELECT change FROM queue ORDER BY seq LIMIT ?1")?;
    let rows = statement.query_map([limit as i64], |row| row.get::<_, String>(0))?;
    Ok(rows
        .filter_map(|text| serde_json::from_str(&text.ok()?).ok())
        .collect())
}

fn records_of(value: Option<&Value>) -> Vec<Record> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|r| serde_json::from_value::<Record>(r.clone()).ok())
        .filter(|r| entities::is_known_type(&r.entity_type))
        .collect()
}

impl Replica {
    /// Opens the replica in a file, or makes a new one, and prepares the clock
    /// (start-up, section 8.2).
    pub fn open(path: &Path) -> Result<Replica> {
        Self::start(Connection::open(path)?)
    }

    /// A replica that lives in memory, for tests.
    pub fn in_memory() -> Result<Replica> {
        Self::start(Connection::open_in_memory()?)
    }

    fn start(db: Connection) -> Result<Replica> {
        db.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS records (
               entity_type TEXT NOT NULL, id TEXT NOT NULL, space_id TEXT,
               data TEXT NOT NULL, field_ts TEXT NOT NULL,
               PRIMARY KEY (entity_type, id));
             CREATE TABLE IF NOT EXISTS queue (
               seq INTEGER PRIMARY KEY AUTOINCREMENT, change_id TEXT NOT NULL UNIQUE,
               entity_type TEXT NOT NULL, entity_id TEXT NOT NULL, change TEXT NOT NULL);",
        )?;
        let client_id = match get_meta(&db, "clientId")? {
            Some(id) => id,
            None => {
                let id = ids::client_id();
                set_meta(&db, "clientId", &id)?;
                id
            }
        };
        // A queued change of a type this client cannot send would be refused
        // and block the queue. (Records of such types are never stored.)
        let known: Vec<String> = crate::model::ENTITY_TYPES
            .iter()
            .map(|t| format!("'{t}'"))
            .collect();
        db.execute(
            &format!("DELETE FROM queue WHERE entity_type NOT IN ({})", known.join(",")),
            [],
        )?;
        db.execute(
            &format!(
                "DELETE FROM records WHERE entity_type NOT IN ({})",
                known.join(",")
            ),
            [],
        )?;

        let mut clock = Clock::new(client_id);
        {
            let mut statement = db.prepare("SELECT field_ts FROM records")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            for text in rows {
                let stamps: BTreeMap<String, String> = serde_json::from_str(&text?).unwrap_or_default();
                stamps.values().for_each(|ts| clock.receive(ts));
            }
        }
        Ok(Replica { db, clock })
    }

    pub fn client_id(&self) -> &str {
        self.clock.client_id()
    }

    pub fn cursor(&self) -> Result<Cursor> {
        Ok(get_meta(&self.db, "cursor")?
            .and_then(|raw| crate::misc::parse_cursor(&raw))
            .unwrap_or_default())
    }

    /// The number of changes waiting to be sent.
    pub fn queue_len(&self) -> Result<usize> {
        Ok(self
            .db
            .query_row("SELECT count(*) FROM queue", [], |row| row.get::<_, i64>(0))? as usize)
    }

    /// Has this replica ever finished a pull?
    pub fn has_synced(&self) -> Result<bool> {
        Ok(get_meta(&self.db, "cursor")?.is_some())
    }

    /// Everything in the replica, deleted entities included.
    pub fn account(&self) -> Result<Account> {
        let mut account = Account::default();
        let mut statement = self.db.prepare("SELECT entity_type, data FROM records")?;
        let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
        for row in rows {
            let (entity_type, data) = row?;
            if let Ok(data) = serde_json::from_str::<Obj>(&data) {
                account.add(&entity_type, &data);
            }
        }
        Ok(account)
    }

    /// A local edit (section 8.3): every write becomes one change, applied to
    /// its record and queued in the same transaction.
    pub fn write(&mut self, writes: &[Write]) -> Result<()> {
        let tx = self.db.transaction()?;
        for write in writes {
            let (entity_type, entity_id, mut fields) = match write {
                Write::Create { entity_type, entity } => {
                    let id = entity
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    (entity_type.clone(), id, entity.clone())
                }
                Write::Change {
                    entity_type,
                    entity_id,
                    fields,
                } => (entity_type.clone(), entity_id.clone(), fields.clone()),
            };
            fields.remove("id");
            if fields.is_empty() || !entities::is_known_type(&entity_type) {
                continue;
            }
            let change = Change {
                id: ids::ulid(),
                entity_type,
                entity_id,
                fields,
                ts: self.clock.now(),
                client_id: self.clock.client_id().to_string(),
            };
            let (mut record, space) = get_record(&tx, &change.entity_type, &change.entity_id)?
                .unwrap_or_else(|| (Record::empty(&change.entity_type, &change.entity_id), None));
            record.apply(&change.fields, &change.ts);
            // A move names the target space; the record takes it at once.
            let space = change
                .fields
                .get("spaceId")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or(space);
            put_record(&tx, &record, space.as_deref())?;
            tx.execute(
                "INSERT INTO queue (change_id, entity_type, entity_id, change) VALUES (?1, ?2, ?3, ?4)",
                params![
                    change.id,
                    change.entity_type,
                    change.entity_id,
                    json_text(&change)
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// One sync round (section 8.4): push the queue, then pull what is new.
    pub fn sync(&mut self, transport: &dyn Transport) -> Result<SyncReport> {
        let mut report = SyncReport::default();
        self.push(transport, &mut report)?;
        self.pull(transport, &mut report)?;
        Ok(report)
    }

    fn push(&mut self, transport: &dyn Transport, report: &mut SyncReport) -> Result<()> {
        loop {
            let batch = queued(&self.db, PUSH_BATCH)?;
            if batch.is_empty() {
                return Ok(());
            }
            let cursor = self.cursor()?;
            let sent: Vec<&str> = batch.iter().map(|c| c.id.as_str()).collect();
            match transport.push(&serde_json::json!(batch), &cursor) {
                Ok(answer) => {
                    let tx = self.db.transaction()?;
                    for id in &sent {
                        tx.execute("DELETE FROM queue WHERE change_id = ?1", [id])?;
                    }
                    for placed in answer
                        .get("placed")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        tx.execute(
                            "UPDATE records SET space_id = ?3 WHERE entity_type = ?1 AND id = ?2",
                            params![
                                placed["entityType"].as_str(),
                                placed["id"].as_str(),
                                placed["spaceId"].as_str()
                            ],
                        )?;
                    }
                    if let Some(update) = answer
                        .get("cursor")
                        .and_then(|c| crate::misc::parse_cursor(&c.to_string()))
                    {
                        set_meta(&tx, "cursor", &json_text(&merge_cursor(&cursor, Some(&update))))?;
                    }
                    tx.commit()?;
                    report.pushed += sent.len();
                }
                Err(ApiError::Refused {
                    status: 400,
                    error,
                    body,
                }) if error == "invalid_changes" => {
                    // These will never be accepted. Left in the queue they would
                    // block everything after them.
                    let refused: Vec<String> = body
                        .get("problems")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|p| p.get("changeId")?.as_str().map(str::to_string))
                        .collect();
                    if refused.is_empty() {
                        return Err(ReplicaError::InvalidBatch(body));
                    }
                    for id in &refused {
                        self.db.execute("DELETE FROM queue WHERE change_id = ?1", [id])?;
                    }
                    report.refused.extend(refused);
                }
                Err(ApiError::Refused {
                    status: 400, body, ..
                }) => return Err(ReplicaError::InvalidBatch(body)),
                Err(other) => return Err(other.into()),
            }
        }
    }

    fn pull(&mut self, transport: &dyn Transport, report: &mut SyncReport) -> Result<()> {
        let mut cursor = self.cursor()?;
        loop {
            let answer = transport.pull(&cursor)?;
            let next = answer
                .get("cursor")
                .and_then(|c| crate::misc::parse_cursor(&c.to_string()))
                .ok_or_else(|| ApiError::Unreachable("the pull answer has no cursor".into()))?;
            let tx = self.db.transaction()?;
            report.changed |= apply_pull(&tx, &mut self.clock, &answer)?;
            set_meta(&tx, "cursor", &json_text(&next))?;
            tx.commit()?;
            cursor = next;
            if answer.get("hasMore") != Some(&Value::Bool(true)) {
                return Ok(());
            }
        }
    }

    /// Forgets everything: for signing out. The file keeps its client id out
    /// of reach of the next user by being removed by the caller.
    pub fn clear(&mut self) -> Result<()> {
        self.db.execute_batch(
            "DELETE FROM records; DELETE FROM queue; DELETE FROM meta WHERE key = 'cursor';",
        )?;
        Ok(())
    }
}

/// Applies one pull answer (section 8.7), in the order the protocol gives.
fn apply_pull(tx: &Transaction, clock: &mut Clock, answer: &Value) -> rusqlite::Result<bool> {
    let spaces: Vec<&Value> = answer
        .get("spaces")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .collect();
    let space_id = |space: &Value| {
        space
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let mut changed = false;

    // 1. Spaces to drop: the ones the user left, and the ones that come whole.
    let mut dropped: HashSet<String> = answer
        .get("removed")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_string))
        .collect();
    dropped.extend(
        spaces
            .iter()
            .filter(|s| s.get("entities").is_some())
            .map(|s| space_id(s)),
    );

    let queue = if dropped.is_empty() {
        vec![]
    } else {
        queued(tx, usize::MAX >> 1)?
    };

    // 2. Drop their records, but keep unsent work.
    if !dropped.is_empty() {
        let personal = spaces
            .iter()
            .find(|s| s.get("kind").and_then(Value::as_str) == Some("personal"))
            .map(|s| space_id(s));
        let unsent: HashSet<(&str, &str)> = queue
            .iter()
            .map(|c| (c.entity_type.as_str(), c.entity_id.as_str()))
            .collect();
        let rows: Vec<(String, String, Option<String>)> = {
            let mut statement = tx.prepare("SELECT entity_type, id, space_id FROM records")?;
            let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for (entity_type, id, space) in rows {
            // A record without a space counts as living in the personal space.
            let lives_in = space.or_else(|| personal.clone());
            if lives_in.is_some_and(|s| dropped.contains(&s))
                && !unsent.contains(&(entity_type.as_str(), id.as_str()))
            {
                tx.execute(
                    "DELETE FROM records WHERE entity_type = ?1 AND id = ?2",
                    params![entity_type, id],
                )?;
                changed = true;
            }
        }
    }

    // 3. Snapshots replace what was there.
    let mut replaced: HashSet<(String, String)> = HashSet::new();
    for space in &spaces {
        for record in records_of(space.get("entities")) {
            record.field_ts.values().for_each(|ts| clock.receive(ts));
            put_record(tx, &record, Some(&space_id(space)))?;
            replaced.insert((record.entity_type.clone(), record.id.clone()));
            changed = true;
        }
    }

    // 4. Unsent changes go back on top of the records that were replaced.
    for change in &queue {
        if replaced.contains(&(change.entity_type.clone(), change.entity_id.clone())) {
            if let Some((mut record, space)) = get_record(tx, &change.entity_type, &change.entity_id)? {
                record.apply(&change.fields, &change.ts);
                put_record(tx, &record, space.as_deref())?;
            }
        }
    }

    // 5. Changes, oldest first. A change never moves a record.
    for space in &spaces {
        for value in space
            .get("changes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Ok(change) = serde_json::from_value::<Change>(value.clone()) else {
                continue;
            };
            clock.receive(&change.ts);
            if !entities::is_known_type(&change.entity_type) {
                continue;
            }
            let (mut record, lives_in) = get_record(tx, &change.entity_type, &change.entity_id)?
                .unwrap_or_else(|| {
                    (
                        Record::empty(&change.entity_type, &change.entity_id),
                        Some(space_id(space)),
                    )
                });
            record.apply(&change.fields, &change.ts);
            put_record(tx, &record, lives_in.as_deref())?;
            changed = true;
        }
    }

    // 6. Arrivals come as whole records, wherever they lived before.
    for space in &spaces {
        for record in records_of(space.get("arrived")) {
            record.field_ts.values().for_each(|ts| clock.receive(ts));
            put_record(tx, &record, Some(&space_id(space)))?;
            changed = true;
        }
    }

    // 7. Departures.
    for space in &spaces {
        for key in space.get("left").and_then(Value::as_array).into_iter().flatten() {
            tx.execute(
                "DELETE FROM records WHERE entity_type = ?1 AND id = ?2",
                params![key["entityType"].as_str(), key["id"].as_str()],
            )?;
            changed = true;
        }
    }
    Ok(changed)
}
