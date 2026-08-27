import { css } from 'styled-system/css'
import { GITHUB_URL, RELEASES_URL } from '../lib/detect'
import { Install } from './Install'
import { EXTERNAL, container } from './ui'

export function Cta() {
  return (
    <div
      className={css({
        borderTop: '1px solid token(colors.border.default)',
        bg: 'bg.subtle',
        position: 'relative',
        overflow: 'hidden',
      })}
    >
      <div
        aria-hidden
        className={css({
          position: 'absolute',
          bottom: '-260px',
          left: '50%',
          ml: '-350px',
          w: '700px',
          h: '460px',
          background: 'radial-gradient(closest-side, token(colors.accent.soft), transparent)',
          filter: 'blur(20px)',
        })}
      />
      <div className={container}>
        <div
          className={css({
            position: 'relative',
            py: { base: '20', md: '28' },
            display: 'flex',
            flexDir: 'column',
            alignItems: 'center',
            textAlign: 'center',
          })}
        >
          <h2
            className={css({
              fontSize: { base: '30px', md: '44px' },
              lineHeight: '1.08',
              letterSpacing: '-0.035em',
              fontWeight: '700',
              maxW: '620px',
            })}
          >
            Stop writing the same task twice.
          </h2>
          <p className={css({ mt: '5', fontSize: '16.5px', color: 'fg.muted', maxW: '440px', lineHeight: '1.55' })}>
            One file in your repo. One binary on the machine.
          </p>
          <div className={css({ mt: '10', display: 'flex', justifyContent: 'center', w: 'full' })}>
            <Install />
          </div>
          <a
            href={RELEASES_URL}
            {...EXTERNAL}
            className={css({
              mt: '5',
              fontSize: '13.5px',
              color: 'fg.faint',
              transition: 'color .15s',
              _hover: { color: 'fg.default' },
            })}
          >
            or download a binary from the releases page →
          </a>
        </div>
      </div>
    </div>
  )
}

export function Footer() {
  return (
    <footer className={css({ borderTop: '1px solid token(colors.border.default)' })}>
      <div className={container}>
        <div
          className={css({
            py: '9',
            display: 'flex',
            flexWrap: 'wrap',
            gap: '5',
            alignItems: 'center',
            justifyContent: 'space-between',
            fontSize: '13.5px',
            color: 'fg.faint',
          })}
        >
          <span>
            <strong className={css({ color: 'fg.muted', fontWeight: '600' })}>chore</strong> — MIT licensed. Built in
            Rust.
          </span>
          <div className={css({ display: 'flex', gap: '6' })}>
            <a className={css({ _hover: { color: 'fg.default' } })} href={GITHUB_URL} {...EXTERNAL}>
              GitHub
            </a>
            <a
              className={css({ _hover: { color: 'fg.default' } })}
              href={`${GITHUB_URL}/blob/main/docs/SPEC.md`}
              {...EXTERNAL}
            >
              Spec
            </a>
            <a className={css({ _hover: { color: 'fg.default' } })} href={RELEASES_URL} {...EXTERNAL}>
              Releases
            </a>
          </div>
        </div>
      </div>
    </footer>
  )
}
