import type { Card } from '../../api/types';
import { isBusy } from '../../format';
import { LANE_LABEL } from '../../board/columns';
import type { HeadProps } from '../../board/look';
import { LaneObject, fold as foldIcon } from './art';
import { laneFact, tickAt } from './facts';

/**
 * A lane's timeline: one tick per session, placed by when it last moved, two weeks ago at the left and
 * now at the right; what waits on you is drawn taller, in the signal colour.
 */
function Timeline({ cards, hot }: { cards: readonly Card[]; hot: boolean }) {
  const ticks = cards.map((c) => ({ id: c.id, x: tickAt(c.last_activity_at) })).filter((t): t is { id: string; x: number } => t.x !== null);
  return (
    <svg class={`tline${hot ? ' hot' : ''}`} height="18" aria-hidden="true">
      <line class="base" x1="0" x2="100%" y1="17.5" y2="17.5" />
      <line class="end" x1="0.25" x2="0.25" y1="13" y2="17.5" />
      <line class="end" x1="100%" x2="100%" y1="13" y2="17.5" />
      {ticks.map((t) => <line key={t.id} class="tk" x1={`${(t.x * 100).toFixed(2)}%`} x2={`${(t.x * 100).toFixed(2)}%`} y1={hot ? 1 : 5} y2="17.5" />)}
    </svg>
  );
}

/** A lane's header: the count, the lane's object, its name, one fact and the timeline. Folds the lane when `fold` is given; `inline` is the grid's one-line version. */
export function Head({ col, inline, fold }: HeadProps) {
  const s = LANE_LABEL[col.id];
  const n = col.cards.length;
  const mark = fold && <span class="foldmark" aria-hidden="true">{foldIcon()}</span>;
  const fact = laneFact(col.id, col.cards);
  // the reel turns while a session in the lane is working
  const busy = col.cards.some(isBusy) ? ' busy' : '';
  if (inline) {
    return (
      <div class={`chead inline${fold ? ' can-fold' : ''}${busy}`}>
        {fold && <button class="foldhit" onClick={fold} aria-label={`Fold ${col.title}`} title="Fold" />}
        <LaneObject id={col.id} px={26} />
        <h2>{s.name}<small>{fact}</small></h2>
        <span class="count" aria-label={`${n} cards`}>{n}</span>
        {mark}
      </div>
    );
  }
  return (
    <div class={`chead${fold ? ' can-fold' : ''}${busy}`}>
      {fold && <button class="foldhit" onClick={fold} aria-label={`Fold ${col.title}`} title="Fold" />}
      <div class="r1">
        <span class="count" aria-label={`${n} cards`}>{n}</span>
        <LaneObject id={col.id} px={34} />
      </div>
      <div class="r2"><h2>{s.name}</h2>{mark}</div>
      <p class="lfact" title={fact}>{fact}</p>
      <div class="tlwrap" title="One tick per session: two weeks ago on the left, now on the right">
        <Timeline cards={col.cards} hot={col.id === 'needs_you'} />
      </div>
    </div>
  );
}
