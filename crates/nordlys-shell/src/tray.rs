//! The system tray: StatusNotifierItem icons (Discord, Steam, VPN clients,
//! Nextcloud…) in the bar, with their menus.
//!
//! Two roles, both on the session bus:
//!
//! * **Watcher** (`org.kde.StatusNotifierWatcher`): where apps register
//!   their icons. Every bar offers to be it, queued for the name, so when
//!   the one that has it exits the next takes over and the apps register
//!   again with it (they watch the name for that).
//! * **Host**: each bar reads the registered items and draws them. It
//!   listens for the items' and the watcher's signals and reads the items
//!   again shortly after any of them, so a new icon or status shows at once.
//!
//! Menus are com.canonical.dbusmenu, which is what libappindicator, Qt and
//! Electron apps export; they open as a panel under the bar.

use iced::Subscription;
use iced::widget::{image, svg};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::mpsc as std_mpsc;
use std::time::Duration;
use zbus::blocking::{Connection, MessageIterator};
use zbus::fdo::RequestNameFlags;
use zbus::message::{Header, Type as MsgType};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM: &str = "org.kde.StatusNotifierItem";
const MENU: &str = "com.canonical.dbusmenu";
const DBUS: &str = "org.freedesktop.DBus";
const PROPS: &str = "org.freedesktop.DBus.Properties";
/// A stuck app is treated as gone rather than freezing the tray.
const PATIENCE: Duration = Duration::from_millis(800);
/// Changes come in bursts (icon, title and tooltip together); read once.
const SETTLE: Duration = Duration::from_millis(150);
/// Pixel size the icons are drawn at, before scaling.
pub const SIZE: u16 = 16;

/// One icon in the tray.
#[derive(Debug, Clone)]
pub struct Item {
    /// The bus name and object path, which identify it.
    pub service: String,
    pub path: String,
    pub title: String,
    pub icon: Icon,
    /// The dbusmenu object, when the app exports one.
    pub menu: Option<String>,
    /// A left click should open the menu, not activate the app.
    pub only_menu: bool,
    pub attention: bool,
}

#[derive(Debug, Clone)]
pub enum Icon {
    Image(image::Handle),
    /// A symbolic SVG takes the text colour.
    Svg(svg::Handle, bool),
    /// Nothing to draw: the title's first letter.
    Letter(char),
}

/// What a click on an icon does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Click {
    Primary,
    Middle,
}

/// One row of an app's menu.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: i32,
    pub label: String,
    pub enabled: bool,
    pub separator: bool,
    /// `Some(on)` for a checkmark or radio item.
    pub toggle: Option<bool>,
    pub children: Vec<Entry>,
}

// ---- Watcher ----

#[derive(Default)]
struct Watcher {
    items: Vec<String>,
    hosts: Vec<String>,
}

/// `name` is a bus name or an object path (libappindicator sends its
/// path; the sender is then the bus name). Stored as "bus/path".
fn entry_for(service: &str, sender: &str) -> String {
    if service.starts_with('/') {
        format!("{sender}{service}")
    } else if service.contains('/') {
        service.to_owned()
    } else {
        format!("{service}/StatusNotifierItem")
    }
}

