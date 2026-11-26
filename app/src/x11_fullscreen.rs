//! Group fullscreen requests on X11 without winit's window activation or raise.
use eframe::egui;
use std::sync::{Arc, Mutex, OnceLock};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent, ConnectionExt, EventMask, GetPropertyReply, Window,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;

x11rb::atom_manager! {
    Atoms: AtomCookies {
        _NET_SUPPORTED, _NET_WM_STATE, _NET_WM_STATE_FULLSCREEN, _NET_ACTIVE_WINDOW,
        _NET_CLIENT_LIST, _NET_WM_PID, _NET_WM_NAME, UTF8_STRING,
    }
}

struct Session {
    main: Window,
    desktop: OnceLock<Option<Desktop>>,
}

struct Desktop {
    connection: RustConnection,
    root: Window,
    atoms: Atoms,
    cached_main_fullscreen: Mutex<Option<bool>>,
}

fn session_id() -> egui::Id {
    egui::Id::new("recordbox_x11_fullscreen")
}

pub(super) fn install(cc: &eframe::CreationContext<'_>) {
    let Ok(handle) = cc.window_handle() else { return };
    let main = match handle.as_raw() {
        RawWindowHandle::Xlib(h) => match Window::try_from(h.window) {
            Ok(id) => id,
            Err(_) => return,
        },
        RawWindowHandle::Xcb(h) => h.window.get(),
        _ => return,
    };
    cc.egui_ctx.data_mut(|d| {
        d.insert_temp(
            session_id(),
            Arc::new(Session {
                main,
                desktop: OnceLock::new(),
            }),
        )
    });
}

fn session(ctx: &egui::Context) -> Option<Arc<Session>> {
    ctx.data(|d| d.get_temp(session_id()))
}

impl Session {
    fn desktop(&self) -> Option<&Desktop> {
        self.desktop.get_or_init(|| Desktop::connect(self.main)).as_ref()
    }
}

impl Desktop {
    fn connect(main: Window) -> Option<Self> {
        let (connection, _) = x11rb::connect(None).ok()?;
        let atoms = Atoms::new(&connection).ok()?.reply().ok()?;
        let root = connection.get_geometry(main).ok()?.reply().ok()?.root;
        let desktop = Self {
            connection,
            root,
            atoms,
            cached_main_fullscreen: Mutex::new(None),
        };
        let supported = desktop.words(root, atoms._NET_SUPPORTED, AtomEnum::ATOM, 1024)?;
        let required = [
            atoms._NET_WM_STATE,
            atoms._NET_WM_STATE_FULLSCREEN,
            atoms._NET_ACTIVE_WINDOW,
            atoms._NET_CLIENT_LIST,
        ];
        if !required.iter().all(|a| supported.contains(a))
            || desktop.words(main, atoms._NET_WM_PID, AtomEnum::CARDINAL, 1)? != [std::process::id()]
        {
            return None;
        }
        // Event masks belong to this connection. winit keeps its own subscriptions.
        desktop
            .connection
            .change_window_attributes(
                main,
                &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
            )
            .ok()?
            .check()
            .ok()?;
        Some(desktop)
    }

    fn words(&self, window: Window, name: Atom, kind: AtomEnum, limit: u32) -> Option<Vec<u32>> {
        let prop = self
            .connection
            .get_property(false, window, name, kind, 0, limit)
            .ok()?
            .reply()
            .ok()?;
        if prop.type_ == x11rb::NONE {
            return Some(Vec::new());
        }
        if prop.type_ != u32::from(kind) || prop.bytes_after != 0 {
            return None;
        }
        let words = prop.value32()?.collect();
        Some(words)
    }

    fn fullscreen(&self, window: Window) -> Option<bool> {
        Some(
            self.words(window, self.atoms._NET_WM_STATE, AtomEnum::ATOM, 256)?
                .contains(&self.atoms._NET_WM_STATE_FULLSCREEN),
        )
    }

