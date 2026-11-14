# Linux / Wayland fork notes

This fork exists to make the desktop app work on a Wayland session without a
wrapper script and without obsolete X11 libraries. Everything below was verified
on Arch (Omarchy, Hyprland, NVIDIA GTX 960).

## What was broken

| Symptom | Cause |
| --- | --- |
| `error while loading shared libraries: libxdo.so.3` | `enigo` (and therefore `get-selected-text`) was built with its `xdo` feature, which links `libxdo.so.3`. Current xdotool ships `libxdo.so.4` only. |
| Whole window is blank / only a strip is painted | WebKitGTK on Wayland: DMABUF renderer aborts with `Gdk-Message: Error 71 (protocol error)`, and without DMABUF only a single damaged strip is committed (NVIDIA). |
| Hotkey does nothing, clipboard stays empty | Selected text was captured by synthesising `Ctrl+C` through X11. Wayland-native windows never receive that key. |
| Screenshot OCR produces a black image | The `screenshots` crate reads the X11 root window, which contains nothing that Wayland-native clients drew. |
| "Insert translation into previous input" does nothing | Same X11-focus problem, on the write side. |

## What changed

### 1. No more `libxdo` (`src-tauri/Cargo.toml`, `src-tauri/vendor/`)

`enigo` 0.2 has a pure-Rust `x11rb` backend that drives the X server directly:

```toml
enigo = { version = "0.2.0", default-features = false, features = ["x11rb"] }
```

`get-selected-text` 0.1.6 hardcodes the `xdo` feature, and Cargo unifies
features across the graph, so it would pull `libxdo.so.3` back in. Its 4-file
source is vendored under `src-tauri/vendor/get-selected-text` with the feature
swapped and wired up through `[patch.crates-io]`.

Result: the binary has no `libxdo` dependency at all — no AUR `libxdo3`, no
`LD_LIBRARY_PATH` symlink trick, no `/etc/ld.so.conf` entry.

### 2. Linux session defaults (`src-tauri/src/main.rs`)

`apply_linux_env_defaults()` runs before GTK opens the display connection:

* `GDK_BACKEND=x11` (always) — the combination that actually paints on this
  driver stack, and what the `x11rb` backend drives anyway. Note that Omarchy
  exports `GDK_BACKEND=wayland,x11,*`, so merely "setting it if unset" is not
  enough; the preference list has to be replaced.
* `WEBKIT_DISABLE_DMABUF_RENDERER=1` (if unset) — avoids the `Gdk error 71`
  abort.
