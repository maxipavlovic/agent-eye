import type { Card } from '../../api/types';

export function projectKey(c: Card): string {
  return c.repo || c.folder || '';
}

export function projectLabel(key: string): string {
  if (!key) return 'other';
  const parts = key.split('/').filter(Boolean);
  const last = parts[parts.length - 1] ?? key;
  return last === '~' ? 'home' : last;
}

export interface Project { key: string; label: string; count: number; needs: number }

/** The five biggest projects, then "other" (key ''). */
export function projects(cards: Card[]): Project[] {
  const m = new Map<string, Project>();
  for (const c of cards) {
    const k = projectKey(c);
    const p = m.get(k) ?? { key: k, label: projectLabel(k), count: 0, needs: 0 };
    p.count++;
    if (c.column === 'needs_you') p.needs++;
    m.set(k, p);
  }
  const ranked = [...m.values()].filter((p) => p.key).sort((a, b) => b.count - a.count);
  const top = ranked.slice(0, 5);
  const keys = new Set(top.map((p) => p.key));
  const rest = [...m.values()].filter((p) => !keys.has(p.key));
  const out = [...top];
  if (rest.length) out.push({ key: '', label: 'other', count: rest.reduce((s, p) => s + p.count, 0), needs: rest.reduce((s, p) => s + p.needs, 0) });
  return out;
}

/** Which tag a card belongs to, given the displayed projects ('' = other). */
export function tagOf(c: Card, ps: Project[]): string {
  const k = projectKey(c);
  return ps.some((p) => p.key === k && p.key) ? k : '';
}
