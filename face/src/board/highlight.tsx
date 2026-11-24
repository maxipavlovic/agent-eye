import { createContext } from 'preact';
import { useContext } from 'preact/hooks';

/** Words of the search that opened the detail panel; null elsewhere. */
export const HlWords = createContext<RegExp | null>(null);

/** Search words to highlight: 2+ characters, matched anywhere, case-insensitive. */
export function hlPattern(query: string): RegExp | null {
  const words = query.toLowerCase().split(/\s+/).filter((w) => w.length >= 2);
  if (!words.length) return null;
  return new RegExp(`(${words.map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('|')})`, 'gi');
}

/** Text with the search words marked, when the panel was opened from a search. */
export function Hl({ s }: { s: string }) {
  const re = useContext(HlWords);
  if (!re || !s) return <>{s}</>;
  return <>{s.split(re).map((part, i) => (i % 2 ? <mark key={i}>{part}</mark> : part))}</>;
}
