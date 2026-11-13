//! Temporary scroll instrumentation, enabled with `NEXTAI_SCROLL_DEBUG=1`.
//!
//! Prints every GDK scroll event the webview widget receives (with direction and
//! deltas) to stderr, and paints a small overlay inside the page showing what
//! the DOM received and how far the scrolling element actually moved. That
//! separates the three ways scrolling can fail here:
//!
//! * nothing in the GDK log  -> the event never reaches the app (compositor /
//!   XWayland side)
//! * GDK log has events, overlay stays at `wheel=0` -> WebKit drops them before
//!   the DOM
//! * overlay counts up, `scrollTop` stays 0 -> layout / overflow problem

#[cfg(target_os = "linux")]
const OVERLAY_JS: &str = r#"
(() => {
  const existing = window.__nextaiScrollDbg;
  if (existing) { existing.paint(); return; }
  const se = () => document.scrollingElement || document.documentElement;
  const cs = (el) => getComputedStyle(el);
  const box = document.createElement('div');
  box.style.cssText = [
    'position:fixed', 'z-index:2147483647', 'top:0', 'left:0', 'padding:4px 8px',
    'background:rgba(0,0,0,.88)', 'color:#7CFC00', 'font:11px/1.4 monospace',
    'white-space:pre-wrap', 'pointer-events:none', 'border-bottom-right-radius:6px',
    'max-width:92vw'
  ].join(';');
  document.documentElement.appendChild(box);
  const st = { n: 0, px: 0, discrete: 0, last: '-', target: '-', chain: '-', prevented: '-' };
  st.paint = () => {
    box.textContent =
      'NEXTAI_SCROLL_DEBUG hash=' + location.hash + '\n' +
      'wheel=' + st.n + ' pixel=' + st.px + ' discrete=' + st.discrete + '\n' +
      'last: ' + st.last + ' prevented=' + st.prevented + '\n' +
      'doc=' + Math.round(se().scrollTop) + '/' + Math.round(se().scrollHeight - se().clientHeight) +
      ' html-overflow=' + cs(document.documentElement).overflowY +
      ' body-overflow=' + cs(document.body).overflowY + '\n' +
      'target: ' + st.target + '\n' +
      'chain: ' + st.chain;
  };
  window.__nextaiScrollDbg = st;
  const describe = (el) => {
    if (!el || !el.tagName) return '-';
    const cls = (el.className && el.className.toString) ? el.className.toString().slice(0, 24) : '';
    return el.tagName.toLowerCase() + (cls ? '.' + cls : '');
  };
  const chainOf = (el) => {
    const out = [];
    for (let e = el; e && e !== document.documentElement; e = e.parentElement) {
      const o = getComputedStyle(e);
      if (/(auto|scroll|overlay)/.test(o.overflowY) && e.scrollHeight > e.clientHeight + 1) {
        out.push(describe(e) + '=' + Math.round(e.scrollTop) + '/' + Math.round(e.scrollHeight - e.clientHeight));
      }
      if (out.length > 3) break;
    }
    if (!out.length) out.push('none scrollable');
    return out.join(' | ');
  };
  document.addEventListener('wheel', (e) => {
    st.n++;
    if (e.deltaMode === 0) st.px++; else st.discrete++;
    st.last = 'mode=' + e.deltaMode + ' y=' + e.deltaY.toFixed(1) + ' tgt=' + describe(e.target);
    st.target = describe(e.target);
    st.chain = chainOf(e.target);
    st.paint();
    setTimeout(() => { st.prevented = String(e.defaultPrevented); st.chain = chainOf(e.target); st.paint(); }, 300);
  }, { passive: true, capture: true });
  st.paint();
})();
"#;

/// Dispatches synthetic (untrusted) wheel events, which never scroll natively.
/// Used to prove that the frontend fallback in
/// `src/tauri/wheelScrollFallback.ts` takes over when WebKit drops a scroll.
#[cfg(target_os = "linux")]
const SELFTEST_JS: &str = r#"
(() => {
  if (window.__nextaiScrollSelftest) { return; }
  window.__nextaiScrollSelftest = true;
  const target = document.elementFromPoint(window.innerWidth / 2, window.innerHeight / 2) || document.body;
  for (let i = 0; i < 20; i++) {
    target.dispatchEvent(new WheelEvent('wheel', { deltaY: 6.25, deltaMode: 0, bubbles: true, cancelable: true }));
  }
})();
"#;

pub fn enabled() -> bool {
    std::env::var_os("NEXTAI_SCROLL_DEBUG").is_some()
}

/// Hooks the window's webview. No-op unless `NEXTAI_SCROLL_DEBUG` is set.
#[cfg(target_os = "linux")]
pub fn attach<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) {
    if !enabled() {
        return;
    }

    let label = window.label().to_string();
    let report_label = label.clone();
    let wv = window.clone();
    let hooked = window.with_webview(move |platform| {
        use gtk::prelude::*;

        // NOTE: do not call `add_events` here. Giving the outer WebKitWebView a
        // scroll event mask makes it the event target, which swallows the
        // scroll before the inner WebKitWebViewBase (the widget that actually
        // scrolls the page) sees it. Connecting the handler alone is enough:
        // unhandled scroll events bubble up from the inner widget.
        let webview = platform.inner();
        let event_label = label.clone();
        webview.connect_scroll_event(move |_, event| {
            let dir = event
                .scroll_direction()
                .map(|d| format!("{d:?}"))
                .unwrap_or_else(|| "Smooth".to_string());
            let (dx, dy) = event.scroll_deltas().unwrap_or((0.0, 0.0));
            eprintln!(
                "[scroll-debug] {event_label} dir={dir} delta=({dx:.2},{dy:.2}) stop={}",
                event.is_scroll_stop_event()
            );
            gtk::glib::Propagation::Proceed
        });
        eprintln!("[scroll-debug] hooked webview for {label}");
    });
    if hooked.is_err() {
        eprintln!("[scroll-debug] with_webview failed for {report_label}");
    }

    let selftest = std::env::var_os("NEXTAI_SCROLL_SELFTEST").is_some();
    let selftest_wv = wv.clone();

    // The page may still be loading; re-inject until the overlay sticks.
    std::thread::spawn(move || {
        for i in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let _ = wv.eval(OVERLAY_JS);
            if selftest && i == 10 {
                let _ = selftest_wv.eval(SELFTEST_JS);
            }
        }
    });
}

#[cfg(not(target_os = "linux"))]
pub fn attach<R: tauri::Runtime>(_window: &tauri::WebviewWindow<R>) {}
