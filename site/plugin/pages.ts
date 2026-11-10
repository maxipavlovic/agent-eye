import type { DocPage } from './docs.ts';
import { REPO, SITE_URL } from './links.ts';

const esc = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

/** Sets the theme before the first paint: the saved choice (shared with the demo board), else daylight. */
const THEME_SCRIPT = `try{var t=localStorage.getItem('eye.theme');if(t==='auto')t=matchMedia('(prefers-color-scheme: dark)').matches?'dark':'light';else if(t!=='dark')t='light';document.documentElement.dataset.theme=t}catch(e){document.documentElement.dataset.theme='light'}`;

export interface PageOptions {
  title: string;
  description: string;
  /** Path under the site root, e.g. `docs/terminal/`; empty for the landing page. */
  path: string;
  body: string;
  /** `landing`, `docs` or `plain`: a class on <body>. */
  kind?: 'landing' | 'docs' | 'plain';
}

/** A whole page: head, header, the body, footer. {{tokens}} are filled in later. */
export function page({ title, description, path, body, kind = 'plain' }: PageOptions): string {
  const url = SITE_URL + path;
  return `<!doctype html>
<html lang="en" data-theme="light">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${esc(title)}</title>
<meta name="description" content="${esc(description)}">
<meta name="theme-color" content="#f2efe6" media="(prefers-color-scheme: light)">
<meta name="theme-color" content="#1e1d1a" media="(prefers-color-scheme: dark)">
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<link rel="canonical" href="${url}">
<meta property="og:type" content="website">
<meta property="og:title" content="${esc(title)}">
<meta property="og:description" content="${esc(description)}">
<meta property="og:url" content="${url}">
<meta property="og:image" content="${SITE_URL}og.png">
<meta name="twitter:card" content="summary_large_image">
<script>${THEME_SCRIPT}</script>
<script type="module" src="/src/site.ts"></script>
</head>
<body class="is-${kind}">
<a class="skip" href="#main">Skip to content</a>
${header()}
<main id="main">
${body}
</main>
${footer()}
</body>
</html>
`;
}

function header(): string {
  const theme = (name: string, label: string) =>
    `<button type="button" data-theme-set="${name}" aria-pressed="false" title="${label}" aria-label="${label} theme">{{sprite:${name}:2:cc}}</button>`;
  return `<header class="site-hd">
  <div class="wrap hd-in">
    <a class="brand" href="{{base}}" aria-label="agent-eye, home">{{eye}}<span class="word">AGENT-EYE</span></a>
    <nav class="nav" aria-label="Main">
      <a href="{{base}}#demo">Demo</a>
      <a href="{{base}}#install">Install</a>
      <a href="{{base}}docs/">Docs</a>
      <a href="${REPO}">GitHub</a>
    </nav>
    <div class="theme" role="group" aria-label="Theme">${theme('sun', 'Light')}${theme('moon', 'Dark')}${theme('auto', 'System')}</div>
  </div>
  <div class="strip" aria-hidden="true"></div>
</header>`;
}

function footer(): string {
  return `<footer class="site-ft">
  <div class="strip" aria-hidden="true"></div>
  <div class="wrap ft-in">
    <p class="ft-brand"><span class="word">AGENT-EYE</span> MIT licence</p>
    <nav class="ft-links" aria-label="Project">
      <a href="${REPO}">GitHub</a>
      <a href="${REPO}/releases">Releases</a>
      <a href="${REPO}/issues">Issues</a>
      <a href="{{base}}docs/">Docs</a>
    </nav>
    <p class="fine">agent-eye is an independent open-source project. It is not affiliated with, endorsed by or sponsored by Anthropic. Claude and Claude Code are trademarks of Anthropic.</p>
  </div>
</footer>`;
}

function docNav(pages: DocPage[], current: string): string {
  const link = (href: string, text: string, on: boolean) => `<li><a href="${href}"${on ? ' aria-current="page"' : ''}>${esc(text)}</a></li>`;
  return `<nav class="docnav" aria-label="Documentation">
  <details class="docmenu" open>
    <summary>Docs</summary>
    <ul>${link('{{base}}docs/', 'Overview', current === '')}${pages.map((p) => link(`{{base}}docs/${p.slug}/`, p.title, current === p.slug)).join('')}</ul>
  </details>
</nav>`;
}

export function docsIndex(pages: DocPage[]): string {
  const cards = pages.map((p) => `<li><a class="doccard" href="{{base}}docs/${p.slug}/"><h2>${esc(p.title)}</h2><p>${esc(p.summary)}</p></a></li>`).join('');
  return page({
    title: 'Documentation · agent-eye',
    description: 'How to use agent-eye: the terminal picker, the board, the API and the configuration.',
    path: 'docs/',
    kind: 'docs',
    body: `<div class="wrap docs">
  ${docNav(pages, '')}
  <article class="doc">
    <h1>Documentation</h1>
    <p class="lead">Everything agent-eye does, one page at a time.</p>
    <ul class="doccards">${cards}</ul>
  </article>
</div>`,
  });
}

export function docPage(pages: DocPage[], doc: DocPage): string {
  const toc = doc.toc.length > 1
    ? `<nav class="toc" aria-label="On this page"><p>On this page</p><ul>${doc.toc.map((t) => `<li class="d${t.depth}"><a href="#${t.id}">${esc(t.text)}</a></li>`).join('')}</ul></nav>`
    : '';
  return page({
    title: `${doc.title} · agent-eye`,
    description: doc.summary,
    path: `docs/${doc.slug}/`,
    kind: 'docs',
    body: `<div class="wrap docs">
  ${docNav(pages, doc.slug)}
  <article class="doc">
    ${doc.html}
    <p class="edit"><a href="${REPO}/blob/main/docs/${doc.slug}.md">Edit this page on GitHub</a></p>
  </article>
  ${toc}
</div>`,
  });
}

export function notFound(): string {
  return page({
    title: 'Page not found · agent-eye',
    description: 'This page does not exist.',
    path: '404.html',
    body: `<div class="wrap notfound">
  <p class="label">404</p>
  <h1>Nothing here.</h1>
  <p class="lead">This page does not exist. Try the <a href="{{base}}">home page</a> or the <a href="{{base}}docs/">docs</a>.</p>
</div>`,
  });
}
