import { css } from 'styled-system/css'
import { GITHUB_URL } from '../lib/detect'
import { Install } from './Install'
import { Terminal } from './Terminal'
import { GitHubIcon, container } from './ui'

const STATS = [
  ['1 binary', 'no runtime'],
  ['0 shells', 'interpreter is built in'],
  ['4 targets', 'mac · linux · win'],
  ['MIT', 'no telemetry'],
] as const

export function Hero() {
  return (
    <div className={css({ position: 'relative', overflow: 'hidden' })}>
      <div
        aria-hidden
        className={css({
          position: 'absolute',
          top: '-340px',
          left: '50%',
          w: '1000px',
          h: '660px',
          ml: '-500px',
          pointerEvents: 'none',
          background: 'radial-gradient(closest-side, token(colors.accent.soft), transparent)',
          filter: 'blur(24px)',
          animation: 'drift 14s ease-in-out infinite',
        })}
      />
      <div
        aria-hidden
        className={css({
          position: 'absolute',
          inset: '0',
          pointerEvents: 'none',
          opacity: { base: 0.5, _dark: 0.35 },
          backgroundImage:
            'linear-gradient(to right, token(colors.border.default) 1px, transparent 1px), linear-gradient(to bottom, token(colors.border.default) 1px, transparent 1px)',
          backgroundSize: '64px 64px',
          maskImage: 'radial-gradient(ellipse 75% 55% at 50% 0%, #000 30%, transparent 75%)',
        })}
      />

      <div className={css({ position: 'relative', pt: { base: '16', md: '24' }, pb: { base: '14', md: '20' } })}>
        <div className={container}>
          <div
            className={css({
              display: 'flex',
              flexDir: 'column',
              alignItems: 'center',
              textAlign: 'center',
              animation: 'fadeUp .7s cubic-bezier(.2,.7,.3,1) both',
            })}
          >
            <a
              href={`${GITHUB_URL}/releases`}
              className={css({
                display: 'inline-flex',
                alignItems: 'center',
                gap: '2',
                px: '3',
                py: '1.5',
                rounded: 'full',
                border: '1px solid token(colors.border.default)',
                bg: 'bg.panel',
                fontSize: '12.5px',
                color: 'fg.muted',
                mb: '8',
                _hover: { borderColor: 'border.strong' },
              })}
            >
              <span className={css({ w: '6px', h: '6px', rounded: 'full', bg: 'accent.solid' })} />
              v0.1.0
              <span className={css({ color: 'fg.faint' })}>→</span>
            </a>

            <h1
              className={css({
                fontSize: { base: '44px', sm: '62px', md: '76px' },
                lineHeight: '0.96',
                letterSpacing: '-0.045em',
                fontWeight: '800',
                maxW: '900px',
              })}
            >
              One binary. Every task.
              <br />
              <span
                className={css({
                  backgroundImage:
                    'linear-gradient(100deg, token(colors.accent.solid), token(colors.ember.300))',
                  backgroundClip: 'text',
                  color: 'transparent',
                })}
              >
                Every OS.
              </span>
            </h1>

            <p
              className={css({
                mt: '7',
                fontSize: { base: '17px', md: '19px' },
                lineHeight: '1.5',
                color: 'fg.muted',
                maxW: '540px',
              })}
            >
              A task runner with the shell built in. Same behaviour on macOS, Linux and Windows.
            </p>

            <div className={css({ mt: '10', w: 'full', display: 'flex', justifyContent: 'center' })}>
              <Install />
            </div>

            <a
              href={GITHUB_URL}
              className={css({
                mt: '5',
                display: 'inline-flex',
                alignItems: 'center',
                gap: '2',
                fontSize: '13.5px',
                color: 'fg.faint',
                _hover: { color: 'fg.default' },
              })}
            >
              <GitHubIcon /> or build from source
            </a>
          </div>

          <div
            className={css({
              mt: { base: '16', md: '20' },
              maxW: '880px',
              mx: 'auto',
              animation: 'fadeUp .7s cubic-bezier(.2,.7,.3,1) .12s both',
            })}
          >
            <Terminal />
          </div>

          <div
            className={css({
              mt: { base: '14', md: '18' },
              pt: '8',
              borderTop: '1px solid token(colors.border.default)',
              display: 'grid',
              gridTemplateColumns: { base: 'repeat(2, 1fr)', md: 'repeat(4, 1fr)' },
              gap: '8',
              textAlign: { base: 'left', md: 'center' },
            })}
          >
            {STATS.map(([big, small]) => (
              <div key={big}>
                <div className={css({ fontSize: '20px', fontWeight: '700', letterSpacing: '-0.02em' })}>{big}</div>
                <div className={css({ mt: '1', fontSize: '13px', color: 'fg.faint' })}>{small}</div>
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  )
}
