//! Native macOS menu bar, laid out as rekordbox 7.2.19 builds it for the current mode
//! (`MainComponent::getMenuBarNames` 0x1009643d8 and `getMenuForIndex` 0x100964638; notes in
//! `checks/re-7.2.19/shell-menus-traffic.md`): File / View / Track / Playlist / Help in EXPORT
//! and PERFORMANCE, File / Edit / View / Help in EDIT. recordbox's own commands, which rekordbox
//! has no item for, come after a separator at the end of the menu they belong to; the macOS app
//! menu and an Edit menu (Undo, Redo and the clipboard commands) are recordbox's too.
//! Items are enabled and ticked from the app's state every frame (`sync`), and their
//! accelerators follow Preferences > Keyboard.

use crate::app::*;
use crate::model::*;
use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use std::cell::RefCell;
use std::collections::HashMap;

enum Entry {
    Plain(MenuItem),
    Check(CheckMenuItem),
}

#[derive(Default)]
struct Handles {
    items: HashMap<&'static str, Entry>,
    subs: HashMap<&'static str, Submenu>,
    /// The macOS app menu (always first).
    app_menu: Option<Submenu>,
    /// What each item was last set to: (enabled, ticked).
    shown: HashMap<&'static str, (bool, bool)>,
    /// What each submenu was last set to.
    subs_shown: HashMap<&'static str, bool>,
    /// The keymap the accelerators were last built from.
    keys: String,
    /// The mode the menus were last laid out for (`MODE_NONE` before the first time).
    mode: u8,
    /// Signatures of what the filled-in submenus were last built from.
    playlists_sig: u64,
    devices_sig: u64,
    /// Text of "Remove from Collection" / "Remove from Playlist".
    remove_label: &'static str,
}

const MODE_NONE: u8 = 255;

thread_local! {
    static HANDLES: RefCell<Handles> = RefCell::new(Handles { mode: MODE_NONE, ..Handles::default() });
}

