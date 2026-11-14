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
//! { "action": "icon", "minChars": 2, "settleMs": 350, "moveAwayPx": 60 }
//! ```
//!
//! `NEXTAI_SELECTION_ALWAYS=1` ignores the "own window is focused" guard and
//! `NEXTAI_SELECTION_ACTION` overrides the file (both are debugging aids).
//!
//! Dismissing the icon: without an input hook we cannot see the click that
//! cancels a selection, so the icon is dropped as soon as any observable sign of
//! "the selection is over" appears — the PRIMARY selection becoming empty, the
//! focused window changing, the pointer moving away from where the selection was
//! made, or 8 seconds passing.

use std::time::{Duration, Instant};

use debug_print::debug_println;
use serde::Deserialize;
use tauri::Manager;

use crate::windows;
use crate::SELECTED_TEXT;

const DEFAULT_SETTLE_MS: u64 = 350;
const DEFAULT_MIN_CHARS: usize = 2;
/// Poll interval while waiting for a new selection.
const POLL_INTERVAL: Duration = Duration::from_millis(250);
/// Poll interval while the icon is up, so it disappears quickly.
const ICON_POLL_INTERVAL: Duration = Duration::from_millis(200);
const ICON_TIMEOUT: Duration = Duration::from_secs(8);
/// Pointer travel beyond which the icon is considered abandoned. The icon sits
/// 7px from where the selection was made, so reaching for it stays well below.
const ICON_MOVE_AWAY: f64 = 60.0;

/// Verbose tracing to stderr, enabled with `NEXTAI_SELECTION_DEBUG=1`.
fn trace(message: impl AsRef<str>) {
    if std::env::var_os("NEXTAI_SELECTION_DEBUG").is_some() {
        eprintln!("[selwatch] {}", message.as_ref());
    }
}

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
    /// Pointer travel (px) that dismisses the icon. `0` disables the check.
    move_away_px: Option<f64>,
}

