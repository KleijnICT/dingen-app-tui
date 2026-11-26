//! Signing in and the account (protocol.md §10, api.md §3 and §5.1–5.3).

use super::segment;
use crate::api::{Api, ApiError, Login};
use serde_json::{json, Value};

/// What a sign-in screen may offer (`GET /api/auth/status`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    pub has_users: bool,
    pub registration_open: bool,
    /// `closed`, `open` or `invite`.
    pub registration: String,
    pub password_reset: bool,
    pub demo: bool,
}

impl Api {
    pub fn status(&self) -> Result<Status, ApiError> {
        let body = self.call("GET", "/api/auth/status", None)?;
        let flag = |k: &str| body.get(k).and_then(Value::as_bool).unwrap_or(false);
        Ok(Status {
            has_users: flag("hasUsers"),
            registration_open: flag("registrationOpen"),
            registration: body
                .get("registration")
                .and_then(Value::as_str)
                .unwrap_or("closed")
                .to_string(),
            password_reset: flag("passwordReset"),
            demo: flag("demo"),
        })
    }

    /// Makes an account and signs it in (§5.1). `invite` is the secret of an
    /// invitation link.
    pub fn register(&mut self, email: &str, password: &str, invite: Option<&str>) -> Result<Login, ApiError> {
        let mut body = json!({ "email": email, "password": password });
        if let Some(invite) = invite {
            body["invite"] = json!(invite);
        }
        let (body, token) = Self::finish(self.request("POST", "/api/auth/register").send_json(body))?;
        self.signed_in(body, token)
    }

    /// Checks an invitation before registering (§5.1).
    pub fn check_invite(&self, token: &str) -> Result<Value, ApiError> {
        self.call("GET", &format!("/api/auth/invite/{}", segment(token)), None)
    }

    /// Mails a reset link (§5.2); the same answer whether or not the address has an account.
    pub fn forgot(&self, email: &str, language: &str) -> Result<(), ApiError> {
        self.call(
            "POST",
            "/api/auth/forgot",
            Some(&json!({ "email": email, "language": language })),
        )
        .map(|_| ())
    }

    /// The whole answer of `GET /api/auth/me`, with every field of `user` (§3.1).
    pub fn me_full(&self) -> Result<Value, ApiError> {
        self.call("GET", "/api/auth/me", None)
    }

    /// Changes the password, or sets the first one (§3.2).
    pub fn change_password(&self, current: Option<&str>, password: &str) -> Result<(), ApiError> {
        let mut body = json!({ "password": password });
        if let Some(current) = current {
            body["currentPassword"] = json!(current);
        }
        self.call("POST", "/api/auth/password", Some(&body)).map(|_| ())
    }

    /// `{ enabled, recoveryCodesLeft }` (§3.3).
    pub fn two_step_status(&self) -> Result<Value, ApiError> {
        self.call("GET", "/api/auth/two-step", None)
    }

    /// A new secret: `{ secret, uri }`.
    pub fn two_step_setup(&self) -> Result<Value, ApiError> {
        self.call("POST", "/api/auth/two-step/setup", None)
    }

    /// Confirms the secret with a code; the recovery codes, shown once.
    pub fn two_step_enable(&self, code: &str) -> Result<Vec<String>, ApiError> {
        let body = self.call(
            "POST",
            "/api/auth/two-step/enable",
            Some(&json!({ "code": code })),
        )?;
        Ok(body
            .get("recoveryCodes")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default())
    }

    /// Turns it off with the password, or else with a code.
    pub fn two_step_disable(&self, password: Option<&str>, code: Option<&str>) -> Result<(), ApiError> {
        let body = match (password, code) {
            (Some(password), _) => json!({ "password": password }),
            (None, Some(code)) => json!({ "code": code }),
            (None, None) => json!({}),
        };
        self.call("POST", "/api/auth/two-step/disable", Some(&body))
            .map(|_| ())
    }

    /// Stores the day "New in Today" was dismissed (§3.4).
    pub fn today_seen(&self, date: &str) -> Result<(), ApiError> {
        self.call("POST", "/api/auth/today-seen", Some(&json!({ "date": date })))
            .map(|_| ())
    }

    /// The demo accounts of a demo server (§5.3).
    pub fn demo_accounts(&self) -> Result<Value, ApiError> {
        self.call("GET", "/api/auth/demo", None)
    }

    /// Signs in as a demo account, without a password.
    pub fn demo_sign_in(&mut self, email: &str) -> Result<Login, ApiError> {
        let path = format!("/api/auth/demo/{}", segment(email));
        let (body, token) = Self::finish(self.request("POST", &path).send_json(json!({})))?;
        self.signed_in(body, token)
    }
}
