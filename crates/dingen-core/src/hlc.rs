//! Hybrid logical clock timestamps (protocol.md, section 3).

use std::time::{SystemTime, UNIX_EPOCH};

/// Milliseconds since the Unix epoch on the system clock.
pub fn wall_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn format(wall: u64, counter: u32, client_id: &str) -> String {
    format!("{wall:015}:{counter:05}:{client_id}")
}

/// Splits a timestamp into wall time, counter and client id.
pub fn parse(ts: &str) -> Option<(u64, u32, &str)> {
    let mut parts = ts.splitn(3, ':');
    let wall = parts.next()?.parse().ok()?;
    let counter = parts.next()?.parse().ok()?;
    Some((wall, counter, parts.next()?))
}

/// Timestamps compare as plain strings, by code unit.
pub fn compare(a: &str, b: &str) -> std::cmp::Ordering {
    a.as_bytes().cmp(b.as_bytes())
}

#[derive(Debug, Clone)]
pub struct Clock {
    last_wall: u64,
    counter: u32,
    client_id: String,
}

impl Clock {
    pub fn new(client_id: impl Into<String>) -> Self {
        Clock {
            last_wall: 0,
            counter: 0,
            client_id: client_id.into(),
        }
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// A timestamp for a local change.
    pub fn now(&mut self) -> String {
        self.now_at(wall_now())
    }

    /// `now` with the wall clock reading given, so it can be tested.
    pub fn now_at(&mut self, wall: u64) -> String {
        if wall > self.last_wall {
            self.last_wall = wall;
            self.counter = 0;
        } else {
            self.counter += 1;
        }
        format(self.last_wall, self.counter, &self.client_id)
    }

    /// Takes a timestamp seen from elsewhere, so later local ones sort after it.
    pub fn receive(&mut self, remote: &str) {
        self.receive_at(wall_now(), remote)
    }

    pub fn receive_at(&mut self, wall: u64, remote: &str) {
        let Some((remote_wall, remote_counter, _)) = parse(remote) else {
            return;
        };
        let max_wall = wall.max(self.last_wall).max(remote_wall);
        if max_wall == self.last_wall && max_wall == remote_wall {
            self.counter = self.counter.max(remote_counter) + 1;
        } else if max_wall == self.last_wall {
            self.counter += 1;
        } else if max_wall == remote_wall {
            self.counter = remote_counter + 1;
        } else {
            self.counter = 0;
        }
        self.last_wall = max_wall;
    }
}
