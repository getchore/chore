import { useEffect, useState, type ReactNode } from 'react'
import { css, cx } from 'styled-system/css'
import { Code } from '../components/Code'
import { Link } from '../components/Link'
import { Prose, mono } from '../components/Prose'
import { Ambient, Eyebrow, Heading, Lede, container } from '../components/ui'
import { scopes, spec, variablesIn } from '../lib/spec'

/**
 * The language reference. Every word of content below comes out of
 * src/spec.json — the output of `chore spec` — so the page cannot describe a
 * builtin, a flag or a variable the binary does not have. Only the section
 * titles and the layout are written here.
 *
 * The rules go first on purpose. They are the parts that surprise people (no
 * shell, word splitting, run-once, what `--dry` does), and someone who reads
 * only the top of this page should still have read them.
 */

const SECTIONS = [
  { id: 'rules', title: 'Rules', count: spec.rules.length },
  { id: 'builtins', title: 'Builtins', count: spec.builtins.length },
  { id: 'variables', title: 'Variables', count: spec.variables.length },
  { id: 'syntax', title: 'Syntax', count: spec.syntax.length },
  { id: 'conditions', title: 'Conditions', count: spec.conditions.length },
  { id: 'chaining', title: 'Chaining', count: spec.chaining.length },
  { id: 'resolution', title: 'Resolution', count: spec.resolution.length },
]

const IDS = SECTIONS.map((s) => s.id)

/**
 * `scope` is a machine word in the spec; these are the human sentences for the
 * two it currently emits. A scope the spec adds later still renders — it just
 * gets no caption — so this can never hide a variable.
 */
const SCOPE_CAPTION: Record<string, string> = {
  run: 'Fixed for the whole invocation. Every task sees the same value.',
  task: 'Depends on where you are, so the value changes as the run moves.',
}

// ---------------------------------------------------------------- primitives

const card = css({
  bg: 'bg.panel',
  border: '1px solid token(colors.border.default)',
  rounded: '14px',
  p: { base: '5', md: '6' },
})

const label = css({
  fontFamily: 'mono',
  fontSize: '11px',
  letterSpacing: '0.14em',
  textTransform: 'uppercase',
  color: 'fg.faint',
})

function Term({ children }: { children: ReactNode }) {
  return (
    <span
      className={css({
        fontFamily: 'mono',
        fontSize: '14px',
        color: 'fg.accent',
        fontWeight: '600',
        overflowWrap: 'anywhere',
      })}
    >
      {children}
    </span>
  )
}

/** A block of literal source: usage strings, syntax forms. Scrolls, never wraps. */
function Mono({ children, tone = 'default' }: { children: ReactNode; tone?: 'default' | 'inset' }) {
  return (
    <div
      className={cx(
        css({
          fontFamily: 'mono',
          fontSize: { base: '12.5px', md: '13px' },
          lineHeight: '1.6',
          color: 'fg.default',
          overflowX: 'auto',
          // Usage strings are long and the pane is narrow; wrapping at spaces
          // beats a scrollbar that hides the last flag.
          whiteSpace: 'pre-wrap',
          wordBreak: 'break-word',
          px: '3.5',
          py: '2.5',
          rounded: '9px',
        }),
        tone === 'inset'
          ? css({ bg: 'bg.inset', border: '1px solid token(colors.border.default)' })
          : css({ bg: 'transparent' }),
      )}
    >
      {children}
    </div>
  )
}

function Sec({
  id,
  title,
  intro,
  children,
}: {
  id: string
  title: string
  intro?: ReactNode
  children: ReactNode
}) {
  return (
    <section
      id={id}
      className={css({
        scrollMarginTop: { base: '124px', lg: '86px' },
        pt: { base: '14', lg: '16' },
        pb: '2',
        _first: { pt: { base: '8', lg: '2' } },
      })}
    >
      <h2
        className={css({
          fontSize: { base: '24px', md: '28px' },
          letterSpacing: '-0.02em',
          fontWeight: '700',
          scrollMarginTop: '100px',
        })}
      >
        <a href={`#${id}`} className={css({ _hover: { color: 'fg.accent' } })}>
          {title}
        </a>
      </h2>
      {intro && (
        <div className={css({ mt: '3', fontSize: '15px', color: 'fg.muted', maxW: '680px' })}>{intro}</div>
      )}
      <div className={css({ mt: '7' })}>{children}</div>
    </section>
  )
}

