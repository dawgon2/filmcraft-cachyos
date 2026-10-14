//! FilmCraft's menus in the desktop's global menu on Linux (fork, owner 2026-10-13): Plasma's Global
//! Menu widget and macOS-style top panels show the window's menu the way they show Qt's or
//! LibreOffice's. Settings ▸ General ▸ Menu bar ▸ Auto (the default) then hides the window's own
//! menu bar, except in full screen, which covers the panel.
//!
//! How: the menu is served as a `com.canonical.dbusmenu` object ([`PATH`]) on the session bus, built
//! from the same [`MenuItem`]s as the window's menu bar and refreshed whenever they change
//! ([`filmcraft_ui_egui::HostHooks::menu_changed`]); a click sends the item's command id to the
//! app's command inbox. KWin learns where the menu is through `org_kde_kwin_appmenu`
//! ([`filmcraft_platform::kde_appmenu`]). Whether a global menu is on screen is read from the
//! `com.canonical.AppMenu.Registrar` service, as Qt does it (Plasma provides it only while a Global
//! Menu widget or title-bar menu button wants menus; `org.kde.kappmenu` alone runs on every Plasma
//! desktop, so it says nothing), and followed live. Elsewhere (GNOME, X11, no widget) nothing changes: the menu bar stays
//! in the window.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};

use filmcraft_ui_egui::menus::{APP_MENU, APP_MENU_SEPARATOR_BEFORE, MENUS, MenuItem, language_of};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use zbus::zvariant::{OwnedValue, StructureBuilder, Value};

/// The menu's object path.
const PATH: &str = "/MenuBar";
const IFACE: &str = "com.canonical.dbusmenu";
/// Present while something on the desktop shows global menus (Qt checks the same name).
const REGISTRAR: &str = "com.canonical.AppMenu.Registrar";
/// More menu entries than any real menu has: a hostile or broken item list can't grow the id map
/// without bound.
const MAX_NODES: usize = 20_000;

/// One menu entry or submenu. Ids are stable for the session (keyed by command id / submenu path).
#[derive(Clone, Debug, Default)]
struct Node {
    label: String,
    /// The command a click runs; `None` = a submenu (or the root).
    command: Option<String>,
    enabled: bool,
    checked: Option<bool>,
    shortcut: Option<String>,
    /// A separator line (no label, not clickable).
    separator: bool,
    children: Vec<i32>,
}

#[derive(Default)]
struct Tree {
    revision: u32,
    nodes: HashMap<i32, Node>,
    ids: HashMap<String, i32>,
    next_id: i32,
}

impl Tree {
    fn id_for(&mut self, key: String) -> Option<i32> {
        if let Some(id) = self.ids.get(&key) {
            return Some(*id);
        }
        if self.ids.len() >= MAX_NODES {
            return None;
        }
        self.next_id = self.next_id.saturating_add(1);
        self.ids.insert(key, self.next_id);
        Some(self.next_id)
    }

