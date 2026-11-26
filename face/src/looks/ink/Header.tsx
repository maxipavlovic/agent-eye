import type { HeaderProps } from '../../board/look';
import type { Project } from '../../board/model';
import { ConnState, HeaderEnd, NeedsButton, SearchField } from '../../board/HeaderParts';
import { Eye } from './Eye';

/** Brand, search, the Needs you button, connection state, the night, USAGE, the settings gear and + IDEA. */
export function Header({ store }: HeaderProps) {
  return (
    <header class="hd">
      <div class="brand"><Eye alert={store.needsYou.value > 0} /><span class="word">EYE</span></div>
      <SearchField store={store} />
      <div class="grow" />
      <NeedsButton store={store} />
      <ConnState store={store} />
      <HeaderEnd store={store} />
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
