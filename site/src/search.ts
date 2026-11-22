import './search.css';

// The docs search. plugin/search.ts writes the box into every docs page and the index (docs/search.json,
// one entry per section); this file loads the index on first use and shows the best sections as you type.
// Everything shown is put in the page as text nodes, never as html.

interface Entry { slug: string; title: string; heading: string; id: string; text: string }
interface Section extends Entry {
  /** Lowercased words with how often each occurs. */
  titleW: Map<string, number>;
  headW: Map<string, number>;
  bodyW: Map<string, number>;
  /** The words in order, joined by spaces: for "the typed words, one after another" checks. */
  headN: string;
  bodyN: string;
  /** Words written in capitals (BUTTON LABELS), lowercased. */
  caps: string[];
}
/** A typed word. Every word must be found; the last (and any of 3+ letters) also as the start of a word. */
interface Term { text: string; prefix: boolean; caps: boolean }
interface Hit { entry: Section; score: number }

const MAX_RESULTS = 10;
const WORD = /[\p{L}\p{N}\p{M}]+/gu;
/** Left out of a query that has other words, so "how to forget a session" is searched as "forget session". */
const STOP = new Set(['a', 'an', 'the', 'of', 'to', 'in', 'on', 'and', 'or', 'is', 'are', 'do', 'does', 'how', 'i', 'it', 'my', 'can', 'for', 'with', 'what']);

const fold = (s: string) => s.toLowerCase().normalize('NFD').replace(/\p{M}/gu, '');
const words = (s: string) => Array.from(s.matchAll(WORD), (m) => fold(m[0]));
const isCaps = (w: string) => w.length > 1 && w === w.toUpperCase() && w !== w.toLowerCase();
const count = (ws: string[]) => {
  const m = new Map<string, number>();
  for (const w of ws) m.set(w, (m.get(w) ?? 0) + 1);
  return m;
};

function prepare(entries: Entry[]): Section[] {
  const titles = new Map<string, Map<string, number>>();
  return entries.map((e) => {
    let titleW = titles.get(e.slug);
    if (!titleW) titles.set(e.slug, (titleW = count(words(e.title))));
    const head = words(e.heading);
    const body = words(e.text);
    const caps = new Set<string>();
    for (const m of `${e.heading} ${e.text}`.matchAll(WORD)) if (isCaps(m[0])) caps.add(fold(m[0]));
    return { ...e, titleW, headW: count(head), bodyW: count(body), headN: head.join(' '), bodyN: body.join(' '), caps: [...caps] };
  });
}

function parse(query: string): { terms: Term[]; phrase: string } {
  const typed = Array.from(query.matchAll(WORD), (m) => m[0]);
  const all = typed.map(fold);
  const last = all.length - 1;
  const terms: Term[] = [];
  all.forEach((text, i) => {
    const caps = isCaps(typed[i] ?? '');
    const same = terms.find((t) => t.text === text);
    if (same) same.caps ||= caps;
    else if (i === last || !STOP.has(text)) terms.push({ text, prefix: i === last || text.length >= 3, caps });
  });
  return { terms, phrase: all.join(' ') };
}

const matches = (t: Term, word: string) => (t.prefix ? word.startsWith(t.text) : word === t.text);

/** How a term is found in a bag of words: 2 as a whole word, 1 as the start of one, 0 not at all; and how many times. */
function look(bag: Map<string, number>, t: Term): { level: 0 | 1 | 2; n: number } {
  const exact = bag.get(t.text) ?? 0;
  let part = 0;
  if (t.prefix) for (const [w, n] of bag) if (w !== t.text && w.startsWith(t.text)) part += n;
  return { level: exact ? 2 : part ? 1 : 0, n: exact + part };
}

/**
 * Ranking, best first: a page whose title has the words (its top section, 32-40 per word), then a
 * heading (12-16), then the text (3-5, a little more for repeats); whole words beat word starts, and
 * the words one after another in a heading or the text earn a bonus, as does a word typed in capitals
 * that the section also writes in capitals (MORE). A page's title also counts (5-6) for its other
 * sections ("night window" finds the window section of Night shift), but never makes a section a hit
 * by itself.
 */
