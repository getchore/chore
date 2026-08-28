import { useEffect, useState } from 'react'

/**
 * Two pages, so two routes and no router.
 *
 * A dependency would buy nothing here: the whole job is "which of two
 * components do I render", and the answer is a string comparison on
 * location.pathname. What a router would have given us for free — history
 * integration and modifier-click handling — is thirty lines below.
 *
 * The site is served under a base path (VITE_BASE=/chore/ on project pages),
 * so every path is built from and parsed against import.meta.env.BASE_URL,
 * never hardcoded. A direct visit to /chore/reference is served by GitHub
 * Pages' 404.html — a copy of index.html, see .github/workflows/website.yml —
 * which boots the app at that pathname, which is why parsing the real path
 * matters and a hash route would not have needed the fallback.
 */
export type Route = 'home' | 'reference'

const BASE = import.meta.env.BASE_URL

/** The href for a route, base path included. */
export function pathFor(route: Route, hash = ''): string {
  const base = BASE.endsWith('/') ? BASE : `${BASE}/`
  return (route === 'home' ? base : `${base}reference`) + hash
}

function parse(pathname: string): Route {
  const base = BASE.endsWith('/') ? BASE : `${BASE}/`
  const rest = (pathname.startsWith(base) ? pathname.slice(base.length) : pathname).replace(
    /^\/+|\/+$/g,
    '',
  )
  return rest === 'reference' ? 'reference' : 'home'
}

/** Fired after a pushState so the app re-reads the location. Back/forward use popstate. */
const NAVIGATED = 'chore:navigated'

export function useRoute(): Route {
  const [route, setRoute] = useState<Route>(() => parse(window.location.pathname))

  useEffect(() => {
    const sync = () => setRoute(parse(window.location.pathname))
    window.addEventListener('popstate', sync)
    window.addEventListener(NAVIGATED, sync)
    return () => {
      window.removeEventListener('popstate', sync)
      window.removeEventListener(NAVIGATED, sync)
    }
  }, [])

  return route
}

export function navigate(route: Route, hash = '') {
  const href = pathFor(route, hash)
  if (href === window.location.pathname + window.location.hash) return
  window.history.pushState(null, '', href)
  window.dispatchEvent(new Event(NAVIGATED))
}

/**
 * True when a click should be left to the browser: a new tab, a new window,
 * a download, a middle click. Intercepting those is the classic bug that
 * makes an in-app link feel broken.
 */
export function isPlainClick(e: React.MouseEvent): boolean {
  return !(e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey)
}
