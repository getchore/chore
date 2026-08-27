import { css } from 'styled-system/css'
import { GITHUB_URL } from '../lib/detect'
import { Install } from './Install'
import { Terminal } from './Terminal'
import { GitHubIcon, container } from './ui'

const STATS = [
  ['single binary', 'no runtime, no node_modules'],
  ['4 targets', 'macOS · Linux · Windows gnu & msvc'],
  ['0 shells spawned', 'the interpreter is built in'],
] as const

export function Hero() {
  return (
    <div className={css({ position: 'relative', overflow: 'hidden' })}>
      {/* ambient glow */}
      <div
        aria-hidden
        className={css({
          position: 'absolute',
          top: '-320px',
          left: '50%',
          w: '900px',
          h: '620px',
          ml: '-450px',
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
          maskImage: 'radial-gradient(ellipse 80% 60% at 50% 0%, #000 30%, transparent 75%)',
        })}
      />

      <div
        className={css({ position: 'relative', pt: { base: '20', md: '28' }, pb: { base: '16', md: '24' } })}
      >
        <div className={container}>
          <div
            className={css({
              display: 'grid',
              gridTemplateColumns: { base: '1fr', lg: '1.05fr 1fr' },
              gap: { base: '14', lg: '16' },
              alignItems: 'center',
            })}
          >
            <div className={css({ animation: 'fadeUp .7s cubic-bezier(.2,.7,.3,1) both' })}>
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
                  mb: '7',
                  transition: 'border-color .15s',
                  _hover: { borderColor: 'border.strong' },
                })}
              >
                <span className={css({ w: '6px', h: '6px', rounded: 'full', bg: 'accent.solid' })} />
                v0.1.0 — first release
                <span className={css({ color: 'fg.faint' })}>→</span>
              </a>

              <h1
                className={css({
                  fontSize: { base: '42px', sm: '54px', md: '64px' },
                  lineHeight: '0.98',
                  letterSpacing: '-0.045em',
                  fontWeight: '800',
                })}
              >
                One binary.
                <br />
                Every task.
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
                  fontSize: { base: '16.5px', md: '18px' },
                  lineHeight: '1.6',
                  color: 'fg.muted',
                  maxW: '480px',
                })}
              >
                <strong className={css({ color: 'fg.default', fontWeight: '600' })}>chore</strong> runs your
                project tasks from a <code className={css({ fontFamily: 'mono', fontSize: '0.92em' })}>chorefile</code>{' '}
                through a built-in POSIX-sh-subset interpreter. It never spawns the host shell, so a task
                behaves the same on a laptop, a runner, and a colleague's Windows box.
              </p>

              <div className={css({ mt: '9' })}>
                <Install />
              </div>

              <div className={css({ mt: '5', display: 'flex', alignItems: 'center', gap: '5', flexWrap: 'wrap' })}>
                <a
                  href={GITHUB_URL}
                  className={css({
                    display: 'inline-flex',
                    alignItems: 'center',
                    gap: '2',
                    fontSize: '13.5px',
                    fontWeight: '500',
                    color: 'fg.muted',
                    transition: 'color .15s',
                    _hover: { color: 'fg.default' },
                  })}
                >
                  <GitHubIcon /> Source on GitHub
                </a>
                <span className={css({ fontSize: '13.5px', color: 'fg.faint' })}>MIT licensed · no telemetry</span>
              </div>
            </div>

            <div
              className={css({
                animation: 'fadeUp .7s cubic-bezier(.2,.7,.3,1) .12s both',
                minW: '0',
              })}
            >
              <Terminal />
            </div>
          </div>

          <div
            className={css({
              mt: { base: '16', md: '24' },
              pt: '8',
              borderTop: '1px solid token(colors.border.default)',
              display: 'grid',
              gridTemplateColumns: { base: '1fr', sm: 'repeat(3, 1fr)' },
              gap: '8',
            })}
          >
            {STATS.map(([big, small]) => (
              <div key={big}>
                <div className={css({ fontSize: '17px', fontWeight: '600', letterSpacing: '-0.01em' })}>{big}</div>
                <div className={css({ mt: '1', fontSize: '13.5px', color: 'fg.faint' })}>{small}</div>
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  )
}
