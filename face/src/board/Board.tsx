import { useEffect, useRef } from 'preact/hooks';
import { useSignal } from '@preact/signals';
import type { Store } from '../store';
import type { Column } from '../api/types';
import { projects, tagOf } from './model';
import { KeyHelpHost } from './KeyHelp';
import { CardMenu } from './CardMenu';
import { ORDER, type BoardCtx } from './columns';
import { Detail } from './Detail';
import { Grid } from './Grid';
import { IdeaFormHost } from './IdeaForm';
import { ForgetHost } from './Forget';
import { RemindFormHost } from './Remind';
import { useKeys } from './keys';
import { Lanes } from './Lanes';
import { useLayout } from './layout';
import { Results } from './Results';
import { Toasts } from './Toasts';
import { NightHost } from './Night';
import { HoursHost } from './Hours';
import { SettingsHost } from './Settings';
import { UsageHost } from './Usage';
import { useWalk } from './walk';
import { useLook } from './look';

/** The board in the look it is wrapped in (`LookProvider`): header, lanes or grid, the panel, the forms, the menus. */
export function Board({ store }: { store: Store }) {
  const { name, Header, ProjectBar, Welcome, Loading, Footer } = useLook();
  const b = store.board.value;
  const sel = store.selected.value;
  const need = store.needsYou.value;
  // The tab (or installed app window) says how many sessions wait on you.
  // Tab and app window read the same; the app window adds "agent-eye - " by itself.
  useEffect(() => {
    const app = matchMedia('(display-mode: standalone)').matches;
    const what = need ? `${need} need you` : 'nothing waits on you';
    document.title = app ? what : `agent-eye - ${what}`;
  }, [need]);
  const { layout, folded, setLayout, setFold } = useLayout(name);
  const filter = useSignal<string | null>(null);

  // On a narrow strip Needs you sits off to the right: bring it into view once, when the board first shows.
  const first = useRef(true);
  useEffect(() => {
    if (!b || !first.current) return;
    first.current = false;
    if (innerWidth > 1100) return;
    // only the lanes scroll sideways: scrollIntoView would also scroll a page the board is embedded in (the site's demo)
    const board = document.querySelector<HTMLElement>('.board.lanes');
    const col = board?.querySelector<HTMLElement>('[data-col="needs_you"]');
    if (!board || !col) return;
    const pad = Number.parseFloat(getComputedStyle(board).scrollPaddingInlineStart) || 0;
    board.scrollLeft += col.getBoundingClientRect().left - board.getBoundingClientRect().left - pad;
  }, [!b]);

  const all = b ? b.columns.flatMap((c) => c.cards) : [];
  const ps = projects(all);
  if (filter.value !== null && !ps.some((p) => p.key === filter.value)) filter.value = null;
  const f = filter.value;
  const cols = b ? ORDER.map((id) => b.columns.find((c) => c.id === id)).filter((c): c is Column => !!c).map((c) => (f === null ? c : { ...c, cards: c.cards.filter((x) => tagOf(x, ps) === f) })) : [];
  const ctx: BoardCtx = { store, ps, showTags: f === null };
  const walk = useWalk(store, cols, folded);
  useKeys(store, walk.step);
  const searching = store.query.value.trim().length > 0;
  // Nothing on the board and nothing in the archive: a new user, who needs a pointer, not empty lanes.
  const fresh = !!b && all.length === 0 && b.archive_count === 0;

  return (
    <div class="app">
      <Header store={store} />
      <div class="strip" />
      {b && !searching && !fresh && <ProjectBar store={store} total={all.length} ps={ps} filter={f} setFilter={(k) => { filter.value = k; }} />}
      {store.error.value && <div class="err">ERROR · {store.error.value}</div>}
      {searching ? (
        <Results store={store} />
      ) : fresh ? (
        <Welcome agents={store.agents.value} />
      ) : b ? (
        layout === 'grid'
          ? <Grid cols={cols} ctx={ctx} folded={folded} setFold={setFold} />
          : <Lanes cols={cols} ctx={ctx} folded={folded} setFold={setFold} />
      ) : (
        <Loading />
      )}
      {Footer && b && !searching && !fresh && <Footer store={store} />}
      {sel && (
        <>
          <div
            class="scrim"
            role="presentation"
            onClick={() => store.close()}
            onContextMenu={(e) => {
              // Right-click beside the panel: close it and open the menu of the card under the pointer.
              e.preventDefault();
              store.close();
              const { clientX, clientY } = e;
              requestAnimationFrame(() => {
                document.elementFromPoint(clientX, clientY)?.dispatchEvent(
                  new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX, clientY }),
                );
              });
            }}
          />
          <aside class="pagewrap" key="detail" aria-label="Session detail">
            <Detail store={store} walk={walk} />
            <div class="dis" />
          </aside>
        </>
      )}
      <IdeaFormHost store={store} />
      <RemindFormHost store={store} />
      <ForgetHost store={store} />
      <UsageHost store={store} />
      <HoursHost store={store} />
      <SettingsHost store={store} layout={layout} setLayout={setLayout} folded={folded} setFold={setFold} />
      <NightHost store={store} />
      <KeyHelpHost store={store} />
      <CardMenu store={store} />
      <Toasts store={store} />
    </div>
  );
}
