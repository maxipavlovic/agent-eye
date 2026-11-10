#!/usr/bin/env node
// Draws the README art into assets/: banner (light and dark), logo, feature icons and the
// social preview, and the site's favicon. Everything is pixel art on a grid, written as SVG paths
// (and one PNG), so the files stay crisp and small. The eye is the board's own: its rows come from
// face/src/looks/ink/eyeArt.ts. Run: node scripts/make-assets.mjs (Node 22.18 or newer, which reads
// that TypeScript file as it is).
import { deflateSync, crc32 } from 'node:zlib';
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { EYE_STILL, GLINT, PUPIL } from '../face/src/looks/ink/eyeArt.ts';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const OUT = join(ROOT, 'assets');
const FAVICON = join(ROOT, 'site', 'public', 'favicon.svg');

// Same palette as the board's "ink" look (face/src/looks/ink/ink.css).
const LIGHT = { paper: '#f2efe6', ink: '#161616', accent: '#ff4a1c' };
const DARK = { paper: '#1e1d1a', ink: '#d6d1c4', accent: '#e8683f' };

// '#' ink, 'v' vermilion, 'd' ink checker (a 50% dither), 'o' paper, '.' nothing.
const FONT = {
  A: ['.###.', '#...#', '#...#', '#####', '#...#', '#...#', '#...#'],
  B: ['####.', '#...#', '#...#', '####.', '#...#', '#...#', '####.'],
  C: ['.###.', '#...#', '#....', '#....', '#....', '#...#', '.###.'],
  D: ['####.', '#...#', '#...#', '#...#', '#...#', '#...#', '####.'],
  E: ['#####', '#....', '#....', '####.', '#....', '#....', '#####'],
  F: ['#####', '#....', '#....', '####.', '#....', '#....', '#....'],
  G: ['.###.', '#...#', '#....', '#.###', '#...#', '#...#', '.###.'],
  H: ['#...#', '#...#', '#...#', '#####', '#...#', '#...#', '#...#'],
  I: ['#####', '..#..', '..#..', '..#..', '..#..', '..#..', '#####'],
  J: ['..###', '...#.', '...#.', '...#.', '...#.', '#..#.', '.##..'],
  K: ['#...#', '#..#.', '#.#..', '##...', '#.#..', '#..#.', '#...#'],
  L: ['#....', '#....', '#....', '#....', '#....', '#....', '#####'],
  M: ['#...#', '##.##', '#.#.#', '#.#.#', '#...#', '#...#', '#...#'],
  N: ['#...#', '#...#', '##..#', '#.#.#', '#..##', '#...#', '#...#'],
  O: ['.###.', '#...#', '#...#', '#...#', '#...#', '#...#', '.###.'],
  P: ['####.', '#...#', '#...#', '####.', '#....', '#....', '#....'],
  Q: ['.###.', '#...#', '#...#', '#...#', '#.#.#', '#..#.', '.##.#'],
  R: ['####.', '#...#', '#...#', '####.', '#.#..', '#..#.', '#...#'],
  S: ['.####', '#....', '#....', '.###.', '....#', '....#', '####.'],
  T: ['#####', '..#..', '..#..', '..#..', '..#..', '..#..', '..#..'],
  U: ['#...#', '#...#', '#...#', '#...#', '#...#', '#...#', '.###.'],
  V: ['#...#', '#...#', '#...#', '#...#', '#...#', '.#.#.', '..#..'],
  W: ['#...#', '#...#', '#...#', '#.#.#', '#.#.#', '##.##', '#...#'],
  X: ['#...#', '#...#', '.#.#.', '..#..', '.#.#.', '#...#', '#...#'],
  Y: ['#...#', '#...#', '.#.#.', '..#..', '..#..', '..#..', '..#..'],
  Z: ['#####', '....#', '...#.', '..#..', '.#...', '#....', '#####'],
  '-': ['.....', '.....', '.....', '#####', '.....', '.....', '.....'],
  '.': ['.....', '.....', '.....', '.....', '.....', '..#..', '..#..'],
  ',': ['.....', '.....', '.....', '.....', '..#..', '..#..', '.#...'],
  ' ': ['.....', '.....', '.....', '.....', '.....', '.....', '.....'],
};

