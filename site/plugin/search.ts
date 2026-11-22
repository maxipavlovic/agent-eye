import type { DocPage } from './docs.ts';

/** One searchable piece of a docs page: the text between one heading and the next (h2/h3). */
export interface SearchEntry {
  slug: string;
  /** The page's title. */
  title: string;
  /** The section's heading; empty for the text above a page's first heading. */
  heading: string;
  /** The heading's anchor id, the same one the page uses; empty for the top of the page. */
  id: string;
  /** The section's text: no markup, whitespace collapsed, code included. */
  text: string;
}

const ENTITIES: Record<string, string> = { amp: '&', lt: '<', gt: '>', quot: '"', apos: "'" };

/** Character references in one pass, so `&amp;lt;` stays `&lt;`. */
const decode = (s: string) => s.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (all, ref: string) => {
  if (ref[0] !== '#') return ENTITIES[ref.toLowerCase()] ?? all;
  const code = ref[1] === 'x' || ref[1] === 'X' ? parseInt(ref.slice(2), 16) : parseInt(ref.slice(1), 10);
  return code > 0 && code <= 0x10ffff ? String.fromCodePoint(code) : all;
});

/** Tags that end a word (a table cell, a list item, a paragraph); inline tags (code, em, a) do not. */
const BLOCK_TAG = /<\/?(?:p|li|ul|ol|tr|td|th|thead|tbody|table|div|pre|blockquote|br|hr|h[1-6])\b[^>]*>/gi;

/** The text of a piece of the page's html. */
const plain = (html: string) => decode(html.replace(BLOCK_TAG, ' ').replace(/<[^>]*>/g, '')).replace(/\s+/g, ' ').trim();

const HEADING = /<h([23]) id="([^"]+)">([\s\S]*?)<\/h\1>/g;
const ANCHOR_LINK = /<a class="anchor"[^>]*>[^<]*<\/a>/g;

/** Every page cut at its h2 and h3 headings, one entry per section, in reading order. */
export function searchIndex(pages: DocPage[]): SearchEntry[] {
  const entries: SearchEntry[] = [];
  for (const p of pages) {
    const marks = [...p.html.matchAll(HEADING)];
    const first = marks[0]?.index ?? p.html.length;
    const top = p.html.slice(0, first).replace(/<h1>[\s\S]*?<\/h1>/, '');
    entries.push({ slug: p.slug, title: p.title, heading: '', id: '', text: plain(top) });
    marks.forEach((m, i) => {
      const end = marks[i + 1]?.index ?? p.html.length;
      const body = p.html.slice((m.index ?? 0) + m[0].length, end);
      entries.push({ slug: p.slug, title: p.title, heading: plain((m[3] ?? '').replace(ANCHOR_LINK, '')), id: m[2] ?? '', text: plain(body) });
    });
  }
  return entries;
}

/**
 * The search box at the top of the docs menu. `{{base}}` is filled in with the rest of the page;
 * src/search.ts does the rest (it loads the index named in data-index on first use).
 */
export function searchBox(): string {
  return `<form class="docsearch" role="search" data-index="{{base}}docs/search.json" autocomplete="off">
    <label class="ds-sr" for="docsearch-q">Search the docs</label>
    <div class="ds-box">
      <input id="docsearch-q" type="search" name="q" placeholder="Search the docs" role="combobox" aria-expanded="false" aria-controls="docsearch-list" aria-autocomplete="list" autocomplete="off" autocapitalize="off" spellcheck="false" enterkeyhint="search">
      <kbd class="ds-key" aria-hidden="true">/</kbd>
    </div>
    <div class="ds-panel" hidden>
      <div class="ds-list" id="docsearch-list" role="listbox" aria-label="Search results"></div>
      <p class="ds-none" hidden>No match</p>
    </div>
    <p class="ds-sr" role="status" aria-live="polite"></p>
  </form>`;
}
