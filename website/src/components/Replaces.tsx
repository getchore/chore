import { useState } from 'react'
import { css, cx } from 'styled-system/css'
import { Code, type Lang } from './Code'
import { Band, Eyebrow, Heading } from './ui'

type Case = {
  id: string
  label: string
  lang: Lang
  afterLang: Lang
  beforeFile: string
  afterFile: string
  before: string
  after: string
  note: string
}

const CASES: Case[] = [
  {
    id: 'actions',
    label: 'GitHub Actions',
    lang: 'yaml',
    afterLang: 'yaml',
    beforeFile: '.github/workflows/build.yml',
    afterFile: '.github/workflows/build.yml',
    note: 'One matrix leg per OS, twice the steps, two shells.',
    before: `- name: Fetch toolchain (Unix)
  if: runner.os != 'Windows'
  run: |
    curl -fsSL "$LLVM_URL" -o llvm.tar.zst
    echo "$LLVM_SHA  llvm.tar.zst" | shasum -a 256 -c
    mkdir -p vendor/llvm
    tar --zstd -xf llvm.tar.zst -C vendor/llvm --strip-components 1

- name: Fetch toolchain (Windows)
  if: runner.os == 'Windows'
  shell: pwsh
  run: |
    Invoke-WebRequest $env:LLVM_URL -OutFile llvm.zip
    $h = (Get-FileHash llvm.zip -Algorithm SHA256).Hash
    if ($h -ne $env:LLVM_SHA) { exit 1 }
    Expand-Archive llvm.zip -DestinationPath vendor/llvm

- name: Build
  run: cmake -B build && cmake --build build --parallel`,
    after: `- uses: getchore/setup-chore@v1
- run: chore build`,
  },
  {
    id: 'make',
    label: 'Makefile',
    lang: 'make',
    afterLang: 'chore',
    beforeFile: 'Makefile',
    afterFile: 'chorefile',
    note: 'Detect the platform by hand, then hope the shell agrees.',
    before: `UNAME := $(shell uname -s)
ifeq ($(UNAME),Darwin)
  PLATFORM := macos-$(shell uname -m)
  EXE :=
else ifeq ($(OS),Windows_NT)
  PLATFORM := windows-x86_64
  EXE := .exe
else
  PLATFORM := linux-$(shell uname -m)
  EXE :=
endif

build:
	mkdir -p dist/$(PLATFORM)
	curl -fsSL $(LLVM_URL) -o llvm.tar.zst
	tar --zstd -xf llvm.tar.zst -C vendor/llvm --strip-components 1
	cmake -B build && cmake --build build --parallel
	cp build/sona$(EXE) dist/$(PLATFORM)/`,
    after: `task build {
  mkdir dist/$PLATFORM
  download $LLVM vendor/llvm.tar.zst --sha256 4f9c2a
  extract vendor/llvm.tar.zst vendor/llvm --strip 1
  cmake -B build
  cmake --build build --parallel
  copy build/sona$EXE dist/$PLATFORM/sona$EXE
}`,
  },
  {
    id: 'cmake',
    label: 'CMake',
    lang: 'cmake',
    afterLang: 'chore',
    beforeFile: 'CMakeLists.txt',
    afterFile: 'chorefile',
    note: 'cmake -E is a portable shell in disguise. chore keeps cmake for the build and takes the chores back.',
    before: `file(DOWNLOAD "\${LLVM_URL}" "\${CMAKE_BINARY_DIR}/llvm.tar.zst"
     EXPECTED_HASH SHA256=4f9c2a
     STATUS dl)
list(GET dl 0 dl_code)
if(NOT dl_code EQUAL 0)
  message(FATAL_ERROR "download failed: \${dl}")
endif()

file(ARCHIVE_EXTRACT INPUT "\${CMAKE_BINARY_DIR}/llvm.tar.zst"
     DESTINATION "\${CMAKE_SOURCE_DIR}/vendor/llvm")

if(WIN32)
  set(EXE ".exe")
else()
  set(EXE "")
endif()

add_custom_target(dist
  COMMAND \${CMAKE_COMMAND} -E make_directory \${DIST}
  COMMAND \${CMAKE_COMMAND} -E copy $<TARGET_FILE:sona> \${DIST}/sona\${EXE}
  COMMAND \${CMAKE_COMMAND} -E tar czf sona-\${PLATFORM}.tar.gz \${DIST})`,
    after: `task dist {
  download $LLVM vendor/llvm.tar.zst --sha256 4f9c2a
  extract vendor/llvm.tar.zst vendor/llvm --strip 1
  cmake --build build --parallel
  mkdir dist/$PLATFORM
  copy build/sona$EXE dist/$PLATFORM/sona$EXE
  archive dist/$PLATFORM sona-$PLATFORM.tar.gz
}`,
  },
]

