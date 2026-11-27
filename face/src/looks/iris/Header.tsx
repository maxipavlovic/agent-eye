import type { HeaderProps, ProjectBarProps } from '../../board/look';
import { ConnState, HeaderEnd, NeedsButton, SearchField } from '../../board/HeaderParts';
import { Eye } from './Eye';

/** The look's code, as a part number on a drawing: in the header and the title block. */
export const CODE = 'IR–1';

/** One row of cells, as on a drawing: the eye and its code, search, Needs you, the connection, the night, usage, HOURS (with its hint), the settings gear and + IDEA. */
export function Header({ store }: HeaderProps) {
  return (
    <header class="hd">
      <div class="brand"><Eye alert={store.needsYou.value > 0} /><span class="word">EYE</span><span class="code">{CODE}</span></div>
      <SearchField store={store} keyAfter />
      <NeedsButton store={store} />
      <ConnState store={store} />
      <HeaderEnd store={store} />
    </header>
  );
}

/** A project's hue on the bar (one of six muted ones), the same for a project every day: from its name. */
function hue(key: string): number {
  let h = 2166136261;
  for (const ch of key) { h ^= ch.charCodeAt(0); h = Math.imul(h, 16777619); }
  return (h >>> 0) % 6;
}

/**
 * The projects (the five biggest and "other"), each hanging from the header's rule by a tick, with a strip
 * under it: its length is how many sessions, the part in the signal colour is how many need you.
 */
export function ProjectBar({ total, ps, filter, setFilter }: ProjectBarProps) {
  const most = Math.max(1, ...ps.map((p) => p.count));
  const strip = (count: number, needs: number, scale: number) => (
    <span class="pstrip" aria-hidden="true">
      <i style={`width:${((count - needs) * scale).toFixed(1)}px`} />
      {needs > 0 && <i class="y" style={`width:${(needs * scale).toFixed(1)}px`} />}
    </span>
  );
  const needs = ps.reduce((s, p) => s + p.needs, 0);
  return (
    <nav class="pbar" aria-label="Projects">
      <button class={filter === null ? 'on' : ''} aria-pressed={filter === null} onClick={() => setFilter(null)}>
        <span class="pname">ALL <b>{total}</b>{needs > 0 && <em title="needs you">{needs}</em>}</span>
        {strip(total, needs, 96 / Math.max(total, 1))}
      </button>
      {ps.map((p) => (
        <button key={p.key || 'other'} class={`ph${p.key ? hue(p.key) : 'x'}${filter === p.key ? ' on' : ''}`} aria-pressed={filter === p.key} onClick={() => setFilter(filter === p.key ? null : p.key)}>
          <span class="pname">{p.label}<b>{p.count}</b>{p.needs > 0 && <em title="needs you">{p.needs}</em>}</span>
          {strip(p.count, p.needs, 96 / most)}
        </button>
      ))}
    </nav>
  );
}
