//! The endpoints besides sync (api.md), as methods of `Api`. Data goes only
//! through the sync protocol; signing in, account, settings, calendars,
//! agents, activity, administration, export and import use these.
//!
//! Answers are returned as JSON (`serde_json::Value`) in the shape api.md
//! gives; errors as `ApiError`, with the `error` code in `Refused`.

mod account;
mod admin;
mod agents;
mod calendars;

pub use account::Status;
pub use agents::ActivityQuery;

use crate::api::{Api, ApiError};
use serde_json::Value;

/// A path segment with everything but unreserved characters escaped.
pub(crate) fn segment(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

impl Api {
    /// A JSON request to an api.md endpoint; `body` is sent as JSON.
    pub fn call(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value, ApiError> {
        let request = self.request(method, path);
        let result = match body {
            Some(body) => request.send_json(body.clone()),
            None => request.call(),
        };
        Self::finish(result).map(|(body, _)| body)
    }

    /// A JSON request with query parameters.
    pub fn call_query(&self, path: &str, query: &[(&str, String)]) -> Result<Value, ApiError> {
        let mut request = self.request("GET", path);
        for (key, value) in query {
            request = request.query(key, value);
        }
        Self::finish(request.call()).map(|(body, _)| body)
    }

    /// A file the server sends (a back-up, an export), as bytes.
    pub fn download(&self, path: &str) -> Result<Vec<u8>, ApiError> {
        match self.request("GET", path).set("accept", "*/*").call() {
            Ok(response) => {
                let mut bytes = vec![];
                std::io::Read::read_to_end(&mut response.into_reader(), &mut bytes)
                    .map_err(|e| ApiError::Unreachable(e.to_string()))?;
                Ok(bytes)
            }
            Err(error) => Self::finish(Err(error)).map(|_| vec![]),
        }
    }

    /// Bytes sent as a JSON body (an export file to import).
    pub fn upload(&self, path: &str, bytes: &[u8]) -> Result<Value, ApiError> {
        let result = self
            .request("POST", path)
            .set("content-type", "application/json")
            .send_bytes(bytes);
        Self::finish(result).map(|(body, _)| body)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn segments_are_escaped() {
        assert_eq!(super::segment("cas@example.com"), "cas%40example.com");
        assert_eq!(super::segment("dingen-manual-1.db"), "dingen-manual-1.db");
    }
}
