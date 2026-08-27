import { useState } from 'react'
import { css, cx } from 'styled-system/css'
import { Code } from './Code'
import { Band, Eyebrow, Heading } from './ui'

const CHOREFILE = `VERSION=0.4.2
DIST=$ROOT/dist/$PLATFORM
LLVM=gh://sona-lang/deps/v3/llvm-$PLATFORM.tar.zst

# Fetch the toolchain and build the compiler
task build {
  mkdir $DIST
  if !exists vendor/llvm {
    download $LLVM vendor/llvm.tar.zst --sha256 4f9c2a
    extract vendor/llvm.tar.zst vendor/llvm --strip 1
  }
  if $OS == windows { cmake -B build -G "MinGW Makefiles" }
  else              { cmake -B build }
  cmake --build build --parallel
  copy build/sona$EXE $DIST/sona$EXE
}

# Run the suite, or one file when given a name
task test name {
  if $# == 0 { for f in $(find tests *.sona) { ./sona$EXE $f } }
  else       { ./sona$EXE tests/$1.sona }
}

# Package the release archive
task release {
  build
  test
  archive $DIST sona-$VERSION-$PLATFORM.tar.gz
  sha256 sona-$VERSION-$PLATFORM.tar.gz > checksums.txt
}`

const CLI = `$ chore list
build      Fetch the toolchain and build the compiler
test       Run the suite, or one file when given a name
release    Package the release archive

# see the whole run before it touches anything
$ chore release --dry
$ mkdir /Users/ada/sona/dist/macos-arm64
$ download gh://sona-lang/deps/v3/llvm-macos-arm64.tar.zst
$ cmake --build build --parallel
$ archive dist/macos-arm64 sona-0.4.2-macos-arm64.tar.gz

# arguments go straight through
$ chore test parser

# lint without running anything
$ chore check
chorefile:14  curl is not portable — use download
chorefile:22  $flag is never assigned — did you mean $flags?

# the whole language as JSON, for agents
$ chore spec | jq '.builtins | keys'`

const TABS = [
  { id: 'chorefile', label: 'chorefile', source: CHOREFILE },
  { id: 'cli', label: 'the CLI', source: CLI },
] as const

export function Showcase() {
  const [tab, setTab] = useState<(typeof TABS)[number]['id']>('chorefile')
  const [open, setOpen] = useState(false)
  const active = TABS.find((t) => t.id === tab)!

  return (
    <Band id="example" ambient="dots">
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
            <Eyebrow>the whole language</Eyebrow>
            <Heading>Learn it in a minute. Ship a compiler with it.</Heading>
          </div>
          <p className={css({ fontSize: '14.5px', color: 'fg.faint', maxW: '280px', lineHeight: '1.55' })}>
            Assignments, <code className={css({ fontFamily: 'mono' })}>if</code> /{' '}
            <code className={css({ fontFamily: 'mono' })}>for</code>, captures, pipes, tasks with arguments. That is
            the entire surface.
          </p>
        </div>

        <div
          className={css({
            mt: '10',
            bg: 'bg.panel',
            border: '1px solid token(colors.border.default)',
            rounded: '16px',
            overflow: 'hidden',
            boxShadow: { base: '0 20px 50px -35px rgba(0,0,0,.4)', _dark: '0 20px 60px -30px rgba(0,0,0,.8)' },
          })}
        >
          <div
            className={css({
              display: 'flex',
              gap: '1',
              px: '2',
              pt: '2',
              bg: 'bg.subtle',
              borderBottom: '1px solid token(colors.border.default)',
            })}
          >
            {TABS.map((t) => (
              <button
                key={t.id}
                type="button"
                onClick={() => setTab(t.id)}
                className={cx(
                  css({
                    px: '3.5',
                    py: '2',
                    fontFamily: 'mono',
                    fontSize: '12px',
                    cursor: 'pointer',
                    bg: 'transparent',
                    color: 'fg.faint',
                    borderBottom: '2px solid transparent',
                    mb: '-1px',
                    _hover: { color: 'fg.default' },
                  }),
                  tab === t.id && css({ color: 'fg.default', borderBottomColor: 'accent.solid' }),
                )}
              >
                {t.label}
              </button>
            ))}
          </div>

          <div className={css({ position: 'relative' })}>
            <div
              className={css({ overflow: 'hidden', transition: 'max-height .35s ease' })}
              style={{ maxHeight: open ? '1400px' : '340px' }}
            >
              <Code source={active.source} />
            </div>

            {!open && (
              <div
                aria-hidden
                className={css({
                  position: 'absolute',
                  insetInline: '0',
                  bottom: '0',
                  h: '130px',
                  pointerEvents: 'none',
                  backgroundImage: 'linear-gradient(to bottom, transparent, token(colors.bg.panel))',
                })}
              />
            )}

            <div
              className={css({
                position: open ? 'static' : 'absolute',
                insetInline: '0',
                bottom: '0',
                display: 'flex',
                justifyContent: 'center',
                pb: '5',
                pt: open ? '0' : '0',
              })}
            >
              <button
                type="button"
                onClick={() => setOpen((o) => !o)}
                className={css({
                  display: 'inline-flex',
                  alignItems: 'center',
                  gap: '2',
                  px: '4',
                  py: '2',
                  rounded: 'full',
                  cursor: 'pointer',
                  fontSize: '13px',
                  fontWeight: '500',
                  border: '1px solid token(colors.border.default)',
                  bg: 'bg.panel',
                  color: 'fg.muted',
                  transition: 'all .15s',
                  _hover: { color: 'fg.default', borderColor: 'border.strong' },
                })}
              >
                {open ? 'Collapse' : 'Show the whole file'}
                <span
                  className={css({ fontSize: '10px', transition: 'transform .25s' })}
                  style={{ transform: open ? 'rotate(180deg)' : 'none' }}
                >
                  ▼
                </span>
              </button>
            </div>
          </div>
        </div>
    </Band>
  )
}