/// Every menu command: (id, label, ticked item). The order here is not the menu order. Some
/// commands are in no menu (they are reached from Preferences > Keyboard or a context menu).
pub const COMMANDS: [(&str, &str, bool); 91] = [
    ("edit_cut", "Cut", false),
    ("edit_copy", "Copy", false),
    ("edit_paste", "Paste", false),
    ("edit_select_all", "Select All", false),
    ("about", "About recordbox", false),
    ("prefs", "Preferences…", false),
    ("import_track", "Import Track", false),
    ("import_folder", "Import Folder", false),
    ("import_playlist", "Import Playlist", false),
    ("backup", "Backup Library", false),
    ("restore", "Restore Library", false),
    ("missing", "Display All Missing Files", false),
    ("dupes", "Display All Duplicated Files", false),
    ("device_backup_mgmt", "Backed up device library management", false),
    ("import_v5", "Import your library from ver. 5 (or earlier)", false),
    ("pack_sample_dl", "Download", false),
    ("pack_sample_import", "Import", false),
    ("pack_gc_dl", "Download", false),
    ("pack_gc_import", "Import", false),
    ("export_xml", "Export Collection in xml format", false),
    ("export_playlists_info", "Export Playlists Information", false),
    ("sync_rekordbox", "Sync rekordbox Library Now", false),
    ("import_xml", "Import Collection from rekordbox xml…", false),
    ("relocate_missing", "Relocate Missing Files…", false),
    ("library_folder", "Show Library Folder in Finder", false),
    ("undo", "Undo", false),
    ("redo", "Redo", false),
    ("fullscreen", "Full screen", true),
    ("appearance_dark", "Dark", true),
    ("appearance_light", "Light", true),
    ("layout_player", "1 Player", true),
    ("layout_2players", "2 Players", true),
    ("layout_dual", "Dual Player", true),
    ("layout_simple", "Simple Player", true),
    ("layout_browser", "Full Browser", true),
    ("perf_2h", "2 Deck Horizontal", true),
    ("perf_2v", "2 Deck Vertical", true),
    ("perf_4h", "4 Deck Horizontal", true),
    ("perf_4v", "4 Deck Vertical", true),
    ("perf_browse", "Browse", true),
    ("pad_platter", "Show / Hide Pad and Platter", false),
    ("split_screen", "Show Split Screen", true),
    ("mytag_window", "My Tag Configuration Window", true),
    ("related_subpanel", "\"Related tracks\" on Subpanel", true),
    ("suggest_subpanel", "\"Track Suggestion\" on Subpanel", true),
    ("info_window", "Information Window", true),
    ("sub_browser", "Vertical sub-browser", true),
    ("sub_browser_h", "Horizontal sub-browser", true),
    ("automix", "Automix", true),
    ("related_window", "Related Track List Window", true),
    ("filter", "Filter", true),
    ("wave_blue", "Waveform: BLUE", true),
    ("wave_rgb", "Waveform: RGB", true),
    ("wave_3band", "Waveform: 3Band", true),
    ("t_import_track", "Import Track", false),
    ("import_to_collection", "Import To Collection", false),
    ("analyze", "Analyze Track", false),
    ("analyze_missing", "Analyze Missing Data", false),
    ("analyze_key", "Analyze Key", false),
    ("analysis_lock", "Analysis Lock", true),
    ("lock_on", "On", false),
    ("lock_off", "Off", false),
    ("auto_on", "Enable", true),
    ("auto_off", "Disable", true),
    ("stop_analysis", "Stop Analyzing", false),
    ("export_tracks", "Export Track", false),
    ("reload_tags", "Reload Tag", false),
    ("relocate", "Relocate", false),
    ("auto_relocate", "Auto Relocate", false),
    ("show_finder", "Show in Finder", false),
    ("remove_collection", "Remove from Collection", false),
    ("property", "Property", false),
    ("load", "Load to Player", false),
    ("trash", "Restore Removed Tracks…", false),
    ("p_import_playlist", "Import Playlist", false),
    ("new_playlist", "Create New Playlist", false),
    ("new_smart", "Create New Intelligent Playlist", false),
    ("new_folder", "Create New Folder", false),
    ("delete_playlist", "Delete Playlist", false),
    ("rename_playlist", "Rename Playlist", false),
    ("export_playlist", "Export Playlist", false),
    ("sort_items", "Sort Items", false),
    ("renumber", "Renumber Track Order", false),
    ("save_playlist_info", "Save Playlist Info", false),
    ("save_info_txt", "Export a playlist to a file (*.txt)", false),
    ("save_info_m3u8", "Export a playlist to a file for music apps (*.m3u8)", false),
    ("export_device", "Export to USB Device…", false),
    ("manual", "User's Manual", false),
    ("shortcuts", "Keyboard Shortcuts", false),
    ("h_about", "About recordbox", false),
    ("copy_to_playlists", "Copy to Playlists", false),
];

