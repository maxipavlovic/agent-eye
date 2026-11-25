import { describe, expect, it } from 'vitest';
import { ACCENTS } from '../../accents';
import { contrast, cssTokens, mix } from '../../colourCheck';
import css from './tokens.css?raw';

interface Set3 { accent: string; accentInk: string; onAccent: string }

/** The colour tokens of the first rule whose selector list starts with `selector`. */
function tokens(selector: string): Partial<Set3> & { paper?: string } {
  const t = cssTokens(css, selector);
  return { accent: t['accent'], accentInk: t['accent-ink'], onAccent: t['on-accent'], paper: t['paper'] };
}

const PAPER = { light: tokens(':root {').paper!, dark: tokens(":root[data-theme='dark'] {").paper! };

/** A preset's three colours in a theme: vermilion is the base of tokens.css, the rest are `data-accent` blocks. */
function preset(id: string, theme: 'light' | 'dark'): Set3 {
  const base = theme === 'light' ? tokens(':root {') : tokens(":root[data-theme='dark'] {");
  const own = id === 'vermilion' ? {} : tokens(theme === 'light' ? `:root[data-accent='${id}']` : `:root[data-theme='dark'][data-accent='${id}']`);
  const s = { accent: own.accent ?? base.accent, accentInk: own.accentInk ?? base.accentInk, onAccent: own.onAccent ?? base.onAccent };
  if (!s.accent || !s.accentInk || !s.onAccent) throw new Error(`${id} (${theme}) misses a colour`);
  return s as Set3;
}

describe('iris colours', () => {
  it('has a paper for each theme', () => {
    expect(PAPER.light).toMatch(/^#/);
    expect(PAPER.dark).toMatch(/^#/);
  });

  for (const { id } of ACCENTS) {
    for (const theme of ['light', 'dark'] as const) {
      it(`${id} (${theme}): a mark box reads (4.5), the accent stands out (3), its text shade reads on the paper (4.5)`, () => {
        const { accent, accentInk, onAccent } = preset(id, theme);
        expect(contrast(onAccent, accent)).toBeGreaterThanOrEqual(4.5);
        expect(contrast(accent, PAPER[theme])).toBeGreaterThanOrEqual(3);
        expect(contrast(accentInk, PAPER[theme])).toBeGreaterThanOrEqual(4.5);
      });

      // Needs you sits on a wash of the accent in light (6 %, 11 % under the pointer; iris.css); at night on the paper
      if (theme === 'light') it(`${id} (light): Needs you's words read on its wash (4.5) and its count stands out (3)`, () => {
        const { accent, accentInk } = preset(id, theme);
        for (const share of [0.06, 0.11]) expect(contrast(accentInk, mix(accent, share, PAPER.light)), `${share}`).toBeGreaterThanOrEqual(4.5);
        expect(contrast(accent, mix(accent, 0.06, PAPER.light))).toBeGreaterThanOrEqual(3);
      });
    }
  }

  for (const { id } of ACCENTS) {
    it(`${id} has a swatch of its own colour in both themes`, () => {
      for (const theme of ['light', 'dark'] as const) {
        const swatch = tokens(theme === 'light' ? `.acc[data-accent='${id}']` : `:root[data-theme='dark'] .acc[data-accent='${id}']`);
        expect(swatch.accent).toBe(preset(id, theme).accent);
      }
    });
  }

  it('has a block for each preset of the picker and for no other', () => {
    const inCss = [...css.matchAll(/^:root\[data-accent='([a-z]+)'\]/gm)].map((m) => m[1]);
    expect(inCss).toEqual(ACCENTS.slice(1).map((a) => a.id));
  });

  it('draws every line with --hair: one pixel, half a pixel on Retina', () => {
    expect(css).toMatch(/--hair: 1px;/);
    expect(css).toMatch(/min-resolution: 2dppx\) \{ :root \{ --hair: \.5px; --sw: \.5; \} \}/);
  });
});
