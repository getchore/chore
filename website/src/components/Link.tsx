import type { ReactNode } from 'react'
import { isPlainClick, navigate, pathFor, type Route } from '../lib/router'

/**
 * An in-app link. It renders a real href — so it is copyable, openable in a
 * new tab, and crawlable — and only takes the click over when the browser
 * would have done a plain same-tab navigation anyway.
 */
export function Link({
  to,
  hash = '',
  className,
  children,
  onNavigate,
}: {
  to: Route
  hash?: string
  className?: string
  children: ReactNode
  onNavigate?: () => void
}) {
  return (
    <a
      href={pathFor(to, hash)}
      className={className}
      onClick={(e) => {
        if (!isPlainClick(e)) return
        e.preventDefault()
        navigate(to, hash)
        onNavigate?.()
      }}
    >
      {children}
    </a>
  )
}