/// One entry of a menu's layout: a command, a separator or a submenu (by key).
#[derive(Clone, Copy)]
enum It {
    C(&'static str),
    Sep,
    Sub(&'static str),
}
use It::{Sep, Sub, C};

/// Submenus: (key, title). "add_to_playlist", "export_track" and "export_playlist" are filled
/// in from the playlist tree and the connected devices (`fill_playlists`, `fill_devices`).
const SUBMENUS: [(&str, &str); 16] = [
    ("file", "File"),
    ("import", "Import"),
    ("additional", "Additional contents"),
    ("sample_pack", "Sample Pack"),
    ("gc_pack", "GROOVE CIRCUIT FACTORY SAMPLES PACK"),
    ("library", "Library"),
    ("edit", "Edit"),
    ("view", "View"),
    ("appearance", "Appearance"),
    ("track", "Track"),
    ("analysis_lock", "Analysis Lock"),
    ("add_to_playlist", "Add To Playlist"),
    ("export_track", "Export Track"),
    ("playlist", "Playlist"),
    ("export_playlist", "Export Playlist"),
    ("save_info", "Save Playlist Info"),
];
const HELP: (&str, &str) = ("help", "Help");

/// The menu-bar mode for an application mode: 0 EXPORT, 1 PERFORMANCE, 2 EDIT.
fn menu_mode(app: &App) -> u8 {
    app.prefs.app_mode.min(2)
}

/// Top-level menus of a mode, in order (rekordbox's `getMenuBarNames`; "edit" outside EDIT
/// mode is recordbox's own).
fn top_level(mode: u8) -> &'static [&'static str] {
    match mode {
        2 => &["file", "edit", "view", "help"],
        _ => &["file", "edit", "view", "track", "playlist", "help"],
    }
}

/// What each menu and submenu holds in a mode, as rekordbox's `getMenuForIndex` builds it.
/// Items that depend on a plan, an account, a streaming service or the cloud are left out
/// (Cloud Library Sync, Mobile Library Sync, Liked Songs, Collaborative playlists, Show in
/// Spotify, the lighting settings in Library).
fn layout(mode: u8) -> Vec<(&'static str, Vec<It>)> {
    let perf = mode == 1;
    let mut file = vec![Sub("import")];
    if perf {
        file.extend([Sep, Sub("additional")]);
    }
    file.extend([Sep, C("missing"), C("dupes"), Sep, Sub("library"), Sep, C("export_xml")]);
    // recordbox's own.
    file.extend([Sep, C("sync_rekordbox"), C("import_xml"), C("relocate_missing"), C("library_folder")]);

    let mut view = vec![C("fullscreen"), Sep, Sub("appearance"), Sep];
    match mode {
        0 => view.extend([C("layout_player"), C("layout_2players"), C("layout_dual"), C("layout_simple"), C("layout_browser"), Sep]),
        1 => view.extend([C("perf_2h"), C("perf_2v"), C("perf_4h"), C("perf_4v"), C("perf_browse"), Sep, C("pad_platter"), Sep, C("split_screen"), Sep]),
        _ => {}
    }
    view.extend([C("mytag_window"), C("related_subpanel"), C("suggest_subpanel"), C("info_window"), C("sub_browser"), C("sub_browser_h")]);
    if perf {
        view.push(C("automix"));
    }

    let track = vec![
        C("import_to_collection"),
        C("analyze"),
        Sub("analysis_lock"),
        Sep,
        Sub("add_to_playlist"),
        C("reload_tags"),
        Sub("export_track"),
        Sep,
        C("remove_collection"),
        Sep,
        C("show_finder"),
        C("auto_relocate"),
        C("relocate"),
        // recordbox's own.
        Sep,
        C("trash"),
    ];
    let playlist = vec![
        Sub("export_playlist"),
        C("p_import_playlist"),
        C("copy_to_playlists"),
        Sep,
        C("new_playlist"),
        C("new_smart"),
        C("new_folder"),
        Sep,
        C("delete_playlist"),
        Sep,
        C("sort_items"),
        C("renumber"),
        Sub("save_info"),
        // recordbox's own.
        Sep,
        C("export_device"),
    ];
    vec![
        ("file", file),
        ("import", vec![C("import_track"), C("import_folder"), C("import_playlist")]),
        ("additional", vec![Sub("sample_pack"), Sub("gc_pack")]),
        ("sample_pack", vec![C("pack_sample_dl"), C("pack_sample_import")]),
        ("gc_pack", vec![C("pack_gc_dl"), C("pack_gc_import")]),
        ("library", vec![C("backup"), C("restore"), C("device_backup_mgmt"), C("import_v5")]),
        ("edit", vec![C("undo"), C("redo"), Sep, C("edit_cut"), C("edit_copy"), C("edit_paste"), C("edit_select_all")]),
        ("view", view),
        ("appearance", vec![C("appearance_dark"), C("appearance_light")]),
        ("track", track),
        ("analysis_lock", vec![C("lock_on"), C("lock_off")]),
        ("playlist", playlist),
        ("save_info", vec![C("save_info_txt"), C("save_info_m3u8")]),
        ("help", vec![C("manual"), C("shortcuts"), Sep, C("h_about")]),
    ]
}

fn it<'a>(h: &'a Handles, id: &str) -> &'a dyn IsMenuItem {
    match h.items.get(id).unwrap_or_else(|| panic!("menu item {id}")) {
        Entry::Plain(i) => i,
        Entry::Check(i) => i,
    }
}

/// Accelerators that are not part of Preferences > Keyboard.
fn fixed_accelerator(id: &str) -> Option<Accelerator> {
    let cmd = Modifiers::META;
    let (m, c) = match id {
        "new_playlist" => (cmd, Code::KeyN),
        "export_device" => (cmd, Code::KeyE),
        // rekordbox: Information Window is Cmd+I, Show in Finder Cmd+R.
        "info_window" => (cmd, Code::KeyI),
        "show_finder" => (cmd, Code::KeyR),
        // Edit commands carry their own shortcuts and are passed on to the focused text field
        // or the track list (the system's predefined items never reach the egui view).
        "edit_cut" => (cmd, Code::KeyX),
        "edit_copy" => (cmd, Code::KeyC),
        "edit_paste" => (cmd, Code::KeyV),
        "edit_select_all" => (cmd, Code::KeyA),
        _ => return None,
    };
    Some(Accelerator::new(m, c))
}

