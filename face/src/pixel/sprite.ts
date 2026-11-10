/**
 * Pixel art authored as text: one string per row, one character per pixel.
 * The palette maps characters to colours; '.' and ' ' (and any unmapped char) are transparent.
 * Colours may be CSS variables (e.g. 'var(--ink)') when drawn inline with <Pix>;
 * data URLs (spriteURL/stripURL) are isolated images and need concrete colours.
 */
export type Rows = readonly string[];
export type Palette = Readonly<Record<string, string>>;

export function size(rows: Rows): { w: number; h: number } {
  return { w: Math.max(0, ...rows.map((r) => r.length)), h: rows.length };
}

/** SVG <rect>s for one frame, horizontal runs merged, shifted by dx. */
export function rects(rows: Rows, pal: Palette, dx = 0): string {
  let out = '';
  rows.forEach((row, y) => {
    let x = 0;
    while (x < row.length) {
      const ch = row[x]!;
      let run = 1;
      while (x + run < row.length && row[x + run] === ch) run++;
      const fill = pal[ch];
      if (fill) out += `<rect x="${x + dx}" y="${y}" width="${run}" height="1" fill="${fill}"/>`;
      x += run;
    }
  });
  return out;
}

export function svg(rows: Rows, pal: Palette): string {
  const { w, h } = size(rows);
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${w} ${h}" width="${w}" height="${h}" shape-rendering="crispEdges">${rects(rows, pal)}</svg>`;
}

const urlCache = new Map<string, string>();

/** data: URL for CSS backgrounds, border-image or <img>. Concrete colours only. */
export function spriteURL(rows: Rows, pal: Palette): string {
  const key = rows.join('\n') + '|' + JSON.stringify(pal);
  let url = urlCache.get(key);
  if (!url) {
    url = 'data:image/svg+xml,' + encodeURIComponent(svg(rows, pal));
    urlCache.set(key, url);
  }
  return url;
}

/** Frames laid side by side, for a CSS `steps(n)` background animation. */
export function stripURL(frames: readonly Rows[], pal: Palette): { url: string; w: number; h: number; n: number } {
  const { w, h } = size(frames[0] ?? []);
  const body = frames.map((f, i) => rects(f, pal, i * w)).join('');
  const s = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${w * frames.length} ${h}" width="${w * frames.length}" height="${h}" shape-rendering="crispEdges">${body}</svg>`;
  return { url: 'data:image/svg+xml,' + encodeURIComponent(s), w, h, n: frames.length };
}

/**
 * CSS for a 9-slice pixel frame: `border-image` from a small tile whose outer `slice` pixels
 * are the border. Drawn at `scale` CSS px per art pixel.
 */
export function frameCSS(rows: Rows, pal: Palette, slice: number, scale: number): string {
  return `border-style: solid; border-width: ${slice * scale}px; border-image: url("${spriteURL(rows, pal)}") ${slice} fill / ${slice * scale}px / 0 repeat; image-rendering: pixelated;`;
}

/** FNV-1a: a stable 32-bit number from a string (e.g. a session id). */
export function hash(s: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

/** mulberry32: a small seeded random generator returning [0, 1). */
export function rng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
