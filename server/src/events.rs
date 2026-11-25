//! Server-Sent Events hub (`GET /api/v1/events`).

use serde::Serialize;
use tokio::sync::broadcast;

/// One SSE message: `event: <name>` / `data: <json>`.
#[derive(Debug, Clone)]
pub struct Event {
    pub name: &'static str,
    pub data: String,
}

#[derive(Clone)]
pub struct Hub {
    tx: broadcast::Sender<Event>,
}

impl Default for Hub {
    fn default() -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self { tx }
    }
}

impl Hub {
    /// Publish to every connected client. No subscribers is not an error.
    pub fn publish<T: Serialize>(&self, name: &'static str, data: &T) {
        match serde_json::to_string(data) {
            Ok(data) => {
                let _ = self.tx.send(Event { name, data });
            }
            Err(e) => tracing::warn!("Could not serialise {name} event: {e}"),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }
}