// 16 x 16 feature icons. They sit on their own paper tile, so one file works on any page.
const ICONS = {
  search: [
    '................',
    '................',
    '....#####.......',
    '...#.....#......',
    '..#.......#.....',
    '..#.vv....#.....',
    '..#.v.....#.....',
    '..#.......#.....',
    '..#.......#.....',
    '...#.....#......',
    '....#####.#.....',
    '..........##....',
    '...........##...',
    '............##..',
    '.............##.',
    '................',
  ],
  keep: [
    '................',
    '................',
    '..############..',
    '..#..........#..',
    '..#..........#..',
    '..############..',
    '...#........#...',
    '...#........#...',
    '...#..vvvv..#...',
    '...#........#...',
    '...#........#...',
    '...#........#...',
    '...##########...',
    '................',
    '................',
    '................',
  ],
  key: [
    '................',
    '..############..',
    '.#............#.',
    '.#....vvv.....#.',
    '.#...v...v....#.',
    '.#...v........#.',
    '.#...v.vvv....#.',
    '.#...v...v....#.',
    '.#...v...v....#.',
    '.#....vvv.....#.',
    '.#............#.',
    '.##############.',
    '.##############.',
    '..############..',
    '................',
    '................',
  ],
  status: [
    '................',
    '.......##.......',
    '.....######.....',
    '..v.#......#.v..',
    '....#......#....',
    '.v..#......#..v.',
    '...#........#...',
    '...#........#...',
    '...#........#...',
    '..#..........#..',
    '.##############.',
    '................',
    '......vvvv......',
    '......vvvv......',
    '.......vv.......',
    '................',
  ],
  board: [
    '................',
    '................',
    '.####.####.####.',
    '................',
    '.####.####.####.',
    '.#..#.#..#.#vv#.',
    '.####.####.####.',
    '................',
    '.####.####......',
    '.#..#.#..#......',
    '.####.####......',
    '................',
    '.####...........',
    '.#..#...........',
    '.####...........',
    '................',
  ],
  local: [
    '................',
    '.....######.....',
    '....#......#....',
    '....#......#....',
    '....#......#....',
    '....#......#....',
    '..############..',
    '..#dddddddddd#..',
    '..#dddddddddd#..',
    '..#ddd.vv.ddd#..',
    '..#dd.vvvv.dd#..',
    '..#dd.vvvv.dd#..',
    '..#ddd.vv.ddd#..',
    '..#dddd.vv.dd#..',
    '..############..',
    '................',
  ],
};