/// "bus/path" back into its parts.
pub fn split_entry(entry: &str) -> (String, String) {
    match entry.find('/') {
        Some(i) => (entry[..i].to_owned(), entry[i..].to_owned()),
        None => (entry.to_owned(), "/StatusNotifierItem".to_owned()),
    }
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    async fn register_status_notifier_item(
        &mut self,
        service: &str,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let sender = header.sender().map(|s| s.to_string()).unwrap_or_default();
        let entry = entry_for(service, &sender);
        if self.items.contains(&entry) {
            return;
        }
        self.items.push(entry.clone());
        let _ = Self::status_notifier_item_registered(&emitter, &entry).await;
        let _ = self
            .registered_status_notifier_items_changed(&emitter)
            .await;
    }

    async fn register_status_notifier_host(
        &mut self,
        service: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        if !self.hosts.iter().any(|h| h == service) {
            self.hosts.push(service.to_owned());
        }
        let _ = Self::status_notifier_host_registered(&emitter).await;
        let _ = self
            .is_status_notifier_host_registered_changed(&emitter)
            .await;
    }

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.items.clone()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        !self.hosts.is_empty()
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_item_unregistered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_registered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

/// Serve the watcher on `bus` and queue for its name. Items and hosts
/// whose bus name goes away are dropped.
fn serve_watcher(bus: &Connection) -> zbus::Result<()> {
    bus.object_server().at(WATCHER_PATH, Watcher::default())?;
    bus.request_name_with_flags(WATCHER, RequestNameFlags::AllowReplacement.into())?;
    let bus = bus.clone();
    std::thread::spawn(move || {
        let Ok(rule) = zbus::MatchRule::builder()
            .msg_type(MsgType::Signal)
            .sender(DBUS)
            .and_then(|r| r.interface(DBUS))
            .and_then(|r| r.member("NameOwnerChanged"))
            .map(|r| r.build())
        else {
            return;
        };
        let Ok(signals) = MessageIterator::for_match_rule(rule, &bus, Some(64)) else {
            return;
        };
        let Ok(iface) = bus.object_server().interface::<_, Watcher>(WATCHER_PATH) else {
            return;
        };
        for msg in signals.flatten() {
            let Ok((name, _old, new)) = msg.body().deserialize::<(String, String, String)>() else {
                continue;
            };
            if !new.is_empty() {
                continue;
            }
            let gone: Vec<String> = {
                let mut w = iface.get_mut();
                let gone = w
                    .items
                    .iter()
                    .filter(|e| split_entry(e).0 == name)
                    .cloned()
                    .collect();
                w.items.retain(|e| split_entry(e).0 != name);
                w.hosts.retain(|h| *h != name);
                gone
            };
            for e in &gone {
                let _ = zbus::block_on(Watcher::status_notifier_item_unregistered(
                    iface.signal_emitter(),
                    e,
                ));
            }
            if !gone.is_empty() {
                let w = iface.get();
                let _ = zbus::block_on(
                    w.registered_status_notifier_items_changed(iface.signal_emitter()),
                );
            }
        }
    });
    Ok(())
}

// ---- Host ----

fn bus() -> Option<&'static Connection> {
    static BUS: OnceLock<Option<Connection>> = OnceLock::new();
    BUS.get_or_init(|| {
        zbus::blocking::connection::Builder::session()
            .ok()?
            .method_timeout(PATIENCE)
            .build()
            .ok()
    })
    .as_ref()
}

/// The tray's icons, sent again whenever they change. Nothing without a
/// session bus.
pub fn items() -> Subscription<Vec<Item>> {
    Subscription::run(host)
}

