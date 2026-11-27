import { describe, expect, it } from 'vitest';
import { ACCENTS } from '../accents';
import inkCss from '../looks/ink/tokens.css?raw';
import irisCss from '../looks/iris/tokens.css?raw';
import { chartColours, distance, FILL_IN_MIN, NEAR, PALETTE, SLOTS } from './chartPalette';

// Machado, Oliveira and Fernandes (2009) at full severity, on linear RGB: the simulation the dataviz validator's floors are set for.
const CVD = {
  protan: [[0.152286, 1.052583, -0.204868], [0.114503, 0.786281, 0.099216], [-0.003882, -0.048116, 1.051998]],
  deutan: [[0.367322, 0.860646, -0.227968], [0.280085, 0.672501, 0.047413], [-0.01182, 0.04294, 0.968881]],
} as const;

const toLinear = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const toHex = (lin: number[]) => '#' + lin.map((v) => { const c = Math.min(1, Math.max(0, v)); return Math.round((c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055) * 255).toString(16).padStart(2, '0'); }).join('');

/** What a colour looks like to a reader with protan or deutan vision, as a hex colour. */
function simulate(hex: string, kind: keyof typeof CVD): string {
  const lin = [1, 3, 5].map((i) => toLinear(Number.parseInt(hex.slice(i, i + 2), 16) / 255));
  return toHex(CVD[kind].map((row) => row[0]! * lin[0]! + row[1]! * lin[1]! + row[2]! * lin[2]!));
}

const tokens = (look: string) => (look === 'ink' ? inkCss : irisCss);

/** The accent of an id in a look and a theme, read from the look's tokens. */
function accentOf(css: string, id: string, dark: boolean): string {
  const rule = css.split('\n').find((l) => l.includes(`data-accent='${id}']`) && l.includes("data-theme='dark'") === dark && l.includes('--accent:'));
  return /--accent: (#[0-9a-f]{6})/i.exec(rule ?? '')![1]!;
}

describe('the chart colours', () => {
  it('keep every chart colour well away from every accent of both looks, in both themes, bar a fifth that is only just under (FILL_IN_MIN)', () => {
    for (const look of ['ink', 'iris']) {
      for (const dark of [false, true]) {
        for (const a of ACCENTS) {
          const accent = accentOf(tokens(look), a.id, dark);
          const got = chartColours(accent, dark);
          expect(got, `${look} ${a.id} ${dark}`).toHaveLength(SLOTS);
          got.forEach((c, i) => expect(distance(c, accent), `${look} ${a.id} ${dark} ${c}`).toBeGreaterThanOrEqual(i < 4 ? NEAR : FILL_IN_MIN));
        }
      }
    }
  });

  it('keeps every pair of colours apart, whichever two keys end up side by side in a stack', () => {
    for (const set of [PALETTE.light, PALETTE.dark]) {
      set.forEach((a, i) => set.slice(i + 1).forEach((b) => expect(distance(a, b), `${a} ${b}`).toBeGreaterThanOrEqual(NEAR)));
    }
  });

  it('keeps every pair of colours at least 15 apart for full colour vision, and at least 6 apart under protan or deutan vision', () => {
    // 6 is the validator's floor for charts with a secondary encoding: every chart has a legend, a tooltip and a table
    for (const set of [PALETTE.light, PALETTE.dark]) {
      set.forEach((a, i) => set.slice(i + 1).forEach((b) => {
        expect(distance(a, b), `${a} ${b}`).toBeGreaterThanOrEqual(15);
        for (const kind of ['protan', 'deutan'] as const) expect(distance(simulate(a, kind), simulate(b, kind)), `${a} ${b} ${kind}`).toBeGreaterThanOrEqual(6);
      }));
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
