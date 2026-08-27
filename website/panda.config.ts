import { defineConfig } from '@pandacss/dev'

export default defineConfig({
  preflight: true,
  include: ['./src/**/*.{ts,tsx}'],
  exclude: [],
  jsxFramework: 'react',
  conditions: {
    extend: {
      light: '[data-theme=light] &, .light &',
      dark: '[data-theme=dark] &, .dark &',
    },
  },
  outdir: 'styled-system',
  theme: {
    extend: {
      tokens: {
        fonts: {
          sans: { value: '"Inter Variable", Inter, -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif' },
          mono: { value: '"JetBrains Mono", ui-monospace, SFMono-Regular, "SF Mono", Menlo, monospace' },
          hand: { value: 'Caveat, "Bradley Hand", "Segoe Script", cursive' },
        },
        colors: {
          ember: {
            50: { value: '#fff7ed' },
            200: { value: '#fed7aa' },
            300: { value: '#fdba74' },
            400: { value: '#fb923c' },
            500: { value: '#f97316' },
            600: { value: '#ea580c' },
            700: { value: '#c2410c' },
          },
        },
      },
      semanticTokens: {
        colors: {
          bg: {
            canvas: { value: { base: '#ffffff', _dark: '#08090c' } },
            subtle: { value: { base: '#fafafa', _dark: '#0d0f14' } },
            panel: { value: { base: '#ffffff', _dark: '#101319' } },
            inset: { value: { base: '#f4f4f5', _dark: '#0a0c11' } },
          },
          fg: {
            default: { value: { base: '#18181b', _dark: '#f4f4f5' } },
            muted: { value: { base: '#52525b', _dark: '#a1a1aa' } },
            faint: { value: { base: '#8b8b93', _dark: '#6b7280' } },
            accent: { value: { base: '#c2410c', _dark: '#fb923c' } },
          },
          border: {
            default: { value: { base: '#e4e4e7', _dark: '#1e222b' } },
            strong: { value: { base: '#d4d4d8', _dark: '#2a2f3a' } },
          },
          accent: {
            solid: { value: { base: '#ea580c', _dark: '#f97316' } },
            soft: { value: { base: 'rgba(234,88,12,0.10)', _dark: 'rgba(249,115,22,0.14)' } },
            ring: { value: { base: 'rgba(234,88,12,0.35)', _dark: 'rgba(249,115,22,0.40)' } },
          },
        },
      },
      keyframes: {
        fadeUp: {
          from: { opacity: '0', transform: 'translateY(14px)' },
          to: { opacity: '1', transform: 'translateY(0)' },
        },
        blink: { '0%,49%': { opacity: '1' }, '50%,100%': { opacity: '0' } },
        drift: {
          '0%,100%': { transform: 'translate3d(0,0,0) scale(1)' },
          '50%': { transform: 'translate3d(0,-24px,0) scale(1.08)' },
        },
      },
    },
  },
  globalCss: {
    'html, body': {
      bg: 'bg.canvas',
      color: 'fg.default',
      fontFamily: 'sans',
      fontFeatureSettings: '"cv02","cv03","cv04","cv11"',
      WebkitFontSmoothing: 'antialiased',
      scrollBehavior: 'smooth',
    },
    '::selection': { bg: 'accent.soft', color: 'fg.accent' },
    '*': { minWidth: '0' },
  },
})
