import type { ReactNode } from 'react'
import { css } from 'styled-system/css'
import { Eyebrow, Heading, Lede, Section } from './ui'

const FEATURES: { title: string; body: ReactNode; tag: string }[] = [
  {
    tag: 'portable',
    title: 'The shell is inside the binary',
    body: 'A POSIX-sh subset, interpreted by chore itself. No bash on the runner, no PowerShell quoting rules, no "works on my machine" between macOS and Windows.',
  },
  {
    tag: 'batteries',
    title: 'Builtins instead of curl and tar',
    body: (
      <>
        <code>download</code>, <code>extract</code>, <code>archive</code>, <code>copy</code>, <code>sha256</code> and a
        dozen more ship in the binary — with retries, timeouts and checksum verification built in.
      </>
    ),
  },
  {
    tag: 'safe',
    title: 'argv goes straight to the OS',
    body: 'Nothing is re-quoted or re-expanded on its way to the process. A quoted word is exactly one argument, every time, on every platform.',
  },
  {
    tag: 'preview',
    title: '--dry that actually predicts',
    body: 'Effects are skipped, but captures and conditions still run — so interpolated paths are real and the preview describes a run that could genuinely happen.',
  },
  {
    tag: 'lint',
    title: 'chore check, before anything runs',
    body: 'Syntax errors, undefined variables, duplicate names, reserved names, and non-portable calls like curl or unzip — each pointed at the builtin that replaces it.',
  },
  {
    tag: 'agents',
    title: 'chore spec, for your tooling',
    body: 'The full language reference as JSON, straight from the binary. Agents and editors read the same source of truth the interpreter does.',
  },
]

export function Features() {
  return (
    <Section id="features">
      <Eyebrow>why chore</Eyebrow>
      <Heading>Task runners break at the shell boundary. This one has no boundary.</Heading>
      <Lede>
        Make needs make. npm scripts need node and whatever <code>sh</code> the OS happens to have. chore needs a file.
      </Lede>

      <div
        className={css({
          mt: '14',
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
              p: { base: '7', md: '8' },
              transition: 'background .2s',
              _hover: { bg: 'bg.subtle' },
              '& code': {
                fontFamily: 'mono',
                fontSize: '0.88em',
                color: 'fg.accent',
                bg: 'accent.soft',
                px: '1',
                py: '0.5',
                rounded: '4px',
              },
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
            <h3 className={css({ mt: '4', fontSize: '17px', fontWeight: '600', letterSpacing: '-0.015em' })}>
              {f.title}
            </h3>
            <p className={css({ mt: '3', fontSize: '14.5px', lineHeight: '1.65', color: 'fg.muted' })}>{f.body}</p>
          </div>
        ))}
      </div>
    </Section>
  )
}
