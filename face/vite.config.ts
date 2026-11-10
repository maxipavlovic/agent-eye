import { defineConfig } from 'vitest/config';
import preact from '@preact/preset-vite';

// The real app, served by `eye serve`. Built files go to dist/.
export default defineConfig({
  plugins: [preact()],
  build: { outDir: 'dist', emptyOutDir: true },
  test: {
    environment: 'node',
    include: ['src/**/*.test.ts'],
    // npm run coverage: coverage/lcov.info for SonarQube, a short summary on the terminal.
    coverage: {
      provider: 'v8',
      include: ['src/**/*.{ts,tsx}'],
      exclude: ['src/**/*.test.ts', 'src/**/main.tsx', 'src/api/types.ts'],
      reporter: ['text-summary', 'lcov'],
      reportsDirectory: 'coverage',
    },
  },
});
