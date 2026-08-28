import { css } from 'styled-system/css'
import { Link } from './Link'
import { Band, Eyebrow, Heading } from './ui'
import { spec } from '../lib/spec'

/**
 * A taste of the reference, not the reference. The full page is generated
 * from `chore spec` and lives at /reference; the landing page shows the
 * builtin names — the one part of the language that is legible at a glance —
 * and gets out of the way.
 *
 * Every name and every count below is read from the spec. Nothing about the
 * language is typed into this file, which is the whole point: the hand-written
 * version of this section shipped a variable that no longer existed and missed
 * three that did.
 */
export function Reference() {
  return (
    <Band id="reference" ambient="corner">
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

      <div className={css({ mt: '11', display: 'flex', flexWrap: 'wrap', gap: '2' })}>
        {spec.builtins.map((b) => (
          <span
            key={b.name}
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
            {b.name}
          </span>
        ))}
      </div>

      <div
        className={css({
          mt: '10',
          display: 'flex',
          flexWrap: 'wrap',
          alignItems: 'center',
          gap: { base: '5', md: '8' },
        })}
      >
        <Link
          to="reference"
          className={css({
            display: 'inline-flex',
            alignItems: 'center',
            gap: '2',
            px: '5',
            h: '42px',
            rounded: '10px',
            fontSize: '14.5px',
            fontWeight: '600',
            bg: 'accent.solid',
            color: 'white',
            transition: 'opacity .15s',
            _hover: { opacity: 0.9 },
          })}
        >
          Read the language reference
          <span aria-hidden className={css({ fontFamily: 'mono' })}>
            →
          </span>
        </Link>

        <p className={css({ fontSize: '14px', color: 'fg.muted', lineHeight: '1.6' })}>
          {spec.builtins.length} builtins, {spec.variables.length} variables and {spec.syntax.length} syntax
          forms, with flags, defaults and the rules that surprise people — generated from{' '}
          <code className={css({ fontFamily: 'mono' })}>chore spec</code> {spec.version}.
        </p>
      </div>
    </Band>
  )
}
