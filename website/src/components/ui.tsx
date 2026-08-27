import type { ReactNode } from 'react'
import { css, cx } from 'styled-system/css'

export const container = css({
  w: 'full',
  maxW: '1160px',
  mx: 'auto',
  px: { base: '6', md: '8' },
})

export function Section({
  id,
  children,
  className,
}: {
  id?: string
  children: ReactNode
  className?: string
}) {
  return (
    <section id={id} className={cx(container, css({ py: { base: '20', md: '28' } }), className)}>
      {children}
    </section>
  )
}

export function Eyebrow({ children }: { children: ReactNode }) {
  return (
    <p
      className={css({
        fontFamily: 'mono',
        fontSize: '11px',
        letterSpacing: '0.16em',
        textTransform: 'uppercase',
        color: 'fg.accent',
        mb: '4',
      })}
    >
      {children}
    </p>
  )
}

export function Heading({ children }: { children: ReactNode }) {
  return (
    <h2
      className={css({
        fontSize: { base: '30px', md: '42px' },
        lineHeight: '1.1',
        letterSpacing: '-0.03em',
        fontWeight: '700',
        maxW: '640px',
      })}
    >
      {children}
    </h2>
  )
}

export function Lede({ children }: { children: ReactNode }) {
  return (
    <p
      className={css({
        mt: '5',
        fontSize: { base: '16px', md: '17.5px' },
        lineHeight: '1.65',
        color: 'fg.muted',
        maxW: '620px',
      })}
    >
      {children}
    </p>
  )
}

export const surface = css({
  bg: 'bg.panel',
  border: '1px solid token(colors.border.default)',
  rounded: '16px',
})

export function CopyIcon() {
  return (
    <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <rect x="9" y="9" width="12" height="12" rx="2" />
      <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
    </svg>
  )
}

export function CheckIcon() {
  return (
    <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="M20 6 9 17l-5-5" />
    </svg>
  )
}

export function GitHubIcon() {
  return (
    <svg width="17" height="17" viewBox="0 0 16 16" fill="currentColor" aria-hidden>
      <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82a7.4 7.4 0 0 1 2-.27c.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8Z" />
    </svg>
  )
}