/// Menu choices travel through this channel; see `wake_on_menu`.
fn menu_channel() -> &'static (crossbeam_channel::Sender<MenuEvent>, crossbeam_channel::Receiver<MenuEvent>) {
    static CH: std::sync::OnceLock<(crossbeam_channel::Sender<MenuEvent>, crossbeam_channel::Receiver<MenuEvent>)> = std::sync::OnceLock::new();
    CH.get_or_init(crossbeam_channel::unbounded)
}

/// A menu choice must be acted on at once: the window only redraws on input, so without a
/// wake-up a command chosen from the menu bar would wait for the next mouse move.
pub fn wake_on_menu(ctx: &eframe::egui::Context) {
    let ctx = ctx.clone();
    MenuEvent::set_event_handler(Some(move |ev: MenuEvent| {
        let _ = menu_channel().0.send(ev);
        ctx.request_repaint();
    }));
}

fn clear(sub: &Submenu) {
    while sub.remove_at(0).is_some() {}
}

/// Lay the menus out for a mode: the top-level menus and what each one holds.
fn fill(menu: &Menu, h: &Handles, mode: u8) {
    while menu.items().len() > 1 {
        if menu.remove_at(1).is_none() {
            break;
        }
    }
    for (key, items) in layout(mode) {
        let Some(sub) = h.subs.get(key) else { continue };
        clear(sub);
        for i in items {
            let _ = match i {
                C(id) => sub.append(it(h, id)),
                Sep => sub.append(&PredefinedMenuItem::separator()),
                Sub(k) => match h.subs.get(k) {
                    Some(s) => sub.append(s),
                    None => Ok(()),
                },
            };
        }
    }
    for key in top_level(mode) {
        if let Some(s) = h.subs.get(key) {
            let _ = menu.append(s);
        }
    }
}

pub fn build() -> Option<Menu> {
    let mut h = Handles { mode: MODE_NONE, remove_label: "Remove from Collection", ..Handles::default() };
    for (id, label, check) in COMMANDS {
        let e = if check { Entry::Check(CheckMenuItem::with_id(id, label, true, false, fixed_accelerator(id))) } else { Entry::Plain(MenuItem::with_id(id, label, true, fixed_accelerator(id))) };
        h.items.insert(id, e);
    }
    for (key, title) in SUBMENUS.iter().chain(std::iter::once(&HELP)) {
        h.subs.insert(key, Submenu::new(title, true));
    }
    let sep = PredefinedMenuItem::separator;
    let menu = Menu::new();
    let app_menu = Submenu::new("recordbox", true);
    app_menu
        .append_items(&[it(&h, "about"), &sep(), it(&h, "prefs"), &sep(), &PredefinedMenuItem::hide(None), &PredefinedMenuItem::hide_others(None), &PredefinedMenuItem::show_all(None), &sep(), &PredefinedMenuItem::quit(None)])
        .ok()?;
    menu.append(&app_menu).ok()?;
    fill(&menu, &h, 0);
    h.mode = 0;
    h.app_menu = Some(app_menu);
    #[cfg(target_os = "macos")]
    menu.init_for_nsapp();
    HANDLES.with(|x| *x.borrow_mut() = h);
    Some(menu)
}

/// "Add To Playlist": the playlist tree with its folders (intelligent playlists left out).
fn fill_playlists(h: &Handles, app: &App) {
    fn level(sub: &Submenu, app: &App, parent: i64, depth: usize) {
        if depth > 32 {
            return;
        }
        for n in app.nodes.iter().filter(|n| n.parent == parent && n.kind != NodeKind::Smart) {
            if n.kind == NodeKind::Folder {
                let s = Submenu::new(&n.name, true);
                level(&s, app, n.id, depth + 1);
                let _ = sub.append(&s);
            } else {
                let _ = sub.append(&MenuItem::with_id(format!("addpl:{}", n.id), &n.name, true, None));
            }
        }
    }
    if let Some(sub) = h.subs.get("add_to_playlist") {
        clear(sub);
        level(sub, app, 0, 0);
    }
}

