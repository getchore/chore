import { css } from 'styled-system/css'
import { Band, Eyebrow, Heading } from './ui'

const FEATURES: { tag: string; title: string; body: string }[] = [
  { tag: 'portable', title: 'The shell is inside the binary', body: 'No bash. No PowerShell quoting. No surprises.' },
  { tag: 'batteries', title: 'download · extract · archive', body: 'Retries, timeouts and checksums, built in.' },
  { tag: 'safe', title: 'argv goes straight to the OS', body: 'Nothing re-quoted. A quoted word is one argument.' },
  { tag: 'preview', title: '--dry that actually predicts', body: 'Effects skipped, conditions still run.' },
  { tag: 'lint', title: 'chore check', body: 'Catches typos and curl before anything runs.' },
  { tag: 'agents', title: 'chore spec', body: 'The whole language as JSON, from the binary.' },
]

export function Features() {
  return (
    <Band id="features" ambient="grid">
      <Eyebrow>why chore</Eyebrow>
      <Heading>Task runners break at the shell. This one has no shell.</Heading>

      <div
        className={css({
          mt: '12',
          display: 'grid',
          gridTemplateColumns: { base: '1fr', sm: 'repeat(2, 1fr)', lg: 'repeat(3, 1fr)' },
          gap: '1px',
          bg: 'border.default',
          border: '1px solid token(colors.border.default)',
          rounded: '18px',
          overflow: 'hidden',
        })}
      >
        {FEATURES.map((f) => (
          <div
            key={f.title}
            className={css({
              bg: 'bg.panel',
              px: { base: '7', md: '8' },
              py: { base: '7', md: '9' },
              transition: 'background .2s',
              _hover: { bg: 'bg.subtle' },
            })}
          >
            <span
              className={css({
                fontFamily: 'mono',
                fontSize: '10.5px',
                letterSpacing: '0.12em',
                textTransform: 'uppercase',
                color: 'fg.faint',
              })}
            >
              {f.tag}
            </span>
            <h3
              className={css({
                mt: '4',
                fontSize: '18px',
                fontWeight: '600',
                letterSpacing: '-0.02em',
                lineHeight: '1.25',
              })}
            >
              {f.title}
            </h3>
            <p className={css({ mt: '2.5', fontSize: '14.5px', lineHeight: '1.5', color: 'fg.faint' })}>{f.body}</p>
          </div>
        ))}
      </div>
    </Band>
  )
}