fn host() -> futures::channel::mpsc::UnboundedReceiver<Vec<Item>> {
    let (tx, rx) = futures::channel::mpsc::unbounded();
    std::thread::spawn(move || {
        let Some(bus) = bus() else {
            eprintln!("nordlys-shell: no session bus, so no tray");
            return;
        };
        if let Err(e) = serve_watcher(bus) {
            eprintln!("nordlys-shell: tray watcher: {e}");
        }
        let host_name = format!("org.kde.StatusNotifierHost-{}", std::process::id());
        let _ = bus.request_name(host_name.as_str());
        let register = || {
            let _ = bus.call_method(
                Some(WATCHER),
                WATCHER_PATH,
                Some(WATCHER),
                "RegisterStatusNotifierHost",
                &(host_name.as_str(),),
            );
        };
        register();

        // Any signal from an item or the watcher, or the watcher's name
        // moving, means "read again".
        let (poke, poked) = std_mpsc::channel::<Poke>();
        let listen = |rule: zbus::Result<zbus::MatchRule<'static>>, kind: Poke| {
            let Ok(rule) = rule else { return };
            let poke = poke.clone();
            let Ok(signals) = MessageIterator::for_match_rule(rule, bus, Some(256)) else {
                return;
            };
            std::thread::spawn(move || {
                for m in signals.flatten() {
                    let h = m.header();
                    let kind = if h.member().is_some_and(|n| n == "NewToolTip") {
                        Poke::Tooltip(h.sender().map(|s| s.to_string()).unwrap_or_default())
                    } else {
                        kind.clone()
                    };
                    if poke.send(kind).is_err() {
                        return;
                    }
                }
            });
        };
        let signal = || zbus::MatchRule::builder().msg_type(MsgType::Signal);
        listen(signal().interface(ITEM).map(|r| r.build()), Poke::Read);
        listen(signal().interface(WATCHER).map(|r| r.build()), Poke::Read);
        listen(
            signal()
                .sender(DBUS)
                .and_then(|r| r.member("NameOwnerChanged"))
                .and_then(|r| r.arg(0, WATCHER))
                .map(|r| r.build()),
            Poke::Rewatch,
        );

        let mut icons = IconCache::default();
        let mut last: Option<Vec<String>> = None;
        loop {
            let (items, tooltip_titled) = read_items(bus, &mut icons);
            let keys: Vec<String> = items.iter().map(fingerprint).collect();
            if last.as_ref() != Some(&keys) {
                last = Some(keys);
                if tx.unbounded_send(items).is_err() {
                    return;
                }
            }
            // Wait for a change (or check anyway every so often), then let
            // the burst settle.
            // A tooltip changing alone (some apps tick theirs every
            // second) only matters to an icon titled by its tooltip. A
            // signal names its sender's unique name, so an item known by
            // another name is always read.
            let wanted = |p: &Poke| match p {
                Poke::Tooltip(from) => tooltip_titled
                    .iter()
                    .any(|s| s == from || !s.starts_with(':')),
                _ => true,
            };
            let first = loop {
                match poked.recv_timeout(Duration::from_secs(20)) {
                    Ok(p) if !wanted(&p) => continue,
                    Ok(p) => break p,
                    Err(std_mpsc::RecvTimeoutError::Timeout) => break Poke::Read,
                    Err(std_mpsc::RecvTimeoutError::Disconnected) => {
                        std::thread::sleep(Duration::from_secs(20));
                        break Poke::Read;
                    }
                }
            };
            let mut rewatch = first == Poke::Rewatch;
            std::thread::sleep(SETTLE);
            while let Ok(p) = poked.try_recv() {
                rewatch |= p == Poke::Rewatch;
            }
            if rewatch {
                // A new watcher: tell it about us.
                register();
            }
        }
    });
    rx
}

/// Why the list should be read again.
#[derive(Clone, PartialEq, Eq)]
enum Poke {
    /// Something an icon shows may have changed.
    Read,
    /// Only a tooltip changed, that of the item with this bus name.
    Tooltip(String),
    /// The watcher changed hands: register with the new one.
    Rewatch,
}

/// What decides whether the bar needs the list again.
fn fingerprint(i: &Item) -> String {
    format!(
        "{}{}|{}|{:?}|{}|{}",
        i.service,
        i.path,
        i.title,
        match &i.icon {
            Icon::Image(h) => format!("{:?}", h.id()),
            Icon::Svg(h, s) => format!("{:?}{s}", h.id()),
            Icon::Letter(c) => c.to_string(),
        },
        i.menu.is_some(),
        i.attention
    )
}

/// The items, and the bus names of those titled by their tooltip.
fn read_items(bus: &Connection, icons: &mut IconCache) -> (Vec<Item>, Vec<String>) {
    let entries: Vec<String> = bus
        .call_method(
            Some(WATCHER),
            WATCHER_PATH,
            Some(PROPS),
            "Get",
            &(WATCHER, "RegisteredStatusNotifierItems"),
        )
        .ok()
        .and_then(|m| m.body().deserialize::<OwnedValue>().ok())
        .and_then(|v| Vec::<String>::try_from(v).ok())
        .unwrap_or_default();
    let mut tooltips = Vec::new();
    let items = entries
        .iter()
        .filter_map(|e| {
            let (service, path) = split_entry(e);
            read_item(bus, service, path, icons, &mut tooltips)
        })
        .collect();
    (items, tooltips)
}

