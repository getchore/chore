import { Fragment, type ReactNode } from 'react'
import { css } from 'styled-system/css'
import { spec } from '../lib/spec'

export type Lang = 'chore' | 'yaml' | 'make' | 'json' | 'cmake'

const KEYWORDS: Record<Lang, Set<string>> = {
  chore: new Set(['task', 'if', 'else', 'for', 'in', 'try', 'exit', 'include', 'as']),
  yaml: new Set(['uses:', 'run:', 'name:', 'if:', 'shell:', 'with:', 'env:', 'steps:', 'needs:']),
  make: new Set(['ifeq', 'else', 'endif', 'ifneq']),
  json: new Set([]),
  cmake: new Set([
    'if', 'else', 'endif', 'set', 'file', 'list', 'message',
    'add_custom_target', 'DOWNLOAD', 'ARCHIVE_EXTRACT', 'COMMAND',
    'EXPECTED_HASH', 'DESTINATION', 'INPUT', 'STATUS', 'FATAL_ERROR',
  ]),
}

// From the spec, so a builtin added to the language highlights without anyone
// remembering to edit this file.
const BUILTINS = new Set(spec.builtins.map((b) => b.name))

const tone = {
  comment: css({ color: 'fg.faint', fontStyle: 'italic' }),
  keyword: css({ color: { base: '#9333ea', _dark: '#c084fc' }, fontWeight: '600' }),
  builtin: css({ color: 'fg.accent' }),
  variable: css({ color: { base: '#c2410c', _dark: '#fdba74' } }),
  string: css({ color: { base: '#15803d', _dark: '#86efac' } }),
  operator: css({ color: 'fg.faint' }),
  plain: css({ color: 'fg.default' }),
}

const OPERATORS = new Set(['contains', 'starts-with', 'ends-with', '&&', '||', '==', '!=', '|', '>', '>>'])

function highlight(line: string, i: number, lang: Lang): ReactNode {
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

  const parts = line.split(/("[^"]*"|'[^']*'|\$\{?[A-Za-z_][A-Za-z0-9_]*\}?|\$\([^)]*\)|\$[@#0-9]|\s+)/g)
  return parts.filter(Boolean).map((p, j) => {
    const key = `${i}-${j}`
    if (/^\s+$/.test(p)) return <Fragment key={key}>{p}</Fragment>
    if (p.startsWith('"') || p.startsWith("'")) return <span key={key} className={tone.string}>{p}</span>
    if (p.startsWith('$')) return <span key={key} className={tone.variable}>{p}</span>
    if (KEYWORDS[lang].has(p)) return <span key={key} className={tone.keyword}>{p}</span>
    if (lang === 'chore' && BUILTINS.has(p)) return <span key={key} className={tone.builtin}>{p}</span>
    // yaml/json keys read as structure, not content
    if (lang !== 'chore' && lang !== 'cmake' && p.endsWith(':')) return <span key={key} className={tone.keyword}>{p}</span>
    if (OPERATORS.has(p)) return <span key={key} className={tone.operator}>{p}</span>
    return <span key={key} className={tone.plain}>{p}</span>
  })
}

export function Code({
  source,
  lang = 'chore',
  size = 'md',
}: {
  source: string
  lang?: Lang
  size?: 'sm' | 'md'
}) {
  return (
    <pre
      className={css({
        fontFamily: 'mono',
        fontSize: size === 'sm' ? { base: '11px', md: '12px' } : { base: '12px', md: '13.5px' },
        lineHeight: size === 'sm' ? '1.7' : '1.75',
        overflowX: 'auto',
        p: { base: '5', md: '7' },
        m: '0',
        tabSize: 2,
      })}
    >
      <code>
        {source.split('\n').map((line, i) => (
          <div key={i} className={css({ minH: '1.7em', whiteSpace: 'pre' })}>
            {highlight(line, i, lang)}
          </div>
        ))}
      </code>
    </pre>
  )
}
