//! The main window: created at startup unless the app started in the
//! background, hidden rather than destroyed when closed while the app keeps
//! running (unsent drafts live in the page), and brought back by the
//! launcher, the tray or a notification.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Manager, WebviewWindow, WebviewWindowBuilder, WindowEvent};

use super::Desktop;
use crate::AppState;

pub const MAIN: &str = "main";

/// What the shell knows of the main window.
#[derive(Default)]
pub struct WindowState {
    /// Shown (not hidden to the background).
    visible: AtomicBool,
    focused: AtomicBool,
}

impl WindowState {
    /// The user is looking at the app: new mail shows in the list, so no
    /// notification banner.
    pub fn in_front(&self) -> bool {
        self.visible.load(Ordering::Relaxed) && self.focused.load(Ordering::Relaxed)
    }
}

/// Creates the main window from its configuration, at the saved text size.
pub fn create_main(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == MAIN)
        .cloned()
        .ok_or_else(|| tauri::Error::WindowNotFound)?;
    let window = WebviewWindowBuilder::from_config(app, &config)?.build()?;
    // The UI sets the zoom when the text size changes; setting the saved
    // one now means the first frame is already the right size.
    if let Some(settings) = app
        .try_state::<AppState>()
        .and_then(|s| s.service.get_settings().ok())
    {
        let zoom = settings.text_size.zoom();
        if let Err(e) = window.set_zoom(zoom) {
            tracing::warn!("could not set the zoom to {zoom}: {e}");
        }
    }
    if let Some(desktop) = app.try_state::<Desktop>() {
        desktop.window.visible.store(true, Ordering::Relaxed);
    }
    Ok(window)
}

/// Shows the main window, creating it if the app started without one, and
/// gives it the focus. `activation_token` (from a notification click) lets
/// it take the focus on Wayland.
pub fn show_main(app: &AppHandle, activation_token: Option<String>) {
    let handle = app.clone();
    let shown = app.run_on_main_thread(move || {
        let window = match handle.get_webview_window(MAIN) {
            Some(window) => window,
            None => match create_main(&handle) {
                Ok(window) => window,
                Err(e) => {
                    tracing::error!("could not open the main window: {e}");
                    return;
                }
            },
        };
        #[cfg(target_os = "linux")]
        if let Some(token) = activation_token.as_deref() {
            use gtk::prelude::GtkWindowExt;
            if let Ok(gtk) = window.gtk_window() {
                gtk.set_startup_id(token);
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = activation_token;
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        if let Some(desktop) = handle.try_state::<Desktop>() {
            desktop.window.visible.store(true, Ordering::Relaxed);
        }
    });
    if let Err(e) = shown {
        tracing::error!("could not show the main window: {e}");
    }
}

/// Hides the main window; the app keeps running.
fn hide_main(window: &tauri::Window, desktop: &Desktop) {
    desktop.window.visible.store(false, Ordering::Relaxed);
    desktop.window.focused.store(false, Ordering::Relaxed);
    if let Err(e) = window.hide() {
        tracing::warn!("could not hide the window: {e}");
    }
}

/// Window events, on the main thread.
pub fn on_window_event(window: &tauri::Window, event: &WindowEvent) {
    if window.label() != MAIN {
        return;
    }
    let app = window.app_handle();
    let Some(desktop) = app.try_state::<Desktop>() else {
        return;
    };
    match event {
        WindowEvent::Focused(focused) => {
            desktop.window.focused.store(*focused, Ordering::Relaxed);
        }
        WindowEvent::CloseRequested { api, .. } => {
            if desktop.stays_alive(app) {
                api.prevent_close();
                hide_main(window, &desktop);
                desktop.closed_to_background(app);
            }
        }
        WindowEvent::Destroyed => {
            desktop.window.visible.store(false, Ordering::Relaxed);
            desktop.window.focused.store(false, Ordering::Relaxed);
            desktop.router.page_gone();
        }
        _ => {}
    }
}