fn read_item(
    bus: &Connection,
    service: String,
    path: String,
    icons: &mut IconCache,
    tooltip_titled: &mut Vec<String>,
) -> Option<Item> {
    let props: HashMap<String, OwnedValue> = bus
        .call_method(
            Some(service.as_str()),
            path.as_str(),
            Some(PROPS),
            "GetAll",
            &(ITEM,),
        )
        .ok()?
        .body()
        .deserialize()
        .ok()?;
    let text = |k: &str| {
        props
            .get(k)
            .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
            .unwrap_or_default()
    };
    let status = text("Status");
    // Passive items ask to be hidden until they matter.
    if status == "Passive" {
        return None;
    }
    let attention = status == "NeedsAttention";
    let tooltip = tooltip_title(&props);
    let titled = text("Title");
    if titled.is_empty() && !tooltip.is_empty() {
        tooltip_titled.push(service.clone());
    }
    let title = [titled, tooltip, text("Id")]
        .into_iter()
        .find(|t| !t.is_empty())
        .unwrap_or_else(|| service.clone());
    let theme_path = text("IconThemePath");
    let (name, pixmap) = if attention {
        ("AttentionIconName", "AttentionIconPixmap")
    } else {
        ("IconName", "IconPixmap")
    };
    let icon = icons
        .named(&text(name), &theme_path)
        .or_else(|| {
            props
                .get(pixmap)
                .and_then(|v| icons.pixmap(&service, &path, v))
        })
        .or_else(|| icons.named(&text("IconName"), &theme_path))
        .or_else(|| {
            props
                .get("IconPixmap")
                .and_then(|v| icons.pixmap(&service, &path, v))
        })
        .unwrap_or_else(|| {
            Icon::Letter(
                title
                    .chars()
                    .find(|c| c.is_alphanumeric())
                    .unwrap_or('?')
                    .to_ascii_uppercase(),
            )
        });
    let menu = props
        .get("Menu")
        .and_then(|v| match &**v {
            Value::ObjectPath(p) => Some(p.to_string()),
            _ => None,
        })
        .filter(|p| p != "/");
    let only_menu = props
        .get("ItemIsMenu")
        .and_then(|v| bool::try_from(v).ok())
        .unwrap_or(false);
    Some(Item {
        service,
        path,
        title,
        icon,
        menu,
        only_menu,
        attention,
    })
}

/// ToolTip is (icon name, icon pixmaps, title, text).
fn tooltip_title(props: &HashMap<String, OwnedValue>) -> String {
    let Some(Value::Structure(s)) = props.get("ToolTip").map(|v| &**v) else {
        return String::new();
    };
    match s.fields().get(2) {
        Some(Value::Str(t)) => t.to_string(),
        _ => String::new(),
    }
}

/// Icons already looked up or decoded, so a redraw doesn't upload them again.
#[derive(Default)]
struct IconCache {
    named: HashMap<(String, String), Option<Icon>>,
    pixmaps: HashMap<(String, String, u64), Icon>,
}

impl IconCache {
    fn named(&mut self, name: &str, theme_path: &str) -> Option<Icon> {
        if name.is_empty() {
            return None;
        }
        self.named
            .entry((name.to_owned(), theme_path.to_owned()))
            .or_insert_with(|| {
                // Status icons (network, battery, volume) often come as a
                // dark PNG made for light panels, with a symbolic SVG beside
                // it. The symbolic one, in the bar's text colour, reads on
                // both light and dark glass. App icons have none and keep
                // their colours.
                let symbolic_name = format!("{name}-symbolic");
                let (file, symbolic) = match name.ends_with("-symbolic") {
                    true => (find_icon(name, theme_path)?, true),
                    false => match find_icon(&symbolic_name, theme_path)
                        .filter(|f| f.extension().is_some_and(|e| e == "svg"))
                    {
                        Some(f) => (f, true),
                        None => (find_icon(name, theme_path)?, false),
                    },
                };
                Some(if file.extension().is_some_and(|e| e == "svg") {
                    Icon::Svg(svg::Handle::from_path(file), symbolic)
                } else {
                    Icon::Image(image::Handle::from_path(file))
                })
            })
            .clone()
    }

