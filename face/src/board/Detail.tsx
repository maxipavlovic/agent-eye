import { useEffect, useLayoutEffect, useRef } from 'preact/hooks';
import { useSignal } from '@preact/signals';
import type { Store } from '../store';
import type { CardDetail, ColumnId } from '../api/types';
import { ago, isBusy, resumeLabel, snippetParts, tok } from '../format';
import { LANE_LABEL } from './columns';
import { copy } from './copy';
import { exportSession } from './save';
import { folderMissing, startButton } from './folder';
import { Conversation } from './Conversation';
import { openForget } from './Forget';
import { Hl, HlWords, hlPattern } from './highlight';
import { ideaForm } from './IdeaForm';
import { keyHelp } from './KeyHelp';
import { hideSession, Keyed, MoveMenu, QuickMove } from './Move';
import { Next } from './Card';
import { Leftovers } from './Loose';
import { tonightWhy } from './hint';
import { canContinueTonight, ContinueTonight, NightRunLine, NightRunSection, TonightButton, TonightIdea, TonightIdeaButton, tonightOpen } from './Night';
import { openRemind, RemindPanel, RemindRow } from './Remind';
import { originChip, RemoteMark, Sigil } from './pieces';
import { Kids, Outputs, Running, Sec, TokenSec, Topics, Where } from './Sections';
import { projectKey, projectLabel } from './model';
import type { Walk } from './walk';

/** The torn-edge outline of the resume receipt, as `--zig` for a look that draws it (ink). */
const ZIG = (() => {
  const n = 36;
  const pts: string[] = [];
  for (let i = 0; i <= n; i++) pts.push(`${((i * 100) / n).toFixed(3)}% ${i % 2 ? 0 : 8}px`);
  for (let i = n; i >= 0; i--) pts.push(`${((i * 100) / n).toFixed(3)}% calc(100% - ${i % 2 ? 0 : 8}px)`);
  return `polygon(${pts.join(',')})`;
})();

/** What an idea's START, IN BACKGROUND and EDIT do; S, B and E do the same. False: nothing to do (the folder cannot be used). */
export function ideaAct(store: Store, d: CardDetail, what: 'start' | 'background' | 'edit'): boolean {
  if (what === 'edit') { ideaForm.value = { edit: d.id }; return true; }
  if (startButton(d).blocked) return false;
  // in the background it needs a prompt: without one, the form opens on it
  if (what === 'background' && !d.prompt) ideaForm.value = { edit: d.id, focus: 'prompt' };
  else void store.startIdea(d, what === 'background');
  return true;
}

