// The one eye: the board header, the Welcome screen, the site header and hero, the README banner and
// logo, the social image and the favicon all draw these rows. This file has no imports, so
// scripts/make-assets.mjs can read it with Node as it is.
//
// An almond outline, an orange iris, a small dark pupil and a one-pixel light highlight. The lashes
// (RAYS in art.ts) belong to the board alone, and only while something needs you.
//
// Characters: '#' outline, 'o' inside the lids (paper), 'v' iris (the accent colour), 'p' pupil,
// 'h' highlight, '.' clear. The pupil and the highlight are the same two colours on light and dark.

/** The pupil and the highlight, the same in every theme. */
export const PUPIL = '#161616';
export const GLINT = '#f2efe6';

/** Open (17 x 9). The inside is 15 wide and 7 tall, so a 5 x 5 iris has a row to spare above and below. */
export const EYE_OPEN: readonly string[] = [
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
/** Half shut, for a blink. */
export const EYE_HALF: readonly string[] = [
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
/** Shut. */
export const EYE_SHUT: readonly string[] = [
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
export const EYE_FRAMES: readonly (readonly string[])[] = [EYE_OPEN, EYE_HALF, EYE_SHUT];

/**
 * The iris (5 x 5) with its pupil and highlight. It sits at column 6, row 2 of the open eye, and
 * moves one pixel at a time to follow the pointer. One dark pixel is the smallest pupil that still
 * shows when the eye is 16 to 24 pixels wide; the highlight beside it keeps it from looking blank.
 */
export const IRIS: readonly string[] = ['.vvv.', 'vhvvv', 'vvpvv', 'vvvvv', '.vvv.'];
/** Where the iris sits when the eye looks straight ahead. */
export const IRIS_AT = { x: 6, y: 2 } as const;

/** The open eye with the iris straight ahead, as one sprite. */
export const EYE_STILL: readonly string[] = EYE_OPEN.map((row, y) =>
  [...row]
    .map((ch, x) => {
      const i = IRIS[y - IRIS_AT.y]?.[x - IRIS_AT.x];
      return i && i !== '.' ? i : ch;
    })
    .join(''),
);