/// "Export Track" and "Export Playlist": one item per connected device.
fn fill_devices(h: &Handles, app: &App) {
    for (key, prefix) in [("export_track", "exptrk"), ("export_playlist", "exppl")] {
        if let Some(sub) = h.subs.get(key) {
            clear(sub);
            for (i, d) in app.devices.iter().enumerate() {
                let _ = sub.append(&MenuItem::with_id(format!("{prefix}:{i}"), &d.name, true, None));
            }
        }
    }
}

fn sig<T: std::hash::Hash>(v: &T) -> u64 {
    use std::hash::Hasher;
    let mut s = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut s);
    s.finish() | 1
}

pub fn poll(app: &mut App) {
    while let Ok(ev) = menu_channel().1.try_recv() {
        let id: &str = ev.id().as_ref();
        handle(app, id);
    }
}

fn selected(app: &App) -> Vec<i64> {
    app.list.iter().copied().filter(|t| app.selected.contains(t)).collect()
}

/// The playlist, intelligent playlist or folder the tree has selected.
fn current_node(app: &App) -> Option<&PlaylistNode> {
    match &app.view {
        View::Playlist(id) => app.node(*id),
        _ => None,
    }
}

/// Whether the menu bar holds this keyboard action's shortcut as an accelerator (then the
/// key loop leaves it alone).
pub fn owns_shortcut(app: &App, action: &str) -> bool {
    app.menu.is_some() && crate::prefs_ext::MENU_ACTIONS.iter().any(|(a, _)| *a == action) && app.prefs.shortcut_for(action).map(|s| crate::prefs_ext::accelerator(&s).is_some()).unwrap_or(false)
}

/// (enabled, ticked) of every command for the app's present state.
pub fn state(app: &App) -> HashMap<&'static str, (bool, bool)> {
    let sel = selected(app);
    let has_sel = !sel.is_empty();
    let one = sel.len() == 1;
    let node = current_node(app);
    let manual = app.is_manual_playlist().is_some();
    let busy = app.dialogs.ext.backup_job.is_some();
    let layout = app.player_layout();
    let locked = has_sel && sel.iter().all(|id| app.track(*id).map(|t| t.grid_locked).unwrap_or(false));
    let any_missing = app.tracks.iter().any(|t| t.missing);
    let mut m = HashMap::new();
    for (id, _, _) in COMMANDS {
        let v = match id {
            "backup" | "restore" => (!busy, false),
            "relocate_missing" | "auto_relocate" => (any_missing, false),
            "export_xml" => (!app.tracks.is_empty(), false),
            "export_playlists_info" => (app.nodes.iter().any(|n| n.kind != NodeKind::Folder), false),
            "sync_rekordbox" => (app.rb_job.is_none(), false),
            "undo" => (!app.undo.is_empty(), false),
            "redo" => (!app.redo.is_empty(), false),
            "fullscreen" => (true, app.dialogs.ext.fullscreen),
            "layout_player" => (true, layout == 0),
            "layout_2players" => (true, layout == 1),
            "layout_dual" => (true, layout == 2),
            "layout_simple" => (true, layout == 3),
            "layout_browser" => (true, layout == 4),
            "mytag_window" => (true, app.prefs.show_mytag),
            "related_window" => (true, app.view == View::Related),
            "info_window" => (layout != 3, layout != 3 && app.deck.panel_tab == 2),
            "sub_browser" => (true, app.prefs.sub_browser),
            "filter" => (true, app.filter_open),
            "wave_blue" => (true, app.prefs.wave_color == 0),
            "wave_rgb" => (true, app.prefs.wave_color == 1),
            "wave_3band" => (true, app.prefs.wave_color == 2),
            "analyze" | "analyze_missing" | "analyze_key" | "export_tracks" | "reload_tags" | "remove_collection" => (has_sel, false),
            "analysis_lock" => (has_sel, locked),
            "auto_on" => (true, app.prefs.auto_analysis),
            "auto_off" => (true, !app.prefs.auto_analysis),
            "stop_analysis" => (app.analysis_total > app.analysis_done, false),
            "relocate" | "show_finder" | "property" | "load" => (one, false),
            "delete_playlist" | "rename_playlist" => (node.is_some(), false),
            "export_playlist" | "save_playlist_info" => (node.map(|n| n.kind != NodeKind::Folder).unwrap_or(false), false),
            "sort_items" => (!app.nodes.is_empty(), false),
            "renumber" => (manual, false),
            _ => (true, false),
        };
        m.insert(id, v);
    }
    m
}

