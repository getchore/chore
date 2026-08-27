import { copyFileSync, existsSync, statSync } from 'node:fs'
import { fileURLToPath, URL } from 'node:url'
import { defineConfig, type Plugin } from 'vite'
import react from '@vitejs/plugin-react'

const here = (p: string) => fileURLToPath(new URL(p, import.meta.url))

const INSTALLERS = ['install.sh', 'install.ps1']

/**
 * The install one-liners point at this site, so the scripts have to ship with
 * it. They stay in installers/ — CI lints them there — and are copied into the
 * build. A missing or empty script would deploy a site whose headline command
 * 404s, so this fails the build instead of shipping quietly.
 */
function installers(): Plugin {
  return {
    name: 'installers',
    apply: 'build',
    buildStart() {
      for (const name of INSTALLERS) {
        const src = here(`../installers/${name}`)
        if (!existsSync(src)) {
          this.error(`installers/${name} is missing — the site publishes it at /${name}`)
        }
        if (statSync(src).size === 0) {
          this.error(`installers/${name} is empty — refusing to publish an empty installer`)
        }
      }
    },
    closeBundle() {
      for (const name of INSTALLERS) {
        copyFileSync(here(`../installers/${name}`), here(`./dist/${name}`))
        const out = here(`./dist/${name}`)
        if (!existsSync(out) || statSync(out).size === 0) {
          throw new Error(`dist/${name} did not land — the install command would 404`)
        }
        console.log(`  installers  dist/${name}  ${(statSync(out).size / 1024).toFixed(1)} kB`)
      }
    },
  }
}

// `base` is overridden in CI for GitHub project pages: VITE_BASE=/chore/
export default defineConfig({
  base: process.env.VITE_BASE ?? '/',
  plugins: [react(), installers()],
  resolve: {
    alias: {
      'styled-system': here('./styled-system'),
    },
  },
})