const grid2 = css({ display: 'grid', gridTemplateColumns: { base: '1fr', md: '1fr 1fr' }, gap: '4' })

// ------------------------------------------------------------------ builtins

/**
 * `effects` is the one field on a builtin that changes what a run does rather
 * than what it means: `--dry` echoes every command and executes only the ones
 * without effects. It gets a pill rather than a sentence because the reader
 * scanning eighteen cards is asking exactly this.
 */
function Effects({ effects }: { effects: boolean }) {
  return (
    <span
      title={
        effects
          ? '--dry echoes this command and skips it'
          : '--dry still runs this command; it has nothing to undo'
      }
      className={cx(
        css({
          display: 'inline-flex',
          alignItems: 'center',
          gap: '1.5',
          flexShrink: '0',
          fontFamily: 'mono',
          fontSize: '10.5px',
          letterSpacing: '0.08em',
          textTransform: 'uppercase',
          px: '2',
          py: '1',
          rounded: '999px',
          border: '1px solid',
        }),
        effects
          ? css({ bg: 'accent.soft', color: 'fg.accent', borderColor: 'accent.ring' })
          : css({ bg: 'bg.inset', color: 'fg.faint', borderColor: 'border.default' }),
      )}
    >
      <span
        aria-hidden
        className={cx(
          css({ w: '5px', h: '5px', rounded: '999px' }),
          effects ? css({ bg: 'accent.solid' }) : css({ bg: 'fg.faint' }),
        )}
      />
      {effects ? 'effects' : 'read-only'}
    </span>
  )
}