function score(s: Section, terms: Term[], phrase: string): number {
  const top = s.id === '';
  let total = 0;
  let own = false;
  let inHeading = 0;
  for (const t of terms) {
    const title = look(s.titleW, t);
    const head = look(s.headW, t);
    const body = look(s.bodyW, t);
    if (!title.level && !head.level && !body.level) return 0;
    let best = 0;
    if (title.level) best = top ? (title.level === 2 ? 40 : 32) : (title.level === 2 ? 6 : 5);
    if (head.level) { best = Math.max(best, head.level === 2 ? 16 : 12); inHeading++; }
    if (body.level) best = Math.max(best, (body.level === 2 ? 4 : 3) + Math.min(body.n, 6) * 0.5);
    if (head.level || body.level || (top && title.level)) own = true;
    // Typed in capitals: the button labels, which the docs write in capitals, come first.
    if (t.caps && s.caps.some((w) => matches(t, w))) best += 6;
    total += best;
  }
  if (!own) return 0;
  if (terms.length > 1 && inHeading === terms.length) total += 8;
  if (terms.length > 1 && phrase) {
    if (` ${s.headN}`.includes(` ${phrase}`)) total += 14;
    if (` ${s.bodyN}`.includes(` ${phrase}`)) total += 6;
  }
  return total;
}

function search(sections: Section[], query: string): { hits: Hit[]; terms: Term[] } {
  const { terms, phrase } = parse(query);
  if (!terms.length) return { hits: [], terms };
  const hits: Hit[] = [];
  for (const entry of sections) {
    const s = score(entry, terms, phrase);
    if (s > 0) hits.push({ entry, score: s });
  }
  // Array.sort is stable: equal scores keep the docs' own order.
  hits.sort((a, b) => b.score - a.score);
  return { hits: hits.slice(0, MAX_RESULTS), terms };
}

/** About 150 characters around the first matched word of the text (or from its start when only the heading matched). */
function snippet(text: string, terms: Term[]): string {
  let at = 0;
  for (const m of text.matchAll(WORD)) {
    if (terms.some((t) => matches(t, fold(m[0])))) { at = m.index ?? 0; break; }
  }
  let start = Math.max(0, at - 40);
  if (start > 0) {
    const space = text.indexOf(' ', start);
    if (space >= 0 && space < at) start = space + 1;
  }
  let end = Math.min(text.length, start + 150);
  if (end < text.length) {
    const space = text.lastIndexOf(' ', end);
    if (space > at) end = space;
  }
  return (start > 0 ? '… ' : '') + text.slice(start, end).trim() + (end < text.length ? ' …' : '');
}

/** The text with every matched word in a <mark>, built from text nodes. */
function highlight(text: string, terms: Term[]): DocumentFragment {
  const out = document.createDocumentFragment();
  let from = 0;
  for (const m of text.matchAll(WORD)) {
    if (!terms.some((t) => matches(t, fold(m[0])))) continue;
    const at = m.index ?? 0;
    if (at > from) out.append(text.slice(from, at));
    const mark = document.createElement('mark');
    mark.textContent = m[0];
    out.append(mark);
    from = at + m[0].length;
  }
  if (from < text.length) out.append(text.slice(from));
  return out;
}

