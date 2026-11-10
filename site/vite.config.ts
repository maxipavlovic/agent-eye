import { resolve } from 'node:path';
import { defineConfig } from 'vite';
import { eyeSite } from './plugin/site.ts';

const repo = resolve(import.meta.dirname, '..');

// The project site, served at https://maxipavlovic.github.io/agent-eye/ . Output: dist/.
export default defineConfig({
  base: '/agent-eye/',
  plugins: [eyeSite({
    docsDir: resolve(repo, 'docs'),
    demoFile: resolve(repo, 'face/dist-looks/ink/index.html'),
    landingFile: resolve(import.meta.dirname, 'pages/landing.html'),
  })],
  build: { outDir: 'dist', emptyOutDir: true },
  server: { fs: { allow: [repo] } },
});
