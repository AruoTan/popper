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
    ]
  }
})
