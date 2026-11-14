//! Turns "the user selected something" into an action on Linux.
//!
//! Upstream reacts to a text selection from a raw `/dev/input` mouse hook, which
//! it skips on Linux ("mouse event hook skipped in linux!") — so on this
//! platform selecting text did nothing at all: no icon, no translation, and the
//! icon's click was handled inside that same hook anyway.
//!
//! Wayland gives us no global mouse events without root, but the PRIMARY
//! selection *is* the "user selected this" signal, and the compositor already
//! synchronises it for Wayland and XWayland clients alike. This module polls it
//! and applies one of three actions:
//!
//! * `icon`      — show the selection icon next to the cursor (upstream UX);
//!                 clicking it translates (see `windows::thumb_clicked`)
//! * `translate` — translate immediately, no icon, no hotkey
//! * `off`       — do nothing
//!
//! Default: `icon`. Selecting text never fires a request by itself; the icon is
//! what turns a selection into a translation. `translate` is available for
//! anyone who wants selection to translate immediately.
//!
//! Optional override file, `selection.json` next to `config.json`:
//!
//! ```json
//! { "action": "translate", "minChars": 2, "settleMs": 350 }
//! ```
//!
//! `NEXTAI_SELECTION_ALWAYS=1` ignores the "own window is focused" guard and
//! `NEXTAI_SELECTION_ACTION` overrides the file (both are debugging aids).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use debug_print::debug_println;
use serde::Deserialize;

use tauri::Manager;

use crate::windows;
use crate::SELECTED_TEXT;

pub static THUMB_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Verbose tracing to stderr, enabled with `NEXTAI_SELECTION_DEBUG=1`.
fn trace(message: impl AsRef<str>) {
    if std::env::var_os("NEXTAI_SELECTION_DEBUG").is_some() {
        eprintln!("[selwatch] {}", message.as_ref());
    }
}

const DEFAULT_SETTLE_MS: u64 = 350;
const DEFAULT_MIN_CHARS: usize = 2;
const POLL_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Action {
    Icon,
    Translate,
    Off,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SelectionConfig {
    action: Option<String>,
    min_chars: Option<usize>,
    settle_ms: Option<u64>,
}

fn config_path() -> Option<std::path::PathBuf> {
    let handle = crate::APP_HANDLE.get()?;
    let dir = handle.path().app_config_dir().ok()?;
    Some(dir.join("selection.json"))
}

fn read_config() -> SelectionConfig {
    config_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn action(config: &SelectionConfig) -> Action {
    if let Ok(value) = std::env::var("NEXTAI_SELECTION_ACTION") {
        return parse_action(&value).unwrap_or(Action::Off);
    }
    if let Some(value) = config.action.as_deref().and_then(parse_action) {
        return value;
    }
    // Fork default: show the icon, translate only when it is clicked. Selecting
    // text must never spend an API request on its own.
    Action::Icon
}

fn parse_action(value: &str) -> Option<Action> {
    match value.trim().to_ascii_lowercase().as_str() {
        "icon" | "icons" => Some(Action::Icon),
        "translate" | "translation" => Some(Action::Translate),
        "off" | "none" | "disabled" => Some(Action::Off),
        _ => None,
    }
}

/// Cursor position as reported by the compositor, used to place the icon.
fn cursor_position() -> Option<(i32, i32)> {
    let output = std::process::Command::new("hyprctl")
        .args(["cursorpos"])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let (x, y) = stdout.trim().split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

/// True when the focused window belongs to this app, so a selection made inside
/// our own UI does not trigger anything.
fn active_window_is_ours() -> bool {
    let output = match std::process::Command::new("hyprctl")
        .args(["activewindow", "-j"])
        .output()
    {
        Ok(output) => output,
        Err(_) => return false,
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.to_ascii_lowercase().contains("\"class\": \"nextai-translator\"")
}

fn schedule_thumb_hide() {
    let this_generation = THUMB_GENERATION.load(Ordering::Acquire);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(8));
        if THUMB_GENERATION.load(Ordering::Acquire) == this_generation {
            if let Some(handle) = crate::APP_HANDLE.get() {
                if let Some(window) = handle.get_webview_window(windows::THUMB_WIN_NAME) {
                    let _ = window.hide();
                }
            }
        }
    });
}

fn act(text: String, action: Action) {
    *SELECTED_TEXT.lock() = text.clone();
    match action {
        Action::Icon => {
            THUMB_GENERATION.fetch_add(1, Ordering::AcqRel);
            match cursor_position() {
                Some((x, y)) => {
                    windows::show_thumb(x, y);
                    schedule_thumb_hide();
                }
                None => {
                    debug_println!("[selection] no cursor position, icon skipped");
                }
            }
        }
        Action::Translate => {
            if let Some(handle) = crate::APP_HANDLE.get() {
                if let Some(window) = handle.get_webview_window(windows::THUMB_WIN_NAME) {
                    let _ = window.hide();
                }
            }
            windows::show_translator_with_text(text);
        }
        Action::Off => {}
    }
}

/// Starts the watcher. No-op outside Hyprland: without a compositor that can
/// report the cursor and the focused window this cannot decide anything.
pub fn start() {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none() {
        trace("not a Hyprland session, watcher disabled");
        return;
    }
    trace("watcher started");

    std::thread::spawn(|| {
        // Whatever was already selected when the app started is not an event.
        let mut last = crate::selected_text::primary_selection().unwrap_or_default();
        let mut pending: Option<(String, std::time::Instant)> = None;

        loop {
            std::thread::sleep(POLL_INTERVAL);

            let text = match crate::selected_text::primary_selection() {
                Some(text) => text,
                None => {
                    if pending.is_some() {
                        trace("primary selection went away");
                    }
                    pending = None;
                    continue;
                }
            };
            if text == last {
                pending = None;
                continue;
            }

            let config = read_config();
            let settle = Duration::from_millis(config.settle_ms.unwrap_or(DEFAULT_SETTLE_MS));
            match &pending {
                // Same text as last poll: act once it has been stable long enough.
                Some((candidate, since)) if *candidate == text => {
                    if since.elapsed() < settle {
                        continue;
                    }
                }
                // Selection changed again: restart the settle timer.
                Some(_) => {
                    pending = Some((text, std::time::Instant::now()));
                    continue;
                }
                None => {
                    pending = Some((text, std::time::Instant::now()));
                    continue;
                }
            }

            last = text.clone();
            pending = None;
            trace(format!("stable selection ({} chars)", text.chars().count()));

            let action = action(&config);
            trace(format!("action = {action:?}"));
            if action == Action::Off {
                continue;
            }
            if text.trim().chars().count() < config.min_chars.unwrap_or(DEFAULT_MIN_CHARS) {
                continue;
            }
            // Text selected inside our own window must not trigger anything.
            if std::env::var_os("NEXTAI_SELECTION_ALWAYS").is_none() && active_window_is_ours() {
                continue;
            }

            debug_println!(
                "[selection] action={action:?} chars={}",
                text.chars().count()
            );
            act(text, action);
        }
    });
}
