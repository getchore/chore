import { Fragment, type ReactNode } from 'react'
import { css } from 'styled-system/css'

const KEYWORDS = new Set(['task', 'if', 'else', 'for', 'in', 'try', 'exit', 'include', 'as'])
const BUILTINS = new Set([
  'download', 'extract', 'archive', 'copy', 'move', 'remove', 'mkdir', 'chmod',
  'which', 'find', 'read', 'write', 'sha256', 'exists', 'echo', 'env', 'fail', 'sleep',
])
const OPERATORS = new Set(['contains', 'starts-with', 'ends-with', '&&', '||', '==', '!=', '|', '>', '>>'])

const tone = {
  comment: css({ color: 'fg.faint', fontStyle: 'italic' }),
  keyword: css({ color: { base: '#9333ea', _dark: '#c084fc' }, fontWeight: '600' }),
  builtin: css({ color: { base: '#0369a1', _dark: '#7dd3fc' } }),
  variable: css({ color: { base: '#c2410c', _dark: '#fdba74' } }),
  string: css({ color: { base: '#15803d', _dark: '#86efac' } }),
  operator: css({ color: 'fg.faint' }),
  plain: css({ color: 'fg.default' }),
}

/** Tiny chorefile tokenizer — no highlighter dependency, no runtime cost worth naming. */
function highlight(line: string, i: number): ReactNode {
  const trimmed = line.trimStart()
  if (trimmed.startsWith('#')) return <span className={tone.comment}>{line}</span>
  if (trimmed.startsWith('$ ')) {
    return (
      <>
        <span className={tone.operator}>$ </span>
        <span className={tone.plain}>{line.slice(line.indexOf('$ ') + 2)}</span>
      </>
    )
  }

  const parts = line.split(/("[^"]*"|\$[A-Za-z_][A-Za-z0-9_]*|\$[@#0-9]|\s+)/g)
  return parts.filter(Boolean).map((p, j) => {
    const key = `${i}-${j}`
    if (/^\s+$/.test(p)) return <Fragment key={key}>{p}</Fragment>
    if (p.startsWith('"')) return <span key={key} className={tone.string}>{p}</span>
    if (p.startsWith('$')) return <span key={key} className={tone.variable}>{p}</span>
    if (KEYWORDS.has(p)) return <span key={key} className={tone.keyword}>{p}</span>
    if (BUILTINS.has(p)) return <span key={key} className={tone.builtin}>{p}</span>
    if (OPERATORS.has(p)) return <span key={key} className={tone.operator}>{p}</span>
    return <span key={key} className={tone.plain}>{p}</span>
  })
}

export function Code({ source }: { source: string }) {
  return (
    <pre
      className={css({
        fontFamily: 'mono',
        fontSize: { base: '12px', md: '13.5px' },
        lineHeight: '1.75',
        overflowX: 'auto',
        p: { base: '5', md: '7' },
        m: '0',
        tabSize: 2,
      })}
    >
      <code>
        {source.split('\n').map((line, i) => (
          <div key={i} className={css({ minH: '1.75em', whiteSpace: 'pre' })}>
            {highlight(line, i)}
          </div>
        ))}
      </code>
    </pre>
  )
}
