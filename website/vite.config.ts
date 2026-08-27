import { copyFileSync, mkdirSync } from 'node:fs'
import { fileURLToPath, URL } from 'node:url'
import { defineConfig, type Plugin } from 'vite'
import react from '@vitejs/plugin-react'

const here = (p: string) => fileURLToPath(new URL(p, import.meta.url))

/**
 * The install one-liners are served from the site, not from raw.githubusercontent,
 * so the URL people paste is short and stable. The scripts stay in installers/ —
 * CI lints them there — and are copied into the build.
 */
function copyInstallers(): Plugin {
  return {
    name: 'copy-installers',
    apply: 'build',
    closeBundle() {
      mkdirSync(here('./dist'), { recursive: true })
      for (const name of ['install.sh', 'install.ps1']) {
        copyFileSync(here(`../installers/${name}`), here(`./dist/${name}`))
      }
    },
  }
}

// `base` is overridden in CI for GitHub project pages: VITE_BASE=/chore/
export default defineConfig({
  base: process.env.VITE_BASE ?? '/',
  plugins: [react(), copyInstallers()],
  resolve: {
    alias: {
      'styled-system': here('./styled-system'),
    },
  },
})
