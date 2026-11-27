import type { Store } from '../store';
import type { Card, Column } from '../api/types';
import { ago, isBusy, nextParts, remindReason, remindShort, remindTitle } from '../format';
import { projectLabel, tagOf } from './model';
import { cardProps } from './CardMenu';
import { useArt } from './look';
import { KIND, type BoardCtx, type Kind } from './columns';
import { folderMissing, startButton } from './folder';
import { hintFor } from './hint';
import { Hl } from './highlight';
import { nightAsk } from './Night';
import { originChip, Sigil } from './pieces';

/** The open session is stopped until you answer in it: a due reminder or a prompt you owe (the card's border pulses, the chip blinks in the panel). */
export function asksYou(c: Card): boolean {
  const who = remindReason(c) ? 'you' : nextParts(c.next, c.agent_name).who;
  return who === 'you' && !!c.blocked;
}

/** Who acts next and what: a chip for you or the card's agent, the owner's name for anyone else; a due reminder says why it is yours. */
export function Next({ c, hot }: { c: Card; hot?: boolean }) {
  const art = useArt();
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
          {art.icon('arrow')}<span class="lbl">{label}</span>
        </span>
      )}
      <Hl s={n.text} />
    </p>
  );
}

/** One mark on a card's top edge: a small box with an icon or a number, and its words in the title. */
export interface CardMark { kind: 'need' | 'hours' | 'clock' | 'night' | 'loose' | 'rc'; title: string }

/** At most this many boxes on a card's top edge; more show as the first ones and a `+N` box. */
export const MAX_MARKS = 4;

/**
 * The states of a card, most important first: Needs you, the clock's hint (`hint`, only for the card you work in),
 * a reminder, a night run, leftovers, Remote Control.
 * They are the card's only place for states, so its frame never changes: a new state becomes a mark here
 * (or goes to the panel), never a row or a chip on the card.
 */
export function cardMarks(c: Card, need: boolean, hint: string | null = null): CardMark[] {
  const out: CardMark[] = [];
  if (need) out.push({ kind: 'need', title: 'Needs you' });
  if (hint) out.push({ kind: 'hours', title: hint });
  const when = remindTitle(c);
  if (when) out.push({ kind: 'clock', title: when });
  if (c.kind === 'idea') return out;
  if (c.night) out.push({ kind: 'night', title: 'Made by a night-shift run: open it to review' });
  if (c.loose) out.push({ kind: 'loose', title: `${c.loose} ${c.loose === 1 ? 'leftover' : 'leftovers'}: open the card to see what the session left behind` });
  if (c.remote_url) out.push({ kind: 'rc', title: 'Remote Control is on for this session' });
  return out;
}

/** The boxes shown for these marks: all of them when they fit, else the first ones and `+N` for the rest. */
export function shownMarks(marks: CardMark[]): { shown: CardMark[]; more: number } {
  if (marks.length <= MAX_MARKS) return { shown: marks, more: 0 };
  return { shown: marks.slice(0, MAX_MARKS - 1), more: marks.length - (MAX_MARKS - 1) };
}

/** The width of a box on the top edge, and of the gap after it: a box with two characters ("9+", "+2") is wider. */
const BOX = 26, WIDE = 36, GAP = 4;

/** The leftovers count as a box shows it. */
const looseText = (n: number) => (n > 9 ? '9+' : String(n));

function MarkBox({ m, n }: { m: CardMark; n?: number }) {
  const art = useArt();
  const text = m.kind === 'loose' ? looseText(n!) : null;
  const body = m.kind === 'need' ? art.icon('bang')
    : m.kind === 'clock' ? art.icon('clock')
      : m.kind === 'night' ? art.icon('crescent')
        : m.kind === 'rc' ? art.icon('signal')
          : m.kind === 'hours' ? art.icon('sunset')
            : <span class="tn">{text}</span>;
  return <span class={`tab ${m.kind}${text && text.length > 1 ? ' wide' : ''}`} role="img" aria-label={m.title} title={m.title}>{body}</span>;
}

