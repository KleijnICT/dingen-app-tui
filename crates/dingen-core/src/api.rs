//! The HTTP side: signing in and the sync endpoints (protocol.md, sections 7,
//! 10 and 11). The other endpoints (api.md) are in `rest`, on the same `Api`.

use crate::misc::Cursor;
use crate::SYNC_PROTOCOL;
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub enum ApiError {
    /// No valid session: the user must sign in again.
    Unauthorized,
    /// 426: this client speaks a protocol the server no longer accepts.
    Outdated { message: String },
    /// Any other refusal, with the `error` code of the body.
    Refused { status: u16, error: String, body: Value },
    /// The server could not be reached, or answered 5xx or something unreadable.
    Unreachable(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Unauthorized => write!(f, "not signed in"),
            ApiError::Outdated { message } => write!(f, "{message}"),
            ApiError::Refused { status, error, .. } => write!(f, "refused ({status} {error})"),
            ApiError::Unreachable(reason) => write!(f, "server unreachable: {reason}"),
        }
    }
}

impl std::error::Error for ApiError {}

/// What `GET /api/health` says about a server.
#[derive(Debug, Clone, PartialEq)]
pub struct Health {
    pub app: String,
    pub version: String,
    pub sync_protocol: u64,
}

impl Health {
    /// Is this a Dingen server this client can sync with?
    pub fn usable(&self) -> bool {
        self.app == "Dingen" && self.sync_protocol >= SYNC_PROTOCOL as u64
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct User {
    pub id: String,
    pub email: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Login {
    /// Signed in; `token` is the session token to store securely.
    /// `recovery_codes_left` is there when a recovery code was used.
    SignedIn {
        user: User,
        token: String,
        recovery_codes_left: Option<u64>,
    },
    /// The account has two-step verification: ask for a code.
    TwoStep { challenge: String },
}

/// The push and pull of a sync round. The replica syncs through this, so it
/// can be tested without a server.
pub trait Transport {
    fn pull(&self, cursor: &Cursor) -> Result<Value, ApiError>;
    fn push(&self, changes: &Value, cursor: &Cursor) -> Result<Value, ApiError>;
}

#[derive(Debug, Clone)]
pub struct Api {
    base: String,
    token: Option<String>,
    agent: ureq::Agent,
}

/// A server address as the user typed it, made into a base URL.
pub fn normalize_server(input: &str) -> String {
    let trimmed = input.trim().trim_end_matches('/');
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    }
}

fn session_token(response: &ureq::Response) -> Option<String> {
    response.all("set-cookie").into_iter().find_map(|cookie| {
        let value = cookie.split(';').next()?.trim().strip_prefix("dingen_session=")?;
        (!value.is_empty()).then(|| value.to_string())
    })
}

fn user(body: &Value) -> Option<User> {
    let user = body.get("user")?;
    Some(User {
        id: user.get("id")?.as_str()?.to_string(),
        email: user.get("email")?.as_str()?.to_string(),
    })
}

impl Api {
    pub fn new(server: &str, token: Option<String>) -> Api {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .user_agent(concat!("dingen-core/", env!("CARGO_PKG_VERSION")))
            .build();
        Api {
            base: normalize_server(server),
            token,
            agent,
        }
    }

    pub fn server(&self) -> &str {
        &self.base
    }

    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    pub(crate) fn request(&self, method: &str, path: &str) -> ureq::Request {
        let mut request = self
            .agent
            .request(method, &format!("{}{path}", self.base))
            .set("x-dingen-protocol", &SYNC_PROTOCOL.to_string())
            .set("accept", "application/json");
        if let Some(token) = &self.token {
            request = request.set("cookie", &format!("dingen_session={token}"));
        }
        request
    }

    pub(crate) fn finish(
        result: Result<ureq::Response, ureq::Error>,
    ) -> Result<(Value, Option<String>), ApiError> {
        match result {
            Ok(response) => {
                let token = session_token(&response);
                let body = response
                    .into_json()
                    .map_err(|e| ApiError::Unreachable(e.to_string()))?;
                Ok((body, token))
            }
            Err(ureq::Error::Status(status, response)) => {
                let body: Value = response.into_json().unwrap_or(Value::Null);
                let error = body
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                Err(match status {
                    401 if error == "unauthorized" => ApiError::Unauthorized,
                    426 => ApiError::Outdated {
                        message: body
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("This app is out of date.")
                            .to_string(),
                    },
                    500..=599 => ApiError::Unreachable(format!("the server answered {status}")),
                    _ => ApiError::Refused { status, error, body },
                })
            }
            Err(other) => Err(ApiError::Unreachable(other.to_string())),
        }
    }

    fn get(&self, path: &str) -> Result<Value, ApiError> {
        Self::finish(self.request("GET", path).call()).map(|(body, _)| body)
    }

    pub fn health(&self) -> Result<Health, ApiError> {
        let body = self.get("/api/health")?;
        Ok(Health {
            app: body
                .get("app")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            version: body
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            sync_protocol: body.get("syncProtocol").and_then(Value::as_u64).unwrap_or(0),
        })
    }

    pub(crate) fn signed_in(&mut self, body: Value, token: Option<String>) -> Result<Login, ApiError> {
        if body.get("twoStep") == Some(&json!(true)) {
            if let Some(challenge) = body.get("challenge").and_then(Value::as_str) {
                return Ok(Login::TwoStep {
                    challenge: challenge.to_string(),
                });
            }
        }
        match (user(&body), token) {
            (Some(user), Some(token)) => {
                self.token = Some(token.clone());
                Ok(Login::SignedIn {
                    user,
                    token,
                    recovery_codes_left: body.get("recoveryCodesLeft").and_then(Value::as_u64),
                })
            }
            _ => Err(ApiError::Unreachable("the server did not start a session".into())),
        }
    }

    /// Signs in with a password. On success this client carries the session.
    pub fn login(&mut self, email: &str, password: &str) -> Result<Login, ApiError> {
        let result = self
            .request("POST", "/api/auth/login")
            .send_json(json!({ "email": email, "password": password }));
        let (body, token) = Self::finish(result)?;
        self.signed_in(body, token)
    }

    /// The second step: a code from the authenticator app, or a recovery code.
    pub fn two_step(&mut self, challenge: &str, code: &str) -> Result<Login, ApiError> {
        let result = self
            .request("POST", "/api/auth/login/two-step")
            .send_json(json!({ "challenge": challenge, "code": code }));
        let (body, token) = Self::finish(result)?;
        self.signed_in(body, token)
    }

    /// Who is signed in; `Unauthorized` when the session has ended.
    pub fn me(&self) -> Result<User, ApiError> {
        user(&self.get("/api/auth/me")?).ok_or_else(|| ApiError::Unreachable("no user in the answer".into()))
    }

    pub fn logout(&mut self) -> Result<(), ApiError> {
        let result = Self::finish(self.request("POST", "/api/auth/logout").call());
        self.token = None;
        result.map(|_| ())
    }
}

impl Transport for Api {
    fn pull(&self, cursor: &Cursor) -> Result<Value, ApiError> {
        let request = self
            .request("GET", "/api/sync/pull")
            .query("cursor", &json!(cursor).to_string());
        Self::finish(request.call()).map(|(body, _)| body)
    }

    fn push(&self, changes: &Value, cursor: &Cursor) -> Result<Value, ApiError> {
        let body = json!({ "changes": changes, "cursor": cursor });
        Self::finish(self.request("POST", "/api/sync/push").send_json(body)).map(|(body, _)| body)
    }
}
