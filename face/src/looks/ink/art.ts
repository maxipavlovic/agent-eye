import { hash, rng, type Rows } from '../../pixel/sprite.ts';
import type { ColumnId, OutputKind, TopicStatus } from '../../api/types.ts';

/** '#' = ink, 'o' = paper (filled inside), 'v' = vermilion, '.' = clear. */
export const INK = { '#': 'var(--ink)', o: 'var(--paper)', v: 'var(--accent)' } as const;
export const INK_CC = { '#': 'currentColor', o: 'var(--paper)' } as const;

export const ICONS: Record<ColumnId, Rows> = {
  ideas: [
    '...#####...',
    '..#ooooo#..',
    '.#oo#oooo#.',
    '.#o#ooooo#.',
    '.#ooooooo#.',
    '..#ooooo#..',
    '...#ooo#...',
    '...#####...',
    '...#ooo#...',
    '...#####...',
    '....###....',
  ],
  running: [
    '....###....',
    '.##.#o#.##.',
    '.#ooooooo#.',
    '..#oo#oo#..',
    '###o###o###',
    '#ooo#o#ooo#',
    '###o###o###',
    '..#oo#oo#..',
    '.#ooooooo#.',
    '.##.#o#.##.',
    '....###....',
  ],
  needs_you: [
    '.....#.....',
    '...#####...',
    '..#ooooo#..',
    '..#ooooo#..',
    '..#ooooo#..',
    '.#ooooooo#.',
    '.#ooooooo#.',
    '#ooooooooo#',
    '###########',
    '....###....',
    '.....#.....',
  ],
  paused: [
    '....#####..',
    '..##ooo##..',
    '.#ooo##....',
    '.#oo#......',
    '#oo#.......',
    '#oo#.......',
    '#oo#.......',
    '#ooo#......',
    '.#ooo##..##',
    '..##ooo###.',
    '....#####..',
  ],
  waiting: [
    '###########',
    '#ooooooooo#',
    '.#ooooooo#.',
    '..#ooooo#..',
    '...#ooo#...',
    '....#o#....',
    '...#o#o#...',
    '..#ooooo#..',
    '.#ooo#ooo#.',
    '#ooooooooo#',
    '###########',
  ],
  done: [
    '.#.........',
    '.#########.',
    '.#o#o#o#o#.',
    '.##o#o#o##.',
    '.#o#o#o#o#.',
    '.#########.',
    '.#.........',
    '.#.........',
    '.#.........',
    '.#.........',
    '###........',
  ],
};

export const EMPTY: Rows = [
  '..###########..',
  '.#ooooooooooo#.',
  '#ooooooooooooo#',
  '#o#o#o#o#o#o#o#',
  '#ooooooooooooo#',
  '###############',
  '#.............#',
  '#.............#',
  '###############',
];

export const MARKS: Record<TopicStatus, Rows> = {
  done: ['#######', '#######', '#####o#', '####o##', '#o#o###', '##o####', '#######'],
  active: ['#######', '#ooooo#', '#o#ooo#', '#o##oo#', '#o#ooo#', '#ooooo#', '#######'],
  paused: ['#######', '#ooooo#', '#o#o#o#', '#o#o#o#', '#o#o#o#', '#ooooo#', '#######'],
  waiting_on_others: ['#######', '.#ooo#.', '..#o#..', '...#...', '..#o#..', '.#ooo#.', '#######'],
  needs_you: ['vvvvvvv', 'vvv#vvv', 'vvv#vvv', 'vvv#vvv', 'vvvvvvv', 'vvv#vvv', 'vvvvvvv'],
};

export const ARROW: Rows = ['....#..', '.....#.', '#######', '.....#.', '....#..'];
export const PIN: Rows = ['.###.', '#ooo#', '#ooo#', '.###.', '..#..', '..#..'];
export const BANG: Rows = ['..##..', '..##..', '..##..', '..##..', '......', '..##..', '..##..'];
export const SUN: Rows = ['....#....', '.#..#..#.', '..#####..', '..#ooo#..', '###ooo###', '..#ooo#..', '..#####..', '.#..#..#.', '....#....'];
export const MOON: Rows = ['..####...', '.##ooo#..', '##oo##...', '#oo#.....', '#oo#.....', '##oo#..##', '.##ooo##.', '..#####..', '.........'];
export const AUTO: Rows = ['#########', '#####oooo#', '#####oooo#', '#####oooo#', '#####oooo#', '#####oooo#', '#####oooo#', '#####oooo#', '#########'];
/** The Lanes / Grid switch in the header. */
export const LAYOUT_ICON = {
  lanes: ['##.###.##', '##.###.##', '##.###.##', '##.###.##', '##.###.##', '##.###.##', '##.###.##', '##.###.##', '##.###.##'],
  grid: ['#####.###', '#####.###', '#####.###', '#####.###', '#####.###', '.........', '####.####', '####.####', '####.####'],
} as const;
export const CHECK: Rows = ['.....#', '....#.', '#..#..', '.##...'];
export const CROSS: Rows = ['#...#', '.#.#.', '..#..', '.#.#.', '#...#'];
/** Two arrowheads pointing in: fold this lane. */
export const FOLD: Rows = ['#.....#', '##...##', '###.###', '##...##', '#.....#'];