    /// IconPixmap is a(iiay): sizes of ARGB32 in network byte order. The
    /// one nearest twice the drawn size stays sharp on HiDPI screens.
    fn pixmap(&mut self, service: &str, path: &str, v: &OwnedValue) -> Option<Icon> {
        let Value::Array(sizes) = &**v else {
            return None;
        };
        let mut best: Option<(i32, i32, Vec<u8>)> = None;
        for size in sizes.iter() {
            let Value::Structure(s) = size else { continue };
            let [Value::I32(w), Value::I32(h), Value::Array(bytes)] = s.fields() else {
                continue;
            };
            let (w, h) = (*w, *h);
            if w <= 0 || h <= 0 {
                continue;
            }
            let want = i32::from(SIZE) * 2;
            let better = best
                .as_ref()
                .is_none_or(|(bw, _, _)| (w - want).abs() < (bw - want).abs());
            if better {
                let data: Vec<u8> = bytes
                    .iter()
                    .filter_map(|b| match b {
                        Value::U8(b) => Some(*b),
                        _ => None,
                    })
                    .collect();
                if data.len() == (w * h * 4) as usize {
                    best = Some((w, h, data));
                }
            }
        }
        let (w, h, argb) = best?;
        let hash = {
            use std::hash::{Hash, Hasher};
            let mut s = std::collections::hash_map::DefaultHasher::new();
            argb.hash(&mut s);
            s.finish()
        };
        let key = (service.to_owned(), path.to_owned(), hash);
        if let Some(icon) = self.pixmaps.get(&key) {
            return Some(icon.clone());
        }
        // Drop this item's older pixmaps.
        self.pixmaps
            .retain(|(s, p, _), _| !(s == service && p == path));
        let icon = Icon::Image(image::Handle::from_rgba(
            w as u32,
            h as u32,
            argb_to_rgba(&argb),
        ));
        self.pixmaps.insert(key, icon.clone());
        Some(icon)
    }
}

fn argb_to_rgba(argb: &[u8]) -> Vec<u8> {
    argb.as_chunks::<4>()
        .0
        .iter()
        .flat_map(|[a, r, g, b]| [*r, *g, *b, *a])
        .collect()
}

// ---- Icon themes ----

/// The icon theme GTK uses, else Adwaita.
fn theme_name() -> String {
    let conf = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    conf.and_then(|c| std::fs::read_to_string(c.join("gtk-3.0/settings.ini")).ok())
        .and_then(|s| {
            s.lines().find_map(|l| {
                let (k, v) = l.split_once('=')?;
                (k.trim() == "gtk-icon-theme-name").then(|| v.trim().to_owned())
            })
        })
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "Adwaita".into())
}

fn icon_dirs() -> &'static [PathBuf] {
    static DIRS: OnceLock<Vec<PathBuf>> = OnceLock::new();
    DIRS.get_or_init(|| {
        let mut out = vec![];
        if let Some(h) = std::env::var_os("HOME") {
            out.push(PathBuf::from(&h).join(".icons"));
        }
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
        out.extend(data_home.map(|d| d.join("icons")));
        let dirs = std::env::var("XDG_DATA_DIRS")
            .ok()
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        out.extend(dirs.split(':').map(|d| Path::new(d).join("icons")));
        out
    })
}

