//! Agents, members, access and activity (api.md §4, protocol.md §11.2–11.3).

use super::segment;
use crate::api::{Api, ApiError};
use serde_json::Value;

/// The filters of `GET /api/activity`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ActivityQuery {
    pub kinds: Vec<String>,
    pub actor: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub before: Option<u64>,
    pub limit: Option<u32>,
}

impl Api {
    /// `{ agents: [Agent] }`.
    pub fn agents(&self) -> Result<Value, ApiError> {
        self.call("GET", "/api/agents", None)
    }

    /// `{ name, icon, color, grants }`; answers `{ agent, token }` (the token only here).
    pub fn create_agent(&self, body: &Value) -> Result<Value, ApiError> {
        self.call("POST", "/api/agents", Some(body))
    }

    /// The fields given; `grants` replaces all of them. `{ agent }`.
    pub fn change_agent(&self, id: &str, fields: &Value) -> Result<Value, ApiError> {
        self.call("PATCH", &format!("/api/agents/{}", segment(id)), Some(fields))
    }

    /// A new token; the old one stops at once. `{ agent, token }`.
    pub fn new_agent_token(&self, id: &str) -> Result<Value, ApiError> {
        self.call("POST", &format!("/api/agents/{}/token", segment(id)), None)
    }

    pub fn revoke_agent(&self, id: &str) -> Result<(), ApiError> {
        self.call("DELETE", &format!("/api/agents/{}", segment(id)), None)
            .map(|_| ())
    }

    /// `{ members: [{ id, kind, name, icon, color }] }`.
    pub fn members(&self) -> Result<Value, ApiError> {
        self.call("GET", "/api/members", None)
    }

    /// Who reaches an area or project: `{ members: [{ member, level }] }`.
    pub fn access(&self, kind: &str, id: &str) -> Result<Value, ApiError> {
        self.call_query("/api/access", &[("type", kind.into()), ("id", id.into())])
    }

    /// A page of the activity log: `{ items, nextBefore }`.
    pub fn activity(&self, query: &ActivityQuery) -> Result<Value, ApiError> {
        let mut params: Vec<(&str, String)> = vec![];
        if !query.kinds.is_empty() {
            params.push(("kinds", query.kinds.join(",")));
        }
        if let Some(actor) = &query.actor {
            params.push(("actor", actor.clone()));
        }
        if let Some(since) = &query.since {
            params.push(("since", since.clone()));
        }
        if let Some(until) = &query.until {
            params.push(("until", until.clone()));
        }
        if let Some(before) = query.before {
            params.push(("before", before.to_string()));
        }
        if let Some(limit) = query.limit {
            params.push(("limit", limit.to_string()));
        }
        self.call_query("/api/activity", &params)
    }
}