export function setupSearch() {
  const form = document.querySelector<HTMLFormElement>('.docsearch');
  const input = form?.querySelector<HTMLInputElement>('input');
  const panel = form?.querySelector<HTMLElement>('.ds-panel');
  const list = form?.querySelector<HTMLElement>('.ds-list');
  const none = form?.querySelector<HTMLElement>('.ds-none');
  const status = form?.querySelector<HTMLElement>('[role="status"]');
  if (!form || !input || !panel || !list || !none || !status) return;

  /** Where the pages are: the index is docs/search.json, so a page is `<root><slug>/`. */
  const root = (form.dataset.index ?? '').replace(/[^/]*$/, '');
  let loading: Promise<Section[]> | null = null;
  let links: HTMLAnchorElement[] = [];
  let active = -1;

  const load = () => {
    if (!loading) {
      loading = fetch(form.dataset.index ?? '')
        .then((r) => { if (!r.ok) throw new Error(`HTTP ${r.status}`); return r.json() as Promise<unknown>; })
        .then((data) => prepare((Array.isArray(data) ? data : []).filter((e): e is Entry => !!e && typeof e.text === 'string' && typeof e.slug === 'string')));
      loading.catch(() => { loading = null; });
    }
    return loading;
  };

  const show = (open: boolean) => {
    panel.hidden = !open;
    input.setAttribute('aria-expanded', String(open));
  };

  const close = () => {
    show(false);
    links = [];
    active = -1;
    list.replaceChildren();
    none.hidden = true;
    input.removeAttribute('aria-activedescendant');
  };

  const setActive = (i: number, scroll = true) => {
    links[active]?.setAttribute('aria-selected', 'false');
    active = i;
    const link = links[i];
    if (!link) return input.removeAttribute('aria-activedescendant');
    link.setAttribute('aria-selected', 'true');
    input.setAttribute('aria-activedescendant', link.id);
    if (scroll) link.scrollIntoView({ block: 'nearest' });
  };

  const update = async () => {
    if (!words(input.value).length) return close();
    let sections: Section[];
    try {
      sections = await load();
    } catch {
      list.replaceChildren();
      links = [];
      none.textContent = 'Search is not available right now';
      none.hidden = false;
      panel.toggleAttribute('data-empty', true);
      show(true);
      return;
    }
    // Typing went on while the index loaded: show what is in the box now.
    const { hits, terms } = search(sections, input.value);
    none.textContent = 'No match';
    list.replaceChildren(...hits.map(({ entry }, i) => {
      const a = document.createElement('a');
      a.className = 'ds-hit';
      a.id = `docsearch-hit-${i}`;
      a.href = `${root}${encodeURIComponent(entry.slug)}/${entry.id ? `#${encodeURIComponent(entry.id)}` : ''}`;
      a.tabIndex = -1;
      a.setAttribute('role', 'option');
      a.setAttribute('aria-selected', 'false');
      const path = document.createElement('span');
      path.className = 'ds-path';
      path.append(highlight(entry.title, terms));
      if (entry.heading) {
        const sep = document.createElement('span');
        sep.className = 'ds-sep';
        sep.textContent = ' › ';
        path.append(sep, highlight(entry.heading, terms));
      }
      a.append(path);
      const text = snippet(entry.text, terms);
      if (text) {
        const snip = document.createElement('span');
        snip.className = 'ds-snip';
        snip.append(highlight(text, terms));
        a.append(snip);
      }
      return a;
    }));
    links = [...list.querySelectorAll<HTMLAnchorElement>('.ds-hit')];
    active = -1;
    none.hidden = hits.length > 0;
    panel.toggleAttribute('data-empty', !hits.length);
    show(true);
    setActive(hits.length ? 0 : -1, false);
    status.textContent = hits.length ? `${hits.length} ${hits.length === 1 ? 'result' : 'results'}` : 'No match';
  };

  input.addEventListener('focus', () => {
    load().catch(() => { /* shown when something is typed */ });
    if (input.value && panel.hidden) void update();
  });
  input.addEventListener('input', () => void update());
  // Enter opens the selected result, or the first.
  const open = () => (links[active] ?? links[0])?.click();
  input.addEventListener('keydown', (e) => {
    if (e.isComposing) return;
    if (e.key === 'Enter') {
      e.preventDefault();
      open();
    } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      if (panel.hidden) return void update();
      if (!links.length) return;
      const step = e.key === 'ArrowDown' ? 1 : -1;
      setActive((active + step + links.length) % links.length);
    } else if (e.key === 'Escape') {
      if (input.value || !panel.hidden) { input.value = ''; close(); status.textContent = ''; } else input.blur();
    }
  });
  // The keyboard's "go" button on a phone submits the form instead of pressing Enter.
  form.addEventListener('submit', (e) => {
    e.preventDefault();
    open();
  });
  // A click on a result: the page follows the link; on this very page, just close the list.
  panel.addEventListener('click', (e) => {
    if ((e.target as Element).closest('.ds-hit')) { close(); input.blur(); }
  });
  // Mouse: the result under the pointer is the selected one. Clicks keep the focus in the box.
  panel.addEventListener('mousemove', (e) => {
    const link = (e.target as Element).closest<HTMLAnchorElement>('.ds-hit');
    const i = link ? links.indexOf(link) : -1;
    if (i >= 0 && i !== active) setActive(i, false);
  });
  panel.addEventListener('mousedown', (e) => e.preventDefault());
  form.addEventListener('focusout', (e) => {
    if (!form.contains(e.relatedTarget as Node | null)) show(false);
  });

  // "/" jumps to the box, unless you are typing somewhere.
  document.addEventListener('keydown', (e) => {
    if (e.key !== '/' || e.defaultPrevented || e.isComposing || e.ctrlKey || e.metaKey || e.altKey) return;
    if ((e.target as Element | null)?.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"])')) return;
    e.preventDefault();
    input.focus();
    input.select();
  });
}
