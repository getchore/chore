import { css } from 'styled-system/css'
import { GITHUB_URL } from '../lib/detect'
import { Section } from './ui'

const LINKS = [
  { path: 'chorefile', href: `${GITHUB_URL}/blob/main/chorefile` },
  { path: 'website/chorefile', href: `${GITHUB_URL}/blob/main/website/chorefile` },
]

export function Dogfood() {
  return (
    <div className={css({ bg: 'bg.subtle', borderY: '1px solid token(colors.border.default)' })}>
      <Section id="dogfood">
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
            The compiler, the tests, the release matrix — and this page.
          </p>

          <div
            className={css({
              mt: '9',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              gap: '4',
              flexWrap: 'wrap',
            })}
          >
            {LINKS.map((l, i) => (
              <span key={l.path} className={css({ display: 'flex', alignItems: 'center', gap: '4' })}>
                {i > 0 && <span className={css({ color: 'fg.faint' })}>·</span>}
                <a
                  href={l.href}
                  className={css({
                    fontFamily: 'mono',
                    fontSize: { base: '14px', md: '15px' },
                    color: 'fg.accent',
                    textDecoration: 'underline',
                    textUnderlineOffset: '5px',
                    textDecorationColor: 'accent.ring',
                    transition: 'text-decoration-color .15s',
                    _hover: { textDecorationColor: 'currentColor' },
                  })}
                >
                  {l.path}
                </a>
              </span>
            ))}
          </div>

          <p className={css({ mt: '4', fontSize: '13px', color: 'fg.faint' })}>
            the second is included from the first
          </p>
        </div>
      </Section>
    </div>
  )
}
