import { useEffect, useRef } from 'react'
import { Footer } from './components/Footer'
import { Nav } from './components/Nav'
import { Landing } from './pages/Landing'
import { ReferencePage } from './pages/Reference'
import { useRoute } from './lib/router'
import { useHashScroll } from './lib/useHashScroll'

export function App() {
  const route = useRoute()
  useHashScroll()
  useRouteScroll(route)

  return (
    <div id="top">
      <Nav route={route} />
      <main>{route === 'reference' ? <ReferencePage /> : <Landing />}</main>
      <Footer />
    </div>
  )
}

/**
 * pushState does not scroll, so a client-side navigation has to say where it
 * lands. A new page starts at the top; a link that carried a hash — the nav's
 * "Example" from the reference page, say — lands on that section once React
 * has put it in the document.
 *
 * Back and forward are the exception: the browser restores the old scroll
 * position itself, and yanking the reader to the top of a page they are
 * returning to is worse than doing nothing. `popstate` fires before the
 * re-render, so a flag is enough to tell the two apart.
 */
function useRouteScroll(route: string) {
  const popped = useRef(false)
  const first = useRef(true)

  useEffect(() => {
    const onPop = () => {
      popped.current = true
    }
    window.addEventListener('popstate', onPop)
    return () => window.removeEventListener('popstate', onPop)
  }, [])

  useEffect(() => {
    // A first paint is a fresh load: the browser and useHashScroll own it.
    if (first.current) {
      first.current = false
      return
    }
    if (popped.current) {
      popped.current = false
      return
    }
    const hash = window.location.hash
    const target = hash.length > 1 ? document.querySelector(hash) : null
    if (target) target.scrollIntoView({ behavior: 'instant', block: 'start' })
    else window.scrollTo({ top: 0, behavior: 'instant' })
  }, [route])
}
