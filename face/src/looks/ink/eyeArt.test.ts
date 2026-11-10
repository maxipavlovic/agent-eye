import { describe, expect, it } from 'vitest';
import { EYE_FRAMES, EYE_OPEN, EYE_STILL, GLINT, IRIS, IRIS_AT, PUPIL } from './eyeArt';
import { INK, RAYS } from './art';

const count = (rows: readonly string[], ch: string) => rows.join('').split(ch).length - 1;

describe('the one eye', () => {
  it('is an almond with an orange iris, one small dark pupil and a one-pixel highlight', () => {
    expect(EYE_FRAMES.every((f) => f.length === 9 && f.every((r) => r.length === 17))).toBe(true);
    expect(IRIS.every((r) => r.length === 5) && IRIS.length === 5).toBe(true);
    expect(count(IRIS, 'p')).toBe(1);
    expect(count(IRIS, 'h')).toBe(1);
    expect(count(IRIS, 'v')).toBeGreaterThan(15);
    expect(count(IRIS, '#') + count(IRIS, 'o')).toBe(0);
    expect(count(EYE_STILL, 'p')).toBe(1);
    expect(count(EYE_STILL, 'h')).toBe(1);
  });

  it('keeps the iris inside the lids wherever the pointer sends it', () => {
    for (let nx = -3; nx <= 3; nx++) {
      for (let ny = -1; ny <= 1; ny++) {
        IRIS.forEach((row, y) => [...row].forEach((ch, x) => {
          if (ch === '.') return;
          const at = EYE_OPEN[IRIS_AT.y + y + ny]?.[IRIS_AT.x + x + nx];
          expect(at, `iris pixel ${x},${y} at ${nx},${ny}`).toBe('o');
        }));
      }
    }
  });

  it('has the same two colours for the pupil and the highlight on a light and a dark page', () => {
    expect(INK.p).toBe(PUPIL);
    expect(INK.h).toBe(GLINT);
    expect(INK.v).toBe('var(--accent)');
  });

  it('leaves the lashes to the board: they are not part of any eye frame', () => {
    expect(EYE_FRAMES.flat().join('')).not.toContain('v');
    expect(RAYS.join('')).toContain('v');
  });
});
