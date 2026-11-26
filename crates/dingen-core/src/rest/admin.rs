//! Administration and data (api.md §5.4–5.10).

use super::segment;
use crate::api::{Api, ApiError};
use serde_json::{json, Value};

impl Api {
    /// `closed`, `invite` or `open`.
    pub fn registration(&self) -> Result<String, ApiError> {
        let body = self.call("GET", "/api/admin/registration", None)?;
        Ok(body
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("closed")
            .to_string())
    }

    pub fn set_registration(&self, mode: &str) -> Result<(), ApiError> {
        self.call("PUT", "/api/admin/registration", Some(&json!({ "mode": mode })))
            .map(|_| ())
    }

    /// `{ invitations: [Invitation] }`, newest first.
    pub fn invitations(&self) -> Result<Value, ApiError> {
        self.call("GET", "/api/invitations", None)
    }

    /// `{ token, invitation }`; the token only here. The link is `<server>/?invite=<token>`.
    pub fn create_invitation(&self) -> Result<Value, ApiError> {
        self.call("POST", "/api/invitations", None)
    }

    pub fn withdraw_invitation(&self, id: &str) -> Result<(), ApiError> {
        self.call("DELETE", &format!("/api/invitations/{}", segment(id)), None)
            .map(|_| ())
    }

    /// `{ users: [AdminUser] }`, oldest first.
    pub fn users(&self) -> Result<Value, ApiError> {
        self.call("GET", "/api/admin/users", None)
    }

    /// Makes or unmakes an administrator, blocks or unblocks; `{ user }`.
    pub fn change_user(
        &self,
        id: &str,
        is_admin: Option<bool>,
        blocked: Option<bool>,
    ) -> Result<Value, ApiError> {
        let mut body = json!({});
        if let Some(is_admin) = is_admin {
            body["isAdmin"] = json!(is_admin);
        }
        if let Some(blocked) = blocked {
            body["blocked"] = json!(blocked);
        }
        self.call("PATCH", &format!("/api/admin/users/{}", segment(id)), Some(&body))
    }

    /// Deletes a blocked account; `confirm` is its email address.
    pub fn delete_user(&self, id: &str, confirm: &str) -> Result<Value, ApiError> {
        self.call(
            "DELETE",
            &format!("/api/admin/users/{}", segment(id)),
            Some(&json!({ "confirm": confirm })),
        )
    }

    /// A reset link to pass on by hand: `{ token, url, expiresAt }`.
    pub fn reset_link(&self, id: &str) -> Result<Value, ApiError> {
        self.call(
            "POST",
            &format!("/api/admin/users/{}/reset-link", segment(id)),
            None,
        )
    }

    pub fn disable_user_two_step(&self, id: &str) -> Result<(), ApiError> {
        self.call(
            "POST",
            &format!("/api/admin/users/{}/totp/disable", segment(id)),
            None,
        )
        .map(|_| ())
    }

    /// `{ backups, canRestore }`, newest first.
    pub fn backups(&self) -> Result<Value, ApiError> {
        self.call("GET", "/api/admin/backups", None)
    }

    /// Makes a manual back-up now; `{ backup }`.
    pub fn create_backup(&self) -> Result<Value, ApiError> {
        self.call("POST", "/api/admin/backups", None)
    }

    /// The back-up file itself.
    pub fn download_backup(&self, name: &str) -> Result<Vec<u8>, ApiError> {
        self.download(&format!("/api/admin/backups/{}", segment(name)))
    }

    /// Puts a back-up in place; the server restarts (poll `health` until it answers).
    pub fn restore_backup(&self, name: &str) -> Result<Value, ApiError> {
        self.call(
            "POST",
            &format!("/api/admin/backups/{}/restore", segment(name)),
            None,
        )
    }

    /// `{ configured, from }`.
    pub fn mail(&self) -> Result<Value, ApiError> {
        self.call("GET", "/api/admin/mail", None)
    }

    /// Sends a test mail to the administrator; `{ ok, to }`.
    pub fn test_mail(&self, language: &str) -> Result<Value, ApiError> {
        self.call(
            "POST",
            "/api/admin/mail/test",
            Some(&json!({ "language": language })),
        )
    }

    /// Everything the user owns, as the export file.
    pub fn export(&self) -> Result<Vec<u8>, ApiError> {
        self.download("/api/export")
    }

    /// Reads an export file into the account; `mode` is `merge` or `reject`.
    /// `{ mode, counts, total, seq }`.
    pub fn import(&self, file: &[u8], mode: &str) -> Result<Value, ApiError> {
        self.upload(&format!("/api/import?mode={}", segment(mode)), file)
    }
}