class Canvas {
  constructor(w, h) {
    this.w = w;
    this.h = h;
    this.px = new Array(w * h).fill(null);
  }
  set(x, y, c) {
    if (x >= 0 && y >= 0 && x < this.w && y < this.h) this.px[y * this.w + x] = c;
  }
  get(x, y) {
    return this.px[y * this.w + x];
  }
  fill(c) {
    this.px.fill(c);
  }
  rect(x, y, w, h, c) {
    for (let j = 0; j < h; j++) for (let i = 0; i < w; i++) this.set(x + i, y + j, c);
  }
  /** Draw rows of chars; `pal` maps a char to a colour; `s` makes each pixel an s x s block. */
  sprite(rows, x, y, pal, s = 1) {
    rows.forEach((row, j) => {
      [...row].forEach((ch, i) => {
        const c = pal[ch];
        if (!c) return;
        for (let b = 0; b < s * s; b++) {
          const px = x + i * s + (b % s);
          const py = y + j * s + Math.floor(b / s);
          const checker = ch === 'd' ? (px + py) % 2 === 0 : true;
          if (checker) this.set(px, py, c);
        }
      });
    });
  }
  /** Text in the 5 x 7 font; letter spacing is one font pixel. Returns the width drawn. */
  text(str, x, y, color, s = 1, colorOf = null) {
    let cx = x;
    [...str].forEach((ch, n) => {
      const g = FONT[ch] ?? FONT[' '];
      this.sprite(g, cx, y, { '#': colorOf ? colorOf(ch, n) : color }, s);
      cx += 6 * s;
    });
    return cx - x - s;
  }
  /** `scale` is the display size of one art pixel; 0 leaves width and height out (a favicon). */
  svg(scale) {
    const rects = new Map();
    for (let y = 0; y < this.h; y++) {
      let x = 0;
      while (x < this.w) {
        const c = this.get(x, y);
        if (!c) {
          x++;
          continue;
        }
        let run = 1;
        while (x + run < this.w && this.get(x + run, y) === c) run++;
        if (!rects.has(c)) rects.set(c, []);
        rects.get(c).push(`M${x} ${y}h${run}v1h-${run}z`);
        x += run;
      }
    }
    const paths = [...rects].map(([c, d]) => `<path fill="${c}" d="${d.join('')}"/>`).join('');
    return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${this.w} ${this.h}"${scale ? ` width="${this.w * scale}" height="${this.h * scale}"` : ''} shape-rendering="crispEdges">${paths}</svg>\n`;
  }
  png(scale) {
    const W = this.w * scale;
    const H = this.h * scale;
    const raw = Buffer.alloc((W * 4 + 1) * H);
    for (let y = 0; y < H; y++) {
      const row = y * (W * 4 + 1);
      raw[row] = 0;
      for (let x = 0; x < W; x++) {
        const c = this.get(Math.floor(x / scale), Math.floor(y / scale));
        const o = row + 1 + x * 4;
        if (c) {
          raw[o] = parseInt(c.slice(1, 3), 16);
          raw[o + 1] = parseInt(c.slice(3, 5), 16);
          raw[o + 2] = parseInt(c.slice(5, 7), 16);
          raw[o + 3] = 255;
        }
      }
    }
    const chunk = (type, data) => {
      const len = Buffer.alloc(4);
      len.writeUInt32BE(data.length);
      const body = Buffer.concat([Buffer.from(type), data]);
      const crc = Buffer.alloc(4);
      crc.writeUInt32BE(crc32(body));
      return Buffer.concat([len, body, crc]);
    };
    const ihdr = Buffer.alloc(13);
    ihdr.writeUInt32BE(W, 0);
    ihdr.writeUInt32BE(H, 4);
    ihdr[8] = 8; // bit depth
    ihdr[9] = 6; // RGBA
    return Buffer.concat([
      Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
      chunk('IHDR', ihdr),
      chunk('IDAT', deflateSync(raw, { level: 9 })),
      chunk('IEND', Buffer.alloc(0)),
    ]);
  }
}

/** Dither shadow: the checker pattern one pixel down and right of every drawn pixel in `mask`. */
function shadow(cv, mask, color) {
  for (let y = 0; y < cv.h; y++) {
    for (let x = 0; x < cv.w; x++) {
      if (mask.get(x, y) || !mask.get(x - 1, y - 1)) continue;
      if ((x + y) % 2 === 0) cv.set(x, y, color);
    }
  }
}

/** The brand eye, open and looking ahead (17 x 9 art pixels): the same rows the board and the site draw. */
function eyeSprite(cv, x, y, p, s = 1) {
  cv.sprite(EYE_STILL, x, y, { '#': p.ink, o: p.paper, v: p.accent, p: PUPIL, h: GLINT }, s);
}

function frame(cv, p) {
  const { w, h } = cv;
  cv.rect(0, 0, w, h, p.paper);
  cv.rect(0, 0, w, 1, p.ink);
  cv.rect(0, h - 1, w, 1, p.ink);
  cv.rect(0, 0, 1, h, p.ink);
  cv.rect(w - 1, 0, 1, h, p.ink);
  for (const [x, y] of [[0, 0], [w - 1, 0], [0, h - 1], [w - 1, h - 1]]) cv.set(x, y, null);
}

