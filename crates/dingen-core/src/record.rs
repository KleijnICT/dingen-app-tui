//! A record in the replica and how a change is applied to it (protocol.md,
//! section 4): last writer wins, per field.

use crate::model::Obj;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub fields: Obj,
    pub ts: String,
    pub client_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub entity_type: String,
    pub id: String,
    pub data: Obj,
    #[serde(default)]
    pub field_ts: BTreeMap<String, String>,
}

impl Record {
    pub fn empty(entity_type: &str, id: &str) -> Record {
        let mut data = Obj::new();
        data.insert("id".into(), json!(id));
        Record {
            entity_type: entity_type.into(),
            id: id.into(),
            data,
            field_ts: BTreeMap::new(),
        }
    }

    /// Applies a change. A field only takes the new value when the change is
    /// newer than what set it last; on equal timestamps the value stays.
    pub fn apply(&mut self, fields: &Obj, ts: &str) {
        for (field, value) in fields {
            if field == "id" || field == "spaceId" {
                continue;
            }
            let newer = self
                .field_ts
                .get(field)
                .is_none_or(|current| ts.as_bytes() > current.as_bytes());
            if newer {
                self.data.insert(field.clone(), value.clone());
                self.field_ts.insert(field.clone(), ts.to_string());
            }
        }
    }

    pub fn is_deleted(&self) -> bool {
        self.data.get("deleted").and_then(Value::as_bool).unwrap_or(false)
    }
}

/// Applies changes in order to a record, or to nothing when the entity is new.
pub fn apply_changes(existing: Option<Record>, changes: &[Change]) -> Option<Record> {
    let mut record = existing;
    for change in changes {
        record
            .get_or_insert_with(|| Record::empty(&change.entity_type, &change.entity_id))
            .apply(&change.fields, &change.ts);
    }
    record
}
