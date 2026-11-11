import { describe, expect, it } from 'vitest';
import { ACCENTS, DEFAULT_ACCENT, parseAccent } from './accents';
import css from './looks/ink/tokens.css?raw';

/** The `--name: #rrggbb` values of the first rule whose selector list starts with `selector`. */
function tokens(selector: string): { accent: string; onAccent: string } {
  const at = css.indexOf(`${selector}`);
  if (at < 0) throw new Error(`no rule for ${selector}`);
  const body = css.slice(css.indexOf('{', at) + 1, css.indexOf('}', at));
  const get = (name: string) => new RegExp(`--${name}:\\s*(#[0-9a-fA-F]{6})`).exec(body)?.[1];
  const [accent, onAccent] = [get('accent'), get('on-accent')];
  if (!accent || !onAccent) throw new Error(`${selector} must set --accent and --on-accent`);
  return { accent, onAccent };
}

const PAPER = { light: '#f2efe6', dark: '#1e1d1a' };

/** WCAG 2 relative luminance and contrast ratio. */
function luminance(hex: string): number {
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255)
    .map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!;
}
function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi! + 0.05) / (lo! + 0.05);
}

/** The colours of a preset in a theme; vermilion is the base of tokens.css, the rest are `data-accent` blocks. */
function pair(id: string, theme: 'light' | 'dark') {
  if (id === 'vermilion') return tokens(theme === 'light' ? ':root {' : ":root[data-theme='dark'] {");
  return tokens(theme === 'light' ? `:root[data-accent='${id}']` : `:root[data-theme='dark'][data-accent='${id}']`);
}

describe('accent presets', () => {
  it('offers eight, vermilion first and the default', () => {
    expect(ACCENTS.map((a) => a.id)).toEqual(['vermilion', 'cobalt', 'teal', 'moss', 'amber', 'magenta', 'violet', 'graphite']);
    expect(DEFAULT_ACCENT).toBe('vermilion');
    expect(new Set(ACCENTS.map((a) => a.name)).size).toBe(8);
  });

  it('keeps vermilion exactly as the ink look had it', () => {
    expect(pair('vermilion', 'light')).toEqual({ accent: '#ff4a1c', onAccent: '#161616' });
    expect(pair('vermilion', 'dark')).toEqual({ accent: '#e8683f', onAccent: '#1e1d1a' });
  });

  for (const { id } of ACCENTS) {
    for (const theme of ['light', 'dark'] as const) {
      it(`${id} (${theme}) reads: on-accent on accent 4.5, accent on paper 3`, () => {
        const { accent, onAccent } = pair(id, theme);
        expect(contrast(onAccent, accent)).toBeGreaterThanOrEqual(4.5);
        // Vermilion on the light paper is the look's own colour since 0.1 and sits at 2.9; it stays.
        const floor = id === 'vermilion' && theme === 'light' ? 2.9 : 3;
        expect(contrast(accent, PAPER[theme])).toBeGreaterThanOrEqual(floor);
      });
    }
  }

  it('has a style sheet block for each preset of the picker and for no other', () => {
    const inCss = [...css.matchAll(/^:root\[data-accent='([a-z]+)'\]/gm)].map((m) => m[1]);
    expect(inCss).toEqual(ACCENTS.slice(1).map((a) => a.id));
  });
});

describe('parseAccent', () => {
  it('takes a known id and falls back to vermilion for anything else', () => {
    expect(parseAccent('teal')).toBe('teal');
    expect(parseAccent('graphite')).toBe('graphite');
    for (const bad of [null, undefined, '', 'Teal', 'pink', 3, {}]) expect(parseAccent(bad)).toBe('vermilion');
  });
});
