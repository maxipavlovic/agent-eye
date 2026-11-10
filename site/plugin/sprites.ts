import { rects, size, type Palette, type Rows } from '../../face/src/pixel/sprite.ts';
import { ARROW, AUTO, EYE_FRAMES, ICONS, INK, INK_CC, IRIS, MARKS, MOON, SUN, only } from '../../face/src/looks/ink/art.ts';

/** Pixel icons only the site uses. Same alphabet as the board: # ink, o paper, v vermilion. */
const SITE_ICONS: Record<string, Rows> = {
  scan: [
    '.##.....##.',
    '#.........#',
    '#.........#',
    '...#####...',
    '...#ovo#...',
    '...#####...',
    '#.........#',
    '#.........#',
    '.##.....##.',
    '...........',
    '...........',
  ],
  note: [
    '.#########.',
    '.#ooooooo#.',
    '.#o#####o#.',
    '.#ooooooo#.',
    '.#o#####o#.',
    '.#ooooooo#.',
    '.#o###vvo#.',
    '.#ooooooo#.',
    '.#o#####o#.',
    '.#ooooooo#.',
    '.#########.',
  ],
  search: [
    '..#####....',
    '.#ooooo#...',
    '#oovvooo#..',
    '#oovooooo#.',
    '#ooooooo#..',
    '.#ooooo#...',
    '..#####....',
    '.....###...',
    '......###..',
    '.......###.',
    '........##.',
  ],
  resume: [
    '###########',
    '#ooooooooo#',
    '#oo#oooooo#',
    '#oo##ooooo#',
    '#oo###oooo#',
    '#oo####ooo#',
    '#oo###oooo#',
    '#oo##ooooo#',
    '#oo#oooooo#',
    '#ooooooooo#',
    '###########',
  ],
  lock: [
    '...#####...',
    '..#ooooo#..',
    '..#o###o#..',
    '..#o#.#o#..',
    '###########',
    '#ooooooooo#',
    '#oooovoooo#',
    '#oooovoooo#',
    '#ooooooooo#',
    '###########',
    '...........',
  ],
  terminal: [
    '###########',
    '#ooooooooo#',
    '#o#ooooooo#',
    '#oo#oooooo#',
    '#ooo#ooooo#',
    '#oo#oooooo#',
    '#o#oo###oo#',
    '#ooooooooo#',
    '#ooooooooo#',
    '#ooooooooo#',
    '###########',
  ],
};

const LANE_ICONS: Record<string, Rows> = { ...ICONS };
const NAMED: Record<string, Rows> = { ...LANE_ICONS, ...SITE_ICONS, arrow: ARROW, sun: SUN, moon: MOON, auto: AUTO };

/** An inline SVG sprite, like the board's Pix: the palette uses CSS variables, so it follows the theme. */
export function pix(rows: Rows, pal: Palette = INK, scale = 4, label?: string): string {
  const { w, h } = size(rows);
  const a11y = label ? `role="img" aria-label="${label}"` : 'aria-hidden="true"';
  return `<svg viewBox="0 0 ${w} ${h}" width="${w * scale}" height="${h * scale}" shape-rendering="crispEdges" ${a11y}>${rects(rows, pal)}</svg>`;
}

/** The brand eye as the board draws it (see Eye.tsx), ready for trackEye. No lashes: they are the board's alone. */
export function eyeMarkup(): string {
  const layers = (ch: string) => EYE_FRAMES.map((f, i) => `<div class="lay f${i}">${pix(only(f, ch))}</div>`).join('');
  return `<div class="eye" data-f="0" aria-hidden="true">${layers('o')}<div class="pclip"><div class="pupil">${pix(IRIS)}</div></div>${layers('#')}</div>`;
}

/** Replaces {{sprite:name}}, {{sprite:name:scale}}, {{sprite:name:scale:currentColor}} and {{eye}}. */
export function renderSprites(html: string): string {
  return html
    .replace(/\{\{eye\}\}/g, eyeMarkup())
    .replace(/\{\{sprite:([a-z_]+)(?::(\d+))?(?::(ink|cc))?\}\}/g, (_m, name: string, scale?: string, pal?: string) => {
      const rows = NAMED[name] ?? (name in MARKS ? MARKS[name as keyof typeof MARKS] : undefined);
      if (!rows) throw new Error(`Unknown sprite "${name}"`);
      return pix(rows, pal === 'cc' ? INK_CC : INK, scale ? Number(scale) : 4);
    });
}