/// A file for an icon name: an absolute path as is, else the app's own
/// theme path, the GTK theme, hicolor and the pixmaps folder. Scalable
/// first, then the largest PNG.
pub fn find_icon(name: &str, theme_path: &str) -> Option<PathBuf> {
    let path = Path::new(name);
    if path.is_absolute() {
        return path.is_file().then(|| path.to_path_buf());
    }
    if !theme_path.is_empty() {
        let dir = Path::new(theme_path);
        for ext in ["svg", "png"] {
            let f = dir.join(format!("{name}.{ext}"));
            if f.is_file() {
                return Some(f);
            }
        }
        if let Some(f) = in_theme(dir, name) {
            return Some(f);
        }
    }
    let themes = [
        theme_name(),
        "Adwaita".into(),
        "breeze".into(),
        "hicolor".into(),
    ];
    for theme in &themes {
        for base in icon_dirs() {
            if let Some(f) = in_theme(&base.join(theme), name) {
                return Some(f);
            }
        }
    }
    for ext in ["svg", "png"] {
        let f = Path::new("/usr/share/pixmaps").join(format!("{name}.{ext}"));
        if f.is_file() {
            return Some(f);
        }
    }
    None
}

/// Look in a theme's size/context folders (`48x48/apps`, `scalable/status`…).
fn in_theme(theme: &Path, name: &str) -> Option<PathBuf> {
    let sizes = std::fs::read_dir(theme).ok()?;
    let mut found: Vec<(u32, PathBuf)> = vec![];
    for size in sizes.flatten() {
        let size_name = size.file_name().to_string_lossy().into_owned();
        // Bigger is better; scalable and symbolic beat every PNG.
        let rank = if size_name == "scalable" || size_name == "symbolic" {
            10_000
        } else {
            size_name
                .split(['x', '@'])
                .next()
                .and_then(|n| n.parse().ok())
                .unwrap_or(0)
        };
        let Ok(contexts) = std::fs::read_dir(size.path()) else {
            continue;
        };
        for ctx in contexts.flatten() {
            for ext in ["svg", "png"] {
                let f = ctx.path().join(format!("{name}.{ext}"));
                if f.is_file() {
                    found.push((rank, f));
                }
            }
        }
    }
    found.into_iter().max_by_key(|(r, _)| *r).map(|(_, f)| f)
}

/// `nordlys-shell tray`: print the icons and their menus, to see what
/// the bar sees.
pub fn print() -> Result<(), String> {
    let bus = bus().ok_or("no session bus")?;
    let owner = bus
        .call_method(
            Some(DBUS),
            "/org/freedesktop/DBus",
            Some(DBUS),
            "GetNameOwner",
            &(WATCHER,),
        )
        .ok()
        .and_then(|m| m.body().deserialize::<String>().ok());
    match owner {
        Some(o) => println!("watcher: {o}"),
        None => println!("watcher: none (the bar starts one)"),
    }
    let (items, _) = read_items(bus, &mut IconCache::default());
    if items.is_empty() {
        println!("no icons");
    }
    for i in items {
        let icon = match &i.icon {
            Icon::Image(_) => "image",
            Icon::Svg(..) => "svg",
            Icon::Letter(_) => "letter",
        };
        println!("{}  ({}{}, {icon})", i.title, i.service, i.path);
        if let Some(m) = &i.menu {
            fn show(entries: &[Entry], depth: usize) {
                for e in entries {
                    if e.separator {
                        println!("{:w$}──", "", w = depth * 2 + 4);
                    } else {
                        println!(
                            "{:w$}{}{}",
                            "",
                            e.label,
                            if e.enabled { "" } else { " (off)" },
                            w = depth * 2 + 4
                        );
                    }
                    show(&e.children, depth + 1);
                }
            }
            show(&read_menu(&i.service, m).unwrap_or_default(), 0);
        }
    }
    Ok(())
}

// ---- Actions ----

/// Activate an icon (left or middle click), off the UI thread.
pub fn click(item: &Item, how: Click) {
    let (service, path) = (item.service.clone(), item.path.clone());
    std::thread::spawn(move || {
        let Some(bus) = bus() else { return };
        let method = match how {
            Click::Primary => "Activate",
            Click::Middle => "SecondaryActivate",
        };
        let _ = bus.call_method(
            Some(service.as_str()),
            path.as_str(),
            Some(ITEM),
            method,
            &(0i32, 0i32),
        );
    });
}

