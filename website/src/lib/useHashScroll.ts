import { useEffect } from 'react'

/**
 * Land on the right section when the page is opened at a #hash.
 *
 * The browser looks for the target before React has rendered it, finds
 * nothing and gives up — which is why a deep link only worked on the second
 * load, where the scroll position is restored instead.
 *
 * Two details worth keeping:
 *   - 'instant', not 'auto'. 'auto' defers to the global scroll-behavior:
 *     smooth, which animates a deep link all the way up from the top.
 *   - Scroll again once webfonts are in. Swapping them reflows the page and
 *     moves the target out from under wherever we first landed. Anything the
 *     reader does — wheel, touch, key — cancels that second jump, so we never
 *     yank the page away from someone who has started scrolling.
 */
export function useHashScroll() {
  useEffect(() => {
    const hash = window.location.hash
    if (hash.length < 2) return

    let cancelled = false
    const cancel = () => {
      cancelled = true
    }

    const jump = () => {
      if (cancelled) return
      document.querySelector(hash)?.scrollIntoView({ behavior: 'instant', block: 'start' })
    }

    jump()

    const events = ['wheel', 'touchstart', 'keydown'] as const
    events.forEach((e) => window.addEventListener(e, cancel, { passive: true, once: true }))

    // Not requestAnimationFrame: it is throttled to zero in a background tab,
    // so a link opened in one would never scroll at all.
    document.fonts?.ready.then(jump)
    const t = window.setTimeout(jump, 250)

    return () => {
      cancelled = true
      window.clearTimeout(t)
      events.forEach((e) => window.removeEventListener(e, cancel))
    }
  }, [])
}
