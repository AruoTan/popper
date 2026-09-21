import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vitest/config'

export default defineConfig({
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./vitest.setup.ts'],
    clearMocks: true,
    restoreMocks: true,
    include: [
      'src/renderer/**/*.test.{ts,tsx}',
      'apps/windows/renderer/**/*.test.{ts,tsx}',
      'src/shared/**/*.test.{ts,tsx}'
    ],
    exclude: []
  },
  resolve: {
    alias: {
      '@shared': fileURLToPath(new URL('./src/shared', import.meta.url)),
      '@renderer': fileURLToPath(new URL('./src/renderer', import.meta.url))
    }
  }
})
