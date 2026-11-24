import type { JSX } from 'preact';
import { AccentPicker } from '../../board/AccentPicker';
import { ideaForm } from '../../board/IdeaForm';
import type { HeaderProps, ProjectBarProps } from '../../board/look';
import { TonightChip } from '../../board/Night';
import { usagePanel } from '../../board/Usage';
import { Eye } from './Eye';

/** The look's code, as a part number on a drawing: in the header and the title block. */
export const CODE = 'IR–1';

const CONN: Record<string, string> = { live: 'LIVE', connecting: 'LINKING', offline: 'OFFLINE', fixture: 'SAMPLE' };

const ico = (body: JSX.Element) => <svg class="ico" width="20" height="20" viewBox="0 0 20 20" aria-hidden="true">{body}</svg>;
const LAYOUT_ICON = {
  lanes: () => ico(<path class="s" d="M2.25 3.25h4v13.5h-4zM8 3.25h4v13.5H8zM13.75 3.25h4v13.5h-4z" />),
  grid: () => ico(<path class="s" d="M2.75 2.75h6v6h-6zM11.25 2.75h6v6h-6zM2.75 11.25h6v6h-6zM11.25 11.25h6v6h-6z" />),
};
const THEME_ICON = {
  light: () => ico(<><circle class="s" cx="10" cy="10" r="3.75" /><path class="s" d="M10 1.75v2.5M10 15.75v2.5M1.75 10h2.5M15.75 10h2.5M4.15 4.15l1.8 1.8M14.05 14.05l1.8 1.8M4.15 15.85l1.8-1.8M14.05 5.95l1.8-1.8" /></>),
  dark: () => ico(<path class="s" d="M16.2 12.6A7 7 0 0 1 7.4 3.8a7 7 0 1 0 8.8 8.8Z" />),
  auto: () => ico(<><circle class="s" cx="10" cy="10" r="7.25" /><path d="M10 2.75a7.25 7.25 0 0 0 0 14.5Z" fill="currentColor" /></>),
};

/** One row of cells, as on a drawing: the eye and its code, search, Needs you, the connection, layout, theme, look and colour, the night, usage and + IDEA. */
export function Header({ store, layout, setLayout }: HeaderProps) {
  const need = store.needsYou.value;
  const conn = store.connection.value;
  const pref = store.themePref.value;
  return (
    <header class="hd">
      <div class="brand"><Eye alert={need > 0} /><span class="word">EYE</span><span class="code">{CODE}</span></div>
      <label class="search" title="Search (/ or ⌘K)">
        <input id="q" type="search" placeholder="Search every session" aria-label="Search every session" autocomplete="off" aria-keyshortcuts="/ Meta+K Control+K" value={store.query.value} onInput={(e) => store.search(e.currentTarget.value)} />
        <kbd>/</kbd>
      </label>
      <button
        class={`needbtn${need ? ' on' : ''}`}
        onClick={() => document.querySelector('[data-col="needs_you"]')?.scrollIntoView({ inline: 'center', block: 'nearest' })}
      >
        <span>{need ? 'NEEDS YOU' : 'ALL CLEAR'}</span><b>{need}</b>
      </button>
      <span class={`conn ${conn}`} title={`data: ${conn}`}><i />{CONN[conn] ?? conn}</span>
      <div class="theme layout" role="group" aria-label="Layout">
        {(['lanes', 'grid'] as const).map((k) => (
          <button key={k} class={layout === k ? 'on' : ''} aria-pressed={layout === k} title={k === 'lanes' ? 'Lanes' : 'Grid'} onClick={() => setLayout(k)}>
            {LAYOUT_ICON[k]()}
          </button>
        ))}
      </div>
      <div class="theme" role="group" aria-label="Theme">
        {(['light', 'dark', 'auto'] as const).map((k) => (
          <button key={k} class={pref === k ? 'on' : ''} aria-pressed={pref === k} title={k} onClick={() => store.setTheme(k)}>
            {THEME_ICON[k]()}
          </button>
        ))}
      </div>
      <AccentPicker store={store} />
      <TonightChip store={store} />
      <button class="btn usage-btn" title="What the sessions used and cost, and where the plan limits stand" aria-haspopup="dialog" onClick={() => { usagePanel.value = !usagePanel.value; }}>USAGE</button>
      <button class="btn plus" onClick={() => { ideaForm.value = {}; }}>+ IDEA</button>
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
