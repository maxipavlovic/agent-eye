import { Pix } from '../../pixel/Pix';
import type { ColumnId, Tokens, Topic } from '../../api/types';
import { tok, tokTitle } from '../../format';
import { EMPTY, INK, MARKS, sigil } from './art';
import { LANE_LABEL } from './columns';

/** The small pixel stamp that tells sessions apart. */
export function Sigil({ id, big }: { id: string; big?: boolean }) {
  return <span class={`sigil${big ? ' big' : ''}`}><Pix rows={sigil(id)} pal={INK} scale={2} /></span>;
}

/** Quiet token count; nothing when the card has none. */
export function Tok({ t }: { t: Tokens | null | undefined }) {
  if (!t) return null;
  // "1.2M" cannot be read as an age ("2d"), so a dot is enough to set them apart.
  return <span class="tok" title={tokTitle(t)}>{tok(t.total)}<i aria-hidden="true">·</i></span>;
}

/** The status mark of a topic or sub-agent. */
export function Mark({ s, scale = 2 }: { s: Topic['status']; scale?: number }) {
  return <Pix rows={MARKS[s]} pal={INK} scale={scale} label={s.replace(/_/g, ' ')} />;
}

export function Empty({ id }: { id: ColumnId }) {
  return (
    <div class="empty">
      <Pix rows={EMPTY} pal={INK} scale={4} />
      <span>{LANE_LABEL[id].empty}</span>
    </div>
  );
}
