//! System tray: state-aware icon and Settings / Quit menu.

use softwake_ipc::{Client, Command, VoiceState, resolve_socket_path};
use std::sync::Mutex;
use std::time::Duration;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Runtime};

const TRAY_ID: &str = "softwake";

struct TrayIcons {
    sleep: Image<'static>,
    awake: Image<'static>,
    hibernate: Image<'static>,
}

impl TrayIcons {
    fn load() -> tauri::Result<Self> {
        Ok(Self {
            sleep: Image::from_bytes(include_bytes!("../icons/tray-sleep.png"))?,
            awake: Image::from_bytes(include_bytes!("../icons/tray-awake.png"))?,
            hibernate: Image::from_bytes(include_bytes!("../icons/tray-hibernate.png"))?,
        })
    }

    fn for_state(&self, state: VoiceState) -> Image<'_> {
        match state {
            VoiceState::Sleep => self.sleep.clone(),
            VoiceState::Awake => self.awake.clone(),
            VoiceState::Hibernate => self.hibernate.clone(),
        }
    }
}

/// Build the tray and start a light status poll that refreshes the icon and label.
///
/// # Errors
///
/// Returns a Tauri error when the menu or tray icon cannot be created.
pub fn install<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let icons = TrayIcons::load()?;
    let status_item = MenuItem::with_id(app, "status", "Status: …", false, None::<&str>)?;
    let settings_item = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&status_item, &sep, &settings_item, &quit_item])?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icons.sleep.clone())
        .menu(&menu)
        .tooltip("Softwake")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "settings" => show_settings(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    let handle = app.clone();
    let last_state = Mutex::new(None::<VoiceState>);
    let _ = std::thread::Builder::new()
        .name("softwake-tray-poll".into())
        .spawn(move || {
            let icons = TrayIcons::load().ok();
            loop {
                if let Some((state, label)) = fetch_state() {
                    let _ = status_item.set_text(&label);
                    let mut guard = last_state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if guard.as_ref() != Some(&state) {
                        *guard = Some(state);
                        if let (Some(icons), Some(tray)) =
                            (icons.as_ref(), handle.tray_by_id(TRAY_ID))
                        {
                            let _ = tray.set_icon(Some(icons.for_state(state)));
                        }
                    }
                } else {
                    let _ = status_item.set_text("Status: daemon unreachable");
                    let mut guard = last_state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    *guard = None;
                }
                std::thread::sleep(Duration::from_millis(1500));
            }
        });

    Ok(())
}

fn fetch_state() -> Option<(VoiceState, String)> {
    let path = resolve_socket_path(None).ok()?;
    let mut client = Client::connect(&path).ok()?;
    let status = client.call(Command::GetStatus).ok()?;
    let capture = if status.capture_running {
        " · capture"
    } else {
        ""
    };
    let label = format!("Status: {}{capture}", status.state.as_str());
    Some((status.state, label))
}

fn show_settings<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
    // Settings must not cover the always-on-top capsule.
    if let Some(hud) = app.get_webview_window("hud") {
        let _ = hud.set_always_on_top(true);
        let _ = hud.show();
    }
}
