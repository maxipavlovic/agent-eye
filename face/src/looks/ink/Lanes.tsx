import { Pix } from '../../pixel/Pix';
import type { Column, ColumnId } from '../../api/types';
import { ICONS, INK } from './art';
import { FOLDABLE, LANE_LABEL, type BoardCtx } from './columns';
import type { SetFold } from './layout';
import { columnCards } from './Card';
import { Head } from './Head';
import { Paused } from './Paused';
import { Empty } from './pieces';

/**
 * A slim rail: a lane folded by hand (`open` unfolds it), or an empty lane, which is a lane again
 * as soon as a card arrives.
 */
function Rail({ col, open }: { col: Column; open?: () => void }) {
  const s = LANE_LABEL[col.id];
  const n = col.cards.length;
  const body = (
    <>
      <Pix rows={ICONS[col.id]} pal={INK} scale={3} />
      <span class="count">{n}</span>
      <span class="vl">{s.name}{open ? ' +' : ''}</span>
    </>
  );
  return open
    ? <button class={`rail folded c-${col.id}`} data-col={col.id} onClick={open} aria-label={`${col.title}, ${n} cards. Unfold`}>{body}</button>
    : <section class={`rail c-${col.id}`} data-col={col.id} aria-label={`${col.title}, no cards`}>{body}</section>;
}

function Lane({ col, ctx, fold }: { col: Column; ctx: BoardCtx; fold?: () => void }) {
  const id = col.id;
  return (
    <section class={`col c-${id}`} data-col={id} aria-label={col.title}>
      <Head col={col} fold={fold} />
      <div class="cbody">
        {col.cards.length === 0 && <Empty id={id} />}
        {id === 'paused' ? <Paused col={col} ctx={ctx} /> : columnCards(col, ctx)}
      </div>
    </section>
  );
}

/** The board as side-by-side lanes. */
export function Lanes({ cols, ctx, folded, setFold }: { cols: Column[]; ctx: BoardCtx; folded: ReadonlySet<ColumnId>; setFold: SetFold }) {
  return (
    <main class="board lanes">
      {cols.map((c) =>
        FOLDABLE.has(c.id) && folded.has(c.id)
          ? <Rail key={c.id} col={c} open={() => setFold(c.id, false)} />
          : c.cards.length === 0 && c.id !== 'done'
            ? <Rail key={c.id} col={c} />
            : <Lane key={c.id} col={c} ctx={ctx} fold={FOLDABLE.has(c.id) ? () => setFold(c.id, true) : undefined} />)}
    </main>
  );
}
