import { resolve } from 'path'
import { defineConfig } from 'electron-vite'
import { build, type Plugin } from 'vite'
import react from '@vitejs/plugin-react'

// Ink is ESM-only. The Electron main bundle is CommonJS, so the terminal UI is a
// sibling ESM file that cli.js loads with a dynamic import when the UI opens.
function buildTui(): Plugin {
  return {
    name: 'build-tui-esm',
    apply: 'build',
    async closeBundle(): Promise<void> {
      await build({
        configFile: false,
        logLevel: 'warn',
        esbuild: { jsx: 'automatic' },
        build: {
          ssr: true,
          outDir: resolve('out/main'),
          emptyOutDir: false,
          minify: false,
          rollupOptions: {
            input: resolve('src/cli/tui/launch.tsx'),
            external: [
              'ink',
              'ink-text-input',
              'react',
              'react/jsx-runtime',
              'react/jsx-dev-runtime',
              'react-dom'
            ],
            output: {
              format: 'es',
              entryFileNames: 'tui.mjs',
              inlineDynamicImports: true
            }
          }
        }
      })
    }
  }
}

export default defineConfig({
  main: {
    plugins: [buildTui()],
    build: {
      rollupOptions: {
        input: {
          index: resolve('src/main/index.ts'),
          cli: resolve('src/cli/index.ts')
        },
        output: {
          entryFileNames: '[name].js'
        }
      }
    }
  },
  preload: {},
  renderer: {
    resolve: {
      alias: {
        '@renderer': resolve('src/renderer/src')
      }
    },
    plugins: [react()]
  }
})
