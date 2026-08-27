import { useState } from 'react'
import { css, cx } from 'styled-system/css'
import { Code } from './Code'
import { Eyebrow, Heading, Lede, Section } from './ui'

const CHOREFILE = `VERSION=0.4.2
DIST=$ROOT/dist/$PLATFORM
LLVM=gh://sona-lang/deps/v3/llvm-$PLATFORM.tar.zst

# Fetch the toolchain and build the compiler
task build {
  mkdir $DIST
  if !exists vendor/llvm {
    download $LLVM vendor/llvm.tar.zst --sha256 4f9c2a --retries 3
    extract vendor/llvm.tar.zst vendor/llvm --strip 1
  }
  if $OS == windows && $ENV == gnu { cmake -B build -G "MinGW Makefiles" }
  else                             { cmake -B build }
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

const CLI = `# every task, with the comment above it as its description
$ chore list
build      Fetch the toolchain and build the compiler
test       Run the suite, or one file when given a name
release    Package the release archive

# see the whole run before it touches anything
$ chore release --dry
$ mkdir /Users/ada/sona/dist/macos-arm64
$ download gh://sona-lang/deps/v3/llvm-macos-arm64.tar.zst ...
$ cmake --build build --parallel
$ archive dist/macos-arm64 sona-0.4.2-macos-arm64.tar.gz

# one file, arguments straight through
$ chore test parser

# lint without running anything
$ chore check
chorefile:14  curl is not portable — use the download builtin
chorefile:22  $flag is never assigned — did you mean $flags?

# the full reference as JSON, for agents and editors
$ chore spec | jq '.builtins | keys'`

const TABS = [
  { id: 'chorefile', label: 'chorefile', source: CHOREFILE },
  { id: 'cli', label: 'the CLI', source: CLI },
] as const

export function Showcase() {
  const [tab, setTab] = useState<(typeof TABS)[number]['id']>('chorefile')
  const active = TABS.find((t) => t.id === tab)!

  return (
    <div className={css({ bg: 'bg.subtle', borderY: '1px solid token(colors.border.default)' })}>
      <Section id="example">
        <div
          className={css({
            display: 'grid',
            gridTemplateColumns: { base: '1fr', lg: '0.85fr 1.15fr' },
            gap: { base: '12', lg: '16' },
            alignItems: 'start',
          })}
        >
          <div className={css({ position: { lg: 'sticky' }, top: '28' })}>
            <Eyebrow>the whole language</Eyebrow>
            <Heading>Short enough to read in a minute. Complete enough to ship a compiler.</Heading>
            <Lede>
              Assignments, interpolation, <code className={css({ fontFamily: 'mono' })}>if</code> /{' '}
              <code className={css({ fontFamily: 'mono' })}>for</code>, captures, pipes, redirects and parameterised
              tasks. That is the entire surface — there is nothing else to learn, and nothing else to get wrong.
            </Lede>
            <p className={css({ mt: '6', fontSize: '14.5px', lineHeight: '1.65', color: 'fg.faint' })}>
              Each task runs once per invocation, keyed on its name <em>and</em> its arguments — so{' '}
              <code className={css({ fontFamily: 'mono' })}>release</code> calling both{' '}
              <code className={css({ fontFamily: 'mono' })}>build</code> and{' '}
              <code className={css({ fontFamily: 'mono' })}>test</code> builds exactly once.
            </p>
          </div>

          <div
            className={css({
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
            <Code source={active.source} />
          </div>
        </div>
      </Section>
    </div>
  )
}