export const OUT_ICON: Record<OutputKind, Rows> = {
  pr: ['#.....', '#..##.', '#...#.', '#..##.', '#.....', '#.....'].map((r) => r) as Rows,
  artifact: ['.####.', '#oooo#', '#o##o#', '#oooo#', '#o##o#', '.####.'],
  jira: ['.####.', '#oooo#', '#o##o#', '#o##o#', '#oooo#', '.####.'],
  commit: ['..#...', '..#...', '#####.', '..#...', '..#...', '......'],
  folder: ['###...', '#oo###', '#oooo#', '#oooo#', '#oooo#', '######'],
};

export const OUT_TAG: Record<OutputKind, string> = { pr: 'PR', artifact: 'ART', jira: 'JIRA', commit: 'CMT', folder: 'DIR' };

// ---- eye (17 x 9) ----
const EYE_OPEN: Rows = [
  '....#########....',
  '..##ooooooooo##..',
  '.#ooooooooooooo#.',
  '#ooooooooooooooo#',
  '#ooooooooooooooo#',
  '#ooooooooooooooo#',
  '.#ooooooooooooo#.',
  '..##ooooooooo##..',
  '....#########....',
];
const EYE_HALF: Rows = [
  '.................',
  '.................',
  '....#########....',
  '.##ooooooooooo##.',
  '#ooooooooooooooo#',
  '.##ooooooooooo##.',
  '....#########....',
  '.................',
  '.................',
];
const EYE_SHUT: Rows = [
  '.................',
  '.................',
  '.................',
  '...##.......##...',
  '#################',
  '...##.......##...',
  '.................',
  '.................',
  '.................',
];
export const EYE_FRAMES = [EYE_OPEN, EYE_HALF, EYE_SHUT];
export const IRIS: Rows = ['.###.', '#o###', '#####', '#####', '.###.'].map((r) => r);
/** The open eye with its pupil straight ahead, as one sprite: the empty board (the header eye moves). */
export const EYE_STILL: Rows = EYE_OPEN.map((row, y) => [...row].map((ch, x) => { const p = IRIS[y - 2]?.[x - 6]; return p && p !== '.' ? p : ch; }).join(''));
export const IRIS_ALERT: Rows = ['.vvv.', 'vovvv', 'vv#vv', 'vvvvv', '.vvv.'];
export const RAYS: Rows = ['..v.....v.....v..', '...v....v....v...'];

/** Pick one char kind from a frame: keeps only `ch`. */
export function only(rows: Rows, ch: string): Rows {
  return rows.map((r) => r.split('').map((c) => (c === ch ? c : '.')).join(''));
}

const sigilCache = new Map<string, Rows>();

/** 9x9 stamp from the session id: a mirrored 7x7 glyph in a frame, or reversed out of a solid block. */
export function sigil(id: string): Rows {
  let s = sigilCache.get(id);
  if (s) return s;
  const h = hash(id);
  const r = rng(h);
  const g: boolean[][] = [];
  let on = 0;
  for (let y = 0; y < 7; y++) {
    const half = [r() < 0.52, r() < 0.52, r() < 0.5, r() < 0.62];
    const row = [...half, half[2]!, half[1]!, half[0]!];
    on += row.filter(Boolean).length;
    g.push(row);
  }
  if (on < 18) for (let y = 0; y < 7; y++) { g[y]![3] = true; g[y]![2] = y % 2 === 0; g[y]![4] = y % 2 === 0; }
  const inverted = (h >>> 9) % 3 === 0;
  const rows: string[] = [];
  for (let y = 0; y < 9; y++) {
    let line = '';
    for (let x = 0; x < 9; x++) {
      const edge = x === 0 || y === 0 || x === 8 || y === 8;
      const inner = !edge;
      const lit = inner && g[y - 1]![x - 1]!;
      if (inverted) {
        const corner = (x === 0 || x === 8) && (y === 0 || y === 8);
        line += corner ? '.' : inner ? (lit ? 'o' : '#') : '#';
      } else {
        line += edge ? '#' : lit ? '#' : 'o';
      }
    }
    rows.push(line);
  }
  s = rows;
  sigilCache.set(id, s);
  return s;
}
