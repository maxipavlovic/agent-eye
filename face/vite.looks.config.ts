import { resolve } from 'node:path';
import { defineConfig } from 'vite';
import preact from '@preact/preset-vite';
import { viteSingleFile } from 'vite-plugin-singlefile';

// Look prototype: npm run look → dist-looks/ink/index.html,
// one self-contained file (fixture data, fonts and sprites inlined) that opens from file://.
const look = 'ink';

export default defineConfig({
  root: resolve(import.meta.dirname, 'looks', look),
  plugins: [preact(), viteSingleFile()],
  build: {
    outDir: resolve(import.meta.dirname, 'dist-looks', look),
    emptyOutDir: true,
    assetsInlineLimit: 100_000_000,
  },
});
