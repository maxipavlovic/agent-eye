import type { ComponentChildren } from 'preact';
import { Pix } from '../../pixel/Pix';
import type { Column, ColumnId } from '../../api/types';
import { ICONS, INK } from './art';
import { LANE_LABEL, type BoardCtx } from './columns';
import type { SetFold } from './layout';
import { CardView, columnCards } from './Card';
import { Head } from './Head';
import { Paused } from './Paused';
import { Empty } from './pieces';

function Box({ col, ctx, children, cls, fold }: {
  col: Column;
  ctx: BoardCtx;
  children?: ComponentChildren;
  cls?: string;
  /** Present when this box can fold: `folded` and the setter. */
  fold?: { folded: boolean; set: (v: boolean) => void };
}) {
  const s = LANE_LABEL[col.id];
  const n = col.cards.length;
  const tile = (
    <>
      <Pix rows={ICONS[col.id]} pal={INK} scale={3} />
      <span>{s.name}{s.sub && <> <small>{s.sub}</small></>}</span>
      <span class="count">{n}</span>
    </>
  );
  if (fold?.folded) {
    return (
      <section class={`box col c-${col.id} ${cls ?? ''}`} data-col={col.id} aria-label={col.title}>
        <button class="minitile folded" onClick={() => fold.set(false)} aria-label={`${col.title}, ${n} cards. Unfold`}>{tile}<span class="plus">+</span></button>
      </section>
    );
  }
  if (!n && col.id !== 'done') {
    return (
      <section class={`box col c-${col.id} ${cls ?? ''}`} data-col={col.id} aria-label={`${col.title}, no cards`}>
        <div class="minitile">{tile}</div>
      </section>
    );
  }
  return (
    <section class={`box col c-${col.id} ${cls ?? ''}`} data-col={col.id} aria-label={col.title}>
      <Head col={col} inline fold={fold ? () => fold.set(true) : undefined} />
      <div class="bbody">{n ? children ?? columnCards(col, ctx) : <Empty id={col.id} />}</div>
    </section>
  );
}

/** The board as a grid: Needs you large, the live lanes beside it, Paused and Done below. */
export function Grid({ cols, ctx, folded, setFold }: { cols: Column[]; ctx: BoardCtx; folded: ReadonlySet<ColumnId>; setFold: SetFold }) {
  const by = (id: ColumnId) => cols.find((c) => c.id === id)!;
  const fold = (id: ColumnId) => ({ folded: folded.has(id), set: (v: boolean) => setFold(id, v) });
  const done = by('done');
  const calm = by('needs_you').cards.length === 0;
  return (
    <main class="board gridview">
      <Box col={by('needs_you')} ctx={ctx} cls={`needsbox${calm ? ' wide' : ''}`}>
        <div class="ngrid">{columnCards(by('needs_you'), ctx)}</div>
      </Box>
      <div class={`side${calm ? ' wide flat' : ''}`}>
        <Box col={by('running')} ctx={ctx} fold={fold('running')} />
        <Box col={by('waiting')} ctx={ctx} fold={fold('waiting')} />
        <Box col={by('ideas')} ctx={ctx} fold={fold('ideas')} />
      </div>
      <Box col={by('paused')} ctx={ctx} cls="wide" fold={fold('paused')}><Paused col={by('paused')} ctx={ctx} grid /></Box>
      <Box col={done} ctx={ctx} cls="wide" fold={fold('done')}>
        <div class="tiles">{done.cards.map((c, i) => <CardView key={c.id} c={c} store={ctx.store} i={i} kind="row" tag={null} />)}</div>
      </Box>
    </main>
  );
}
