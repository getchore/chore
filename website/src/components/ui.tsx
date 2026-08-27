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
    <section
      id={id}
      className={cx(container, css({ py: { base: '20', md: '28' }, scrollMarginTop: '80px' }), className)}
    >
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

/// Spread onto any link that leaves the site. `noopener` keeps the opened
/// page from reaching back through `window.opener`; `noreferrer` also drops
/// the referrer. Anchors within the page must not use it, since sending a reader
/// to a new tab to read the next section would be hostile.
export const EXTERNAL = { target: '_blank', rel: 'noopener noreferrer' } as const

export type AmbientVariant = 'grid' | 'dots' | 'rail' | 'corner'

/**
 * Ambient background for a section. In dark, a flat #08090c band reads as a
 * hole in the page. The hero has a grid and a glow, and everything below it
 * looked unfinished by comparison. Each variant pairs one texture with one
 * light source in a different place, so the sections feel related without
 * repeating. Deliberately near-invisible: it should register as depth, not
 * decoration, and it stays out of the way in light mode.
 *
 * Must sit inside a `position: relative; overflow: hidden` parent.
 */
export function Ambient({ variant }: { variant: AmbientVariant }) {
  const texture =
    variant === 'dots'
      ? css({
          backgroundImage: 'radial-gradient(circle at 1px 1px, token(colors.fg.faint) 1px, transparent 0)',
          backgroundSize: '24px 24px',
          maskImage: 'radial-gradient(ellipse 65% 75% at 50% 40%, #000, transparent 72%)',
          opacity: { base: 0.28, _dark: 0.4 },
        })
      : variant === 'rail'
        ? css({
            backgroundImage:
              'linear-gradient(to right, token(colors.border.default) 1px, transparent 1px)',
            backgroundSize: '128px 100%',
            maskImage: 'linear-gradient(to bottom, transparent, #000 30%, #000 70%, transparent)',
            opacity: { base: 0.4, _dark: 0.4 },
          })
        : variant === 'grid'
          ? css({
              backgroundImage:
                'linear-gradient(to right, token(colors.border.default) 1px, transparent 1px), linear-gradient(to bottom, token(colors.border.default) 1px, transparent 1px)',
              backgroundSize: '64px 64px',
              maskImage: 'radial-gradient(ellipse 70% 60% at 15% 0%, #000 20%, transparent 70%)',
              opacity: { base: 0.35, _dark: 0.35 },
            })
          : css({
              backgroundImage:
                'linear-gradient(to bottom, token(colors.border.default) 1px, transparent 1px)',
              backgroundSize: '100% 96px',
              maskImage: 'radial-gradient(ellipse 70% 70% at 85% 100%, #000, transparent 70%)',
              opacity: { base: 0.35, _dark: 0.32 },
            })

  // where the light comes from, per variant
  const glowPos: Record<AmbientVariant, string> = {
    grid: 'top:-220px; left:-120px;',
    dots: 'top:-260px; left:50%; margin-left:-450px;',
    rail: 'top:-180px; right:-160px;',
    corner: 'bottom:-280px; right:-140px;',
  }
  const style = Object.fromEntries(
    glowPos[variant]
      .split(';')
      .filter(Boolean)
      .map((d) => {
        const [k, v] = d.split(':')
        return [k.trim().replace(/-([a-z])/g, (_, c) => c.toUpperCase()), v.trim()]
      }),
  )

  return (
    <>
      <div aria-hidden className={cx(css({ position: 'absolute', inset: '0', pointerEvents: 'none' }), texture)} />
      <div
        aria-hidden
        style={style}
        className={css({
          position: 'absolute',
          w: '900px',
          h: '560px',
          pointerEvents: 'none',
          background: 'radial-gradient(closest-side, token(colors.accent.soft), transparent)',
          filter: 'blur(20px)',
          // light mode needs far less of it, or the wash reads as a smudge
          opacity: { base: 0.4, _dark: 1 },
        })}
      />
    </>
  )
}

/** A full-width band: optional raised surface, an ambient layer, and a Section. */
export function Band({
  id,
  ambient,
  surface = false,
  children,
}: {
  id?: string
  ambient: AmbientVariant
  surface?: boolean
  children: ReactNode
}) {
  return (
    <div
      className={cx(
        css({ position: 'relative', overflow: 'hidden' }),
        surface &&
          css({
            bg: 'bg.subtle',
            borderY: '1px solid token(colors.border.default)',
            // the border alone barely reads in dark; a hairline of light gives
            // the band an edge, the way a raised panel catches it
            boxShadow: { _dark: 'inset 0 1px 0 rgba(255,255,255,0.05)' },
          }),
      )}
    >
      <Ambient variant={ambient} />
      <Section id={id} className={css({ position: 'relative' })}>
        {children}
      </Section>
    </div>
  )
}
