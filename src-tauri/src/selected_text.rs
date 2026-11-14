//! Selected-text retrieval with a Wayland-native fast path.
//!
//! Upstream synthesises Ctrl+C and reads the clipboard back. Under Wayland that
//! keystroke is delivered to whatever window holds the X11 focus, so a
//! Wayland-native window (foot, kitty, a GTK4 app, the browser's Wayland
//! client, ...) never receives it and the clipboard stays empty - the "hotkey
//! does nothing" symptom.
//!
//! Reading the PRIMARY selection instead works for Wayland *and* XWayland
//! clients, because the compositor keeps one primary selection for the seat.
//! Set `NEXTAI_NO_PRIMARY=1` to go back to the clipboard heuristic.
//!
//! The same idea is what makes this usable without libxdo: no synthetic Ctrl+C
//! means no dependency on an X11 key-injection library.

use debug_print::debug_println;

/// MIME types to try, best first. X11 clients usually publish `UTF8_STRING`,
/// GTK 4 clients `text/plain;charset=utf-8`, some terminals only `text/plain`.
#[cfg(target_os = "linux")]
const PRIMARY_TYPES: &[&str] = &[
    "text/plain;charset=utf-8",
    "text/plain",
    "UTF8_STRING",
];

/// Reads the Wayland PRIMARY selection, i.e. the text the user last selected.
#[cfg(target_os = "linux")]
pub fn primary_selection() -> Option<String> {
    use std::process::{Command, Stdio};

    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return None;
    }
    if std::env::var_os("NEXTAI_NO_PRIMARY").is_some() {
        return None;
    }

    for mime in PRIMARY_TYPES {
        let debug = std::env::var_os("NEXTAI_SELECTION_DEBUG").is_some();
        let output = match Command::new("wl-paste")
            .args(["--primary", "--no-newline", "--type", mime])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
        {
            Ok(output) => output,
            // wl-clipboard is not installed at all: nothing to try.
            Err(_) => return None,
        };
        if !output.status.success() {
            if debug {
                eprintln!(
                    "[selected_text] wl-paste {mime} failed: {:?} {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            continue;
        }
        if debug {
            eprintln!(
                "[selected_text] wl-paste {mime} ok: {} bytes",
                output.stdout.len()
            );
        }
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        if !text.trim().is_empty() {
            debug_println!(
                "[selected_text] PRIMARY selection via {mime} ({} chars)",
                text.len()
            );
            return Some(text);
        }
    }
    None
}

#[cfg(not(target_os = "linux"))]
pub fn primary_selection() -> Option<String> {
    None
}

/// Selected text, PRIMARY selection first, upstream clipboard heuristic second.
pub fn get_selected_text() -> String {
    if let Some(text) = primary_selection() {
        return text;
    }
    get_selected_text::get_selected_text().unwrap_or_default()
}