/** An idea's panel: start it, change it, and what it keeps (the notes stay here; only the prompt is sent). */
function IdeaBody({ d, store }: { d: CardDetail; store: Store }) {
  const start = startButton(d);
  return (
    <>
      {/* one row; REMIND ME and ARCHIVE are in MORE */}
      <div class="acts">
        <button class="btn big" disabled={!!start.blocked} aria-keyshortcuts="S" title={start.blocked ?? 'Start it now (S)'} onClick={() => ideaAct(store, d, 'start')}><Keyed label={start.label} k="s" /></button>
        <button class="btn" disabled={!!start.blocked} aria-keyshortcuts="B" title={d.prompt ? `${d.agent_name} works on it without you (B)` : 'Needs a prompt first (B)'} onClick={() => ideaAct(store, d, 'background')}><Keyed label="IN BACKGROUND" k="b" /></button>
        <TonightIdeaButton d={d} store={store} />
        <button class="btn" aria-keyshortcuts="E" title="Edit the idea (E)" onClick={() => ideaAct(store, d, 'edit')}><Keyed label="EDIT" k="e" /></button>
      </div>
      <TonightIdea d={d} store={store} />
      {(start.blocked || folderMissing(d)) && <p class="fnote" role="status">{start.blocked ?? d.folder_note}</p>}
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

/** The command that resumes the session, printed like a till receipt: with the action row, the panel's end. */
function Receipt({ d }: { d: CardDetail }) {
  if (!d.resumable || !d.resume_cmd) return null;
  const { label, via } = resumeLabel(d.resume_mode, d.resume_terminal);
  return (
    <div class="receipt" style={`--zig:${ZIG}`}>
      <div class="rt"><span>RESUME</span><span>{label}{via && <small>{via}</small>}</span></div>
      <code>{d.resume_cmd}</code>
      <div class="barcode" aria-hidden="true" />
    </div>
  );
}

/** The action row that ends the panel: RESUME, and the other actions with their keys (MORE at the top has them too). */
function Actions({ d, store }: { d: CardDetail; store: Store }) {
  // an idea has START, EDIT and ARCHIVE in its own row
  if (d.kind === 'idea') return null;
  // While the clock says your corrections usually rise, continuing tonight comes first, with the reason
  const why = tonightWhy(d, store.clockHint.value);
  return (
    <div class="acts end">
      {why && <p class="why">{why}</p>}
      {why && <TonightButton d={d} store={store} />}
      {d.resumable && <button class="btn big" aria-keyshortcuts="R" title="Resume (R)" onClick={() => void store.resume(d.id)}>RESUME</button>}
      {d.resumable && d.resume_cmd && <button class="btn" aria-keyshortcuts="F" title="Resume as a new session branched from this one (F)" onClick={() => void store.resume(d.id, true)}><Keyed label="FORK" k="f" /></button>}
      {d.resume_cmd && <button class="btn" aria-keyshortcuts="C" title="Copy the resume command (C)" onClick={() => copy(d.resume_cmd!, store)}><Keyed label="COPY" k="c" /></button>}
      <button class="btn" aria-keyshortcuts="E" title="Save this session as one readable page; secrets are masked (E)" onClick={() => void exportSession(store, d.id)}><Keyed label="EXPORT" k="e" /></button>
      {!why && <TonightButton d={d} store={store} />}
      <button class="btn ghost" aria-keyshortcuts="H" title="Hide this session from the board (H)" onClick={() => hideSession(store, d.id)}><Keyed label="HIDE" k="h" /></button>
      <button class="btn ghost" aria-keyshortcuts="X" title="Remove this session from eye, and optionally from its agent (X)" onClick={() => openForget(d)}>FORGET…</button>
      <MoveMenu d={d} store={store} />
    </div>
  );
}

/** One action in MORE, with the key that does the same. */
interface MoreItem { label: string; key?: string; title: string; run: () => void; /** One plain line under the item, saying why it is first. */ why?: string }

/** What a card can do besides its main buttons, each with its key: MORE opens them in the top bar. An idea has REMIND ME and ARCHIVE. */
function moreItems(d: CardDetail, store: Store): MoreItem[] {
  const archive: MoreItem = { label: 'ARCHIVE', title: 'Out of the lanes, into the archive', run: () => { void store.move(d.id, 'archive').then(() => store.close()); } };
  if (d.kind === 'idea') return [{ label: 'REMIND ME…', key: 'M', title: 'Pick when eye moves it to Needs you', run: () => openRemind(d.id) }, archive];
  const out: MoreItem[] = [];
  const tonight: MoreItem = { label: 'CONTINUE TONIGHT…', key: 'T', title: 'Fork this session tonight, in its own worktree, with a prompt you write now', run: () => { tonightOpen.value = d.id; } };
  const why = tonightWhy(d, store.clockHint.value);
  if (why) out.push({ ...tonight, why });
  if (d.resumable && d.resume_cmd) out.push({ label: 'FORK', key: 'F', title: 'Resume as a new session branched from this one', run: () => void store.resume(d.id, true) });
  if (d.resume_cmd) out.push({ label: 'COPY RESUME COMMAND', key: 'C', title: 'Copy the command that resumes it', run: () => copy(d.resume_cmd!, store) });
  out.push({ label: 'EXPORT', key: 'E', title: 'Save this session as one readable page; secrets are masked', run: () => void exportSession(store, d.id) });
  if (!why && canContinueTonight(d, store)) out.push(tonight);
  out.push({ label: 'REMIND ME…', key: 'M', title: 'Pick when eye moves it to Needs you', run: () => openRemind(d.id) });
  out.push({ label: 'HIDE', key: 'H', title: 'Hide this session from the board', run: () => hideSession(store, d.id) });
  out.push({ label: 'FORGET…', key: 'X', title: 'Remove this session from eye, and optionally from its agent', run: () => openForget(d) });
  out.push(archive);
  return out;
}

/** MORE ▾: the card's other actions in a small menu in the panel's top bar; a click elsewhere, or Esc, closes it. */
function More({ d, store }: { d: CardDetail; store: Store }) {
  const open = useSignal(false);
  const box = useRef<HTMLDivElement>(null);
  // before the paint (a layout effect): an Esc the moment the menu shows is already the menu's, not the board's
  useLayoutEffect(() => {
    if (!open.value) return;
    const away = (e: PointerEvent) => { if (!box.current?.contains(e.target as Node)) open.value = false; };
    // still listening for a moment after it closed: an Esc then is the panel's
    const esc = (e: KeyboardEvent) => { if (e.key === 'Escape' && open.peek()) { e.stopPropagation(); open.value = false; } };
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
              {m.why && <p class="why">{m.why}</p>}
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
        {/* the top bar's right end: MORE (the card's other actions), the keys list and close */}
        <div class="ptop">
          <More d={d} store={store} />
          <button class="x qk" onClick={() => { keyHelp.value = true; }} aria-label="Keyboard shortcuts (?)" title="Keyboard shortcuts (?)">?</button>
          <button class="x" onClick={() => store.close()} aria-label="Close (Esc)">ESC ✕</button>
        </div>
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
        {session && <div class="dbar"><QuickMove d={d} store={store} /></div>}
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
        {d.first_prompt && <Sec t="FIRST PROMPT"><blockquote class="prompt first"><Hl s={d.first_prompt} /></blockquote></Sec>}
        <Where d={d} />
        {/* 3. the long or seldom needed parts, folded to one line until you open them (eye remembers which) */}
        <div class="details">
          <NightRunSection d={d} store={store} />
          {session && <Conversation store={store} id={d.id} agent={d.agent_name} fallback={prompts} />}
          {d.tokens && <TokenSec key={d.id} t={d.tokens} />}
          <RemindPanel c={d} store={store} />
        </div>
        {/* the end: how to resume it, and the actions */}
        <Receipt d={d} />
        <Actions d={d} store={store} />
      </div>
    </HlWords.Provider>
  );
}
