import { Pix } from '../../pixel/Pix';
import type { Store } from '../../store';
import type { Project } from './model';
import { AccentPicker } from './AccentPicker';
import { AUTO, INK_CC, LAYOUT_ICON, MOON, SUN } from './art';
import { Eye } from './Eye';
import { ideaForm } from './IdeaForm';
import type { Layout } from './layout';

const CONN: Record<string, string> = { live: 'LIVE', connecting: 'LINKING', offline: 'OFFLINE', fixture: 'SAMPLE' };

/** Brand, search, the Needs you button, connection state, layout and theme switches, and + IDEA. */
export function Header({ store, layout, setLayout }: { store: Store; layout: Layout; setLayout: (l: Layout) => void }) {
  const need = store.needsYou.value;
  const conn = store.connection.value;
  const pref = store.themePref.value;
  return (
    <header class="hd">
      <div class="brand"><Eye alert={need > 0} /><span class="word">EYE</span></div>
      <label class="search" title="Search (/ or ⌘K)">
        <kbd>/</kbd>
        <input id="q" type="search" placeholder="Search every session" autocomplete="off" aria-keyshortcuts="/ Meta+K Control+K" value={store.query.value} onInput={(e) => store.search(e.currentTarget.value)} />
      </label>
      <div class="grow" />
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
            <Pix rows={LAYOUT_ICON[k]} pal={INK_CC} scale={2} />
          </button>
        ))}
      </div>
      <div class="theme" role="group" aria-label="Theme">
        {([['light', SUN], ['dark', MOON], ['auto', AUTO]] as const).map(([k, rows]) => (
          <button key={k} class={pref === k ? 'on' : ''} aria-pressed={pref === k} title={k} onClick={() => store.setTheme(k)}>
            <Pix rows={rows} pal={INK_CC} scale={2} />
          </button>
        ))}
      </div>
      <AccentPicker store={store} />
      <button class="btn plus" onClick={() => { ideaForm.value = {}; }}>+ IDEA</button>
    </header>
  );
}

/** One button per project (the five biggest and "other"), with how many sessions and how many need you. */
export function ProjectBar({ total, ps, filter, setFilter }: { total: number; ps: Project[]; filter: string | null; setFilter: (key: string | null) => void }) {
  return (
    <nav class="pbar" aria-label="Projects">
      <button class={filter === null ? 'on' : ''} onClick={() => setFilter(null)}>ALL <b>{total}</b></button>
      {ps.map((p) => (
        <button key={p.key || 'other'} class={filter === p.key ? 'on' : ''} onClick={() => setFilter(filter === p.key ? null : p.key)}>
          {p.label}<b>{p.count}</b>{p.needs > 0 && <em title="needs you">{p.needs}</em>}
        </button>
      ))}
    </nav>
  );
}
