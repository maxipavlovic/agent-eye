import { Pix } from '../../pixel/Pix';
import type { HeaderProps } from '../../board/look';
import type { Project } from '../../board/model';
import { ConnState, HeaderEnd, LayoutSwitch, NeedsButton, SearchField, ThemeSwitch } from '../../board/HeaderParts';
import { AUTO, INK_CC, LAYOUT_ICON, MOON, SUN } from './art';
import { Eye } from './Eye';

const THEME_ICON = { light: SUN, dark: MOON, auto: AUTO };

/** Brand, search, the Needs you button, connection state, layout and theme switches, and + IDEA. */
export function Header({ store, layout, setLayout }: HeaderProps) {
  return (
    <header class="hd">
      <div class="brand"><Eye alert={store.needsYou.value > 0} /><span class="word">EYE</span></div>
      <SearchField store={store} />
      <div class="grow" />
      <NeedsButton store={store} />
      <ConnState store={store} />
      <LayoutSwitch layout={layout} setLayout={setLayout} icon={(k) => <Pix rows={LAYOUT_ICON[k]} pal={INK_CC} scale={2} />} />
      <ThemeSwitch store={store} icon={(k) => <Pix rows={THEME_ICON[k]} pal={INK_CC} scale={2} />} />
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
