import { css } from 'styled-system/css'
import { Band, Eyebrow, Heading } from './ui'

type Row = { label: string; chore: string; just: string; edge: 'chore' | 'just' }

const ROWS: Row[] = [
  { label: 'The shell', chore: 'Built into the binary', just: 'Your sh, or PowerShell', edge: 'chore' },
  { label: 'Same behaviour everywhere', chore: 'By design', just: 'Depends on the shell', edge: 'chore' },
  { label: 'download · extract · archive · sha256', chore: 'Builtins', just: 'Shell out to tools', edge: 'chore' },
  { label: 'Portability lint', chore: 'chore check', just: 'No equivalent', edge: 'chore' },
  { label: 'Flags and validation per task', chore: 'Positional arguments', just: 'Flags, patterns, help', edge: 'just' },
  { label: 'Python or Node inline', chore: 'Call them as commands', just: 'Inline with a shebang', edge: 'just' },
]

function Cell({ text, won }: { text: string; won: boolean }) {
  return (
    <div
      className={css({
        px: { base: '3', md: '4' },
        py: '3',
        fontSize: { base: '13px', md: '13.5px' },
        lineHeight: '1.45',
        color: won ? 'fg.default' : 'fg.faint',
        fontWeight: won ? '500' : '400',
        bg: won ? 'accent.soft' : 'transparent',
      })}
    >
      {text}
    </div>
  )
}

/** chorefile -> one interpreter -> one behaviour. The justfile fans out. */
function Flow() {
  const box = css({
    px: '4',
    py: '2.5',
    rounded: '10px',
    fontFamily: 'mono',
    fontSize: '12.5px',
    textAlign: 'center',
    border: '1px solid token(colors.border.default)',
    bg: 'bg.panel',
    whiteSpace: 'nowrap',
  })
  const arrow = css({ color: 'fg.faint', fontSize: '14px', lineHeight: '1' })
  const column = css({ display: 'flex', flexDir: 'column', alignItems: 'center', gap: '2.5', flex: '1', minW: '0' })

  return (
    <div
      className={css({
        display: 'grid',
        gridTemplateColumns: { base: '1fr', md: '1fr 1fr' },
        gap: { base: '10', md: '6' },
        mt: '12',
        p: { base: '6', md: '9' },
        rounded: '18px',
        border: '1px solid token(colors.border.default)',
        bg: 'bg.panel',
      })}
    >
      <div className={column}>
        <span className={css({ fontSize: '11px', letterSpacing: '0.12em', textTransform: 'uppercase', color: 'fg.accent', fontFamily: 'mono', mb: '1' })}>
          chore
        </span>
        <div className={box} style={{ borderColor: 'var(--colors-accent-ring)' }}>chorefile</div>
        <span className={arrow}>↓</span>
        <div className={box} style={{ borderColor: 'var(--colors-accent-ring)' }}>chore's own interpreter</div>
        <span className={arrow}>↓</span>
        <div className={css({ fontSize: '13px', color: 'fg.default', fontWeight: '500', textAlign: 'center' })}>
          same argv, same filesystem
        </div>
        <span className={arrow}>↓</span>
        <div className={css({ fontFamily: 'mono', fontSize: '12.5px', color: 'fg.muted', textAlign: 'center' })}>
          macOS · Linux · Windows
        </div>
      </div>

      <div className={column}>
        <span className={css({ fontSize: '11px', letterSpacing: '0.12em', textTransform: 'uppercase', color: 'fg.faint', fontFamily: 'mono', mb: '1' })}>
          a justfile
        </span>
        <div className={box}>justfile</div>
        <span className={arrow}>↓</span>
        <div className={css({ display: 'flex', gap: '2', flexWrap: 'wrap', justifyContent: 'center' })}>
          <div className={box}>sh</div>
          <div className={box}>pwsh</div>
          <div className={box}>whatever is set</div>
        </div>
        <span className={arrow}>↓</span>
        <div className={css({ fontSize: '13px', color: 'fg.faint', textAlign: 'center' })}>
          quoting and tools vary
        </div>
        <span className={arrow}>↓</span>
        <div className={css({ fontFamily: 'mono', fontSize: '12.5px', color: 'fg.faint', textAlign: 'center' })}>
          macOS · Linux · Windows
        </div>
      </div>
    </div>
  )
}

export function Versus() {
  return (
    <Band id="versus" ambient="corner">
      <div className={css({ display: 'flex', flexWrap: 'wrap', alignItems: 'end', justifyContent: 'space-between', gap: '6' })}>
        <div>
          <Eyebrow>chore or just</Eyebrow>
          <Heading>just is the one to compare against.</Heading>
        </div>
        <p className={css({ fontSize: '14.5px', color: 'fg.faint', maxW: '300px', lineHeight: '1.55' })}>
          It is a good tool, and where it wins it wins on breadth. One row is the reason chore exists.
        </p>
      </div>

      <div
        className={css({
          mt: '10',
          border: '1px solid token(colors.border.default)',
          rounded: '16px',
          overflow: 'hidden',
        })}
      >
        <div
          className={css({
            display: 'grid',
            gridTemplateColumns: { base: '1fr 1fr', md: '1.1fr 1fr 1fr' },
            bg: 'bg.subtle',
            borderBottom: '1px solid token(colors.border.default)',
            fontSize: '12px',
            fontWeight: '600',
          })}
        >
          <div className={css({ px: { base: '3', md: '4' }, py: '3', display: { base: 'none', md: 'block' }, color: 'fg.faint' })} />
          <div className={css({ px: { base: '3', md: '4' }, py: '3', color: 'fg.accent' })}>chore</div>
          <div className={css({ px: { base: '3', md: '4' }, py: '3', color: 'fg.muted' })}>just</div>
        </div>

        {ROWS.map((r) => (
          <div
            key={r.label}
            className={css({
              display: 'grid',
              gridTemplateColumns: { base: '1fr 1fr', md: '1.1fr 1fr 1fr' },
              borderBottom: '1px solid token(colors.border.default)',
              _last: { borderBottom: 'none' },
            })}
          >
            <div
              className={css({
                gridColumn: { base: '1 / -1', md: 'auto' },
                px: { base: '3', md: '4' },
                pt: { base: '3', md: '3' },
                pb: { base: '0', md: '3' },
                fontSize: { base: '13px', md: '13.5px' },
                fontWeight: '500',
                color: 'fg.default',
                lineHeight: '1.4',
              })}
            >
              {r.label}
            </div>
            <Cell text={r.chore} won={r.edge === 'chore'} />
            <Cell text={r.just} won={r.edge === 'just'} />
          </div>
        ))}
      </div>

      <Flow />

      <p className={css({ mt: '8', fontSize: '15px', lineHeight: '1.6', color: 'fg.muted', maxW: '640px' })}>
        Pick <strong className={css({ color: 'fg.default', fontWeight: '600' })}>just</strong> when you control the
        machine and want the richest task CLI. Pick{' '}
        <strong className={css({ color: 'fg.default', fontWeight: '600' })}>chore</strong> when the file has to run on
        a machine you do not.
      </p>
    </Band>
  )
}