function Builtins() {
  return (
    <div
      className={css({
        border: '1px solid token(colors.border.default)',
        rounded: '16px',
        overflow: 'hidden',
        bg: 'bg.panel',
      })}
    >
      {spec.builtins.map((b) => (
        <div
          key={b.name}
          className={css({
            display: 'grid',
            // The descriptions are paragraphs, so they get the whole width
            // rather than half of it; the identity of the command sits in a
            // fixed rail on the left where the eye can run down it.
            gridTemplateColumns: { base: '1fr', md: '210px 1fr' },
            gap: { base: '4', md: '7' },
            px: { base: '5', md: '6' },
            py: { base: '6', md: '7' },
            borderBottom: '1px solid token(colors.border.default)',
            _last: { borderBottom: 'none' },
          })}
        >
          <div>
            <div className={css({ display: 'flex', alignItems: 'center', gap: '3', flexWrap: 'wrap' })}>
              <h3 className={css({ fontFamily: 'mono', fontSize: '17px', fontWeight: '600', color: 'fg.accent' })}>
                {b.name}
              </h3>
              <Effects effects={b.effects} />
            </div>
            <p className={css({ mt: '2', fontSize: '14px', color: 'fg.muted', lineHeight: '1.5' })}>{b.summary}</p>
          </div>

          <div className={css({ minW: '0' })}>
            <Mono tone="inset">{b.usage}</Mono>
            <Prose text={b.description} className={css({ mt: '3.5', fontSize: '14.5px', color: 'fg.muted' })} />

            {b.flags.length > 0 && (
              <div className={css({ mt: '5' })}>
                <div className={cx(label, css({ mb: '2' }))}>flags</div>
                <div
                  className={css({
                    border: '1px solid token(colors.border.default)',
                    rounded: '10px',
                    overflow: 'hidden',
                  })}
                >
                  {b.flags.map((f) => (
                    <div
                      key={f.name}
                      className={css({
                        display: 'grid',
                        gridTemplateColumns: { base: '1fr', sm: '170px 1fr' },
                        gap: { base: '1', sm: '5' },
                        px: '4',
                        py: '3',
                        borderBottom: '1px solid token(colors.border.default)',
                        _last: { borderBottom: 'none' },
                        bg: 'bg.inset',
                        '& > *': { minW: '0' },
                      })}
                    >
                      <div className={css({ fontFamily: 'mono', fontSize: '13px' })}>
                        <span className={css({ color: 'fg.accent' })}>{f.name}</span>
                        {f.argument && <span className={css({ color: 'fg.faint' })}> {f.argument}</span>}
                      </div>
                      <div className={css({ fontSize: '13.5px', color: 'fg.muted', lineHeight: '1.55' })}>
                        <Prose text={f.meaning} />
                        {f.default !== null && (
                          <div className={css({ mt: '1', color: 'fg.faint', fontSize: '12.5px' })}>
                            default: <span className={mono}>{f.default}</span>
                          </div>
                        )}
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            )}
          </div>
        </div>
      ))}
    </div>
  )
}

// ----------------------------------------------------------------- variables

function Variables() {
  return (
    <div className={css({ display: 'flex', flexDirection: 'column', gap: '8' })}>
      {scopes().map((scope) => (
        <div key={scope}>
          <div className={css({ display: 'flex', alignItems: 'baseline', gap: '3', flexWrap: 'wrap', mb: '4' })}>
            <h3 className={cx(label, css({ color: 'fg.accent' }))}>{scope} scope</h3>
            {SCOPE_CAPTION[scope] && (
              <span className={css({ fontSize: '13.5px', color: 'fg.faint' })}>{SCOPE_CAPTION[scope]}</span>
            )}
          </div>
          <div
            className={css({
              border: '1px solid token(colors.border.default)',
              rounded: '14px',
              overflow: 'hidden',
              bg: 'bg.panel',
            })}
          >
            {variablesIn(scope).map((v) => (
              <div
                key={v.name}
                className={css({
                  display: 'grid',
                  gridTemplateColumns: { base: '1fr', md: '150px 190px 1fr' },
                  gap: { base: '1.5', md: '5' },
                  alignItems: 'baseline',
                  px: { base: '4', md: '5' },
                  py: '3.5',
                  borderBottom: '1px solid token(colors.border.default)',
                  _last: { borderBottom: 'none' },
                  '& > *': { minW: '0' },
                })}
              >
                <Term>${v.name}</Term>
                <div className={css({ fontFamily: 'mono', fontSize: '12.5px', color: 'fg.muted' })}>{v.values}</div>
                <Prose text={v.meaning} className={css({ fontSize: '14px', color: 'fg.muted' })} />
              </div>
            ))}
          </div>
        </div>
      ))}
    </div>
  )
}

// -------------------------------------------------------------------- syntax

function Syntax() {
  return (
    <div className={css({ display: 'grid', gridTemplateColumns: { base: '1fr', lg: '1fr 1fr' }, gap: '4' })}>
      {spec.syntax.map((s) => (
        <div key={s.name} className={card}>
          <div className={css({ display: 'flex', alignItems: 'baseline', gap: '3', mb: '3' })}>
            <h3 className={css({ fontSize: '15px', fontWeight: '600' })}>{s.name}</h3>
          </div>
          <Mono tone="inset">{s.syntax}</Mono>
          <Prose text={s.meaning} className={css({ mt: '3', fontSize: '14px', color: 'fg.muted' })} />
          <div className={css({ mt: '4' })}>
            <div className={cx(label, css({ mb: '1.5' }))}>example</div>
            <div
              className={css({
                bg: 'bg.inset',
                border: '1px solid token(colors.border.default)',
                rounded: '10px',
                overflow: 'hidden',
              })}
            >
              <Code source={s.example} size="sm" />
            </div>
          </div>
        </div>
      ))}
    </div>
  )
}

// -------------------------------------------------------- two-column tables

function Rows({ rows }: { rows: { term: string; meaning: string }[] }) {
  return (
    <div
      className={css({
        border: '1px solid token(colors.border.default)',
        rounded: '14px',
        overflow: 'hidden',
        bg: 'bg.panel',
      })}
    >
      {rows.map((r) => (
        <div
          key={r.term}
          className={css({
            display: 'grid',
            gridTemplateColumns: { base: '1fr', md: '210px 1fr' },
            gap: { base: '1.5', md: '5' },
            alignItems: 'baseline',
            px: { base: '4', md: '5' },
            py: '3.5',
            borderBottom: '1px solid token(colors.border.default)',
            _last: { borderBottom: 'none' },
            '& > *': { minW: '0' },
          })}
        >
          <Term>{r.term}</Term>
          <Prose text={r.meaning} className={css({ fontSize: '14px', color: 'fg.muted' })} />
        </div>
      ))}
    </div>
  )
}

// ---------------------------------------------------------------------- page

function SideNav({ active, onPick }: { active: string; onPick?: () => void }) {
  return (
    <>
      {SECTIONS.map((s) => (
        <a
          key={s.id}
          href={`#${s.id}`}
          onClick={onPick}
          aria-current={active === s.id ? 'true' : undefined}
          className={cx(
            css({
              display: 'flex',
              alignItems: 'center',
              gap: '2',
              flexShrink: '0',
              px: '3',
              py: '2',
              rounded: '9px',
              fontSize: '13.5px',
              whiteSpace: 'nowrap',
              border: '1px solid transparent',
              transition: 'color .15s, background-color .15s, border-color .15s',
              color: 'fg.muted',
              _hover: { color: 'fg.default', bg: 'bg.subtle' },
            }),
            // The current section is marked by weight and a quiet panel, not by
            // a filled ember block: the rail sits beside eighteen cards that
            // already carry the accent, and a solid orange chip fought them.
            active === s.id &&
              css({
                color: 'fg.default',
                bg: 'bg.subtle',
                borderColor: 'border.default',
                fontWeight: '600',
              }),
          )}
        >
          {s.title}
          <span className={css({ fontFamily: 'mono', fontSize: '11px', color: 'fg.faint' })}>{s.count}</span>
        </a>
      ))}
    </>
  )
}

/** Which section the reader is in: the last one whose top has passed the header. */
function useActiveSection(): string {
  const [active, setActive] = useState(IDS[0])

  useEffect(() => {
    let frame = 0
    const measure = () => {
      frame = 0
      const line = 150
      let current = IDS[0]
      for (const id of IDS) {
        const el = document.getElementById(id)
        if (el && el.getBoundingClientRect().top <= line) current = id
      }
      // The last section is often shorter than the viewport, so it would never
      // reach the line; at the bottom of the page it is what you are reading.
      if (window.innerHeight + window.scrollY >= document.documentElement.scrollHeight - 4) {
        current = IDS[IDS.length - 1]
      }
      setActive(current)
    }
    const onScroll = () => {
      if (!frame) frame = requestAnimationFrame(measure)
    }
    measure()
    // A deep link scrolls after this effect has run — the parent's hash scroll,
    // then again once webfonts reflow the page — and neither reliably produces
    // a scroll event. Re-measure a couple of times so a link to #builtins does
    // not open with #rules highlighted.
    const settle = [window.setTimeout(measure, 50), window.setTimeout(measure, 400)]
    window.addEventListener('scroll', onScroll, { passive: true })
    window.addEventListener('resize', onScroll)
    window.addEventListener('hashchange', onScroll)
    return () => {
      if (frame) cancelAnimationFrame(frame)
      settle.forEach(window.clearTimeout)
      window.removeEventListener('scroll', onScroll)
      window.removeEventListener('resize', onScroll)
      window.removeEventListener('hashchange', onScroll)
    }
  }, [])

  return active
}

export function ReferencePage() {
  const active = useActiveSection()

  return (
    <>
      <div className={css({ position: 'relative', overflow: 'hidden', borderBottom: '1px solid token(colors.border.default)' })}>
        <Ambient variant="grid" />
        <div className={cx(container, css({ position: 'relative', pt: { base: '14', md: '20' }, pb: { base: '10', md: '14' } }))}>
          <Eyebrow>reference</Eyebrow>
          <Heading>The whole language, on one page.</Heading>
          <Lede>
            Generated from the binary, not written by hand: this page is a rendering of{' '}
            <code className={mono}>chore spec</code>, so it describes the release below and nothing else.
          </Lede>
          <div className={css({ mt: '7', display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: '3' })}>
            <span
              className={css({
                fontFamily: 'mono',
                fontSize: '12.5px',
                px: '3',
                py: '1.5',
                rounded: '999px',
                bg: 'accent.soft',
                color: 'fg.accent',
                border: '1px solid token(colors.accent.ring)',
              })}
            >
              chore {spec.version}
            </span>
            <Link
              to="home"
              className={css({
                fontSize: '13.5px',
                color: 'fg.muted',
                transition: 'color .15s',
                _hover: { color: 'fg.default' },
              })}
            >
              ← back to the overview
            </Link>
          </div>
        </div>
      </div>

      {/* Mobile: a scrolling tab strip pinned under the nav. */}
      <div
        className={css({
          display: { base: 'block', lg: 'none' },
          position: 'sticky',
          top: '60px',
          zIndex: '40',
          borderBottom: '1px solid token(colors.border.default)',
          bg: { base: 'rgba(255,255,255,0.86)', _dark: 'rgba(8,9,12,0.86)' },
          backdropFilter: 'saturate(180%) blur(12px)',
        })}
      >
        <div className={cx(container, css({ display: 'flex', gap: '1.5', overflowX: 'auto', py: '2.5' }))}>
          <SideNav active={active} />
        </div>
      </div>

      <div
        className={cx(
          container,
          css({
            display: 'grid',
            gridTemplateColumns: { base: '1fr', lg: '196px 1fr' },
            gap: { base: '0', lg: '12' },
            alignItems: 'start',
            pb: { base: '20', md: '28' },
          }),
        )}
      >
        {/* Desktop: a sticky rail. */}
        <nav
          aria-label="Reference sections"
          className={css({
            display: { base: 'none', lg: 'flex' },
            flexDirection: 'column',
            gap: '1',
            position: 'sticky',
            top: '84px',
            pt: '16',
          })}
        >
          <div className={cx(label, css({ px: '3', pb: '2' }))}>sections</div>
          <SideNav active={active} />
        </nav>

        <div className={css({ minW: '0' })}>
          <Sec
            id="rules"
            title="Rules"
            intro="The parts that behave differently from the shell you are used to. Read these first; everything below assumes them."
          >
            <div className={grid2}>
              {spec.rules.map((r) => (
                <div key={r.name} className={card}>
                  <h3 className={css({ fontFamily: 'mono', fontSize: '13.5px', color: 'fg.accent', mb: '2.5' })}>
                    {r.name}
                  </h3>
                  <Prose text={r.rule} className={css({ fontSize: '14px', color: 'fg.muted' })} />
                </div>
              ))}
            </div>
          </Sec>

          <Sec
            id="builtins"
            title="Builtins"
            intro={
              <>
                Portable commands that resolve before <code className={mono}>PATH</code>, so a chorefile runs on a
                bare container with nothing installed. The pill says whether a command touches the world:{' '}
                <Effects effects />  is echoed and skipped by <code className={mono}>--dry</code>,{' '}
                <Effects effects={false} /> still runs under it.
              </>
            }
          >
            <Builtins />
          </Sec>

          <Sec
            id="variables"
            title="Variables"
            intro="Set by chore before anything runs. Grouped by how long a value lasts."
          >
            <Variables />
          </Sec>

          <Sec id="syntax" title="Syntax" intro="Every form the language has. There are no others.">
            <Syntax />
          </Sec>

          <Sec
            id="conditions"
            title="Conditions"
            intro={
              <>
                What can go between <code className={mono}>if</code> and its block.
              </>
            }
          >
            <Rows rows={spec.conditions.map((c) => ({ term: c.syntax, meaning: c.meaning }))} />
          </Sec>

          <Sec id="chaining" title="Chaining" intro="Joining commands and moving their output around.">
            <Rows rows={spec.chaining.map((c) => ({ term: c.symbol, meaning: c.meaning }))} />
          </Sec>

          <Sec
            id="resolution"
            title="Resolution"
            intro="What a bare word at the start of a line means, in this order."
          >
            <Rows rows={spec.resolution.map((r) => ({ term: r.name, meaning: r.rule }))} />

            <div className={cx(card, css({ mt: '4' }))}>
              <div className={cx(label, css({ mb: '3' }))}>reserved</div>
              <div className={css({ display: 'flex', flexWrap: 'wrap', gap: '2', alignItems: 'center' })}>
                {spec.reserved_tasks.map((t) => (
                  <span
                    key={t}
                    className={css({
                      fontFamily: 'mono',
                      fontSize: '13px',
                      px: '2.5',
                      py: '1.5',
                      rounded: '8px',
                      bg: 'bg.inset',
                      color: 'fg.default',
                      border: '1px solid token(colors.border.default)',
                    })}
                  >
                    {t}
                  </span>
                ))}
                <span className={css({ fontSize: '13.5px', color: 'fg.muted' })}>
                  are subcommands, so no task may take those names — and{' '}
                  <code className={mono}>{spec.namespace_separator}</code> is reserved for include namespaces.
                </span>
              </div>
            </div>
          </Sec>
        </div>
      </div>
    </>
  )
}
