import { css } from 'styled-system/css'
import { GITHUB_URL } from '../lib/detect'
import { EXTERNAL, Section } from './ui'

const LINKS = [
  { path: 'chorefile', href: `${GITHUB_URL}/blob/main/chorefile` },
  { path: 'website/chorefile', href: `${GITHUB_URL}/blob/main/website/chorefile` },
]

export function Dogfood() {
  return (
    <div
      className={css({
        position: 'relative',
        overflow: 'hidden',
        bg: 'bg.subtle',
        borderY: '1px solid token(colors.border.default)',
        // In dark, `bg.subtle` sits a few points off the canvas and the border
        // barely reads, so the band had no surface and no edge. A hairline of
        // light along the top gives it one, the way a raised panel catches it.
        boxShadow: { _dark: 'inset 0 1px 0 rgba(255,255,255,0.05)' },
      })}
    >
      <div
        aria-hidden
        className={css({
          position: 'absolute',
          top: '50%',
          left: '50%',
          w: '900px',
          h: '520px',
          ml: '-450px',
          mt: '-260px',
          pointerEvents: 'none',
          background: 'radial-gradient(closest-side, token(colors.accent.soft), transparent)',
          filter: 'blur(28px)',
          // Dark needs the glow to give the band a surface at all. Light
          // already has one, so the same wash there only tints the page.
          opacity: { base: 0.28, _dark: 1 },
        })}
      />
      <div
        aria-hidden
        className={css({
          position: 'absolute',
          inset: '0',
          pointerEvents: 'none',
          opacity: { base: 0.3, _dark: 0.34 },
          backgroundImage:
            'linear-gradient(to right, token(colors.border.default) 1px, transparent 1px), linear-gradient(to bottom, token(colors.border.default) 1px, transparent 1px)',
          backgroundSize: '64px 64px',
          maskImage: 'radial-gradient(ellipse 70% 60% at 50% 50%, #000 20%, transparent 78%)',
        })}
      />

      <Section id="dogfood" className={css({ position: 'relative' })}>
        <div
          className={css({
            display: 'flex',
            flexDir: 'column',
            alignItems: 'center',
            textAlign: 'center',
          })}
        >
          <p
            className={css({
              fontFamily: 'hand',
              fontSize: { base: '26px', md: '30px' },
              lineHeight: '1',
              color: 'fg.accent',
              transform: 'rotate(-3deg)',
              mb: '4',
            })}
          >
            yes, really
          </p>

          <h2
            className={css({
              fontSize: { base: '32px', md: '46px' },
              lineHeight: '1.05',
              letterSpacing: '-0.035em',
              fontWeight: '700',
            })}
          >
            We use chore to build chore.
          </h2>

          <p
            className={css({
              mt: '5',
              fontSize: { base: '15.5px', md: '16.5px' },
              lineHeight: '1.6',
              color: 'fg.muted',
              maxW: '620px',
            })}
          >
            The compiler, the tests, the release matrix, and this page.
          </p>

          <div
            className={css({
              mt: '9',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              gap: '3',
              flexWrap: 'wrap',
            })}
          >
            {LINKS.map((l) => (
              <a
                key={l.path}
                href={l.href}
                {...EXTERNAL}
                // A bordered chip rather than an underlined word: on a dark
                // surface an underline alone left the links floating with
                // nothing to sit on.
                className={css({
                  fontFamily: 'mono',
                  fontSize: { base: '13.5px', md: '14px' },
                  color: 'fg.accent',
                  bg: 'bg.panel',
                  border: '1px solid token(colors.border.default)',
                  rounded: '9px',
                  px: '3.5',
                  py: '2',
                  transition: 'border-color .16s, background-color .16s, transform .16s',
                  _hover: {
                    borderColor: 'accent.ring',
                    bg: 'accent.soft',
                    transform: 'translateY(-1px)',
                  },
                })}
              >
                {l.path}
              </a>
            ))}
          </div>

          <p className={css({ mt: '5', fontSize: '13px', color: 'fg.faint' })}>
            the second is included from the first
          </p>
        </div>
      </Section>
    </div>
  )
}
