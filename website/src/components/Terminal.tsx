import { useEffect, useRef, useState } from 'react'
import { css } from 'styled-system/css'

type Line = { text: string; kind: 'prompt' | 'echo' | 'out' | 'ok' }

const SCRIPT: Line[] = [
  { text: 'chore build', kind: 'prompt' },
  { text: '$ mkdir -p build', kind: 'echo' },
  { text: '$ download gh://sona-lang/deps/v3/llvm-arm64.tar.zst vendor/llvm.tar.zst', kind: 'echo' },
  { text: '  vendor/llvm.tar.zst  18.4 MB  sha256 ok', kind: 'out' },
  { text: '$ extract vendor/llvm.tar.zst vendor/llvm --strip 1', kind: 'echo' },
  { text: '$ cmake -B build -DCMAKE_BUILD_TYPE=Release', kind: 'echo' },
  { text: '  -- Configuring done', kind: 'out' },
  { text: '$ cmake --build build --parallel', kind: 'echo' },
  { text: '  [100%] Built target sona', kind: 'out' },
  { text: 'build ok  ·  macos-arm64  ·  6.2s', kind: 'ok' },
]

const COLOR = {
  prompt: css({ color: 'fg.default', fontWeight: '500' }),
  echo: css({ color: { base: '#0369a1', _dark: '#7dd3fc' } }),
  out: css({ color: 'fg.faint' }),
  ok: css({ color: { base: '#15803d', _dark: '#86efac' }, fontWeight: '500' }),
}

export function Terminal() {
  const [shown, setShown] = useState(0)
  const [typed, setTyped] = useState('')
  const timers = useRef<number[]>([])

  useEffect(() => {
    const cmd = SCRIPT[0].text
    let i = 0
    const type = window.setInterval(() => {
      i += 1
      setTyped(cmd.slice(0, i))
      if (i >= cmd.length) {
        window.clearInterval(type)
        SCRIPT.slice(1).forEach((_, n) => {
          timers.current.push(window.setTimeout(() => setShown(n + 1), 380 + n * 330))
        })
      }
    }, 55)
    timers.current.push(type)
    const t = timers.current
    return () => t.forEach(window.clearTimeout)
  }, [])

  const done = shown >= SCRIPT.length - 1

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
          h: '40px',
          borderBottom: '1px solid token(colors.border.default)',
          bg: 'bg.subtle',
        })}
      >
        {['#ff5f57', '#febc2e', '#28c840'].map((c) => (
          <span key={c} className={css({ w: '10px', h: '10px', rounded: 'full' })} style={{ background: c }} />
        ))}
        <span
          className={css({
            ml: 'auto',
            fontFamily: 'mono',
            fontSize: '11px',
            color: 'fg.faint',
          })}
        >
          ~/sona
        </span>
      </div>

      <div
        className={css({
          fontFamily: 'mono',
          fontSize: { base: '11.5px', md: '12.5px' },
          lineHeight: '1.95',
          p: '5',
          minH: '340px',
        })}
      >
        <div className={COLOR.prompt}>
          <span className={css({ color: 'fg.accent', mr: '2' })}>❯</span>
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
        {SCRIPT.slice(1, shown + 1).map((l, i) => (
          <div
            key={i}
            className={COLOR[l.kind]}
            style={{ animation: 'fadeUp .28s ease-out both', whiteSpace: 'pre-wrap', wordBreak: 'break-word' }}
          >
            {l.kind === 'ok' ? `✓ ${l.text}` : l.text}
          </div>
        ))}
      </div>
    </div>
  )
}
