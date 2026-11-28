//! The D-Bus side: `org.freedesktop.Notifications`, the standard every app
//! sends to, plus `de.nordlys.Notifications` on the same object so the
//! bar can read counts and recent notifications and switch Do not
//! disturb.

use super::store::{Closed, Request, Store, Urgency, plain};
use futures::channel::mpsc::{UnboundedSender, unbounded};
use futures::{SinkExt, Stream, StreamExt};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};
use zbus::{Connection, interface};

pub const NAME: &str = "org.freedesktop.Notifications";
pub const PATH: &str = "/org/freedesktop/Notifications";
pub const CONTROL: &str = "de.nordlys.Notifications";

pub type Shared = Arc<Mutex<Store>>;

#[derive(Debug, Clone)]
pub enum Event {
    Ready {
        conn: Connection,
        store: Shared,
    },
    /// Something in the store changed.
    Changed,
    /// The bus or the name isn't ours.
    Failed(String),
}

/// Lock the store, recovering from a poisoned lock (a panic elsewhere
/// shouldn't take notifications down with it).
pub fn lock(store: &Shared) -> std::sync::MutexGuard<'_, Store> {
    store.lock().unwrap_or_else(|e| e.into_inner())
}

struct Server {
    store: Shared,
    ping: UnboundedSender<Event>,
}

fn hint_str(hints: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    match hints.get(key).map(|v| &**v) {
        Some(Value::Str(s)) => Some(s.to_string()),
        _ => None,
    }
}

#[interface(name = "org.freedesktop.Notifications")]
impl Server {
    fn get_capabilities(&self) -> Vec<String> {
        ["actions", "body", "persistence"]
            .map(String::from)
            .to_vec()
    }

    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        _app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let app = if app_name.trim().is_empty() {
            hint_str(&hints, "desktop-entry").unwrap_or_else(|| "Notification".into())
        } else {
            app_name
        };
        let req = Request {
            app,
            replaces: replaces_id,
            summary: plain(&summary),
            body: plain(&body),
            actions: actions
                .chunks(2)
                .filter_map(|c| match c {
                    [key, label] => Some((key.clone(), label.clone())),
                    _ => None,
                })
                .collect(),
            urgency: hints
                .get("urgency")
                .and_then(|v| v.downcast_ref::<u8>().ok())
                .map(Urgency::from_byte),
            timeout: expire_timeout,
            transient: hints
                .get("transient")
                .and_then(|v| v.downcast_ref::<bool>().ok())
                .unwrap_or(false),
        };
        let id = lock(&self.store).notify(req, Instant::now());
        let _ = self.ping.unbounded_send(Event::Changed);
        id
    }

    async fn close_notification(
        &self,
        id: u32,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let was_shown = lock(&self.store).close(id, Closed::Called);
        if was_shown {
            Self::notification_closed(&emitter, id, Closed::Called as u32).await?;
        }
        let _ = self.ping.unbounded_send(Event::Changed);
        Ok(())
    }

    #[zbus(out_args("name", "vendor", "version", "spec_version"))]
    fn get_server_information(&self) -> (String, String, String, String) {
        (
            "Nordlys".into(),
            "nordlys-de".into(),
            env!("CARGO_PKG_VERSION").into(),
            "1.2".into(),
        )
    }

    #[zbus(signal)]
    async fn notification_closed(
        emitter: &SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(
        emitter: &SignalEmitter<'_>,
        id: u32,
        action_key: &str,
    ) -> zbus::Result<()>;
}

struct Control {
    store: Shared,
    ping: UnboundedSender<Event>,
}

#[interface(name = "de.nordlys.Notifications")]
impl Control {
    #[zbus(out_args("waiting", "history", "dnd"))]
    fn status(&self) -> (u32, u32, bool) {
        lock(&self.store).counts()
    }

    /// Newest first: (id, app, summary, body).
    fn recent(&self, count: u32) -> Vec<(u32, String, String, String)> {
        lock(&self.store)
            .recent(count as usize)
            .into_iter()
            .map(|n| (n.id, n.app.clone(), n.summary.clone(), n.body.clone()))
            .collect()
    }

    fn set_dnd(&self, on: bool) {
        lock(&self.store).set_dnd(on, Instant::now());
        let _ = self.ping.unbounded_send(Event::Changed);
    }

    fn show_last(&self) -> bool {
        let shown = lock(&self.store).show_last(Instant::now());
        let _ = self.ping.unbounded_send(Event::Changed);
        shown
    }

    fn clear(&self) {
        lock(&self.store).clear();
        let _ = self.ping.unbounded_send(Event::Changed);
    }
}

/// Connect, take the well-known name and report what happens.
pub fn serve() -> impl Stream<Item = Event> {
    iced::stream::channel(32, async |mut output| {
        let store: Shared = Arc::default();
        let (ping, mut pings) = unbounded();
        let conn = match connect(&store, &ping).await {
            Ok(conn) => conn,
            Err(e) => {
                let _ = output.send(Event::Failed(e)).await;
                return;
            }
        };
        let _ = output
            .send(Event::Ready {
                conn: conn.clone(),
                store,
            })
            .await;
        while let Some(ev) = pings.next().await {
            if output.send(ev).await.is_err() {
                break;
            }
        }
    })
}

async fn connect(store: &Shared, ping: &UnboundedSender<Event>) -> Result<Connection, String> {
    let conn = zbus::connection::Builder::session()
        .and_then(|b| {
            b.serve_at(
                PATH,
                Server {
                    store: store.clone(),
                    ping: ping.clone(),
                },
            )
        })
        .and_then(|b| {
            b.serve_at(
                PATH,
                Control {
                    store: store.clone(),
                    ping: ping.clone(),
                },
            )
        })
        .map_err(|e| e.to_string())?
        .build()
        .await
        .map_err(|e| format!("can't reach the session bus: {e}"))?;
    let taken = || {
        format!(
            "another notification daemon owns {NAME}; stop it first (for dunst: `killall dunst`, \
             and remove it from your autostart)"
        )
    };
    match conn
        .request_name_with_flags(NAME, RequestNameFlags::DoNotQueue.into())
        .await
    {
        Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => Ok(conn),
        Ok(_) => Err(taken()),
        Err(zbus::Error::NameTaken) => Err(taken()),
        Err(e) => Err(e.to_string()),
    }
}

/// Tell the sender a toast went away.
pub async fn closed(conn: Connection, id: u32, reason: Closed) {
    let _ = conn
        .emit_signal(
            None::<zbus::names::BusName<'_>>,
            PATH,
            NAME,
            "NotificationClosed",
            &(id, reason as u32),
        )
        .await;
}

/// Tell the sender one of its buttons (or the toast itself) was clicked.
pub async fn invoked(conn: Connection, id: u32, key: String) {
    let _ = conn
        .emit_signal(
            None::<zbus::names::BusName<'_>>,
            PATH,
            NAME,
            "ActionInvoked",
            &(id, key.as_str()),
        )
        .await;
}