/// Everything needed to decide whether the visible icon is still wanted.
#[derive(Clone)]
struct IconState {
    anchor: (i32, i32),
    window: String,
    shown: Instant,
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
    let stdout = hyprctl(&["cursorpos"])?;
    let (x, y) = stdout.trim().split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

/// Identity of the focused window. Changes whenever focus moves, which is the
/// cheapest observable sign that the selection was dismissed.
fn active_window_key() -> String {
    let Some(stdout) = hyprctl(&["activewindow", "-j"]) else {
        return String::new();
    };
    stdout
        .split("\"address\":")
        .nth(1)
        .map(|rest| {
            rest.trim_start()
                .split(',')
                .next()
                .unwrap_or_default()
                .trim()
                .trim_matches('"')
                .to_string()
        })
        .unwrap_or_default()
}

/// True when the focused window still belongs to this app, so a selection made
/// inside our own UI does not trigger anything.
fn active_window_is_ours() -> bool {
    match hyprctl(&["activewindow", "-j"]) {
        Some(stdout) => stdout
            .to_ascii_lowercase()
            .contains("\"class\": \"nextai-translator\""),
        None => false,
    }
}

/// The icon window takes focus when it is shown, which must not count as "the
/// user moved focus away". It is the only 20x20 window we have.
fn focus_is_icon_window() -> bool {
    match hyprctl(&["activewindow", "-j"]) {
        Some(stdout) => stdout.contains("\"size\": [20, 20]"),
        None => false,
    }
}

fn hyprctl(args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("hyprctl").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Why the icon should go away, if it should.
fn dismissal_reason(
    state: &IconState,
    selection: Option<&String>,
    move_away: f64,
) -> Option<&'static str> {
    match selection {
        None => return Some("selection cleared"),
        Some(text) if text.trim().is_empty() => return Some("selection cleared"),
        _ => {}
    }
    if state.shown.elapsed() > ICON_TIMEOUT {
        return Some("timeout");
    }
    if !focus_is_icon_window() && active_window_key() != state.window {
        return Some("focus moved away");
    }
    if move_away > 0.0 {
        if let Some((x, y)) = cursor_position() {
            let dx = (x - state.anchor.0) as f64;
            let dy = (y - state.anchor.1) as f64;
            if (dx * dx + dy * dy).sqrt() > move_away {
                return Some("pointer moved away");
            }
        }
    }
    None
}

fn act(text: String, action: Action) -> Option<IconState> {
    *SELECTED_TEXT.lock() = text.clone();
    match action {
        Action::Icon => {
            let anchor = match cursor_position() {
                Some(position) => position,
                None => {
                    debug_println!("[selection] no cursor position, icon skipped");
                    return None;
                }
            };
            // Capture the focused window first: showing the icon makes the icon
            // window itself focused, which would look like "focus moved away".
            let window = active_window_key();
            windows::show_thumb(anchor.0, anchor.1);
            // GTK's "accept focus" hints are ignored under XWayland/Hyprland and
            // Hyprland switches focus slightly after the map request, so hand
            // focus back explicitly once the icon is up: the user is usually
            // still typing or extending the selection when it pops up, and a
            // stolen focus would swallow those keys.
            if !window.is_empty() {
                std::thread::sleep(Duration::from_millis(80));
                hyprctl(&[
                    "dispatch",
                    &format!("hl.dsp.focus({{ window = \"address:{window}\" }})"),
                ]);
            }
            Some(IconState {
                anchor,
                window,
                shown: Instant::now(),
            })
        }
        Action::Translate => {
            windows::hide_thumb();
            windows::show_translator_with_text(text);
            None
        }
        Action::Off => None,
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
        let mut pending: Option<(String, Instant)> = None;
        let mut icon: Option<IconState> = None;

        loop {
            std::thread::sleep(if icon.is_some() {
                ICON_POLL_INTERVAL
            } else {
                POLL_INTERVAL
            });

            let selection = crate::selected_text::primary_selection();
            let current = selection.clone().unwrap_or_default();

            if current != last {
                if current.trim().is_empty() {
                    trace("selection went away");
                    last.clear();
                    pending = None;
                    if icon.take().is_some() {
                        windows::hide_thumb();
                    }
                    continue;
                }

                let config = read_config();
                let settle = Duration::from_millis(config.settle_ms.unwrap_or(DEFAULT_SETTLE_MS));
                match &pending {
                    // Same text as last poll: act once it has been stable long enough.
                    Some((candidate, since)) if *candidate == current => {
                        if since.elapsed() < settle {
                            continue;
                        }
                    }
                    // Selection changed again: restart the settle timer.
                    Some(_) => {
                        pending = Some((current, Instant::now()));
                        continue;
                    }
                    None => {
                        pending = Some((current, Instant::now()));
                        continue;
                    }
                }

                last = current.clone();
                pending = None;

                let action = action(&config);
                if action == Action::Off {
                    trace("action = Off");
                    continue;
                }
                if current.trim().chars().count() < config.min_chars.unwrap_or(DEFAULT_MIN_CHARS)
                {
                    trace("selection too short");
                    continue;
                }
                // Text selected inside our own window must not trigger anything.
                if std::env::var_os("NEXTAI_SELECTION_ALWAYS").is_none() && active_window_is_ours()
                {
                    trace("selection made inside our own window, ignored");
                    continue;
                }

                trace(format!(
                    "stable selection ({} chars) action={action:?}",
                    current.chars().count()
                ));
                icon = act(current, action);
                continue;
            }

            // Nothing new selected: drop the icon as soon as it is not wanted.
            if let Some(state) = icon.as_ref() {
                let move_away = read_config().move_away_px.unwrap_or(ICON_MOVE_AWAY);
                if let Some(reason) = dismissal_reason(state, selection.as_ref(), move_away) {
                    trace(format!("dismissing icon: {reason}"));
                    windows::hide_thumb();
                    icon = None;
                }
            }
        }
    });
}
