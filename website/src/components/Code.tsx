import { Fragment, type ReactNode } from 'react'
import { css } from 'styled-system/css'
import { spec } from '../lib/spec'

export type Lang = 'chore' | 'yaml' | 'make' | 'json' | 'cmake' | 'console'

const KEYWORDS: Record<Lang, Set<string>> = {
  chore: new Set(['task', 'if', 'else', 'for', 'in', 'try', 'exit', 'include', 'as']),
  yaml: new Set(['uses:', 'run:', 'name:', 'if:', 'shell:', 'with:', 'env:', 'steps:', 'needs:']),
  make: new Set(['ifeq', 'else', 'endif', 'ifneq']),
  json: new Set([]),
  // console is line oriented and never consults this table
  console: new Set([]),
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
  // the terminal palette, matching what the binary prints since 1.3.0
  name: css({ color: { base: '#0e7490', _dark: '#67e8f9' } }),
  bad: css({ color: { base: '#b91c1c', _dark: '#fca5a5' } }),
  good: css({ color: { base: '#15803d', _dark: '#86efac' } }),
}

/**
 * A terminal transcript, coloured the way chore colours it. Line oriented
 * rather than token oriented: what a line means here depends on where it sits,
 * so `build` is a task name in a listing and a plain word in a command.
 */
function console_(line: string, key: string | number): ReactNode {
  if (line.trimStart().startsWith('#')) return <span className={tone.comment}>{line}</span>

  // `$ ` prompt, then the command, with the program itself picked out
  if (line.startsWith('$ ')) {
    const [program, ...rest] = line.slice(2).split(' ')
    return (
      <>
        <span className={tone.operator}>$ </span>
        <span className={tone.plain} style={{ fontWeight: 500 }}>{program}</span>
        <span className={tone.operator}>{rest.length ? ` ${rest.join(' ')}` : ''}</span>
      </>
    )
  }

  // a finding: `chorefile:14  message`
  const finding = line.match(/^(\S+:\d+)(\s+)(.*)$/)
  if (finding) {
    return (
      <>
        <span className={tone.operator}>{finding[1]}</span>
        {finding[2]}
        <span className={tone.bad}>{finding[3]}</span>
      </>
    )
  }

  // a listing row: name, gutter, description
  const row = line.match(/^(\s*)(\S+)(\s{2,})(.*)$/)
  if (row) {
    return (
      <>
        {row[1]}
        <span className={tone.name}>{row[2]}</span>
        {row[3]}
        <span className={tone.comment} style={{ fontStyle: 'normal' }}>{row[4]}</span>
      </>
    )
  }

  if (line.startsWith('Available tasks:')) return <span className={tone.plain} style={{ fontWeight: 600 }}>{line}</span>
  if (line.startsWith('added to') || line.startsWith('wrote ')) return <span className={tone.good}>{line}</span>
  return <span className={tone.operator} key={key}>{line}</span>
}

const OPERATORS = new Set(['contains', 'starts-with', 'ends-with', '&&', '||', '==', '!=', '|', '>', '>>'])

function highlight(line: string, i: number, lang: Lang): ReactNode {
  if (lang === 'console') return console_(line, i)
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
