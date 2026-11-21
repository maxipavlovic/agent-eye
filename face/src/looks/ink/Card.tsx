import { Pix } from '../../pixel/Pix';
import type { Store } from '../../store';
import type { Card, Column } from '../../api/types';
import { ago, isBusy, nextParts, remindReason, remindShort, remindTitle } from '../../format';
import { projectLabel, tagOf } from './model';
import { cardProps } from './CardMenu';
import { ARROW, BANG, CLOCK, INK_CC } from './art';
import { KIND, type BoardCtx, type Kind } from './columns';
import { folderMissing, startButton } from './folder';
import { Hl } from './highlight';
import { AgentTag, LooseMark, RemoteMark, Sigil, Tok } from './pieces';

/** The open session is stopped until you answer in it: a due reminder or a prompt you owe (the card's border pulses, the chip blinks in the panel). */
export function asksYou(c: Card): boolean {
  const who = remindReason(c) ? 'you' : nextParts(c.next, c.agent_name).who;
  return who === 'you' && !!c.blocked;
}

/** Who acts next and what: a chip for you or the card's agent, the owner's name for anyone else; a due reminder says why it is yours. */
export function Next({ c, hot }: { c: Card; hot?: boolean }) {
  const why = remindReason(c);
  const n = why ? { who: 'you' as const, name: null, text: why } : nextParts(c.next, c.agent_name);
  if (!n.who && !n.text) return null;
  const label = n.who === 'you' ? 'YOU' : n.who === 'agent' ? c.agent_name.toUpperCase() : (n.name ?? '').toUpperCase();
  // Blinks (in the panel) while the open session is stopped, waiting for your input to go on.
  const now = asksYou(c);
  return (
    <p class="next">
      {n.who && (
        <span class={`chip ${n.who}${n.who === 'you' && hot ? ' hot' : ''}${now ? ' now' : ''}`} title={now ? `${c.agent_name} is stopped until you answer in the session` : undefined}>
          <Pix rows={ARROW} pal={INK_CC} scale={2} /><span class="lbl">{label}</span>
        </span>
      )}
      <Hl s={n.text} />
    </p>
  );
}

/** The marks on a card's top edge: a clock while a reminder is set, `!` in Needs you; the clock's title says when. */
function Tabs({ c, need, clock = true }: { c: Card; need: boolean; clock?: boolean }) {
  const title = clock ? remindTitle(c) : null;
  if (!need && !title) return null;
  return (
    <span class="tabs">
      {title && <span class="tab clock" role="img" aria-label={title} title={title}><Pix rows={CLOCK} pal={{ '#': 'var(--on-accent)' }} scale={2} /></span>}
      {need && <span class="tab"><Pix rows={BANG} pal={{ '#': 'var(--on-accent)' }} scale={2} /></span>}
    </span>
  );
}

/** A set reminder in words on the card: the accent clock and "Tomorrow 9:00", with its note on a full card.
 *  Rows and idea cards have no room for a tab beside the text, so this line is all they show of it. */
function When({ c, row }: { c: Card; row?: boolean }) {
  const title = remindTitle(c);
  const when = remindShort(c);
  if (!title || !when) return null;
  // a due reminder already says why the card is yours on its YOU line
  const note = !row && !c.remind_due && c.remind_note ? c.remind_note : null;
  if (c.remind_due && !row) return null;
  const Tag = row ? 'span' : 'p';
  return (
    <Tag class="when" title={title}>
      <span class="rclock" role="img" aria-label={title}><Pix rows={CLOCK} pal={{ '#': 'var(--accent)' }} scale={2} /></span>
      <b>{when}</b>
      {note && <span class="wn"> · {note}</span>}
    </Tag>
  );
}

/** tag: the project key to show as a tag, or null when a project filter is active. */
export function CardView({ c, store, i, kind, tag }: { c: Card; store: Store; i: number; kind: Kind; tag: string | null }) {
  const busy = isBusy(c);
  const cls = `card ${kind}${busy ? ' busy' : ''}${c.remind_at ? ' clock' : ''}${asksYou(c) ? ' ask' : ''}${store.selected.value === c.id ? ' sel' : ''}`;
  const style = `--d:${Math.min(i, 12) * 28}ms`;
  const base = { class: cls, style, 'data-id': c.id, ...cardProps(c, store) };
  const age = ago(c.last_activity_at);
  const ptag = tag !== null && <span class="ptag">{projectLabel(tag)}</span>;
  // which agent, only when there is more than one to tell apart
  const agent = store.agents.value.length > 1 && <AgentTag name={c.agent_name} />;

  if (kind === 'row') {
    return (
      <article {...base} title={c.title}>
        <Sigil id={c.id} />
        <span class="tw"><span class="t">{c.title}</span><When c={c} row /></span>
        <span class="age">{age}</span>
      </article>
    );
  }
  // an idea stays an idea card in Needs you when its reminder is due
  if (c.kind === 'idea') {
    const start = startButton(c);
    return (
      <article {...base}>
        <Tabs c={c} need={kind === 'need'} />
        <div class="top"><Sigil id={c.id} /><h3>{c.title}</h3></div>
        {folderMissing(c) && <p class="fmark" title={c.folder_note ?? undefined}>The folder is new</p>}
        <Next c={c} hot={kind === 'need'} />
        <When c={c} />
        <div class="acts">
          <button class="btn sm" disabled={!!start.blocked} title={start.blocked ?? undefined} onClick={(e) => { e.stopPropagation(); void store.startIdea(c, false); }}>{start.label}</button>
        </div>
      </article>
    );
  }
  return (
    <article {...base}>
      {busy && <i class="ants" />}
      <Tabs c={c} need={kind === 'need'} />
      <div class="meta">
        <span class="ml">
          {ptag}
          {agent}
          {c.remote_url && <RemoteMark />}
          <LooseMark n={c.loose} />
          {kind === 'run' && <span class="live"><i />LIVE</span>}
        </span>
        <span class="mr"><Tok t={c.tokens} /><span class="age">{age}</span></span>
      </div>
      <div class="top"><Sigil id={c.id} /><h3>{c.title}</h3></div>
      {kind === 'run' && c.activity && <p class="activity">{c.activity}</p>}
      {kind === 'run' && c.state && <p class="state">{c.state}</p>}
      {kind !== 'run' && <Next c={c} hot={kind === 'need'} />}
      <When c={c} />
    </article>
  );
}

/** A lane's cards, in the face of the lane. */
export function columnCards(col: Column, ctx: BoardCtx) {
  return col.cards.map((c, i) => (
    <CardView key={c.id} c={c} store={ctx.store} i={i} kind={KIND[col.id]} tag={ctx.showTags ? tagOf(c, ctx.ps) : null} />
  ));
}