/// Ask the app to show its own menu, for items without a dbusmenu.
pub fn context_menu(item: &Item) {
    let (service, path) = (item.service.clone(), item.path.clone());
    std::thread::spawn(move || {
        let Some(bus) = bus() else { return };
        let _ = bus.call_method(
            Some(service.as_str()),
            path.as_str(),
            Some(ITEM),
            "ContextMenu",
            &(0i32, 0i32),
        );
    });
}

/// Scroll on an icon (volume apps use it).
pub fn scroll(item: &Item, delta: i32) {
    let (service, path) = (item.service.clone(), item.path.clone());
    std::thread::spawn(move || {
        let Some(bus) = bus() else { return };
        let _ = bus.call_method(
            Some(service.as_str()),
            path.as_str(),
            Some(ITEM),
            "Scroll",
            &(delta, "vertical"),
        );
    });
}

/// An app's menu, read fresh. Blocking: run it in the background.
pub fn read_menu(service: &str, menu: &str) -> Option<Vec<Entry>> {
    let bus = bus()?;
    // Some apps fill the menu only when told it's about to show.
    let _ = bus.call_method(Some(service), menu, Some(MENU), "AboutToShow", &(0i32,));
    let names: Vec<&str> = vec![];
    let reply = bus
        .call_method(
            Some(service),
            menu,
            Some(MENU),
            "GetLayout",
            &(0i32, -1i32, names),
        )
        .ok()?;
    let (_rev, (_id, _props, children)): (u32, Layout) = reply.body().deserialize().ok()?;
    Some(tidy(children.iter().filter_map(|c| entry(c)).collect()))
}

/// No separator at either end or twice in a row (hidden rows leave them).
fn tidy(entries: Vec<Entry>) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::with_capacity(entries.len());
    for mut e in entries {
        if e.separator && out.last().is_none_or(|l| l.separator) {
            continue;
        }
        e.children = tidy(std::mem::take(&mut e.children));
        out.push(e);
    }
    while out.last().is_some_and(|l| l.separator) {
        out.pop();
    }
    out
}

/// A dbusmenu node: id, properties, children (each a variant of the same).
type Layout = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

fn plain<'a>(v: &'a Value<'a>) -> &'a Value<'a> {
    match v {
        Value::Value(inner) => plain(inner),
        other => other,
    }
}

/// One `(ia{sv}av)` node; hidden ones are left out.
fn entry(v: &Value<'_>) -> Option<Entry> {
    let Value::Structure(s) = plain(v) else {
        return None;
    };
    let [Value::I32(id), Value::Dict(props), Value::Array(kids)] = s.fields() else {
        return None;
    };
    let mut label = String::new();
    let mut enabled = true;
    let mut visible = true;
    let mut separator = false;
    let mut toggle_kind = false;
    let mut toggle_on = false;
    for (k, val) in props.iter() {
        let (Value::Str(k), val) = (plain(k), plain(val)) else {
            continue;
        };
        match (k.as_str(), val) {
            ("label", Value::Str(s)) => label = mnemonic_free(s),
            ("enabled", Value::Bool(b)) => enabled = *b,
            ("visible", Value::Bool(b)) => visible = *b,
            ("type", Value::Str(t)) => separator = t.as_str() == "separator",
            ("toggle-type", Value::Str(t)) => toggle_kind = !t.is_empty(),
            ("toggle-state", Value::I32(n)) => toggle_on = *n == 1,
            _ => {}
        }
    }
    if !visible {
        return None;
    }
    Some(Entry {
        id: *id,
        label,
        enabled,
        separator,
        toggle: toggle_kind.then_some(toggle_on),
        children: kids.iter().filter_map(|k| entry(k)).collect(),
    })
}

