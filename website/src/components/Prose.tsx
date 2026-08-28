import { Fragment } from 'react'
import { css, cx } from 'styled-system/css'

export const mono = css({ fontFamily: 'mono' })

const inlineCode = css({
  fontFamily: 'mono',
  fontSize: '0.88em',
  px: '1.5',
  py: '0.5',
  rounded: '5px',
  bg: 'bg.inset',
  border: '1px solid token(colors.border.default)',
  color: 'fg.default',
  // A long token — `^find src -name '*.rs'` — must not be the thing that makes
  // a phone scroll sideways, so it wraps rather than holding its line.
  overflowWrap: 'anywhere',
})

/**
 * `chore spec` writes its prose with markdown-style backticks around code —
 * `--dry`, `$ROOT`, `gh://owner/repo`. Rendering them literally would put
 * backticks on the page, and stripping them would lose the distinction
 * between a word and a token, so they become inline code.
 *
 * Deliberately not a markdown parser: backticks are the only markup the spec
 * uses, and anything else in the text is meant literally.
 */
export function Prose({ text, className }: { text: string; className?: string }) {
  return (
    <p className={cx(css({ lineHeight: '1.7' }), className)}>
      {text.split(/`([^`]+)`/g).map((part, i) =>
        i % 2 === 1 ? (
          <code key={i} className={inlineCode}>
            {part}
          </code>
        ) : (
          <Fragment key={i}>{part}</Fragment>
        ),
      )}
    </p>
  )
}
