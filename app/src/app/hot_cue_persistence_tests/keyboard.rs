use super::{add_track, isolated, load, open_app};
use crate::app::App;
use crate::model::CueKind;
use anyhow::Result;
use eframe::egui::{self, Context, Event, Key, Modifiers, ViewportId};

fn key(logical: Key, physical: Key, shift: bool, pressed: bool) -> Event {
    Event::Key { key: logical, physical_key: Some(physical), pressed, repeat: false, modifiers: Modifiers { shift, ..Default::default() } }
}

fn frame(ctx: &Context, app: &mut App, viewport_id: ViewportId, focused: bool, events: Vec<Event>, mut text: Option<&mut String>) {
    let mut raw = egui::RawInput { viewport_id, focused, events, ..Default::default() };
    raw.viewports.entry(viewport_id).or_default().focused = Some(focused);
    let field = egui::Id::new("track search");
    let _ = ctx.run(raw, |ctx| {
        ctx.memory_mut(|m| if text.is_some() { m.request_focus(field) } else { m.surrender_focus(field) });
        crate::shortcuts(app, ctx);
        if let Some(s) = text.as_deref_mut() {
            egui::CentralPanel::default().show(ctx, |ui| ui.add(egui::TextEdit::singleline(s).id(field)));
        }
    });
}

#[test]
fn shifted_number_keys_clear_once_and_respect_logical_bindings() -> Result<()> {
    isolated(concat!(module_path!(), "::shifted_number_keys_clear_once_and_respect_logical_bindings"), |dir| {
        let mut app = open_app();
        let id = add_track(&mut app, &dir.join("keyboard.wav"))?;
        load(&mut app, 0, id);
        let ctx = Context::default();
        let root = ViewportId::ROOT;
        let library = crate::DetachedLibrary::id();
        let set = |app: &App, slot: i32| app.db.cues(id).unwrap().iter().any(|c| c.kind == CueKind::Hot && c.slot == slot);
        let tap = |app: &mut App, window, logical, digit, shift| {
            frame(&ctx, app, window, true, vec![key(logical, digit, shift, true)], None);
            frame(&ctx, app, window, true, vec![key(digit, digit, false, false)], None);
        };

        // The US ! and Swiss / are supported logical keys; the other US shifted symbols fall
        // back to the physical digit in egui-winit. Use both the player and the detached library.
        for (slot, digit) in crate::HOT_CUE_DIGITS.into_iter().enumerate() {
            let window = if slot % 2 == 0 { root } else { library };
            let logical = match slot { 0 => Key::Exclamationmark, 6 => Key::Slash, _ => digit };
            tap(&mut app, window, digit, digit, false);
            assert!(set(&app, slot as i32));
            let undo = app.undo_len();
            frame(&ctx, &mut app, window, true, vec![key(logical, digit, true, true)], None);
            assert!(!set(&app, slot as i32));
            // Release Shift before the digit: a repeated press can now report a different key.
            frame(&ctx, &mut app, window, true, vec![key(digit, digit, false, true)], None);
            frame(&ctx, &mut app, window, true, vec![key(digit, digit, false, false)], None);
            assert!(!set(&app, slot as i32));
            assert_eq!(app.undo_len(), undo + 1);
        }

        tap(&mut app, root, Key::Num1, Key::Num1, false);
        frame(&ctx, &mut app, root, true, vec![key(Key::Num1, Key::Num1, false, true)], None);
        frame(&ctx, &mut app, root, true, vec![key(Key::Exclamationmark, Key::Num1, true, true)], None);
        frame(&ctx, &mut app, root, true, vec![key(Key::Exclamationmark, Key::Num1, true, false)], None);
        assert!(set(&app, 0), "holding a pad and adding Shift must not clear it");
        tap(&mut app, root, Key::Num2, Key::Num2, false);
        app.prefs.keymap.insert("clear_hot2".into(), "Shift+Exclamationmark".into());
        tap(&mut app, root, Key::Exclamationmark, Key::Num1, true);
        assert!(set(&app, 0), "the explicit logical shortcut took precedence");
        assert!(!set(&app, 1));

        app.prefs.keymap.remove("clear_hot2");
        tap(&mut app, root, Key::Num2, Key::Num2, false);
        app.prefs.zoom = 2;
        tap(&mut app, root, Key::ArrowDown, Key::ArrowDown, true);
        assert_eq!(app.prefs.zoom, 3);
        assert!(set(&app, 1));
        tap(&mut app, root, Key::ArrowDown, Key::Num2, true);
        assert_eq!(app.prefs.zoom, 3, "shifted keypad 2 should only clear the hot cue");
        assert!(!set(&app, 1));
        tap(&mut app, root, Key::Num2, Key::Num2, false);
        app.prefs.keymap.insert("zoom_out".into(), "Shift+ArrowDown".into());
        tap(&mut app, root, Key::ArrowDown, Key::Num2, true);
        assert_eq!(app.prefs.zoom, 4);
        assert!(set(&app, 1), "explicit navigation binding should override keypad fallback");
        app.prefs.keymap.remove("zoom_out");

        app.prefs.keymap.insert("clear_hot1".into(), "Shift+F8".into());
        tap(&mut app, root, Key::Plus, Key::Num1, true);
        assert!(set(&app, 0), "reassigned clear still answered the old key");
        tap(&mut app, root, Key::F8, Key::F8, true);
        assert!(!set(&app, 0));

        app.prefs.keymap.remove("clear_hot1");
        tap(&mut app, library, Key::Num1, Key::Num1, false);
        app.prefs.app_mode = 1;
        app.prefs.keymap.insert("padmode_sampler".into(), "Shift+Plus".into());
        tap(&mut app, library, Key::Plus, Key::Num1, true);
        assert!(set(&app, 0), "the sampler's logical shortcut should win in Performance");
        app.prefs.app_mode = 0;
        tap(&mut app, library, Key::Plus, Key::Num1, true);
        assert!(!set(&app, 0), "sampler bindings are inactive in Export");
        tap(&mut app, library, Key::Num1, Key::Num1, false);
        // Incoming repeats after the application regains focus must not become clear presses.
        frame(&ctx, &mut app, library, true, vec![key(Key::Exclamationmark, Key::Num1, true, true), Event::Text("!".into())], Some(&mut String::new()));
        frame(&ctx, &mut app, library, false, vec![Event::WindowFocused(false)], None);
        frame(&ctx, &mut app, library, true, vec![Event::WindowFocused(true), key(Key::Exclamationmark, Key::Num1, true, true)], None);
        assert!(set(&app, 0));
        frame(&ctx, &mut app, library, true, vec![key(Key::Num1, Key::Num1, false, false)], None);

        let mut search = String::new();
        frame(&ctx, &mut app, library, true, vec![key(Key::Exclamationmark, Key::Num1, true, true), Event::Text("!".into())], Some(&mut search));
        assert_eq!(search, "!");
        assert!(set(&app, 0), "typing in a text field deleted a cue");
        Ok(())
    })
}
