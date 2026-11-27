// The categorical colours of the stacked charts. Seven hues chosen for these looks (blue, magenta, gold, green,
// orange, violet, teal), in an order that keeps neighbours apart under the common colour-vision differences, for the
// light and for the dark paper (checked with the dataviz palette validator, every accent included). The accent is the signal colour ("needs you"), so no chart colour may sit near it: the
// slots too close to the accent are skipped at render and the next ones take their place.

/** Light and dark steps of the same seven hues, in slot order. */
export const PALETTE = {
  light: ['#3772bb', '#a5538c', '#907904', '#1c6402', '#b66028', '#755db1', '#078968'],
  dark: ['#5894e0', '#c670ab', '#a9912b', '#297613', '#d07843', '#9881d9', '#15ab83'],
} as const;

/** How many series colours a chart can show at once. */
export const SLOTS = 5;

/** The distance (OKLab, times 100) under which two colours read as the same family. */
export const NEAR = 12;

const lin = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);

/** A `#rrggbb` colour in OKLab, or null when it is anything else. */
function oklab(hex: string): [number, number, number] | null {
  const m = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex.trim());
  if (!m) return null;
  const [r, g, b] = [m[1]!, m[2]!, m[3]!].map((x) => lin(parseInt(x, 16) / 255)) as [number, number, number];
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const mm = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  return [0.2104542553 * l + 0.793617785 * mm - 0.0040720468 * s, 1.9779984951 * l - 2.428592205 * mm + 0.4505937099 * s, 0.0259040371 * l + 0.7827717662 * mm - 0.808675766 * s];
}

/** How far apart two `#rrggbb` colours are (OKLab times 100); Infinity when either is not one. */
export function distance(a: string, b: string): number {
  const x = oklab(a), y = oklab(b);
  return x && y ? 100 * Math.hypot(x[0] - y[0], x[1] - y[1], x[2] - y[2]) : Infinity;
}

/**
 * The colours the chart may use under this accent: the palette without the slots near the accent, in order. When the
 * accent leaves fewer than five, the nearest ones fill in from the farthest down, so a chart never runs out.
 */
export function chartColours(accent: string, dark: boolean): string[] {
  const all = [...PALETTE[dark ? 'dark' : 'light']];
  const far = all.filter((c) => distance(c, accent) >= NEAR);
  const near = all.filter((c) => distance(c, accent) < NEAR).sort((a, b) => distance(b, accent) - distance(a, accent));
  return [...far, ...near].slice(0, SLOTS);
}

/** The colours for what is on screen now: the accent and the theme as the page sets them. */
export function liveColours(): string[] {
  if (typeof document === 'undefined') return chartColours('', false);
  const root = document.documentElement;
  return chartColours(getComputedStyle(root).getPropertyValue('--accent'), root.dataset.theme === 'dark');
}
