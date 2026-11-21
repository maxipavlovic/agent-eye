import { Pix } from '../../pixel/Pix';
import type { ColumnId, Tokens, Topic } from '../../api/types';
import { tok, tokTitle } from '../../format';
import { EMPTY, INK, INK_CC, MARKS, SIGNAL, sigil } from './art';
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

/** Which agent runs a session: the same chip on a card and in its panel. */
export function AgentTag({ name }: { name: string }) {
  return <span class="mk agent-tag" title={`${name} session`}>{name}</span>;
}

/** Remote Control is on for a session. A plain chip on a card; with `url`, the panel's link to it, opened in a new tab. */
export function RemoteMark({ url }: { url?: string }) {
  const body = <><Pix rows={SIGNAL} pal={INK_CC} scale={2} /><span>Remote</span></>;
  return url
    ? <a class="mk rc" href={url} target="_blank" rel="noopener" title="Remote Control is on: open this session in the browser">{body}</a>
    : <span class="mk rc" title="Remote Control is on for this session">{body}</span>;
}

/** "2 loose ends": what the session left behind. A calm chip in neutral ink: the accent stays for what needs you. */
export function LooseMark({ n }: { n: number | undefined }) {
  if (!n || n < 1) return null;
  const text = `${n} loose end${n === 1 ? '' : 's'}`;
  return <span class="mk loose" title="The session left things behind: open it to see what">{text}</span>;
}
