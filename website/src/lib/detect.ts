export type OsKey = 'macos' | 'linux' | 'windows'

export const OS_LABEL: Record<OsKey, string> = {
  macos: 'macOS',
  linux: 'Linux',
  windows: 'Windows',
}

const REPO = 'getchore/chore'
const SITE = 'https://getchore.github.io/chore'

/**
 * The scripts live in installers/ and are copied into the build (see
 * vite.config.ts), so these URLs are served by this very site. They fetch
 * release assets from github.com/getchore/chore/releases/latest, which is
 * not published yet.
 */
export const INSTALL: Record<OsKey, { shell: string; command: string }> = {
  macos: {
    shell: 'bash · zsh',
    command: `curl -fsSL ${SITE}/install.sh | sh`,
  },
  linux: {
    shell: 'bash · sh',
    command: `curl -fsSL ${SITE}/install.sh | sh`,
  },
  windows: {
    shell: 'PowerShell',
    command: `irm ${SITE}/install.ps1 | iex`,
  },
}

export function detectOs(): OsKey {
  if (typeof navigator === 'undefined') return 'macos'
  const ua = `${navigator.userAgent} ${navigator.platform ?? ''}`.toLowerCase()
  if (ua.includes('win')) return 'windows'
  if (ua.includes('mac') || ua.includes('iphone') || ua.includes('ipad')) return 'macos'
  return 'linux'
}

export const GITHUB_URL = `https://github.com/${REPO}`
export const RELEASES_URL = `${GITHUB_URL}/releases/latest`
