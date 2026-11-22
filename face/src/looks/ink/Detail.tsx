import { useEffect, useRef } from 'preact/hooks';
import { useSignal } from '@preact/signals';
import type { Store } from '../../store';
import type { CardDetail, ColumnId } from '../../api/types';
import { ago, isBusy, resumeLabel, snippetParts, tok } from '../../format';
import { LANE_LABEL } from './columns';
import { copy } from './copy';
import { folderMissing, startButton } from './folder';
import { Conversation } from './Conversation';
import { openForget } from './Forget';
import { Hl, HlWords, hlPattern } from './highlight';
import { ideaForm } from './IdeaForm';
import { keyHelp } from './KeyHelp';
import { hideSession, QuickMove } from './Move';
import { Next } from './Card';
import { Leftovers } from './Loose';
import { canContinueTonight, ContinueTonight, NightRunLine, NightRunSection, TonightIdea, tonightOpen } from './Night';
import { openRemind, RemindPanel, RemindRow } from './Remind';
import { originChip, RemoteMark, Sigil } from './pieces';
import { Fold, Kids, Outputs, Running, Sec, TokenSec, Topics, Where } from './Sections';
import { projectKey, projectLabel } from './model';
import type { Walk } from './walk';

/** The torn-edge outline of the resume receipt. */
const ZIG = (() => {
  const n = 36;
  const pts: string[] = [];
  for (let i = 0; i <= n; i++) pts.push(`${((i * 100) / n).toFixed(3)}% ${i % 2 ? 0 : 8}px`);
  for (let i = n; i >= 0; i--) pts.push(`${((i * 100) / n).toFixed(3)}% calc(100% - ${i % 2 ? 0 : 8}px)`);
  return `polygon(${pts.join(',')})`;
})();

/** An idea's panel: start it, change it, and what it keeps (the notes stay here; only the prompt is sent). */
function IdeaBody({ d, store }: { d: CardDetail; store: Store }) {
  const edit = (focus?: 'prompt') => { ideaForm.value = { edit: d.id, focus }; };
  const start = startButton(d);
  return (
    <>
      <div class="acts">
        <button class="btn big" disabled={!!start.blocked} title={start.blocked ?? undefined} onClick={() => void store.startIdea(d, false)}>{start.label}</button>
        <button class="btn" disabled={!!start.blocked} title={d.prompt ? `${d.agent_name} works on it without you` : 'Needs a prompt first'} onClick={() => (d.prompt ? void store.startIdea(d, true) : edit('prompt'))}>IN BACKGROUND</button>
        <button class="btn" onClick={() => edit()}>EDIT</button>
      </div>
      {store.night.value && <TonightIdea d={d} store={store} />}
      {(start.blocked || folderMissing(d)) && <p class="fnote" role="status">{start.blocked ?? d.folder_note}</p>}
      <div class="acts">
        <button class="btn ghost" onClick={() => { void store.move(d.id, 'archive').then(() => store.close()); }}>ARCHIVE</button>
      </div>
      <Sec t="NOTES">{d.notes ? <p class="notes"><Hl s={d.notes} /></p> : <p class="dim">None.</p>}</Sec>
      <Sec t={`PROMPT FOR ${d.agent_name.toUpperCase()}`}>
        {d.prompt ? <blockquote class="prompt first"><Hl s={d.prompt} /></blockquote> : <p class="dim">None: START opens {d.agent_name} in the folder and waits for you.</p>}
      </Sec>
    </>
  );
}

function WalkNav({ walk }: { walk: Walk }) {
  if (walk.total <= 1) return null;
  return (
    <nav class="pnav" aria-label="Walk the board">
      <button onClick={() => walk.step(-1)} aria-label="Previous card (←)" title="Previous card (←)">‹</button>
      <span>{walk.at < 0 ? '–' : walk.at + 1} / {walk.total}</span>
      <button onClick={() => walk.step(1)} aria-label="Next card (→)" title="Next card (→)">›</button>
    </nav>
  );
}

