import { Pix } from '../../pixel/Pix';
import type { Store } from '../../store';
import type { Card, Column } from '../../api/types';
import { ago, isBusy, nextParts } from '../../format';
import { projectLabel, tagOf } from './model';
import { cardProps } from './CardMenu';
import { ARROW, BANG, INK_CC } from './art';
import { KIND, type BoardCtx, type Kind } from './columns';
import { folderMissing, startButton } from './folder';
import { Hl } from './highlight';
import { Sigil, Tok } from './pieces';

/** Who acts next and what: a chip for you or the card's agent, the owner's name for anyone else. */
export function Next({ c, hot }: { c: Card; hot?: boolean }) {
  const n = nextParts(c.next, c.agent_name);
  if (!n.who && !n.text) return null;
  const label = n.who === 'you' ? 'YOU' : n.who === 'agent' ? c.agent_name.toUpperCase() : (n.name ?? '').toUpperCase();
  // Blinks while the open session is stopped, waiting for your input to go on.
  const now = n.who === 'you' && c.blocked;
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

/** tag: the project key to show as a tag, or null when a project filter is active. */
export function CardView({ c, store, i, kind, tag }: { c: Card; store: Store; i: number; kind: Kind; tag: string | null }) {
  const busy = isBusy(c);
  const cls = `card ${kind}${busy ? ' busy' : ''}${store.selected.value === c.id ? ' sel' : ''}`;
  const style = `--d:${Math.min(i, 12) * 28}ms`;
  const base = { class: cls, style, 'data-id': c.id, ...cardProps(c, store) };
  const age = ago(c.last_activity_at);
  const ptag = tag !== null && <span class="ptag">{projectLabel(tag)}</span>;
  // which agent, only when there is more than one to tell apart
  const agent = store.agents.value.length > 1 && <span class="agent-tag" title={`${c.agent_name} session`}>{c.agent_name}</span>;

  if (kind === 'row') {
    return (
      <article {...base} title={c.title}>
        <Sigil id={c.id} />
        <span class="t">{c.title}</span>
        <span class="age">{age}</span>
      </article>
    );
  }
  if (kind === 'idea') {
    const start = startButton(c);
    return (
      <article {...base}>
        <div class="top"><Sigil id={c.id} /><h3>{c.title}</h3></div>
        {folderMissing(c) && <p class="fmark" title={c.folder_note ?? undefined}>The folder is new</p>}
        <div class="acts">
          <button class="btn sm" disabled={!!start.blocked} title={start.blocked ?? undefined} onClick={(e) => { e.stopPropagation(); void store.startIdea(c, false); }}>{start.label}</button>
        </div>
      </article>
    );
  }
  return (
    <article {...base}>
      {busy && <i class="ants" />}
      {kind === 'need' && <span class="tab"><Pix rows={BANG} pal={{ '#': 'var(--on-accent)' }} scale={2} /></span>}
      <div class="meta">
        {ptag}
        {agent}
        <span class="sp" />
        {kind === 'run' && <span class="live"><i />LIVE</span>}
        <Tok t={c.tokens} />
        <span class="age">{age}</span>
      </div>
      <div class="top"><Sigil id={c.id} /><h3>{c.title}</h3></div>
      {kind === 'run' && c.state && <p class="state">{c.state}</p>}
      {kind !== 'run' && <Next c={c} hot={kind === 'need'} />}
    </article>
  );
}

/** A lane's cards, in the face of the lane. */
export function columnCards(col: Column, ctx: BoardCtx) {
  return col.cards.map((c, i) => (
    <CardView key={c.id} c={c} store={ctx.store} i={i} kind={KIND[col.id]} tag={ctx.showTags ? tagOf(c, ctx.ps) : null} />
  ));
}