/** The marks on a card's top edge, the most important at the right end. */
function Tabs({ c, marks }: { c: Card; marks: CardMark[] }) {
  if (!marks.length) return null;
  const { shown, more } = shownMarks(marks);
  const rest = marks.slice(shown.length).map((m) => m.title).join('; ');
  return (
    <span class="tabs">
      {more > 0 && <span class="tab more wide" role="img" aria-label={rest} title={rest}><span class="tn">+{more}</span></span>}
      {[...shown].reverse().map((m) => <MarkBox key={m.kind} m={m} n={m.kind === 'loose' ? c.loose : undefined} />)}
    </span>
  );
}

/** How much of the top edge the marks take (px), so the top line keeps clear of them. */
export function marksWidth(c: Card, marks: CardMark[]): number {
  const { shown, more } = shownMarks(marks);
  const w = (m: CardMark) => (m.kind === 'loose' && looseText(c.loose ?? 0).length > 1 ? WIDE : BOX) + GAP;
  return shown.reduce((a, m) => a + w(m), 0) + (more > 0 ? WIDE + GAP : 0);
}

/** A set reminder in words on the card: the accent clock and "Tomorrow 9:00", with its note on a full card.
 *  Rows and idea cards have no room for a tab beside the text, so this line is all they show of it. */
function When({ c, row }: { c: Card; row?: boolean }) {
  const art = useArt();
  const title = remindTitle(c);
  const when = remindShort(c);
  if (!title || !when) return null;
  // a due reminder already says why the card is yours on its YOU line
  const note = !row && !c.remind_due && c.remind_note ? c.remind_note : null;
  if (c.remind_due && !row) return null;
  const Tag = row ? 'span' : 'p';
  return (
    <Tag class="when" title={title}>
      <span class="rclock" role="img" aria-label={title}>{art.icon('clock-accent')}</span>
      <b>{when}</b>
      {note && <span class="wn"> · {note}</span>}
    </Tag>
  );
}

/** tag: the project key to show as a tag, or null when a project filter is active. */
export function CardView({ c, store, i, kind, tag }: { c: Card; store: Store; i: number; kind: Kind; tag: string | null }) {
  const busy = isBusy(c);
  const marks = kind === 'row' ? [] : cardMarks(c, kind === 'need', hintFor(c, store.selected.value, store.clockHint.value?.text ?? null));
  const cls = `card ${kind}${busy ? ' busy' : ''}${c.remind_at ? ' clock' : ''}${asksYou(c) ? ' ask' : ''}${store.selected.value === c.id ? ' sel' : ''}`;
  const style = `--d:${Math.min(i, 12) * 28}ms;--tabw:${marksWidth(c, marks)}px`;
  const base = { class: cls, style, 'data-id': c.id, ...cardProps(c, store) };
  const age = ago(c.last_activity_at);
  // the top line, in plain words: the project, the agent (only when there is more than one to tell
  // apart) and where it was started (not for a night run's card: its mark says so)
  const origin = c.night && c.origin === 'night' ? null : originChip(c.origin);
  const facts = [store.agents.value.length > 1 ? c.agent_name : null, origin].filter((x): x is string => !!x);

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
        <Tabs c={c} marks={marks} />
        <div class="top"><Sigil id={c.id} /><h3>{c.title}</h3></div>
        {folderMissing(c) && <p class="fmark" title={c.folder_note ?? undefined}>The folder is new</p>}
        <Next c={c} hot={kind === 'need'} />
        <When c={c} />
        <div class="acts">
          <button class="btn sm" disabled={!!start.blocked} title={start.blocked ?? undefined} onClick={(e) => { e.stopPropagation(); void store.startIdea(c, false); }}>{start.label}</button>
          {store.night.value && <button class="btn sm ghost" title="Run it while you sleep, in its own worktree" onClick={(e) => { e.stopPropagation(); nightAsk.value = c.id; store.open(c.id); }}>TONIGHT</button>}
        </div>
      </article>
    );
  }
  return (
    <article {...base}>
      {busy && <i class="ants" />}
      <Tabs c={c} marks={marks} />
      <div class="meta">
        <span class="ml">
          {tag !== null && <b class="ptag">{projectLabel(tag)}</b>}
          {facts.map((f) => <span key={f} class="mf">{f}</span>)}
        </span>
        <span class="age">{age}</span>
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