function lines(s: string) {
  return s.split('\n').length
}

function Panel({
  kind,
  file,
  source,
  lang,
}: {
  kind: 'before' | 'after'
  file: string
  source: string
  lang: Lang
}) {
  const after = kind === 'after'
  return (
    <div
      className={css({
        display: 'flex',
        flexDir: 'column',
        minW: '0',
        // fixed so switching tabs never changes the page height
        h: { base: '400px', lg: '500px' },
        bg: 'bg.panel',
        border: '1px solid',
        borderColor: after ? 'accent.ring' : 'border.default',
        rounded: '14px',
        overflow: 'hidden',
      })}
    >
      <div
        className={css({
          display: 'flex',
          alignItems: 'center',
          gap: '3',
          px: '4',
          py: '2.5',
          borderBottom: '1px solid token(colors.border.default)',
          bg: 'bg.subtle',
        })}
      >
        <span
          className={cx(
            css({
              fontFamily: 'mono',
              fontSize: '10.5px',
              letterSpacing: '0.1em',
              textTransform: 'uppercase',
              fontWeight: '600',
              px: '2',
              py: '1',
              rounded: '5px',
            }),
            after
              ? css({ bg: 'accent.soft', color: 'fg.accent' })
              : css({ bg: 'bg.inset', color: 'fg.faint' }),
          )}
        >
          {kind}
        </span>
        <span className={css({ fontFamily: 'mono', fontSize: '11.5px', color: 'fg.faint' })}>{file}</span>
        <span className={css({ ml: 'auto', fontFamily: 'mono', fontSize: '11.5px', color: after ? 'fg.accent' : 'fg.faint' })}>
          {lines(source)} lines
        </span>
      </div>
      <div className={css({ flex: '1', minH: '0', overflow: 'auto' })}>
        <Code source={source} lang={lang} size="sm" />
      </div>
    </div>
  )
}

export function Replaces() {
  const [id, setId] = useState(CASES[0].id)
  const c = CASES.find((x) => x.id === id)!

  return (
    <Band id="replaces" ambient="rail" surface>
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
            <Eyebrow>before / after</Eyebrow>
            <Heading>Delete the platform branches.</Heading>
          </div>
          <div className={css({ display: 'flex', gap: '1.5', flexWrap: 'wrap' })}>
            {CASES.map((x) => (
              <button
                key={x.id}
                type="button"
                onClick={() => setId(x.id)}
                className={cx(
                  css({
                    px: '3.5',
                    py: '2',
                    rounded: '9px',
                    fontSize: '13px',
                    fontWeight: '500',
                    cursor: 'pointer',
                    border: '1px solid token(colors.border.default)',
                    bg: 'bg.panel',
                    color: 'fg.muted',
                    transition: 'all .15s',
                    _hover: { color: 'fg.default', borderColor: 'border.strong' },
                  }),
                  id === x.id &&
                    css({ bg: 'accent.soft', borderColor: 'accent.ring', color: 'fg.accent' }),
                )}
              >
                {x.label}
              </button>
            ))}
          </div>
        </div>

        <div
          className={css({
            mt: '10',
            display: 'grid',
            gridTemplateColumns: { base: '1fr', lg: '1fr auto 1fr' },
            alignItems: 'stretch',
            gap: { base: '4', lg: '5' },
          })}
        >
          <Panel kind="before" file={c.beforeFile} source={c.before} lang={c.lang} />

          <div
            className={css({
              display: 'flex',
              flexDir: { base: 'row', lg: 'column' },
              alignItems: 'center',
              justifyContent: 'center',
              gap: '3',
            })}
          >
            <span
              className={css({
                display: 'grid',
                placeItems: 'center',
                w: '34px',
                h: '34px',
                rounded: 'full',
                bg: 'accent.solid',
                color: 'white',
                fontSize: '15px',
                fontWeight: '700',
                flexShrink: '0',
              })}
            >
              →
            </span>
            <span
              className={css({
                fontFamily: 'mono',
                fontSize: '12px',
                fontWeight: '600',
                color: 'fg.accent',
                whiteSpace: 'nowrap',
              })}
            >
              −{lines(c.before) - lines(c.after)} lines
            </span>
          </div>

          <Panel kind="after" file={c.afterFile} source={c.after} lang={c.afterLang} />
        </div>

        <p className={css({ mt: '6', fontSize: '14px', color: 'fg.faint' })}>{c.note}</p>
    </Band>
  )
}
