import { useEffect, useRef } from 'preact/hooks';
import { useSignal } from '@preact/signals';
import { Pix } from '../../pixel/Pix';
import type { Store } from '../../store';
import type { Column } from '../../api/types';
import { projects, tagOf } from './model';
import { CardMenu } from './CardMenu';
import { EMPTY, INK } from './art';
import { ORDER, type BoardCtx } from './columns';
import { Detail } from './Detail';
import { Grid } from './Grid';
import { Header, ProjectBar } from './Header';
import { IdeaFormHost } from './IdeaForm';
import { RemindFormHost } from './Remind';
import { useKeys } from './keys';
import { Lanes } from './Lanes';
import { useLayout } from './layout';
import { Results } from './Results';
import { Toasts } from './Toasts';
import { useWalk } from './walk';
import { Welcome } from './Welcome';

export function Root({ store }: { store: Store }) {
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
  const { layout, folded, setLayout, setFold } = useLayout();
  const filter = useSignal<string | null>(null);

  // On a narrow strip Needs you sits off to the right: bring it into view once, when the board first shows.
  const first = useRef(true);
  useEffect(() => {
    if (!b || !first.current) return;
    first.current = false;
    if (innerWidth <= 1100) document.querySelector('.board.lanes [data-col="needs_you"]')?.scrollIntoView({ inline: 'start', block: 'nearest' });
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
      <Header store={store} layout={layout} setLayout={setLayout} />
      <div class="strip" />
      {b && !searching && !fresh && <ProjectBar total={all.length} ps={ps} filter={f} setFilter={(k) => { filter.value = k; }} />}
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
        <div class="loading"><Pix rows={EMPTY} pal={INK} scale={8} /><span>LOADING<i class="dots" /></span></div>
      )}
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
      <CardMenu store={store} />
      <Toasts store={store} />
    </div>
  );
}
