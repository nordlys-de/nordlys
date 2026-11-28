//! The D-Bus side: `org.freedesktop.Notifications`, the standard every app
//! sends to, plus `de.nordlys.Notifications` on the same object so the
//! bar can read counts and recent notifications and switch Do not
//! disturb.

use super::store::{Closed, Request, Store, Urgency, plain};
use futures::channel::mpsc::{UnboundedSender, unbounded};
use futures::{SinkExt, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Type, Value};
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
    /// The panel asked for an action on a notification (id, key).
    Invoke(u32, String),
    /// These toasts were taken off screen by the panel: tell their senders.
    Gone(Vec<u32>),
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

/// One notification as the panel reads it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Entry {
    pub id: u32,
    pub app: String,
    pub summary: String,
    pub body: String,
    /// (key, label) pairs the sender offered.
    pub actions: Vec<(String, String)>,
    /// Seconds since it arrived.
    pub age: u64,
    /// Seconds until a snoozed one comes back; 0 when it isn't snoozed.
    pub snoozed: u64,
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

    /// Newest first.
    fn recent(&self, count: u32) -> Vec<Entry> {
        let now = Instant::now();
        lock(&self.store)
            .recent(count as usize)
            .into_iter()
            .map(|n| Entry {
                id: n.id,
                app: n.app.clone(),
                summary: n.summary.clone(),
                body: n.body.clone(),
                actions: n.actions.clone(),
                age: now.saturating_duration_since(n.received).as_secs(),
                snoozed: n
                    .snoozed_until
                    .map_or(0, |t| t.saturating_duration_since(now).as_secs()),
            })
            .collect()
    }

    /// Apps whose notifications are kept out of sight.
    fn muted(&self) -> Vec<String> {
        lock(&self.store).muted.iter().cloned().collect()
    }

    /// Run one of a notification's actions, as clicking it on a toast would.
    fn invoke(&self, id: u32, key: String) {
        let _ = self.ping.unbounded_send(Event::Invoke(id, key));
    }

    /// Remove a notification from the list.
    fn dismiss(&self, id: u32) {
        if lock(&self.store).dismiss(id) {
            let _ = self.ping.unbounded_send(Event::Gone(vec![id]));
        }
        let _ = self.ping.unbounded_send(Event::Changed);
    }

    /// Put a notification away for `minutes`, then bring it back as a toast.
    fn snooze(&self, id: u32, minutes: u32) {
        let until = Instant::now() + Duration::from_secs(u64::from(minutes) * 60);
        if lock(&self.store).snooze(id, until) {
            let _ = self.ping.unbounded_send(Event::Gone(vec![id]));
        }
        let _ = self.ping.unbounded_send(Event::Changed);
    }

    /// Stop (or resume) showing an app's notifications.
    fn mute(&self, app: String, on: bool) {
        {
            let mut store = lock(&self.store);
            store.set_muted(&app, on);
            save_muted(&store.muted);
        }
        let _ = self.ping.unbounded_send(Event::Changed);
    }

    /// Remove every notification from one app.
    fn clear_app(&self, app: String) {
        let shown = lock(&self.store).clear_app(&app);
        if !shown.is_empty() {
            let _ = self.ping.unbounded_send(Event::Gone(shown));
        }
        let _ = self.ping.unbounded_send(Event::Changed);
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
        lock(&store).muted = load_muted();
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

/// Where muted apps are kept: `$XDG_STATE_HOME/nordlys/muted-apps`, one per line.
fn muted_file() -> Option<std::path::PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/state"))
        })?;
    Some(state.join("nordlys/muted-apps"))
}

fn load_muted() -> std::collections::BTreeSet<String> {
    muted_file()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| {
            s.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Best effort: a failed save only means the mute is forgotten at restart.
fn save_muted(apps: &std::collections::BTreeSet<String>) {
    let Some(path) = muted_file() else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let text: String = apps.iter().map(|a| format!("{a}\n")).collect();
    let _ = std::fs::write(path, text);
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