/// Bring the menu bar in line with the app: enabled, ticks and accelerators. Cheap when
/// nothing changed.
pub fn sync(app: &App) {
    if app.menu.is_none() {
        return;
    }
    let want = state(app);
    let keys = format!("{:?}", app.prefs.keymap);
    HANDLES.with(|h| {
        let mut h = h.borrow_mut();
        let h = &mut *h;
        for (id, (en, on)) in &want {
            if h.shown.get(id) == Some(&(*en, *on)) {
                continue;
            }
            match h.items.get(id) {
                Some(Entry::Plain(i)) => i.set_enabled(*en),
                Some(Entry::Check(i)) => {
                    i.set_enabled(*en);
                    i.set_checked(*on);
                }
                None => {}
            }
            h.shown.insert(*id, (*en, *on));
        }
        // A click on a ticked item flips its tick natively; put back what the state says.
        for (id, e) in &h.items {
            if let (Entry::Check(i), Some((_, on))) = (e, want.get(id)) {
                if i.is_checked() != *on {
                    i.set_checked(*on);
                }
            }
        }
        if h.keys != keys || h.keys.is_empty() {
            for (action, item) in crate::prefs_ext::MENU_ACTIONS {
                let acc = app.prefs.shortcut_for(action).and_then(|s| crate::prefs_ext::accelerator(&s));
                match h.items.get(item) {
                    Some(Entry::Plain(i)) => {
                        let _ = i.set_accelerator(acc);
                    }
                    Some(Entry::Check(i)) => {
                        let _ = i.set_accelerator(acc);
                    }
                    None => {}
                }
            }
            h.keys = if keys.is_empty() { " ".into() } else { keys };
        }
    });
}

const AUDIO_EXT: [&str; 9] = ["mp3", "m4a", "aac", "wav", "aif", "aiff", "flac", "alac", "mp4"];