    /// Rebuild the nodes from the app's menu: the top-level menus in [`MENUS`] order, submenus by
    /// path where their first item appears (as the window's menu bar does).
    fn rebuild(&mut self, items: &[MenuItem]) {
        let language = language_of(items);
        let mut nodes: HashMap<i32, Node> = HashMap::new();
        let mut root = Node { enabled: true, ..Node::default() };
        // the FilmCraft menu first (Preferences…, a line, Quit FilmCraft), as in the window
        let app_items: Vec<&MenuItem> = items.iter().filter(|i| i.path.first().map(String::as_str) == Some(APP_MENU)).collect();
        if !app_items.is_empty()
            && let Some(app_id) = self.id_for(format!("sub:{APP_MENU}"))
        {
            let mut menu = Node { label: APP_MENU.to_string(), enabled: true, ..Node::default() };
            for it in app_items {
                if APP_MENU_SEPARATOR_BEFORE.contains(&it.id.as_str())
                    && let Some(sep) = self.id_for(format!("sep:{}", it.id))
                {
                    nodes.insert(sep, Node { separator: true, enabled: true, ..Node::default() });
                    menu.children.push(sep);
                }
                let Some(leaf) = self.id_for(format!("cmd:{}", it.id)) else { continue };
                nodes.insert(leaf, leaf_node(it));
                menu.children.push(leaf);
            }
            nodes.insert(app_id, menu);
            root.children.push(app_id);
        }
        for top in MENUS {
            let mine: Vec<&MenuItem> = items.iter().filter(|i| i.path.first().map(String::as_str) == Some(top)).collect();
            if mine.is_empty() {
                continue;
            }
            let Some(top_id) = self.id_for(format!("sub:{top}")) else { continue };
            root.children.push(top_id);
            nodes.insert(top_id, Node { label: language.tr(top).to_string(), enabled: true, ..Node::default() });
            for it in mine {
                let Some(leaf) = self.id_for(format!("cmd:{}", it.id)) else { continue };
                let mut parent = top_id;
                for depth in 1..it.path.len() {
                    let key = format!("sub:{}", it.path.get(..=depth).unwrap_or_default().join("\u{1f}"));
                    let Some(sub) = self.id_for(key) else { break };
                    if let std::collections::hash_map::Entry::Vacant(e) = nodes.entry(sub) {
                        let label = it.path.get(depth).map(|p| language.tr(p).to_string()).unwrap_or_default();
                        e.insert(Node { label, enabled: true, ..Node::default() });
                        if let Some(p) = nodes.get_mut(&parent) {
                            p.children.push(sub);
                        }
                    }
                    parent = sub;
                }
                nodes.insert(leaf, leaf_node(it));
                if let Some(p) = nodes.get_mut(&parent)
                    && !p.children.contains(&leaf)
                {
                    p.children.push(leaf);
                }
            }
        }
        nodes.insert(0, root);
        self.nodes = nodes;
        self.revision = self.revision.wrapping_add(1);
    }

    fn props(&self, id: i32, names: &[String]) -> HashMap<String, OwnedValue> {
        let mut out = HashMap::new();
        let Some(n) = self.nodes.get(&id) else { return out };
        let mut put = |k: &str, v: Value<'_>| {
            if (names.is_empty() || names.iter().any(|x| x == k))
                && let Ok(v) = OwnedValue::try_from(v)
            {
                out.insert(k.to_string(), v);
            }
        };
        if n.separator {
            put("type", Value::from("separator"));
            return out;
        }
        if id != 0 {
            // dbusmenu labels use `_` for mnemonics: a literal underscore is doubled
            put("label", Value::from(n.label.replace('_', "__")));
        }
        if !n.enabled {
            put("enabled", Value::from(false));
        }
        if n.command.is_none() {
            put("children-display", Value::from("submenu"));
        }
        if let Some(c) = n.checked {
            put("toggle-type", Value::from("checkmark"));
            put("toggle-state", Value::from(i32::from(c)));
        }
        if let Some(keys) = n.shortcut.as_deref().and_then(shortcut_keys) {
            put("shortcut", Value::from(vec![keys]));
        }
        out
    }

    /// `(id, properties, children)` with children `depth` levels down (-1 = all).
    fn layout(&self, id: i32, depth: i32, names: &[String], guard: u32) -> Layout {
        let mut children = Vec::new();
        // the tree is built from paths, so it is shallow; the guard only bounds a corrupt tree
        if depth != 0
            && guard < 16
            && let Some(n) = self.nodes.get(&id)
        {
            for c in &n.children {
                let (cid, props, grand) = self.layout(*c, depth.saturating_sub(1).max(-1), names, guard + 1);
                let built = StructureBuilder::new()
                    .append_field(Value::from(cid))
                    .append_field(Value::from(props.into_iter().map(|(k, v)| (k, Value::from(v))).collect::<HashMap<String, Value>>()))
                    .append_field(Value::from(grand.into_iter().map(Value::from).collect::<Vec<Value>>()))
                    .build();
                if let Ok(s) = built
                    && let Ok(v) = OwnedValue::try_from(Value::from(s))
                {
                    children.push(v);
                }
            }
        }
        (id, self.props(id, names), children)
    }
}

