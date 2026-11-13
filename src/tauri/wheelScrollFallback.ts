/**
 * Wheel-scrolling fallback for WebKitGTK.
 *
 * On some Linux/WebKitGTK combinations, precise (touchpad / high-resolution
 * wheel) scroll events reach the page as ordinary `wheel` events but never move
 * the scroll offset: the DOM sees `wheel=N`, nothing is `preventDefault()`ed,
 * and `scrollTop` stays put — while dragging the scrollbar and injecting
 * discrete wheel clicks both scroll fine. The user-visible effect is "the wheel
 * and the touchpad do nothing, but the scrollbar can be dragged".
 *
 * Instead of taking scrolling over unconditionally, this watches the first
 * burst of wheel events that should have moved the page: if the offset has not
 * changed shortly after, it switches to manual scrolling for the rest of the
 * session. Platforms where native scrolling works never flip that switch, so
 * momentum, overscroll and phases behave exactly as before.
 */

const SETTLE_MS = 180

function isScrollable(el: Element): el is HTMLElement {
    if (!(el instanceof HTMLElement)) {
        return false
    }
    const overflowY = getComputedStyle(el).overflowY
    return /(auto|scroll|overlay)/.test(overflowY) && el.scrollHeight > el.clientHeight + 1
}

/** Nearest scrollable ancestor, falling back to the document scroller. */
function scrollerFor(target: EventTarget | null): HTMLElement | null {
    let el = target instanceof Element ? target : null
    while (el && el !== document.body && el !== document.documentElement) {
        if (isScrollable(el)) {
            return el
        }
        el = el.parentElement
    }
    const root = document.scrollingElement
    return root instanceof HTMLElement ? root : null
}

/** Converts a wheel delta to pixels. */
function deltaPixels(event: WheelEvent): number {
    if (event.deltaMode === WheelEvent.DOM_DELTA_LINE) {
        return event.deltaY * 16
    }
    if (event.deltaMode === WheelEvent.DOM_DELTA_PAGE) {
        return event.deltaY * window.innerHeight
    }
    return event.deltaY
}

export function installWheelScrollFallback(): void {
    let manual = false
    let pending = 0
    let probe: number | null = null
    let probeTarget: { scroller: HTMLElement; before: number; max: number } | null = null
    // `scrollTop` is read back rounded in some builds, so adding a sub-pixel delta
    // to it every event would stall. Keep the exact target offset per scroller.
    const offsets = new WeakMap<HTMLElement, number>()

    const manualScroll = (scroller: HTMLElement, delta: number, deltaX: number) => {
        const known = offsets.get(scroller)
        // Resync when something else (scrollbar drag, keyboard, app code) moved it.
        const base = known !== undefined && Math.abs(scroller.scrollTop - known) <= 2 ? known : scroller.scrollTop
        const max = scroller.scrollHeight - scroller.clientHeight
        const next = Math.max(0, Math.min(max, base + delta))
        offsets.set(scroller, next)
        scroller.scrollTop = next
        if (deltaX !== 0) {
            scroller.scrollLeft += deltaX
        }
    }

    const onWheel = (event: WheelEvent) => {
        if (!event.cancelable) {
            return
        }
        const scroller = scrollerFor(event.target)
        if (!scroller) {
            return
        }
        const delta = deltaPixels(event)
        const deltaX = event.deltaX
        if (delta === 0 && deltaX === 0) {
            return
        }
        // Ctrl+wheel is zoom (or a pinch gesture); leave that to the webview.
        if (event.ctrlKey) {
            return
        }

        if (manual) {
            event.preventDefault()
            manualScroll(scroller, delta, deltaX)
            return
        }

        const before = scroller.scrollTop
        const max = scroller.scrollHeight - scroller.clientHeight
        const shouldMove = delta !== 0 && (delta > 0 ? before < max - 1 : before > 1)
        if (!shouldMove) {
            return
        }

        pending += delta
        if (probe !== null) {
            return
        }

        probeTarget = { scroller, before, max }
        probe = window.setTimeout(() => {
            probe = null
            const target = probeTarget
            probeTarget = null
            const total = pending
            pending = 0
            if (!target || Math.abs(target.scroller.scrollTop - target.before) >= 1) {
                // Native scrolling handled it; keep our hands off.
                return
            }
            manual = true
            console.warn('[nextai] native wheel scrolling is not moving the page; scrolling manually')
            manualScroll(target.scroller, total, 0)
        }, SETTLE_MS)
    }

    document.addEventListener('wheel', onWheel, { passive: false })
}