/// "_Quit" → "Quit", "__" → "_".
fn mnemonic_free(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '_' {
            if chars.peek() == Some(&'_') {
                out.push('_');
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Tell the app a menu row was clicked.
pub fn menu_clicked(service: &str, menu: &str, id: i32) {
    let (service, menu) = (service.to_owned(), menu.to_owned());
    std::thread::spawn(move || {
        let Some(bus) = bus() else { return };
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as u32)
            .unwrap_or(0);
        let _ = bus.call_method(
            Some(service.as_str()),
            menu.as_str(),
            Some(MENU),
            "Event",
            &(id, "clicked", Value::from(0i32), stamp),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_keep_bus_and_path() {
        assert_eq!(
            entry_for("/org/ayatana/NotificationItem/steam", ":1.42"),
            ":1.42/org/ayatana/NotificationItem/steam"
        );
        assert_eq!(
            entry_for("org.kde.StatusNotifierItem-12-1", ":1.9"),
            "org.kde.StatusNotifierItem-12-1/StatusNotifierItem"
        );
        assert_eq!(
            split_entry(":1.42/org/ayatana/NotificationItem/steam"),
            (":1.42".into(), "/org/ayatana/NotificationItem/steam".into())
        );
    }

    #[test]
    fn pixmaps_turn_into_rgba() {
        assert_eq!(argb_to_rgba(&[0xff, 1, 2, 3]), vec![1, 2, 3, 0xff]);
    }

    #[test]
    fn mnemonics_are_dropped() {
        assert_eq!(mnemonic_free("_Quit"), "Quit");
        assert_eq!(mnemonic_free("snake__case"), "snake_case");
    }

    #[test]
    fn separators_are_tidied() {
        let e = |id, separator| Entry {
            id,
            label: String::new(),
            enabled: true,
            separator,
            toggle: None,
            children: vec![],
        };
        let ids: Vec<i32> = tidy(vec![
            e(1, true),
            e(2, false),
            e(3, true),
            e(4, true),
            e(5, false),
            e(6, true),
        ])
        .iter()
        .map(|e| e.id)
        .collect();
        assert_eq!(ids, vec![2, 3, 5]);
    }

    /// Needs a bus of its own: `dbus-run-session -- cargo test -p
    /// nordlys-shell watcher -- --ignored`.
    #[test]
    #[ignore]
    fn watcher_keeps_items_until_they_leave() {
        let ours = zbus::blocking::connection::Builder::session()
            .unwrap()
            .build()
            .unwrap();
        serve_watcher(&ours).unwrap();
        let app = zbus::blocking::connection::Builder::session()
            .unwrap()
            .build()
            .unwrap();
        app.call_method(
            Some(WATCHER),
            WATCHER_PATH,
            Some(WATCHER),
            "RegisterStatusNotifierItem",
            &("/org/ayatana/NotificationItem/test",),
        )
        .unwrap();
        let read = || -> Vec<String> {
            let v: OwnedValue = ours
                .call_method(
                    Some(WATCHER),
                    WATCHER_PATH,
                    Some(PROPS),
                    "Get",
                    &(WATCHER, "RegisteredStatusNotifierItems"),
                )
                .unwrap()
                .body()
                .deserialize()
                .unwrap();
            Vec::<String>::try_from(v).unwrap()
        };
        let name = app.unique_name().unwrap().to_string();
        assert_eq!(
            read(),
            vec![format!("{name}/org/ayatana/NotificationItem/test")]
        );
        drop(app);
        let mut left = read();
        for _ in 0..50 {
            if left.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
            left = read();
        }
        assert!(left.is_empty(), "still registered: {left:?}");
    }

    #[test]
    fn menu_nodes_parse() {
        let mut props = HashMap::new();
        props.insert("label".to_owned(), Value::from("_Open"));
        props.insert("toggle-type".to_owned(), Value::from("checkmark"));
        props.insert("toggle-state".to_owned(), Value::from(1i32));
        let node = Value::from((7i32, props, Vec::<Value>::new()));
        let e = entry(&node).expect("parses");
        assert_eq!(e.id, 7);
        assert_eq!(e.label, "Open");
        assert_eq!(e.toggle, Some(true));
    }
}
