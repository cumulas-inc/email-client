//! The main window: created at startup unless the app started in the
//! background, hidden rather than destroyed when closed while the app keeps
//! running (unsent drafts live in the page), and brought back by the
//! launcher, the tray or a notification.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager, WebviewWindow, WebviewWindowBuilder, WindowEvent};

use super::Desktop;
use crate::AppState;

pub const MAIN: &str = "main";

/// Asks the page to quit (see [`quit`]).
const QUIT_REQUESTED: &str = "quit-requested";
/// How long the page has to quit when nothing would be lost, before the
/// window comes up for the question it asks instead.
const QUIT_ANSWER: Duration = Duration::from_millis(500);

/// What the shell knows of the main window.
#[derive(Default)]
pub struct WindowState {
    /// Shown (not hidden to the background).
    visible: AtomicBool,
    focused: AtomicBool,
    /// When the page was asked to quit, while it hasn't said no: it asks
    /// the user whether to lose an unsent message, or it doesn't answer.
    /// Cleared when the user keeps the message ([`Self::forget_quit`], from
    /// `quit_canceled`), and when the window is hidden or gone.
    quit_asked: Mutex<Option<Instant>>,
}

impl WindowState {
    /// The user is looking at the app: new mail shows in the list, so no
    /// notification banner.
    pub fn in_front(&self) -> bool {
        self.visible.load(Ordering::Relaxed) && self.focused.load(Ordering::Relaxed)
    }

    /// Asking the page to quit at `now`: false when it was asked already
    /// and hasn't said no, so it's asking the user (or doesn't answer), and
    /// Quit again means quit now.
    fn ask_quit(&self, now: Instant) -> bool {
        let mut asked = self.quit_asked.lock().unwrap();
        if asked.take().is_some() {
            return false;
        }
        *asked = Some(now);
        true
    }

    /// The page was asked to quit at `at`, and it's still there.
    fn quit_pending(&self, at: Instant) -> bool {
        *self.quit_asked.lock().unwrap() == Some(at)
    }

    /// The user kept the unsent message, the app quits, or the page went
    /// out of sight: nothing is asked anymore, and the next Quit asks again.
    pub fn forget_quit(&self) {
        *self.quit_asked.lock().unwrap() = None;
    }
}

/// Quit from outside the page (the tray). The page decides, as for its own
/// Quit, because it may hold an unsent message and ask first; the window
/// comes up when it doesn't quit at once. With no page, or Quit again while
/// it asks (until the user keeps the message: `quit_canceled`), or while it
/// doesn't answer, the app quits.
pub fn quit(app: &AppHandle) {
    let Some(desktop) = app.try_state::<Desktop>() else {
        app.exit(0);
        return;
    };
    let asked = Instant::now();
    if app.get_webview_window(MAIN).is_none() || !desktop.window.ask_quit(asked) {
        app.exit(0);
        return;
    }
    if let Err(e) = app.emit_to(MAIN, QUIT_REQUESTED, ()) {
        tracing::warn!("could not ask the page to quit: {e}");
        app.exit(0);
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(QUIT_ANSWER).await;
        if app.state::<Desktop>().window.quit_pending(asked) {
            show_main(&app, None);
        }
    });
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
    desktop.window.forget_quit();
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
            desktop.window.forget_quit();
            desktop.router.page_gone();
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_again_quits_only_while_the_page_asks() {
        let window = WindowState::default();
        let start = Instant::now();
        assert!(window.ask_quit(start));
        assert!(window.quit_pending(start));
        // The page asks (an unsent message), for as long as the user takes:
        // Quit again quits.
        assert!(!window.ask_quit(start + Duration::from_secs(10 * 60)));
        assert!(!window.quit_pending(start));

        // The user keeps the message (Cancel): the next Quit asks again,
        // however soon.
        assert!(window.ask_quit(start));
        window.forget_quit();
        assert!(!window.quit_pending(start));
        let again = start + Duration::from_secs(1);
        assert!(window.ask_quit(again));
        assert!(window.quit_pending(again));
        // And Quit while that asks quits.
        assert!(!window.ask_quit(again + Duration::from_secs(1)));
    }
}
