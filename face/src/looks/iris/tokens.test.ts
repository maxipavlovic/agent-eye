import { describe, expect, it } from 'vitest';
import { ACCENTS } from '../../accents';
import css from './tokens.css?raw';

interface Set3 { accent: string; accentInk: string; onAccent: string }

/** The colour tokens of the first rule whose selector list starts with `selector`. */
function tokens(selector: string): Partial<Set3> & { paper?: string } {
  const at = css.indexOf(selector);
  if (at < 0) throw new Error(`no rule for ${selector}`);
  const body = css.slice(css.indexOf('{', at) + 1, css.indexOf('}', at));
  const get = (name: string) => new RegExp(`--${name}:\\s*(#[0-9a-fA-F]{6})`).exec(body)?.[1];
  return { accent: get('accent'), accentInk: get('accent-ink'), onAccent: get('on-accent'), paper: get('paper') };
}

/** WCAG 2 relative luminance and contrast ratio. */
function luminance(hex: string): number {
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255)
    .map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!;
}
const contrast = (a: string, b: string) => {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi! + 0.05) / (lo! + 0.05);
};

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
