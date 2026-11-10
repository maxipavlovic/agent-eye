import { Pix } from '../../pixel/Pix';
import type { Column } from '../../api/types';
import { FOLD, ICONS, INK } from './art';
import { LANE_LABEL } from './columns';

/** A lane's header: icon, count and name. Folds the lane when `fold` is given; `inline` puts the name beside the icon. */
export function Head({ col, inline, fold }: { col: Column; inline?: boolean; fold?: () => void }) {
  const s = LANE_LABEL[col.id];
  // The whole header folds the lane; the fold mark shows on hover (always on touch screens).
  const mark = fold && <span class="foldmark" aria-hidden="true"><Pix rows={FOLD} pal={INK} scale={2} /></span>;
  return (
    <div class={`chead${inline ? ' inline' : ''}${fold ? ' can-fold' : ''}`}>
      {fold && <button class="foldhit" onClick={fold} aria-label={`Fold ${col.title}`} title="Fold" />}
      <div class="r1">
        <Pix rows={ICONS[col.id]} pal={INK} scale={4} />
        {inline && <h2>{s.name}{s.sub && <small>{s.sub}</small>}</h2>}
        <span class="count" aria-label={`${col.cards.length} cards`}>{col.cards.length}</span>
      </div>
      {!inline && <div class="r2"><h2>{s.name}{s.sub && <small>{s.sub}</small>}</h2>{mark}</div>}
      {inline && mark}
    </div>
  );
}
