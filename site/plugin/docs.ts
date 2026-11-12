import fs from 'node:fs';
import path from 'node:path';
import { Marked, type Token, type Tokens } from 'marked';
import { REPO } from './links.ts';

export interface DocPage {
  /** URL name: `terminal` for docs/terminal.md. */
  slug: string;
  title: string;
  /** The first paragraph as plain text. */
  summary: string;
  html: string;
  toc: Array<{ id: string; text: string; depth: 2 | 3 }>;
}

/** Pages listed first, in this order; any other page follows alphabetically. */
const ORDER = ['getting-started', 'install', 'terminal', 'board', 'config', 'agents', 'api'];

const slugify = (s: string) => s.toLowerCase().replace(/`/g, '').replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'section';
const plain = (s: string) => unlink(s.replace(/[`*_]/g, ''));

/** `[label](url)` as `label`, in one pass (the regex for it backtracks on a long run of `[`). */
function unlink(s: string): string {
  let out = '';
  let i = 0;
  for (;;) {
    const open = s.indexOf('[', i);
    const close = open < 0 ? -1 : s.indexOf(']', open + 1);
    if (close < 0) break;
    if (s[close + 1] !== '(') {
      out += s.slice(i, close + 1);
      i = close + 1;
      continue;
    }
    const end = s.indexOf(')', close + 2);
    if (end < 0) break;
    out += s.slice(i, open) + s.slice(open + 1, close);
    i = end + 1;
  }
  return out + s.slice(i);
}

/** A link in docs/<file>.md as it should read on the site: other docs become pages, other repo files open on GitHub. */
export function siteLink(href: string): string {
  if (/^([a-z][a-z0-9+.-]*:|#|\/)/i.test(href)) return href;
  const [file = '', hash = ''] = href.split(/(?=#)/);
  const target = path.posix.normalize(path.posix.join('docs', file));
  const doc = /^docs\/([^/]+)\.md$/.exec(target);
  if (doc) return `{{base}}docs/${doc[1]}/${hash}`;
  return `${REPO}/${file.endsWith('/') ? 'tree' : 'blob'}/main/${target}${hash}`;
}

function render(markdown: string) {
  const ids = new Set<string>();
  const toc: DocPage['toc'] = [];
  const marked = new Marked({
    gfm: true,
    renderer: {
      heading({ tokens, depth, text }: Tokens.Heading) {
        const inner = this.parser.parseInline(tokens);
        if (depth === 1) return `<h1>${inner}</h1>\n`;
        let id = slugify(text);
        for (let n = 2; ids.has(id); n++) id = `${slugify(text)}-${n}`;
        ids.add(id);
        if (depth === 2 || depth === 3) toc.push({ id, text: plain(text), depth });
        return `<h${depth} id="${id}">${inner}<a class="anchor" href="#${id}" aria-label="Link to this section">#</a></h${depth}>\n`;
      },
      link({ href, title, tokens }: Tokens.Link) {
        const t = title ? ` title="${title.replace(/"/g, '&quot;')}"` : '';
        return `<a href="${siteLink(href)}"${t}>${this.parser.parseInline(tokens)}</a>`;
      },
    },
  });
  const tokens = marked.lexer(markdown);
  const h1 = tokens.find((t): t is Tokens.Heading => t.type === 'heading' && t.depth === 1);
  const first = tokens.find((t: Token): t is Tokens.Paragraph => t.type === 'paragraph');
  const html = (marked.parser(tokens) as string)
    .replace(/<table>/g, '<div class="table-wrap"><table>')
    .replace(/<\/table>/g, '</table></div>')
    // Page templates use {{...}} tokens; only the base path one comes from our own links.
    .replace(/\{\{(?!base\}\})/g, '&#123;&#123;');
  return { title: h1 ? plain(h1.text) : '', summary: first ? plain(first.text).replace(/\s+/g, ' ') : '', html, toc };
}

/** Every docs/*.md as a page, in reading order. */
export function loadDocs(dir: string): DocPage[] {
  if (!fs.existsSync(dir)) return [];
  const pages = fs.readdirSync(dir)
    .filter((f) => f.endsWith('.md'))
    .map((f): DocPage => {
      const slug = f.slice(0, -3);
      const r = render(fs.readFileSync(path.join(dir, f), 'utf8'));
      return { slug, ...r, title: r.title || slug };
    });
  const rank = (p: DocPage) => { const i = ORDER.indexOf(p.slug); return i < 0 ? ORDER.length : i; };
  return pages.sort((a, b) => rank(a) - rank(b) || a.slug.localeCompare(b.slug));
}
