import { useEffect, useState } from 'react'
import { css, cx } from 'styled-system/css'
import { INSTALL, OS_LABEL, detectOs, type OsKey } from '../lib/detect'
import { CheckIcon, CopyIcon } from './ui'

const part = {
  cmd: css({ color: 'fg.default', fontWeight: '600' }),
  flag: css({ color: 'fg.faint' }),
  url: css({ color: 'fg.accent' }),
  pipe: css({ color: 'fg.faint' }),
}

/** The one-liner is the first thing anyone reads, so colour it like code. */
function highlightCommand(command: string) {
  return command.split(' ').map((word, i) => {
    const space = i === 0 ? '' : ' '
    let tone = part.pipe
    if (i === 0) tone = part.cmd
    else if (word.startsWith('-')) tone = part.flag
    else if (word.startsWith('http')) tone = part.url
    return (
      <span key={i} className={tone}>
        {space}
        {word}
      </span>
    )
  })
}

const ORDER: OsKey[] = ['macos', 'linux', 'windows']

function OsIcon({ os }: { os: OsKey }) {
  const p = {
    macos: 'M11.2 5.9c.7 0 1.7-.5 2.3-1.1.5-.6.9-1.4.9-2.2 0-.1 0-.2 0-.3-.9 0-2 .6-2.6 1.3-.5.6-1 1.4-1 2.2 0 .1 0 .2 0 .3h.4Zm2.6 1.3c-1.4-.1-2.6.8-3.2.8-.7 0-1.7-.7-2.8-.7-1.4 0-2.8.9-3.5 2.2-1.5 2.6-.4 6.5 1.1 8.6.7 1 1.6 2.2 2.7 2.2 1.1 0 1.5-.7 2.8-.7s1.6.7 2.8.7c1.2 0 1.9-1 2.6-2.1.8-1.2 1.2-2.4 1.2-2.5-.1 0-2.3-.9-2.3-3.4 0-2.1 1.7-3.1 1.8-3.2-1-1.4-2.5-1.6-3-1.7Z',
    linux: 'M12 2c-2 0-3.2 1.6-3.2 3.7 0 1.3.2 2 .1 2.9-.2 1.1-1.4 2.6-2 4.1-.6 1.4-.4 2.8-.9 3.3-.6.6-.5 1.3.2 1.6.6.3 1.5.4 2.2 1 .7.5 1.4 1.4 2.9 1.4 1.6 0 2.7-.8 3.6-1.3.9-.5 2.2-.7 2.6-1.3.4-.6.1-1.2-.5-1.9-.5-.6-.7-2-1.3-3.3-.7-1.5-1.8-2.8-2-3.9-.2-.9 0-1.6 0-2.6C13.7 3.6 13.4 2 12 2Zm-1.4 3.1c.4 0 .7.4.7.9s-.3.9-.7.9-.7-.4-.7-.9.3-.9.7-.9Zm2.8 0c.4 0 .7.4.7.9s-.3.9-.7.9-.7-.4-.7-.9.3-.9.7-.9Z',
    windows: 'M3 5.6 10.2 4.6v6.9H3V5.6Zm0 12.8 7.2 1v-6.8H3v5.8Zm8.1 1.2L21 21V12.5h-9.9v7.1Zm0-15.2v7.1H21V3l-9.9 1.4Z',
  }[os]
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor" aria-hidden>
      <path d={p} />
    </svg>
  )
}

export function Install() {
  const [os, setOs] = useState<OsKey>('macos')
  const [copied, setCopied] = useState(false)

  useEffect(() => setOs(detectOs()), [])
  useEffect(() => {
    if (!copied) return
    const t = setTimeout(() => setCopied(false), 1800)
    return () => clearTimeout(t)
  }, [copied])

  const { command, shell } = INSTALL[os]

  async function copy() {
    try {
      await navigator.clipboard.writeText(command)
      setCopied(true)
    } catch {
      /* clipboard blocked, but the text stays selectable */
    }
  }

  return (
    <div
      className={css({
        w: 'full',
        maxW: '620px',
        bg: 'bg.panel',
        border: '1px solid token(colors.border.default)',
        rounded: '14px',
        overflow: 'hidden',
        boxShadow: { base: '0 1px 2px rgba(0,0,0,.05), 0 12px 32px -18px rgba(0,0,0,.3)', _dark: '0 12px 40px -20px rgba(0,0,0,.9)' },
      })}
    >
      <div
        className={css({
          display: 'flex',
          alignItems: 'center',
          gap: '1',
          px: '2',
          pt: '2',
          borderBottom: '1px solid token(colors.border.default)',
          bg: 'bg.subtle',
        })}
      >
        {ORDER.map((key) => (
          <button
            key={key}
            type="button"
            onClick={() => setOs(key)}
            aria-pressed={os === key}
            className={cx(
              css({
                display: 'flex',
                alignItems: 'center',
                gap: '1.5',
                px: '3',
                py: '2',
                fontSize: '12.5px',
                fontWeight: '500',
                cursor: 'pointer',
                bg: 'transparent',
                color: 'fg.faint',
                borderBottom: '2px solid transparent',
                mb: '-1px',
                transition: 'color .15s, border-color .15s',
                _hover: { color: 'fg.default' },
              }),
              os === key &&
                css({ color: 'fg.default', borderBottomColor: 'accent.solid' }),
            )}
          >
            <OsIcon os={key} />
            {OS_LABEL[key]}
          </button>
        ))}
        <span
          className={css({
            ml: 'auto',
            mr: '2',
            fontFamily: 'mono',
            fontSize: '10.5px',
            color: 'fg.faint',
            display: { base: 'none', sm: 'block' },
          })}
        >
          {shell}
        </span>
      </div>

      <div className={css({ display: 'flex', alignItems: 'center', gap: '3', px: '4', py: '3.5' })}>
        <span className={css({ fontFamily: 'mono', fontSize: '13px', color: 'fg.faint', userSelect: 'none' })}>
          {os === 'windows' ? '>' : '$'}
        </span>
        <code
          className={css({
            fontFamily: 'mono',
            fontSize: { base: '11.5px', sm: '13px' },
            color: 'fg.default',
            flex: '1',
            overflowX: 'auto',
            whiteSpace: 'nowrap',
            scrollbarWidth: 'none',
            // the one-liner is longer than the box on narrow columns; fade rather than cut
            maskImage: 'linear-gradient(to right, #000 calc(100% - 28px), transparent)',
            '&::-webkit-scrollbar': { display: 'none' },
          })}
        >
          {highlightCommand(command)}
        </code>
        <button
          type="button"
          onClick={copy}
          aria-label={copied ? 'Copied' : 'Copy install command'}
          className={css({
            display: 'flex',
            alignItems: 'center',
            gap: '1.5',
            flexShrink: '0',
            px: '2.5',
            py: '1.5',
            rounded: '8px',
            cursor: 'pointer',
            fontSize: '12px',
            fontWeight: '500',
            border: '1px solid token(colors.border.default)',
            bg: 'bg.subtle',
            color: copied ? 'fg.accent' : 'fg.muted',
            transition: 'all .15s',
            _hover: { color: 'fg.default', borderColor: 'border.strong' },
          })}
        >
          {copied ? <CheckIcon /> : <CopyIcon />}
          <span className={css({ display: { base: 'none', sm: 'inline' } })}>{copied ? 'Copied' : 'Copy'}</span>
        </button>
      </div>
    </div>
  )
}