pub fn handle(app: &mut App, id: &str) {
    let sel = selected(app);
    match id {
        "about" | "h_about" => app.dialogs.about = true,
        "edit_cut" | "edit_copy" | "edit_paste" | "edit_select_all" => {
            app.edit_cmds.push(match id {
                "edit_cut" => 0,
                "edit_copy" => 1,
                "edit_paste" => 2,
                _ => 3,
            });
            app.ctx.request_repaint();
        }
        "undo" => app.undo_edit(),
        "redo" => app.redo_edit(),
        "trash" => app.dialogs.trash = Some(Default::default()),
        "prefs" => app.dialogs.prefs_open = true,
        "manual" => match crate::manual::open(&app.prefs) {
            Ok(_) => {}
            Err(e) => app.toast_err(format!("The manual could not be opened: {e}")),
        },
        "shortcuts" => app.dialogs.ext.shortcuts_open = true,
        "import_track" | "t_import_track" => {
            if let Some(files) = rfd::FileDialog::new().set_title("Import Track").add_filter("Audio", &AUDIO_EXT).pick_files() {
                let pl = app.is_manual_playlist();
                app.import_paths(files, pl);
            }
        }
        "import_folder" => {
            if let Some(d) = rfd::FileDialog::new().set_title("Import Folder").pick_folder() {
                let pl = app.is_manual_playlist();
                app.import_paths(vec![d], pl);
            }
        }
        "import_playlist" | "p_import_playlist" => {
            if let Some(f) = rfd::FileDialog::new().set_title("Import Playlist").add_filter("Playlist", &["m3u", "m3u8", "pls", "txt"]).pick_file() {
                let parent = crate::prefs_ext::current_parent(app);
                if let Some(id) = app.import_playlist_file(&f, parent) {
                    app.side = SideMode::Playlists;
                    app.set_view(View::Playlist(id));
                }
            }
        }
        "backup" => {
            if app.dialogs.ext.backup_job.is_none() {
                app.dialogs.ext.backup_ask = true;
            }
        }
        "restore" => {
            if app.dialogs.ext.backup_job.is_none() {
                if let Some(f) = rfd::FileDialog::new().set_title("Open backup file").add_filter("recordbox library backup", &["zip"]).pick_file() {
                    match crate::backup::read_manifest(&f) {
                        Ok(_) => app.dialogs.ext.restore_ask = Some(f),
                        Err(e) => app.dialogs.ext.backup_done = Some(("Library Restoration".into(), format!("Unable to restore the collection.\n{e}"))),
                    }
                }
            }
        }
        "missing" => app.open_missing_file_manager(),
        "relocate_missing" => {
            if let Some(d) = rfd::FileDialog::new().set_title("Folder to search for the missing files").pick_folder() {
                app.relocate_missing(&d);
            }
        }
        "auto_relocate" => {
            // The selected missing tracks, or every missing track when none is selected.
            let picked: Vec<i64> = sel.iter().copied().filter(|id| app.track(*id).map(|t| t.missing).unwrap_or(false)).collect();
            app.auto_relocate(if picked.is_empty() { None } else { Some(picked) });
        }
        "relocate" => {
            if let Some(t) = sel.first().and_then(|t| app.track(*t)).cloned() {
                let mut d = rfd::FileDialog::new().set_title(format!("Relocate {}", t.display_title())).add_filter("Audio", &AUDIO_EXT);
                if let Some(p) = std::path::Path::new(&t.path).parent().filter(|p| p.is_dir()) {
                    d = d.set_directory(p);
                }
                if let Some(f) = d.pick_file() {
                    if app.relocate_track(t.id, &f.to_string_lossy()) {
                        app.toast("Track relocated.");
                    } else {
                        app.toast_err("That file could not be used for this track.");
                    }
                }
            }
        }
        "sync_rekordbox" => {
            if crate::rbimport::library_dir().is_some() {
                app.start_rekordbox_sync();
            } else {
                app.toast_err("No rekordbox library was found on this computer.");
            }
        }
        "import_xml" => {
            let mut d = rfd::FileDialog::new().add_filter("XML", &["xml"]);
            if let Some(p) = app.prefs.rekordbox_xml_path.as_deref().and_then(|p| std::path::Path::new(p).parent()).filter(|p| p.is_dir()) {
                d = d.set_directory(p);
            }
            if let Some(f) = d.pick_file() {
                app.import_xml(&f);
            }
        }
        "export_xml" => {
            if let Some(f) = rfd::FileDialog::new().set_file_name("collection.xml").save_file() {
                app.export_collection_xml(&f);
            }
        }
        "export_playlists_info" => {
            if let Some(d) = rfd::FileDialog::new().set_title("Export Playlists Information").pick_folder() {
                app.export_all_playlists_info(&d);
            }
        }
        "library_folder" => {
            let _ = std::process::Command::new("/usr/bin/open").arg(crate::db::data_dir()).spawn();
        }
        "fullscreen" => app.dialogs.ext.toggle_fullscreen = true,
        "layout_player" => app.set_player_layout(0),
        "layout_2players" => app.set_player_layout(1),
        "layout_dual" => app.set_player_layout(2),
        "layout_simple" => app.set_player_layout(3),
        "layout_browser" => app.set_player_layout(4),
        "mytag_window" => {
            app.prefs.show_mytag = !app.prefs.show_mytag;
            app.save_prefs();
        }
        "related_window" => {
            if app.view == View::Related {
                app.side = SideMode::Collection;
                app.set_view(View::Collection);
            } else {
                app.side = SideMode::Related;
                app.set_view(View::Related);
            }
        }
        "info_window" => {
            app.deck.panel_tab = if app.deck.panel_tab == 2 { 0 } else { 2 };
        }
        "sub_browser" => {
            app.prefs.sub_browser = !app.prefs.sub_browser;
            app.save_prefs();
        }
        "filter" => {
            app.filter_open = !app.filter_open;
            app.list_dirty = true;
        }
        "wave_blue" | "wave_rgb" | "wave_3band" => {
            app.prefs.wave_color = match id {
                "wave_blue" => 0,
                "wave_rgb" => 1,
                _ => 2,
            };
            app.save_prefs();
        }
        "load" => {
            if let Some(t) = sel.first() {
                app.load_track(*t);
            }
        }
        "analyze" => app.reanalyze(&sel),
        "analyze_missing" => {
            app.analyze_missing(&sel);
        }
        "analyze_key" => app.analyze_key_tracks(&sel),
        "analysis_lock" => {
            let locked = sel.iter().all(|id| app.track(*id).map(|t| t.grid_locked).unwrap_or(false));
            for id in &sel {
                if let Some(t) = app.track_mut(*id) {
                    t.grid_locked = !locked;
                }
                app.save_track(*id);
            }
        }
        "auto_on" | "auto_off" => {
            app.prefs.auto_analysis = id == "auto_on";
            app.save_prefs();
        }
        "stop_analysis" => app.pool.stop(),
        "export_tracks" => crate::ui_browser::open_export(app, vec![], sel),
        "property" => {
            if let Some(t) = sel.first() {
                app.dialogs.info_track = app.track(*t).cloned();
            }
        }
        "reload_tags" => app.reload_tags_from_files(&sel),
        "show_finder" => {
            if let Some(t) = sel.first().and_then(|t| app.track(*t)) {
                let _ = std::process::Command::new("/usr/bin/open").arg("-R").arg(&t.path).spawn();
            }
        }
        "remove_collection" => {
            if !sel.is_empty() {
                app.dialogs.confirm_remove = Some(sel);
            }
        }
        "new_playlist" | "new_smart" | "new_folder" => {
            let parent = match current_node(app) {
                Some(n) if n.kind == NodeKind::Folder => n.id,
                _ => 0,
            };
            app.side = SideMode::Playlists;
            if parent != 0 {
                app.expanded.insert(parent);
            }
            app.create_node(parent, match id {
                "new_playlist" => NodeKind::Playlist,
                "new_smart" => NodeKind::Smart,
                _ => NodeKind::Folder,
            });
        }
        "delete_playlist" => {
            if let Some(n) = current_node(app).cloned() {
                app.dialogs.confirm_delete_node = Some((n.id, n.name.clone(), n.kind == NodeKind::Folder));
            }
        }
        "rename_playlist" => {
            if let Some(n) = current_node(app).cloned() {
                app.side = SideMode::Playlists;
                app.dialogs.rename = Some((n.id, n.name));
            }
        }
        "export_playlist" => {
            if let Some(n) = current_node(app).cloned() {
                crate::ui_browser::open_export(app, vec![n.id], vec![]);
            }
        }
        "sort_items" => {
            let parent = match current_node(app) {
                Some(n) if n.kind == NodeKind::Folder => n.id,
                Some(n) => n.parent,
                None => 0,
            };
            crate::prefs_ext::sort_items(app, parent);
        }
        "renumber" => {
            // The order shown (after sorting by a column) becomes the playlist's own order.
            if let Some(pl) = app.is_manual_playlist() {
                if app.list_dirty {
                    app.rebuild_list();
                }
                let order = app.list.clone();
                let all = app.db.playlist_tracks(pl).unwrap_or_default();
                if order.len() == all.len() {
                    let _ = app.db.set_playlist_tracks(pl, &order);
                    app.prefs.sort_col = Col::Num;
                    app.prefs.sort_asc = true;
                    app.save_prefs();
                    app.list_dirty = true;
                    app.toast("Track order renumbered.");
                } else {
                    app.toast_err("Clear the search and filter first: Renumber Track Order needs the whole playlist.");
                }
            }
        }
        "save_playlist_info" => {
            if let Some(n) = current_node(app).cloned() {
                if let Some(f) = rfd::FileDialog::new().set_title("Save Playlist Info").set_file_name(format!("{}.txt", n.name)).add_filter("Text (rekordbox)", &["txt"]).add_filter("M3U8 playlist", &["m3u8"]).add_filter("PLS playlist", &["pls"]).save_file() {
                    app.export_playlist_file(n.id, &f);
                }
            }
        }
        "export_device" => crate::ui_browser::open_export(app, vec![], vec![]),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_are_unique_and_keyboard_actions_reach_them() {
        let mut ids = std::collections::HashSet::new();
        for (id, label, _) in COMMANDS {
            assert!(ids.insert(id), "duplicate menu command {id}");
            assert!(!label.is_empty());
        }
        for (_, item) in crate::prefs_ext::MENU_ACTIONS {
            assert!(ids.contains(item), "{item} is not a menu command");
        }
        // Keyboard actions that go through the menu handler name real commands.
        for (id, _, _, _) in crate::prefs_ext::ACTIONS {
            if let Some(crate::prefs_ext::Act::Menu(m)) = crate::prefs_ext::Act::parse(id) {
                assert!(ids.contains(m), "{id} -> {m}");
            }
        }
        // The menu's own fixed accelerators do not collide with default keyboard shortcuts.
        for fixed in ["Cmd+N", "Cmd+E", "Cmd+I"] {
            assert!(!crate::prefs_ext::ACTIONS.iter().any(|a| a.2 == fixed), "{fixed}");
        }
    }
}
