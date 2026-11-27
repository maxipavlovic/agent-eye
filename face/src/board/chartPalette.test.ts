import { describe, expect, it } from 'vitest';
import { ACCENTS } from '../accents';
import inkCss from '../looks/ink/tokens.css?raw';
import irisCss from '../looks/iris/tokens.css?raw';
import { chartColours, distance, NEAR, PALETTE, SLOTS } from './chartPalette';

const tokens = (look: string) => (look === 'ink' ? inkCss : irisCss);

/** The accent of an id in a look and a theme, read from the look's tokens. */
function accentOf(css: string, id: string, dark: boolean): string {
  const rule = css.split('\n').find((l) => l.includes(`data-accent='${id}']`) && l.includes("data-theme='dark'") === dark && l.includes('--accent:'));
  return /--accent: (#[0-9a-f]{6})/i.exec(rule ?? '')![1]!;
}

describe('the chart colours', () => {
  it('keep every chart colour well away from every accent of both looks, in both themes, bar a fifth that is only just under', () => {
    for (const look of ['ink', 'iris']) {
      for (const dark of [false, true]) {
        for (const a of ACCENTS) {
          const accent = accentOf(tokens(look), a.id, dark);
          const got = chartColours(accent, dark);
          expect(got, `${look} ${a.id} ${dark}`).toHaveLength(SLOTS);
          // a green accent leaves three of the seven too near, so the fifth colour may be the farthest of them (never under 13)
          got.forEach((c, i) => expect(distance(c, accent), `${look} ${a.id} ${dark} ${c}`).toBeGreaterThanOrEqual(i < 4 ? NEAR : 13));
        }
      }
    }
  });

  it('keeps every pair of colours apart, whichever two keys end up side by side in a stack', () => {
    for (const set of [PALETTE.light, PALETTE.dark]) {
      set.forEach((a, i) => set.slice(i + 1).forEach((b) => expect(distance(a, b), `${a} ${b}`).toBeGreaterThanOrEqual(NEAR)));
    }
  });

  it('skips only the slots near the accent and keeps the order of the rest', () => {
    const got = chartColours(PALETTE.light[1], false);
    expect(got).not.toContain(PALETTE.light[1]);
    expect(got.slice(0, 2)).toEqual([PALETTE.light[0], PALETTE.light[2]]);
    expect(chartColours('#3a3a3a', false)).toEqual(PALETTE.light.slice(0, SLOTS));
  });

  it('uses the first five when the accent cannot be read', () => {
    expect(chartColours('', true)).toEqual(PALETTE.dark.slice(0, SLOTS));
  });
});
