import type { Card, Column } from '../../api/types';
import { projectLabel, tagOf } from './model';
import type { BoardCtx } from './columns';
import { CardView } from './Card';
import { Empty } from './pieces';

function groupByAge(cards: Card[]): Array<{ label: string; cards: Card[] }> {
  const day = 86400000;
  const now = Date.now();
  const g = [{ label: 'THIS WEEK', cards: [] as Card[] }, { label: 'OLDER', cards: [] as Card[] }];
  for (const c of cards) {
    const d = c.last_activity_at ? now - c.last_activity_at : Infinity;
    (d < 7 * day ? g[0]! : g[1]!).cards.push(c);
  }
  return g.filter((x) => x.cards.length);
}

/** Paused sessions as rows: by age in a lane, by project (as tiles) in the grid. */
export function Paused({ col, ctx, grid }: { col: Column; ctx: BoardCtx; grid?: boolean }) {
  if (!col.cards.length) return <Empty id="paused" />;
  if (grid) {
    const by = new Map<string, Card[]>();
    for (const c of col.cards) {
      const k = tagOf(c, ctx.ps);
      by.set(k, [...(by.get(k) ?? []), c]);
    }
    const order = ctx.ps.map((p) => p.key).filter((k) => by.has(k));
    let n = 0;
    return (
      <div class="pgroups">
        {order.map((k) => (
          <div class="pgroup" key={k}>
            <div class="ageh"><span>{projectLabel(k).toUpperCase()} · {by.get(k)!.length}</span></div>
            <div class="tiles">
              {by.get(k)!.map((c) => <CardView key={c.id} c={c} store={ctx.store} i={n++} kind="row" tag={null} />)}
            </div>
          </div>
        ))}
      </div>
    );
  }
  let n = 0;
  return (
    <>
      {groupByAge(col.cards).map((g) => (
        <div key={g.label}>
          <div class="ageh"><span>{g.label} · {g.cards.length}</span></div>
          {g.cards.map((c) => <CardView key={c.id} c={c} store={ctx.store} i={n++} kind="row" tag={null} />)}
        </div>
      ))}
    </>
  );
}