/** The command that resumes the session, printed like a till receipt, folded with the details. */
function Receipt({ d }: { d: CardDetail }) {
  if (!d.resumable || !d.resume_cmd) return null;
  const { label, via } = resumeLabel(d.resume_mode, d.resume_terminal);
  return (
    <Fold name="resume-command" t="RESUME COMMAND" text={d.resume_cmd}>
      <div class="receipt" style={`clip-path:${ZIG}`}>
        <div class="rt"><span>RESUME</span><span>{label}{via && <small>{via}</small>}</span></div>
        <code>{d.resume_cmd}</code>
        <div class="barcode" aria-hidden="true" />
      </div>
    </Fold>
  );
}

/** Save the session as one readable page (E, and EXPORT in MORE): secrets are masked. */
export function exportSession(id: string) {
  const a = document.createElement('a');
  a.href = `/api/sessions/${encodeURIComponent(id)}/export?format=html`;
  a.download = '';
  document.body.appendChild(a);
  a.click();
  a.remove();
}

/** One action in MORE, with the key that does the same. */
interface MoreItem { label: string; key?: string; title: string; run: () => void }

/** What a session can do besides moving and RESUME, each with its key: MORE opens them under the action row. */
function moreItems(d: CardDetail, store: Store): MoreItem[] {
  const out: MoreItem[] = [];
  if (d.resumable && d.resume_cmd) out.push({ label: 'FORK', key: 'F', title: 'Resume as a new session branched from this one', run: () => void store.resume(d.id, true) });
  if (d.resume_cmd) out.push({ label: 'COPY RESUME COMMAND', key: 'C', title: 'Copy the command that resumes it', run: () => copy(d.resume_cmd!, store) });
  out.push({ label: 'EXPORT', key: 'E', title: 'Save this session as one readable page; secrets are masked', run: () => exportSession(d.id) });
  if (canContinueTonight(d, store)) out.push({ label: 'CONTINUE TONIGHT…', key: 'T', title: 'Fork this session tonight, in its own worktree, with a prompt you write now', run: () => { tonightOpen.value = d.id; } });
  out.push({ label: 'REMIND ME…', key: 'M', title: 'Pick when eye moves it to Needs you', run: () => openRemind(d.id) });
  out.push({ label: 'HIDE', key: 'H', title: 'Hide this session from the board', run: () => hideSession(store, d.id) });
  out.push({ label: 'FORGET…', key: 'X', title: 'Remove this session from eye, and optionally from its agent', run: () => openForget(d) });
  out.push({ label: 'ARCHIVE', title: 'Out of the lanes, into the archive', run: () => { void store.move(d.id, 'archive').then(() => store.close()); } });
  return out;
}