    fn active_window(&self) -> Option<Window> {
        // xfwm4 can append a zero word. The first WINDOW is the active client.
        let reply = self
            .connection
            .get_property(false, self.root, self.atoms._NET_ACTIVE_WINDOW, AtomEnum::WINDOW, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        active_client(&reply)
    }

    fn main_fullscreen(&self, main: Window) -> Option<bool> {
        let mut cached = self.cached_main_fullscreen.lock().ok()?;
        // Drain available notifications without waiting for the WM or scheduling redraws.
        while let Some(event) = self.connection.poll_for_event().ok()? {
            if matches!(event, Event::PropertyNotify(e) if e.window == main && e.atom == self.atoms._NET_WM_STATE) {
                *cached = None;
            }
        }
        if cached.is_none() {
            *cached = self.fullscreen(main);
        }
        *cached
    }

    fn library(&self, main: Window) -> Option<Window> {
        let clients = self.words(self.root, self.atoms._NET_CLIENT_LIST, AtomEnum::WINDOW, 1024)?;
        // Pipeline PID lookups: a remote X11 display still needs only one round trip for the list.
        let mut pending = Vec::new();
        for window in clients.into_iter().filter(|&w| w != main) {
            let pid = self
                .connection
                .get_property(false, window, self.atoms._NET_WM_PID, AtomEnum::CARDINAL, 0, 1)
                .ok()?;
            pending.push((window, pid));
        }
        let mut library = None;
        for (window, pid) in pending {
            let Ok(pid) = pid.reply() else { continue }; // A client may have just closed.
            if pid.bytes_after != 0 || pid.value32().and_then(|mut words| words.next()) != Some(std::process::id()) {
                continue;
            }
            let name = self
                .connection
                .get_property(false, window, self.atoms._NET_WM_NAME, self.atoms.UTF8_STRING, 0, 64)
                .ok()?
                .reply()
                .ok()?;
            if name.format == 8 && name.bytes_after == 0 && name.value == crate::DetachedLibrary::TITLE.as_bytes() {
                if library.is_some() {
                    return None;
                }
                library = Some(window);
            }
        }
        library
    }

    fn request(&self, window: Window, fullscreen: bool) -> Option<()> {
        // EWMH source 1 = application; set or remove explicitly so mixed states converge.
        let event = ClientMessageEvent::new(
            32,
            window,
            self.atoms._NET_WM_STATE,
            [u32::from(fullscreen), self.atoms._NET_WM_STATE_FULLSCREEN, 0, 1, 0],
        );
        self.connection
            .send_event(
                false,
                self.root,
                EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                event,
            )
            .ok()?
            .ignore_error();
        Some(())
    }
}

fn active_client(reply: &GetPropertyReply) -> Option<Window> {
    if reply.type_ == x11rb::NONE {
        Some(x11rb::NONE)
    } else if reply.type_ == u32::from(AtomEnum::WINDOW) {
        reply.value32()?.next()
    } else {
        None
    }
}

/// Read the WM's actual state. Passive requests bypass winit's cached fullscreen value.
pub(super) fn main_fullscreen(ctx: &egui::Context) -> Option<bool> {
    let session = session(ctx)?;
    session.desktop()?.main_fullscreen(session.main)
}

/// Returns true when X11 handled the shortcut. Never focus a window that lost the keyboard.
pub(super) fn toggle(ctx: &egui::Context, origin: egui::ViewportId, detached: bool) -> bool {
    let Some(session) = session(ctx) else { return false };
    let Some(desktop) = session.desktop() else { return false };
    let main = session.main;
    let in_library = origin == crate::DetachedLibrary::id();
    if origin != egui::ViewportId::ROOT && !(detached && in_library) {
        return false;
    }
    let (origin, peer) = if detached {
        let Some(library) = desktop.library(main) else {
            return false;
        };
        if in_library {
            (library, Some(main))
        } else {
            (main, Some(library))
        }
    } else {
        (main, None)
    };
    let Some(active) = desktop.active_window() else {
        return false;
    };
    if active != origin {
        return true;
    }
    let Some(current) = desktop.fullscreen(origin) else {
        return false;
    };
    // Queue both on one connection before flushing. Neither a focus request nor a WM round
    // trip between them should change which window is under the user's keyboard.
    if let Some(peer) = peer {
        if desktop.request(peer, !current).is_none() {
            return false;
        }
    }
    desktop.request(origin, !current).is_some() && desktop.connection.flush().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_active_client_even_when_the_wm_appends_another_word() {
        let window: Window = 0x600009;
        let reply = GetPropertyReply {
            type_: AtomEnum::WINDOW.into(),
            format: 32,
            value_len: 1,
            bytes_after: 4,
            value: window.to_ne_bytes().to_vec(),
            ..Default::default()
        };
        assert_eq!(active_client(&reply), Some(window));
        assert_eq!(active_client(&GetPropertyReply::default()), Some(x11rb::NONE));
    }
}