* `GDK_SCALE=1` **only** when `hyprctl getoption xwayland:force_zero_scaling`
  reports `true` and the session exported a different scale. With zero scaling
  the compositor already hands X11 clients a 1:1 surface, so an inherited
  `GDK_SCALE=2` (as this session's `monitors.lua` sets) paints every widget at
  twice its size inside a normal-sized window — the "zoomed UI" symptom.

Escape hatches: `NEXTAI_NATIVE_WAYLAND=1` keeps native Wayland,
`NEXTAI_KEEP_GDK_SCALE=1` keeps the session's `GDK_SCALE`.

### 3. Wayland-native selected text (`src-tauri/src/selected_text.rs`)

Reads the PRIMARY selection with `wl-paste --primary --no-newline` (trying
`text/plain;charset=utf-8`, `text/plain`, `UTF8_STRING` in turn) and falls back
to the upstream `Ctrl+C` + clipboard heuristic when that yields nothing.
`NEXTAI_NO_PRIMARY=1` disables the fast path.

### 4. Wayland-native typing (`src-tauri/src/insertion.rs`)

Insertion goes through `wtype` (virtual-keyboard protocol) when the session is
Wayland and `wtype` is installed, so the text lands in whichever window really
has focus, Wayland or XWayland. `enigo` remains the fallback.

### 5. Wayland-native screenshots (`src-tauri/src/ocr.rs`)

`grim` captures the compositor framebuffer instead of the empty X11 root
window. Falls back to the `screenshots` crate when `grim` is missing, so the X11
path still behaves exactly like upstream.

### 6. Wheel/touchpad scrolling fallback (`src/tauri/wheelScrollFallback.ts`)

On this stack WebKitGTK delivers precise (touchpad / high-resolution wheel)
scroll events to the page but never moves the scroll offset: the DOM receives
`wheel` events (not `preventDefault`ed) and `scrollTop` stays put, while
dragging the scrollbar and discrete wheel clicks both scroll normally.

The fallback watches the first burst of wheel events that *should* have moved
the page; if the offset has not changed 180 ms later it switches to manual
scrolling for the session (with sub-pixel accumulation, because `scrollTop`
reads back rounded and 0.1 px deltas would otherwise stall). On platforms where
native scrolling works it never engages, so momentum/phases are untouched.

`NEXTAI_SCROLL_DEBUG=1` prints every GDK scroll event (direction + deltas) to
stderr and paints what the DOM received into a corner overlay;
`NEXTAI_SCROLL_SELFTEST=1` additionally dispatches synthetic wheel events to
prove the fallback engages.

### 7. Wayland entry points for "select and translate"

Upstream's selection flow has two Linux-hostile pieces: the icon is surfaced by a
`/dev/input` mouse hook that is skipped on Linux (so selecting text never showed
it), and clicking that icon is detected inside the same hook (so the icon would
have been inert anyway).

This fork adds:

* `nextai-translator --translate-selection` — grabs the PRIMARY selection and
  opens the translator with it, honouring the single-instance plugin so it works
  whether the app is running or not. Bind it in the compositor.
* `src/tauri/windows/ThumbWindow.tsx` gets a real click handler
  (`thumb_clicked`), and `watch_primary_selection_for_icon()` polls the PRIMARY
  selection on Hyprland to show the icon (respects the existing
  `alwaysShowIcons` setting; `NEXTAI_NO_AUTO_ICON=1` and `NEXTAI_ICON_ALWAYS=1`
  exist for debugging).
* `src-tauri/src/selection_watch.rs` — polls the PRIMARY selection and applies
  one of three actions: `icon` (default: show the icon next to the cursor,
  translate when it is clicked — selecting text never spends a request by
  itself), `translate` (translate right away, no icon and no hotkey) or `off`.
  Override with `selection.json` next to `config.json`:

  ```json
  { "action": "translate", "minChars": 2, "settleMs": 350 }
  ```

  `NEXTAI_SELECTION_DEBUG=1` traces every decision, `NEXTAI_SELECTION_ALWAYS=1`
  ignores the "own window has focus" guard, `NEXTAI_ICON_OFFSET` moves the icon
  relative to the cursor (default `7`).

  Dismissing the icon: there is no input hook, so the icon is dropped as soon as
  any observable sign of "the selection is over" appears — the PRIMARY selection
  becoming empty, the focused window changing, the pointer moving more than
  `moveAwayPx` (default 60) away from where the selection was made, or 8 seconds
  passing. Showing the icon must not steal focus: the user is often still typing
  or extending the selection, so focus is handed straight back (GTK's
  `focus_on_map`/`accept_focus` hints are not enough under XWayland, and
  `accept_focus = false` also swallowed the icon's own clicks).
* `show_thumb` pins the icon window at 20x20 through GTK: the builder's size
  clamp is ignored under XWayland and the icon opened as a 200x200 square.

* `clip-extensions/linux/nextai-translator.sh` — the Linux port of the official
  PopClip/SnipDo extensions, which only ever needed
  `curl -d "$TEXT" --unix-socket /tmp/openai-translator.sock http://nextai-translator`.

### 8. Fork hygiene

* Updater endpoint points at this fork instead of upstream, so an official
  update can never overwrite these patches.
* Aptabase analytics are opt-in: `NEXTAI_TELEMETRY=1`.
* `tauri.conf.json` no longer declares `xdotool` as a deb dependency.
* Version is `0.6.55` in `package.json`, `src-tauri/Cargo.toml` and
  `tauri.conf.json` — upstream leaves the first two at `0.1.0`, which made the
  settings header advertise a version that does not exist (and made updater
  comparisons nonsense).

## Build

```bash
pnpm install
pnpm build-tauri-renderer
cargo build --release --manifest-path src-tauri/Cargo.toml --bin app
```

Rust 1.78+ and pnpm 9 are enough; `sherpa-onnx` downloads its own prebuilt
static libraries, so no cmake toolchain is required.

## Package (Arch)

```bash
cd packaging/arch
makepkg -si
```

Runtime dependencies: `gtk3`, `webkit2gtk-4.1`, `libayatana-appindicator`, plus
`wl-clipboard`, `grim` and `wtype` for the Wayland paths.