/** MORE ▾: the session's other actions in a small menu beside the moves; a click elsewhere, or Esc, closes it. */
function More({ d, store }: { d: CardDetail; store: Store }) {
  const open = useSignal(false);
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open.value) return;
    const away = (e: PointerEvent) => { if (!box.current?.contains(e.target as Node)) open.value = false; };
    const esc = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.stopPropagation(); open.value = false; } };
    addEventListener('pointerdown', away, true);
    addEventListener('keydown', esc, true);
    box.current?.querySelector<HTMLElement>('[role=menuitem]')?.focus({ preventScroll: true });
    return () => { removeEventListener('pointerdown', away, true); removeEventListener('keydown', esc, true); };
  }, [open.value]);
  return (
    <div class="move more" ref={box}>
      <button class="btn" aria-haspopup="menu" aria-expanded={open.value} onClick={() => (open.value = !open.value)}>MORE ▾</button>
      {open.value && (
        <ul class="menu" role="menu" aria-label="More actions">
          {moreItems(d, store).map((m) => (
            <li key={m.label}>
              <button role="menuitem" title={m.title} onClick={() => { open.value = false; m.run(); }}>
                <span>{m.label}</span>{m.key && <kbd>{m.key}</kbd>}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** The side panel of one card: its state, outputs, conversation and the actions on it. */
export function Detail({ store, walk }: { store: Store; walk: Walk }) {
  const d = store.detail.value;
  const id = store.selected.value!;
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => { ref.current?.scrollTo(0, 0); }, [id]);
  // a CONTINUE TONIGHT form open for this card closes with it
  useEffect(() => () => { if (tonightOpen.peek() === id) tonightOpen.value = null; }, [id]);
  if (!d) {
    return <div class="page ld" ref={ref}><button class="x" onClick={() => store.close()}>ESC</button><p class="state">Loading…<i class="dots" /></p></div>;
  }
  const colId = d.column as ColumnId;
  const need = colId === 'needs_you';
  const kids = d.streams.filter((s) => s.kind === 'child');
  const prompts = d.prompts.slice(-5).reverse();
  const session = d.kind !== 'idea';
  // Opened from a search: mark the search words, and show the matching snippet at the top.
  const q = store.query.value.trim();
  const re = q ? hlPattern(q) : null;
  const snip = q ? store.results.value?.find((h) => h.card.id === d.id)?.snippet ?? null : null;
  // the facts in one line of plain words: project, agent (when there is more than one), where it was started, age, kind, tokens
  const proj = projectKey(d);
  const facts = [
    proj ? projectLabel(proj) : null,
    store.agents.value.length > 1 ? d.agent_name : null,
    originChip(d.origin),
    ago(d.last_activity_at),
    d.kind.replace('_', ' '),
    d.tokens ? `${tok(d.tokens.total)} tokens` : null,
  ].filter((x): x is string => !!x);
  return (
    <HlWords.Provider value={re}>
      <div class="page" ref={ref}>
        <button class="x" onClick={() => store.close()} aria-label="Close (Esc)">ESC ✕</button>
        <button class="x qk" onClick={() => { keyHelp.value = true; }} aria-label="Keyboard shortcuts (?)" title="Keyboard shortcuts (?)">?</button>
        <WalkNav walk={walk} />
        <header class="dh">
          <Sigil id={d.id} big />
          <div>
            <h2><Hl s={d.title} /></h2>
            <p class="meta">
              {!session && <span class={`tag${need ? ' hot' : ''}`}>{LANE_LABEL[colId]?.name ?? colId}</span>}
              {isBusy(d) && <span class="tag live">LIVE</span>}
              {d.manual && <span class="tag">PINNED</span>}
              <span class="facts">{facts.join(' · ')}</span>
              {d.remote_url && <RemoteMark url={d.remote_url} />}
            </p>
          </div>
        </header>
        {/* 1. now: what to do, the next step, and the forms you open (continue tonight, remind) */}
        {session && <div class="dbar"><QuickMove d={d} store={store} /><More d={d} store={store} /></div>}
        <RemindRow c={d} store={store} />
        {/* opened from the menu, T or MORE: at the top, where nothing that loads later can push it away */}
        <ContinueTonight d={d} store={store} />
        {snip && <p class="match"><b>MATCH</b><span>{snippetParts(snip).map((p, j) => (p.hit ? <mark key={j}>{p.text}</mark> : p.text))}</span></p>}
        {session && d.state && <p class="lead"><Hl s={d.state} /></p>}
        <Next c={d} hot={need} />
        <NightRunLine d={d} />
        <Leftovers key={d.id} d={d} store={store} />
        <Running d={d} />
        {/* 2. what happened */}
        {d.kind === 'idea' && d.idea_id != null && <IdeaBody d={d} store={store} />}
        {d.recap && <Sec t="RECAP"><p class="recap"><Hl s={d.recap} /></p></Sec>}
        {d.idea?.notes && <Sec t="IDEA NOTES"><p class="notes"><Hl s={d.idea.notes} /></p></Sec>}
        {d.topics.length > 0 && <Topics topics={d.topics} />}
        {d.child_count > 0 && (
          <Sec t="SUB-AGENTS" n={`${d.child_done}/${d.child_count}`}>
            <Kids key={d.id} kids={kids} total={d.child_count} />
          </Sec>
        )}
        <Outputs key={d.id} outputs={d.outputs} />
        {/* 3. the details, folded: one line each until you open it (eye remembers which) */}
        <div class="details">
          <NightRunSection d={d} store={store} />
          {d.first_prompt && <Fold name="first-prompt" t="FIRST PROMPT" text={d.first_prompt}><blockquote class="prompt first"><Hl s={d.first_prompt} /></blockquote></Fold>}
          {session && <Conversation store={store} id={d.id} agent={d.agent_name} fallback={prompts} />}
          {d.tokens && <TokenSec key={d.id} t={d.tokens} />}
          <Where d={d} />
          <RemindPanel c={d} store={store} />
          <Receipt d={d} />
        </div>
      </div>
    </HlWords.Provider>
  );
}
