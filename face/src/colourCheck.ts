// Colour checks for the looks' token tests: read `--name: #rrggbb` tokens out of a sheet and measure contrast.

/** The `--name: #rrggbb` tokens of the first rule in `css` whose selector list starts with `selector`. */
export function cssTokens(css: string, selector: string): Record<string, string> {
  const at = css.indexOf(selector);
  if (at < 0) throw new Error(`no rule for ${selector}`);
  const body = css.slice(css.indexOf('{', at) + 1, css.indexOf('}', at));
  const out: Record<string, string> = {};
  for (const decl of body.split(';')) {
    const [name, value] = decl.split(':').map((s) => s.trim());
    if (name?.startsWith('--') && value && /^#[\da-f]{6}$/i.test(value)) out[name.slice(2)] = value;
  }
  return out;
}

/** WCAG 2 relative luminance. */
function luminance(hex: string): number {
  const [r, g, b] = [1, 3, 5].map((i) => Number.parseInt(hex.slice(i, i + 2), 16) / 255)
    .map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!;
}

/** WCAG 2 contrast ratio of two colours. */
export function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi! + 0.05) / (lo! + 0.05);
}

/** `color-mix(in srgb, a share, b)`. */
export function mix(a: string, share: number, b: string): string {
  const ch = (hex: string, i: number) => Number.parseInt(hex.slice(i, i + 2), 16);
  return '#' + [1, 3, 5].map((i) => Math.round(ch(a, i) * share + ch(b, i) * (1 - share)).toString(16).padStart(2, '0')).join('');
}
