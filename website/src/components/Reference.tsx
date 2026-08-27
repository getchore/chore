import { css } from 'styled-system/css'
import { Eyebrow, Heading, Section } from './ui'

const BUILTINS = [
  'download', 'extract', 'archive', 'copy', 'move', 'remove',
  'mkdir', 'chmod', 'which', 'find', 'read', 'write',
  'sha256', 'exists', 'echo', 'env', 'fail', 'sleep',
]

const VARS: [string, string][] = [
  ['$OS', 'macos | linux | windows'],
  ['$ARCH', 'x86_64 | arm64'],
  ['$ENV', 'gnu | msvc'],
  ['$PLATFORM', '$OS-$ARCH'],
  ['$EXE', '"" or ".exe"'],
  ['$ROOT', 'chorefile dir'],
  ['$HOME', 'user home'],
  ['$CWD', 'current dir'],
  ['$TASK', 'running task'],
  ['$NOW', 'ISO timestamp'],
]

export function Reference() {
  return (
    <Section id="reference">
      <div
        className={css({
          display: 'flex',
          flexWrap: 'wrap',
          alignItems: 'end',
          justifyContent: 'space-between',
          gap: '6',
        })}
      >
        <div>
          <Eyebrow>reference</Eyebrow>
          <Heading>Everything portable, already in the box.</Heading>
        </div>
        <p className={css({ fontSize: '14.5px', color: 'fg.faint', maxW: '280px', lineHeight: '1.55' })}>
          These resolve before <code className={css({ fontFamily: 'mono' })}>PATH</code>, so a chorefile runs on a
          bare container with nothing installed.
        </p>
      </div>

      <div
        className={css({
          mt: '12',
          display: 'grid',
          gridTemplateColumns: { base: '1fr', lg: '1.35fr 1fr' },
          gap: { base: '10', lg: '14' },
        })}
      >
        <div>
          <h3 className={css({ fontSize: '12.5px', fontWeight: '600', color: 'fg.faint', mb: '5' })}>
            18 builtin commands
          </h3>
          <div className={css({ display: 'flex', flexWrap: 'wrap', gap: '2' })}>
            {BUILTINS.map((b) => (
              <span
                key={b}
                className={css({
                  fontFamily: 'mono',
                  fontSize: '13.5px',
                  px: '3',
                  py: '2',
                  rounded: '9px',
                  color: 'fg.accent',
                  bg: 'accent.soft',
                  border: '1px solid transparent',
                  transition: 'border-color .15s',
                  _hover: { borderColor: 'accent.ring' },
                })}
              >
                {b}
              </span>
            ))}
          </div>
        </div>

        <div>
          <h3 className={css({ fontSize: '12.5px', fontWeight: '600', color: 'fg.faint', mb: '5' })}>
            Builtin variables
          </h3>
          <div
            className={css({
              bg: 'bg.inset',
              border: '1px solid token(colors.border.default)',
              rounded: '14px',
              px: '5',
              py: '2',
            })}
          >
            {VARS.map(([name, desc]) => (
              <div
                key={name}
                className={css({
                  display: 'flex',
                  justifyContent: 'space-between',
                  gap: '4',
                  py: '2.5',
                  fontFamily: 'mono',
                  fontSize: '12.5px',
                  borderBottom: '1px solid token(colors.border.default)',
                  _last: { borderBottom: 'none' },
                })}
              >
                <span className={css({ color: 'fg.accent' })}>{name}</span>
                <span className={css({ color: 'fg.faint', textAlign: 'right' })}>{desc}</span>
              </div>
            ))}
          </div>
        </div>
      </div>
    </Section>
  )
}
