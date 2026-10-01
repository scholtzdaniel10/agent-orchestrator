import { defineConfig } from 'vitest/config'

export default defineConfig({
  test: {
    include: ['src/**/*.gate.ts'],
    testTimeout: 900_000,
    hookTimeout: 120_000,
    environment: 'node',
    // The printed table is the evidence; the quiet reporter Vitest picks under an AI agent hides it.
    reporters: ['default']
  }
})