/// A clickable entry for `it`.
fn leaf_node(it: &MenuItem) -> Node {
    Node {
        label: it.label.clone(),
        command: Some(it.id.clone()),
        enabled: it.enabled,
        checked: it.checked,
        shortcut: it.shortcut.clone(),
        separator: false,
        children: Vec::new(),
    }
}

/// `Cmd+Shift+S` → `["Control", "Shift", "S"]` (dbusmenu's `shortcut` property; FilmCraft's Cmd is
/// Control on Linux).
fn shortcut_keys(s: &str) -> Option<Vec<String>> {
    let parts: Vec<String> = s
        .split('+')
        .filter(|p| !p.is_empty())
        .map(|p| match p {
            "Cmd" | "Ctrl" => "Control".to_string(),
            "Opt" => "Alt".to_string(),
            other => other.to_string(),
        })
        .collect();
    if parts.is_empty() { None } else { Some(parts) }
}

/// A dbusmenu layout node: `(ia{sv}av)`.
type Layout = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

struct DbusMenu {
    tree: Arc<Mutex<Tree>>,
    commands: Sender<String>,
    ctx: egui::Context,
}

impl DbusMenu {
    fn click(&self, id: i32, event: &str) -> bool {
        let tree = self.tree.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(node) = tree.nodes.get(&id) else { return false };
        if event == "clicked"
            && node.enabled
            && let Some(cmd) = &node.command
        {
            let _ = self.commands.send(cmd.clone());
            self.ctx.request_repaint();
        }
        true
    }
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl DbusMenu {
    fn get_layout(&self, parent_id: i32, recursion_depth: i32, property_names: Vec<String>) -> (u32, Layout) {
        let tree = self.tree.lock().unwrap_or_else(PoisonError::into_inner);
        (tree.revision, tree.layout(parent_id, recursion_depth, &property_names, 0))
    }

    fn get_group_properties(&self, ids: Vec<i32>, property_names: Vec<String>) -> Vec<(i32, HashMap<String, OwnedValue>)> {
        let tree = self.tree.lock().unwrap_or_else(PoisonError::into_inner);
        let ids: Vec<i32> = if ids.is_empty() { tree.nodes.keys().copied().collect() } else { ids };
        ids.into_iter().filter(|id| tree.nodes.contains_key(id)).map(|id| (id, tree.props(id, &property_names))).collect()
    }

    fn get_property(&self, id: i32, name: String) -> zbus::fdo::Result<OwnedValue> {
        let tree = self.tree.lock().unwrap_or_else(PoisonError::into_inner);
        tree.props(id, std::slice::from_ref(&name)).remove(&name).ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("no property {name} on {id}")))
    }

    fn event(&self, id: i32, event_id: String, _data: OwnedValue, _timestamp: u32) {
        self.click(id, &event_id);
    }

    fn event_group(&self, events: Vec<(i32, String, OwnedValue, u32)>) -> Vec<i32> {
        events.into_iter().filter(|(id, ev, _, _)| !self.click(*id, ev)).map(|(id, ..)| id).collect()
    }

    fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    fn about_to_show_group(&self, ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        let tree = self.tree.lock().unwrap_or_else(PoisonError::into_inner);
        (Vec::new(), ids.into_iter().filter(|id| !tree.nodes.contains_key(id)).collect())
    }

    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }

    #[zbus(property)]
    fn text_direction(&self) -> String {
        "ltr".into()
    }

    #[zbus(property)]
    fn status(&self) -> String {
        "normal".into()
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Keeps the menu on the bus and linked to the window; dropped with the app.
pub struct GlobalMenu {
    _appmenu: filmcraft_platform::kde_appmenu::AppMenu,
    _conn: zbus::blocking::Connection,
}

/// What [`install`] gives the app: the command inbox, the menu hook and the "a global menu shows
/// this window's menu" flag.
pub struct Installed {
    pub menu: GlobalMenu,
    pub commands: Receiver<String>,
    pub changed: Box<dyn FnMut(&[MenuItem])>,
    pub shown: Arc<AtomicBool>,
}

/// Serve the menu and link it to the window. `None` (with the reason logged) on X11, on
/// compositors without `org_kde_kwin_appmenu`, or without a session bus: the menu bar then stays
/// in the window.
pub fn install(cc: &eframe::CreationContext<'_>) -> Option<Installed> {
    match try_install(cc) {
        Ok(i) => Some(i),
        Err(why) => {
            log::info!("no global menu: {why}");
            None
        }
    }
}

fn try_install(cc: &eframe::CreationContext<'_>) -> Result<Installed, String> {
    let (display, surface) = match (cc.display_handle().map(|h| h.as_raw()), cc.window_handle().map(|h| h.as_raw())) {
        (Ok(RawDisplayHandle::Wayland(d)), Ok(RawWindowHandle::Wayland(w))) => (d.display, w.surface),
        _ => return Err("not a Wayland window".into()),
    };
    let tree = Arc::new(Mutex::new(Tree { nodes: HashMap::from([(0, Node { enabled: true, ..Node::default() })]), ..Tree::default() }));
    let (tx, rx) = channel();
    let ctx = cc.egui_ctx.clone();
    let conn = zbus::blocking::connection::Builder::session()
        .and_then(|b| b.serve_at(PATH, DbusMenu { tree: tree.clone(), commands: tx, ctx: ctx.clone() }))
        .and_then(|b| b.build())
        .map_err(|e| format!("session bus: {e}"))?;
    let service = conn.unique_name().map(|n| n.to_string()).ok_or("no unique bus name")?;
    let appmenu = filmcraft_platform::kde_appmenu::register(display, surface, &service, PATH)?;
    let shown = Arc::new(AtomicBool::new(false));
    watch_registrar(&conn, shown.clone(), ctx);
    let signal_conn = conn.clone();
    let changed: Box<dyn FnMut(&[MenuItem])> = Box::new(move |items: &[MenuItem]| {
        let revision = {
            let mut t = tree.lock().unwrap_or_else(PoisonError::into_inner);
            t.rebuild(items);
            t.revision
        };
        if let Err(e) = signal_conn.emit_signal(None::<&str>, PATH, IFACE, "LayoutUpdated", &(revision, 0i32)) {
            log::debug!("global menu update: {e}");
        }
    });
    log::info!("menu in the desktop's global menu ({service}{PATH})");
    Ok(Installed { menu: GlobalMenu { _appmenu: appmenu, _conn: conn }, commands: rx, changed, shown })
}

/// Follow the registrar coming and going (a Global Menu widget added or removed) on a worker
/// thread; the UI is woken only when it changes.
fn watch_registrar(conn: &zbus::blocking::Connection, shown: Arc<AtomicBool>, ctx: egui::Context) {
    let Ok(proxy) = zbus::blocking::fdo::DBusProxy::new(conn) else { return };
    let present = zbus::names::BusName::try_from(REGISTRAR).ok().and_then(|n| proxy.name_has_owner(n).ok()).unwrap_or(false);
    shown.store(present, Ordering::Relaxed);
    let spawned = std::thread::Builder::new().name("global-menu".into()).spawn(move || {
        // only the registrar's changes, not every client connecting to the bus
        let Ok(changes) = proxy.receive_name_owner_changed_with_args(&[(0, REGISTRAR)]) else { return };
        for change in changes {
            let Ok(args) = change.args() else { continue };
            if args.name().as_str() != REGISTRAR {
                continue;
            }
            let now = args.new_owner().is_some();
            if shown.swap(now, Ordering::Relaxed) != now {
                ctx.request_repaint();
            }
        }
    });
    if let Err(e) = spawned {
        log::warn!("global menu watcher: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, path: &[&str]) -> MenuItem {
        MenuItem { id: id.into(), label: id.into(), path: path.iter().map(|s| s.to_string()).collect(), shortcut: None, enabled: true, checked: None }
    }

    #[test]
    fn tree_follows_the_menu_bar() {
        let mut t = Tree::default();
        let items = vec![
            item("file.save", &["File"]),
            MenuItem { checked: Some(true), shortcut: Some("Cmd+`".into()), ..item("window.toggleFullScreen", &["View"]) },
            item("clip.interp.frameSampling", &["Clip", "Video Options", "Time Interpolation"]),
            item("clip.interp.opticalFlow", &["Clip", "Video Options", "Time Interpolation"]),
            item("odd", &["NotAMenu"]),
        ];
        t.rebuild(&items);
        let root = t.nodes.get(&0).expect("root");
        let tops: Vec<&str> = root.children.iter().filter_map(|c| t.nodes.get(c)).map(|n| n.label.as_str()).collect();
        assert_eq!(tops, ["File", "Clip", "View"], "MENUS order, empty and unknown menus left out");
        let (_, _, clip_children) = t.layout(root.children[1], -1, &[], 0);
        assert_eq!(clip_children.len(), 1, "Clip ▸ Video Options");
        // ids stay put across rebuilds
        let id = t.ids.get("cmd:file.save").copied();
        let rev = t.revision;
        t.rebuild(&items);
        assert_eq!(t.ids.get("cmd:file.save").copied(), id);
        assert_ne!(t.revision, rev);
        let fs = t.ids.get("cmd:window.toggleFullScreen").copied().expect("full screen");
        let p = t.props(fs, &[]);
        assert!(p.contains_key("toggle-state") && p.contains_key("shortcut") && p.contains_key("label"));
        assert!(!p.contains_key("children-display"));
    }

    #[test]
    fn the_filmcraft_menu_comes_first_with_a_line_above_quit() {
        let mut t = Tree::default();
        t.rebuild(&[item("file.save", &["File"]), item("app.preferences", &["FilmCraft"]), item("app.quit", &["FilmCraft"])]);
        let root = t.nodes.get(&0).expect("root");
        let first = t.nodes.get(&root.children[0]).expect("FilmCraft");
        assert_eq!(first.label, "FilmCraft");
        let kinds: Vec<(bool, Option<String>)> = first.children.iter().filter_map(|c| t.nodes.get(c)).map(|n| (n.separator, n.command.clone())).collect();
        assert_eq!(kinds, [(false, Some("app.preferences".into())), (true, None), (false, Some("app.quit".into()))]);
        let sep = first.children[1];
        let p = t.props(sep, &[]);
        assert_eq!(p.len(), 1, "a separator has only its type: {p:?}");
        assert!(p.contains_key("type"));
    }

    #[test]
    fn labels_escape_underscores_and_shortcuts_map_to_control() {
        let mut t = Tree::default();
        t.rebuild(&[item("snake_case", &["File"])]);
        let id = t.ids.get("cmd:snake_case").copied().expect("id");
        let label = t.props(id, &["label".into()]).remove("label").expect("label");
        assert_eq!(String::try_from(label).ok().as_deref(), Some("snake__case"));
        assert_eq!(shortcut_keys("Cmd+Shift+S"), Some(vec!["Control".into(), "Shift".into(), "S".into()]));
        assert_eq!(shortcut_keys("F11"), Some(vec!["F11".into()]));
        assert_eq!(shortcut_keys("+"), None);
    }

    #[test]
    fn a_huge_menu_is_capped() {
        let mut t = Tree::default();
        let items: Vec<MenuItem> = (0..MAX_NODES + 50).map(|i| item(&format!("c{i}"), &["File"])).collect();
        t.rebuild(&items);
        assert!(t.ids.len() <= MAX_NODES);
        // and an unknown id is answered, not a panic
        assert!(t.props(-5, &[]).is_empty());
        let (_, _, c) = t.layout(-5, -1, &[], 0);
        assert!(c.is_empty());
    }
}
