import fs from 'node:fs';
import path from 'node:path';
import type { Plugin } from 'vite';
import { loadDocs } from './docs.ts';
import { SITE_URL } from './links.ts';
import { docPage, docsIndex, notFound, page } from './pages.ts';
import { renderSprites } from './sprites.ts';

interface Options {
  /** The repo's docs/ folder; every .md in it becomes a page. */
  docsDir: string;
  /** The board's offline build (`npm run look` in face/), served at demo/. */
  demoFile: string;
  /** The landing page's body. */
  landingFile: string;
}

const LANDING = {
  title: 'agent-eye: every coding-agent session on your machine, found, sorted, resumable',
  description: 'A local board and terminal picker for your coding-agent sessions: what is running, what needs you, and one key to resume. Works with Claude Code and Codex CLI. Open source, local only.',
};

/** On its own page the demo board gets a way back to the site; inside the site's frame it does not. */
const BACK_LINK = `<a href="../" id="site-back" style="position:fixed;left:12px;bottom:12px;z-index:90;font:400 14px/1 var(--f-label);background:var(--ink);color:var(--paper);padding:10px 12px 9px;text-decoration:none;box-shadow:3px 3px 0 var(--accent)">&larr; BACK TO THE SITE</a><script>if(self!==top)document.getElementById('site-back').remove()</script>`;

export function withBackLink(html: string): string {
  const at = html.lastIndexOf('</body>');
  return at < 0 ? html + BACK_LINK : html.slice(0, at) + BACK_LINK + html.slice(at);
}

/**
 * The whole site as one plugin: the landing page and the docs pages are generated (from landing.html
 * and docs/*.md) and handed to Vite as HTML entries, so CSS, fonts and scripts are bundled and hashed
 * the usual way. The demo board is copied in as demo/index.html.
 */
export function eyeSite(opts: Options): Plugin {
  let root = '';
  let base = '/';
  const pages = new Map<string, () => string>();

  const build = (): Map<string, () => string> => {
    const docs = loadDocs(opts.docsDir);
    const map = new Map<string, () => string>();
    map.set(path.join(root, 'index.html'), () => page({ ...LANDING, path: '', kind: 'landing', body: fs.readFileSync(opts.landingFile, 'utf8') }));
    map.set(path.join(root, '404.html'), notFound);
    map.set(path.join(root, 'docs/index.html'), () => docsIndex(docs));
    for (const d of docs) map.set(path.join(root, `docs/${d.slug}/index.html`), () => docPage(docs, d));
    return map;
  };

  const fill = (html: string) => renderSprites(html.replace(/\{\{base\}\}/g, base));
  const demo = () => {
    if (!fs.existsSync(opts.demoFile)) throw new Error(`The demo board is missing: run "npm run demo" first (${opts.demoFile}).`);
    return withBackLink(fs.readFileSync(opts.demoFile, 'utf8'));
  };

  return {
    name: 'agent-eye-site',

    config(config) {
      root = path.resolve(config.root ?? process.cwd());
      pages.clear();
      for (const [id, render] of build()) pages.set(id, render);
      return {
        build: { rollupOptions: { input: Object.fromEntries([...pages.keys()].map((id) => [path.relative(root, id).replace(/\.html$/, '') || 'index', id])) } },
      };
    },

    configResolved(config) {
      base = config.base;
    },

    resolveId(id) {
      return pages.has(id) ? id : null;
    },

    load(id) {
      return pages.get(id)?.() ?? null;
    },

    transformIndexHtml(html) {
      return fill(html);
    },

    generateBundle() {
      this.emitFile({ type: 'asset', fileName: 'demo/index.html', source: demo() });
      this.emitFile({ type: 'asset', fileName: 'robots.txt', source: `User-agent: *\nAllow: /\nSitemap: ${SITE_URL}sitemap.xml\n` });
      const urls = [...pages.keys()]
        .map((id) => path.relative(root, id).replace(/index\.html$/, ''))
        .filter((p) => !p.startsWith('404'))
        .map((p) => `  <url><loc>${SITE_URL}${p}</loc></url>`);
      this.emitFile({ type: 'asset', fileName: 'sitemap.xml', source: `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${urls.join('\n')}\n</urlset>\n` });
    },

    // Dev: serve the generated pages (docs are read again on every request) and the demo.
    configureServer(server) {
      server.middlewares.use(async (req, res, next) => {
        const url = (req.url ?? '').split('?')[0]!;
        if (!url.startsWith(base)) return next();
        const rel = url.slice(base.length);
        if (rel === 'demo/' || rel === 'demo/index.html') {
          res.setHeader('content-type', 'text/html; charset=utf-8');
          res.end(demo());
          return;
        }
        const id = path.join(root, rel.endsWith('/') || rel === '' ? `${rel}index.html` : rel);
        const render = build().get(id);
        if (!render) return next();
        res.setHeader('content-type', 'text/html; charset=utf-8');
        res.end(await server.transformIndexHtml(url, render()));
      });
    },
  };
}
