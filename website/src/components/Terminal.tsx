import { useEffect, useRef, useState } from 'react'
import { css } from 'styled-system/css'

type Line =
  | { kind: 'cmd'; verb: string; rest: string }
  | { kind: 'out'; text: string }
  | { kind: 'ok'; text: string }

const SCRIPT: Line[] = [
  { kind: 'cmd', verb: 'download', rest: '$LLVM vendor/llvm.tar.zst' },
  { kind: 'out', text: '18.4 MB · sha256 ok' },
  { kind: 'cmd', verb: 'extract', rest: 'vendor/llvm.tar.zst vendor/llvm' },
  { kind: 'cmd', verb: 'cmake', rest: '--build build --parallel' },
  { kind: 'out', text: '[100%] Built target sona' },
  { kind: 'ok', text: 'build ok · macos-arm64 · 6.2s' },
]

export function Terminal() {
  const [shown, setShown] = useState(0)
  const [typed, setTyped] = useState('')
  const timers = useRef<number[]>([])

  useEffect(() => {
    const cmd = 'chore build'
    let i = 0
    const type = window.setInterval(() => {
      setTyped(cmd.slice(0, ++i))
      if (i >= cmd.length) {
        window.clearInterval(type)
        SCRIPT.forEach((_, n) => {
          timers.current.push(window.setTimeout(() => setShown(n + 1), 320 + n * 300))
        })
      }
    }, 60)
    timers.current.push(type)
    const t = timers.current
    return () => t.forEach(window.clearTimeout)
  }, [])

  const done = shown >= SCRIPT.length

  return (
    <div
      className={css({
        bg: { base: '#fbfbfc', _dark: '#0b0d12' },
        border: '1px solid token(colors.border.default)',
        rounded: '16px',
        overflow: 'hidden',
        boxShadow: {
          base: '0 2px 4px rgba(0,0,0,.04), 0 24px 60px -30px rgba(0,0,0,.35)',
          _dark: '0 24px 70px -30px rgba(0,0,0,1)',
        },
      })}
    >
      <div
        className={css({
          display: 'flex',
          alignItems: 'center',
          gap: '2',
          px: '4',
          h: '38px',
          borderBottom: '1px solid token(colors.border.default)',
          bg: 'bg.subtle',
        })}
      >
        {['#ff5f57', '#febc2e', '#28c840'].map((c) => (
          <span key={c} className={css({ w: '10px', h: '10px', rounded: 'full' })} style={{ background: c }} />
        ))}
        <span className={css({ ml: 'auto', fontFamily: 'mono', fontSize: '11px', color: 'fg.faint' })}>~/sona</span>
      </div>

      <div
        className={css({
          fontFamily: 'mono',
          fontSize: { base: '12px', md: '13.5px' },
          lineHeight: '2.05',
          px: { base: '5', md: '7' },
          py: '6',
          minH: '270px',
          whiteSpace: 'nowrap',
          overflowX: 'auto',
          scrollbarWidth: 'none',
        })}
      >
        <div className={css({ fontWeight: '500' })}>
          <span className={css({ color: 'accent.solid', mr: '2.5' })}>❯</span>
          {typed}
          {!done && (
            <span
              className={css({
                display: 'inline-block',
                w: '7px',
                h: '1em',
                ml: '1px',
                bg: 'accent.solid',
                verticalAlign: 'text-bottom',
                animation: 'blink 1s steps(1) infinite',
              })}
            />
          )}
        </div>

        {SCRIPT.slice(0, shown).map((l, i) => (
          <div key={i} style={{ animation: 'fadeUp .26s ease-out both' }}>
            {l.kind === 'cmd' && (
              <>
                <span className={css({ color: 'fg.faint', mr: '2.5' })}>$</span>
                <span className={css({ color: 'fg.accent' })}>{l.verb}</span>
                <span className={css({ color: 'fg.muted' })}> {l.rest}</span>
              </>
            )}
            {l.kind === 'out' && <span className={css({ color: 'fg.faint', pl: '6' })}>{l.text}</span>}
            {l.kind === 'ok' && (
              <span className={css({ color: { base: '#15803d', _dark: '#4ade80' }, fontWeight: '500' })}>
                ✓ {l.text}
              </span>
            )}
          </div>
        ))}
      </div>
    </div>
  )
}