/** Eye and wordmark, centred on (cx, cy); `s` is the size of one font pixel. */
function wordmark(cv, p, cx, cy, s) {
  const word = 'AGENT-EYE';
  const textW = word.length * 6 * s - s;
  const eyeW = 17 * s;
  const gap = 4 * s;
  const left = Math.round(cx - (eyeW + gap + textW) / 2);
  const top = Math.round(cy - (9 * s) / 2);
  const mask = new Canvas(cv.w, cv.h);
  const color = (ch) => (ch === '-' ? p.accent : p.ink);
  mask.text(word, left + eyeW + gap, top + s, 'x', s);
  const textTop = top + s; // 7 font rows centred on the 9 eye rows
  shadow(cv, mask, p.ink);
  cv.text(word, left + eyeW + gap, textTop, p.ink, s, color);
  eyeSprite(cv, left, top, p, s);
  return { left, top, textW };
}

function banner(p) {
  const cv = new Canvas(180, 40);
  frame(cv, p);
  wordmark(cv, p, 90, 20, 2);
  return cv;
}

function logo() {
  const cv = new Canvas(21, 21);
  cv.fill(LIGHT.paper);
  eyeSprite(cv, 2, 6, LIGHT);
  return cv;
}

/** The site's tab icon: 17 x 17, so the 17-pixel-wide eye fills it and a 16 or 32 pixel icon keeps one art pixel to about one device pixel. */
function favicon() {
  const cv = new Canvas(17, 17);
  cv.fill(LIGHT.paper);
  eyeSprite(cv, 0, 4, LIGHT);
  return cv;
}

function icon(rows) {
  const cv = new Canvas(16, 16);
  cv.fill(LIGHT.paper);
  cv.sprite(rows, 0, 0, { '#': LIGHT.ink, d: LIGHT.ink, v: LIGHT.accent });
  return cv;
}

function social() {
  const p = LIGHT;
  const cv = new Canvas(320, 160);
  frame(cv, p);
  // dithered inner border, like a board card
  for (let i = 8; i < cv.w - 8; i += 2) {
    cv.set(i, 8, p.ink);
    cv.set(i, cv.h - 9, p.ink);
  }
  for (let j = 8; j < cv.h - 8; j += 2) {
    cv.set(8, j, p.ink);
    cv.set(cv.w - 9, j, p.ink);
  }
  wordmark(cv, p, 160, 50, 3);
  const tag = 'FIND, RESUME AND KEEP YOUR CODING-AGENT SESSIONS';
  cv.text(tag, Math.round(160 - (tag.length * 6 - 1) / 2), 92, p.ink, 1);
  const names = Object.keys(ICONS);
  const x0 = Math.round(160 - (names.length * 16 + (names.length - 1) * 8) / 2);
  names.forEach((n, i) => cv.sprite(ICONS[n], x0 + i * 24, 116, { '#': p.ink, d: p.ink, v: p.accent }));
  return cv;
}

// Each file: [canvas, display pixels per art pixel]. PREVIEW=<dir> also writes PNG files to look at.
const files = {
  'banner-light.svg': [banner(LIGHT), 4],
  'banner-dark.svg': [banner(DARK), 4],
  'logo.svg': [logo(), 8],
  ...Object.fromEntries(Object.entries(ICONS).map(([n, rows]) => [`icon-${n}.svg`, [icon(rows), 4]])),
};
mkdirSync(OUT, { recursive: true });
for (const [name, [cv, scale]] of Object.entries(files)) {
  writeFileSync(join(OUT, name), cv.svg(scale));
  if (process.env.PREVIEW) writeFileSync(join(process.env.PREVIEW, name.replace('.svg', '.png')), cv.png(scale));
}
writeFileSync(join(OUT, 'social.png'), social().png(4));
writeFileSync(FAVICON, favicon().svg(0));
console.log('assets written to', OUT, 'and', FAVICON);
