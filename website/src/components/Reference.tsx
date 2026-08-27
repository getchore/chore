import { css } from 'styled-system/css'
import { Eyebrow, Heading, Lede, Section } from './ui'

const BUILTINS: [string, string][] = [
  ['download <url> <dest>', 'http(s) and gh://owner/repo/tag/asset, with --retries, --timeout, --sha256'],
  ['extract <archive> <dest>', 'zip, tar, .gz .xz .zst — with --member and --strip'],
  ['archive <src> <dest>', 'format inferred from the extension'],
  ['copy / move <src> <dest>', 'file or directory, recursive'],
  ['remove <path...>', 'recursive, no error when missing'],
  ['mkdir <path...>', '-p semantics'],
  ['find <root> <name...>', 'every match, recursive, one per line'],
  ['read / write <file>', 'contents trimmed; >> appends'],
  ['sha256 <file>', 'hex digest on stdout'],
  ['exists <path>', 'exit 0/1 — for use in if'],
  ['which <name>', 'prints the path, or fails'],
  ['env <NAME> [value]', 'get or set'],
]

const VARS: [string, string][] = [
  ['$OS', 'macos | linux | windows'],
  ['$ARCH', 'x86_64 | arm64'],
  ['$ENV', 'gnu | msvc | ""'],
  ['$PLATFORM', '$OS-$ARCH'],
  ['$EXE', '"" or ".exe"'],
  ['$ROOT', 'dir of the top-level chorefile'],
  ['$HOME', 'user home'],
  ['$CWD', 'current directory'],
  ['$TASK', 'name of the running task'],
  ['$NOW', 'ISO timestamp'],
]

export function Reference() {
  return (
    <Section id="reference">
      <Eyebrow>reference</Eyebrow>
      <Heading>Everything portable, already in the box.</Heading>
      <Lede>
        These resolve before <code className={css({ fontFamily: 'mono' })}>PATH</code>, so a chorefile that downloads,
        verifies and unpacks a toolchain runs on a bare Windows container with nothing installed.
      </Lede>

      <div
        className={css({
          mt: '14',
          display: 'grid',
          gridTemplateColumns: { base: '1fr', lg: '1.5fr 1fr' },
          gap: { base: '10', lg: '14' },
        })}
      >
        <div>
          <h3 className={css({ fontSize: '13px', fontWeight: '600', color: 'fg.muted', mb: '5' })}>
            Builtin commands
          </h3>
          <dl className={css({ display: 'grid', gap: '0' })}>
            {BUILTINS.map(([sig, desc]) => (
              <div
                key={sig}
                className={css({
                  display: 'grid',
                  gridTemplateColumns: { base: '1fr', sm: '260px 1fr' },
                  gap: { base: '1', sm: '6' },
                  py: '3.5',
                  borderBottom: '1px solid token(colors.border.default)',
                })}
              >
                <dt className={css({ fontFamily: 'mono', fontSize: '12.5px', color: 'fg.accent' })}>{sig}</dt>
                <dd className={css({ fontSize: '13.5px', color: 'fg.muted', lineHeight: '1.55' })}>{desc}</dd>
              </div>
            ))}
          </dl>
        </div>

        <div>
          <h3 className={css({ fontSize: '13px', fontWeight: '600', color: 'fg.muted', mb: '5' })}>
            Builtin variables
          </h3>
          <div
            className={css({
              bg: 'bg.inset',
              border: '1px solid token(colors.border.default)',
              rounded: '14px',
              p: '5',
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
                  _last: { borderBottom: 'none', pb: '0' },
                })}
              >
                <span className={css({ color: 'fg.accent' })}>{name}</span>
                <span className={css({ color: 'fg.faint', textAlign: 'right' })}>{desc}</span>
              </div>
            ))}
          </div>
          <p className={css({ mt: '4', fontSize: '13px', color: 'fg.faint', lineHeight: '1.6' })}>
            Read-only and always set. Paths are written with <code className={css({ fontFamily: 'mono' })}>/</code>{' '}
            everywhere and converted on Windows.
          </p>
        </div>
      </div>
    </Section>
  )
}
