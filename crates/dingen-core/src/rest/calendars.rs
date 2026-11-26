//! Calendars and their events (api.md §2.2).

use super::segment;
use crate::api::{Api, ApiError};
use serde_json::{json, Value};

impl Api {
    /// `{ calendars: [Calendar] }`.
    pub fn calendars(&self) -> Result<Value, ApiError> {
        self.call("GET", "/api/calendars", None)
    }

    /// Adds a calendar by its secret address; `{ calendar }`.
    pub fn add_calendar(
        &self,
        name: &str,
        url: &str,
        color: Option<&str>,
        area: Option<&str>,
    ) -> Result<Value, ApiError> {
        let body = json!({ "name": name, "url": url, "color": color, "areaId": area });
        self.call("POST", "/api/calendars", Some(&body))
    }

    /// Changes `name`, `color` or `areaId` (the fields given); `{ calendar }`.
    pub fn change_calendar(&self, id: &str, fields: &Value) -> Result<Value, ApiError> {
        self.call("PATCH", &format!("/api/calendars/{}", segment(id)), Some(fields))
    }

    pub fn remove_calendar(&self, id: &str) -> Result<(), ApiError> {
        self.call("DELETE", &format!("/api/calendars/{}", segment(id)), None)
            .map(|_| ())
    }

    /// The events touching `from` to `to` (dates, at most 62 days):
    /// `{ timezone, calendars, events, failed }`.
    pub fn events(&self, from: &str, to: &str) -> Result<Value, ApiError> {
        self.call_query(
            "/api/calendars/events",
            &[("from", from.into()), ("to", to.into())],
        )
    }
}
