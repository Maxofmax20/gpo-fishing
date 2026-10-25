use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::app::AppState;
use crate::config::Hotkeys;
use crate::windows;

#[derive(Clone, Copy)]
enum Action {
    Toggle,
    Overlay,
    Quit,
    HideHud,
    RecordToggle,
}

pub fn register(app: &AppHandle, keys: &Hotkeys) -> Result<(), String> {
    static REGISTERED: std::sync::Mutex<Option<Hotkeys>> = std::sync::Mutex::new(None);
    let mut previous = REGISTERED.lock().map_err(|_| "hotkey lock poisoned")?;
    validate(keys)?;
    if let Err(error) = bind(app, keys) {
        if let Some(old) = previous.as_ref() {
            if let Err(rollback) = bind(app, old) {
                return Err(format!(
                    "{error}; restoring previous shortcuts failed: {rollback}"
                ));
            }
        } else {
            let _ = app.global_shortcut().unregister_all();
        }
        return Err(error);
    }
    *previous = Some(keys.clone());
    Ok(())
}

fn validate(keys: &Hotkeys) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for combo in [
        &keys.toggle,
        &keys.overlay,
        &keys.quit,
        &keys.hide_hud,
        &keys.record_toggle,
    ] {
        if combo.trim().is_empty() {
            continue;
        }
        let shortcut: Shortcut = combo
            .parse()
            .map_err(|_| format!("Invalid hotkey: {combo}"))?;
        if !seen.insert(shortcut.id()) {
            return Err(format!("Duplicate hotkey: {combo}"));
        }
    }
    Ok(())
}

fn bind(app: &AppHandle, keys: &Hotkeys) -> Result<(), String> {
    let gs = app.global_shortcut();
    gs.unregister_all().map_err(|e| e.to_string())?;
    let bindings = [
        (keys.toggle.as_str(), Action::Toggle),
        (keys.overlay.as_str(), Action::Overlay),
        (keys.quit.as_str(), Action::Quit),
        (keys.hide_hud.as_str(), Action::HideHud),
        (keys.record_toggle.as_str(), Action::RecordToggle),
    ];
    for (combo, action) in bindings {
        if combo.trim().is_empty() {
            continue;
        }
        let shortcut: Shortcut = combo
            .parse()
            .map_err(|_| format!("Invalid hotkey: {combo}"))?;
        gs.on_shortcut(shortcut, move |app, _sc, ev| {
            if ev.state() != ShortcutState::Pressed {
                return;
            }
            dispatch(app, action);
        })
        .map_err(|e| format!("Could not bind {combo}: {e}"))?;
    }
    Ok(())
}

fn dispatch(app: &AppHandle, action: Action) {
    let st = app.state::<AppState>();
    match action {
        Action::Toggle => {
            let bot = std::sync::Arc::clone(&st.bot);
            std::thread::spawn(move || bot.toggle());
        }
        Action::Overlay => {
            if windows::overlay_visible(app) {
                windows::hide_overlay(app);
            } else if let Err(e) = crate::commands::open_regions_editor(app) {
                tracing::warn!("regions editor: {e}");
            }
        }
        Action::Quit => {
            st.bot.stop();
            app.exit(0);
        }
        Action::HideHud => {
            if let Err(e) = crate::commands::hud_toggle(app.clone(), st) {
                tracing::warn!("HUD settings save failed: {e}");
            }
        }
        Action::RecordToggle => {
            let res = crate::bot::recorder::toggle_recording();
            match res {
                Ok(now_recording) => {
                    tracing::info!(
                        "📼 Macro recording toggled via hotkey. Recording: {now_recording}"
                    );
                }
                Err(e) => {
                    tracing::warn!("📼 Macro record hotkey error: {e}");
                }
            }
            let _ = app.emit("macro:status_changed", ());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_every_binding_and_detects_duplicate_shortcuts() {
        let mut keys = Hotkeys::default();
        assert!(validate(&keys).is_ok());
        keys.record_toggle = "DefinitelyNotAKey".into();
        assert!(validate(&keys).unwrap_err().contains("Invalid hotkey"));
        keys.record_toggle = keys.toggle.clone();
        assert!(validate(&keys).unwrap_err().contains("Duplicate hotkey"));
    }
}
