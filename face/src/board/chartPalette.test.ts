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
  it('keep the first four colours well away from every accent of both looks, in both themes, and the fifth too bar one case', () => {
    let filled = 0;
    for (const look of ['ink', 'iris']) {
      for (const dark of [false, true]) {
        for (const a of ACCENTS) {
          const accent = accentOf(tokens(look), a.id, dark);
          const got = chartColours(accent, dark);
          expect(got, `${look} ${a.id} ${dark}`).toHaveLength(SLOTS);
          got.forEach((c, i) => {
            if (distance(c, accent) >= NEAR) return;
            // only the fifth colour may be a fill-in, and only where the accent sits in the middle of three hues
            expect(i, `${look} ${a.id} ${dark} ${c}`).toBe(4);
            filled++;
          });
        }
      }
    }
    expect(filled).toBe(1);
  });

  it('skips only the slots near the accent and keeps the order of the rest', () => {
    const got = chartColours('#3a73b8', false);
    expect(got).not.toContain(PALETTE.light[0]);
    expect(got.slice(0, 2)).toEqual([PALETTE.light[1], PALETTE.light[2]]);
    expect(chartColours('#3a3a3a', false)).toEqual(PALETTE.light.slice(0, SLOTS));
  });

  it('uses the first five when the accent cannot be read', () => {
    expect(chartColours('', true)).toEqual(PALETTE.dark.slice(0, SLOTS));
  });
});
