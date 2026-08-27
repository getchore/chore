import { css } from 'styled-system/css'
import { GITHUB_URL } from '../lib/detect'
import { useTheme } from '../lib/useTheme'
import { GitHubIcon, container } from './ui'

const LINKS = [
  ['Why', '#features'],
  ['Example', '#example'],
  ['Reference', '#reference'],
]

function SunMoon({ dark }: { dark: boolean }) {
  return dark ? (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden>
      <circle cx="12" cy="12" r="4" />
      <path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" />
    </svg>
  ) : (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8Z" />
    </svg>
  )
}

export function Nav() {
  const { theme, toggle } = useTheme()

  return (
    <header
      className={css({
        position: 'sticky',
        top: '0',
        zIndex: '50',
        borderBottom: '1px solid token(colors.border.default)',
        bg: { base: 'rgba(255,255,255,0.72)', _dark: 'rgba(8,9,12,0.72)' },
        backdropFilter: 'saturate(180%) blur(12px)',
      })}
    >
      <div className={container}>
        <div className={css({ display: 'flex', alignItems: 'center', gap: '8', h: '60px' })}>
              <a
                href="#top"
                className={css({ display: 'flex', alignItems: 'center', gap: '2.5', fontWeight: '700', letterSpacing: '-0.02em' })}
              >
                <span
                  className={css({
                    display: 'grid',
                    placeItems: 'center',
                    w: '26px',
                    h: '26px',
                    rounded: '7px',
                    bg: 'accent.solid',
                    color: 'white',
                    fontFamily: 'mono',
                    fontSize: '13px',
                    fontWeight: '700',
                  })}
                >
                  ❯
                </span>
                chore
              </a>

              <nav className={css({ display: { base: 'none', md: 'flex' }, gap: '7', ml: '2' })}>
                {LINKS.map(([label, href]) => (
                  <a
                    key={href}
                    href={href}
                    className={css({
                      fontSize: '14px',
                      color: 'fg.muted',
                      transition: 'color .15s',
                      _hover: { color: 'fg.default' },
                    })}
                  >
                    {label}
                  </a>
                ))}
              </nav>

              <div className={css({ ml: 'auto', display: 'flex', alignItems: 'center', gap: '2' })}>
                <button
                  type="button"
                  onClick={toggle}
                  aria-label={`Switch to ${theme === 'dark' ? 'light' : 'dark'} mode`}
                  className={css({
                    display: 'grid',
                    placeItems: 'center',
                    w: '34px',
                    h: '34px',
                    rounded: '9px',
                    cursor: 'pointer',
                    bg: 'transparent',
                    color: 'fg.muted',
                    border: '1px solid transparent',
                    transition: 'all .15s',
                    _hover: { color: 'fg.default', bg: 'bg.subtle', borderColor: 'border.default' },
                  })}
                >
                  <SunMoon dark={theme === 'dark'} />
                </button>
                <a
                  href={GITHUB_URL}
                  className={css({
                    display: 'flex',
                    alignItems: 'center',
                    gap: '2',
                    px: '3.5',
                    h: '34px',
                    rounded: '9px',
                    fontSize: '13.5px',
                    fontWeight: '500',
                    border: '1px solid token(colors.border.default)',
                    bg: 'bg.panel',
                    color: 'fg.default',
                    transition: 'border-color .15s',
                    _hover: { borderColor: 'border.strong' },
                  })}
                >
                  <GitHubIcon />
                  <span className={css({ display: { base: 'none', sm: 'inline' } })}>GitHub</span>
                </a>
              </div>
        </div>
      </div>
    </header>
  )
}
