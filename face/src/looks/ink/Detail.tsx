import { useEffect, useRef } from 'preact/hooks';
import type { Store } from '../../store';
import type { CardDetail, ColumnId } from '../../api/types';
import { ago, isBusy, resumeLabel, snippetParts } from '../../format';
import { LANE_LABEL } from './columns';
import { copy } from './copy';
import { Conversation } from './Conversation';
import { Hl, HlWords, hlPattern } from './highlight';
import { ideaForm } from './IdeaForm';
import { MoveMenu, QuickMove } from './Move';
import { Next } from './Card';
import { Sigil } from './pieces';
import { Kids, Outputs, Sec, TokenSec, Topics, Where } from './Sections';
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
  return (
    <>
      <div class="acts">
        <button class="btn big" onClick={() => void store.startIdea(d, false)}>START</button>
        <button class="btn" title={d.prompt ? `${d.agent_name} works on it without you` : 'Needs a prompt first'} onClick={() => (d.prompt ? void store.startIdea(d, true) : edit('prompt'))}>IN BACKGROUND</button>
        <button class="btn" onClick={() => edit()}>EDIT</button>
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

/** The command that resumes the session, printed like a till receipt. */
function Receipt({ d }: { d: CardDetail }) {
  if (!d.resumable || !d.resume_cmd) return null;
  const { label, via } = resumeLabel(d.resume_mode, d.resume_terminal);
  return (
    <div class="receipt" style={`clip-path:${ZIG}`}>
      <div class="rt"><span>RESUME</span><span>{label}{via && <small>{via}</small>}</span></div>
      <code>{d.resume_cmd}</code>
      <div class="barcode" aria-hidden="true" />
    </div>
  );
}

function Actions({ d, store }: { d: CardDetail; store: Store }) {
  return (
    <div class="acts end">
      {d.resumable && <button class="btn big" onClick={() => void store.resume(d.id)}>RESUME</button>}
      {d.resumable && d.resume_cmd && <button class="btn" title="Resume as a new session branched from this one" onClick={() => void store.resume(d.id, true)}>FORK</button>}
      {d.resume_cmd && <button class="btn" onClick={() => copy(d.resume_cmd!, store)}>COPY</button>}
      {d.kind !== 'idea' ? <MoveMenu d={d} store={store} /> : <button class="btn ghost" onClick={() => { void store.move(d.id, 'archive').then(() => store.close()); }}>ARCHIVE</button>}
    </div>
  );
}

/** The side panel of one card: its state, outputs, conversation and the actions on it. */
export function Detail({ store, walk }: { store: Store; walk: Walk }) {
  const d = store.detail.value;
  const id = store.selected.value!;
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => { ref.current?.scrollTo(0, 0); }, [id]);
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
  return (
    <HlWords.Provider value={re}>
      <div class="page" ref={ref}>
        <button class="x" onClick={() => store.close()} aria-label="Close (Esc)">ESC ✕</button>
        <WalkNav walk={walk} />
        <header class="dh">
          <Sigil id={d.id} big />
          <div>
            <h2><Hl s={d.title} /></h2>
            <p class="meta">
              {!session && <span class={`tag${need ? ' hot' : ''}`}>{LANE_LABEL[colId]?.name ?? colId}</span>}
              {isBusy(d) && <span class="tag live">LIVE</span>}
              {d.manual && <span class="tag">PINNED</span>}
              {store.agents.value.length > 1 && <span class="tag agent-tag">{d.agent_name}</span>}
              <span>{ago(d.last_activity_at)}</span>
              <span>{d.kind.replace('_', ' ')}</span>
            </p>
          </div>
        </header>
        {session && <QuickMove d={d} store={store} />}
        {snip && <p class="match"><b>MATCH</b><span>{snippetParts(snip).map((p, j) => (p.hit ? <mark key={j}>{p.text}</mark> : p.text))}</span></p>}
        {session && d.state && <p class="lead"><Hl s={d.state} /></p>}
        <Next c={d} hot={need} />
        {d.kind === 'idea' && d.idea_id != null && <IdeaBody d={d} store={store} />}
        {d.recap && <Sec t="RECAP"><p class="recap"><Hl s={d.recap} /></p></Sec>}
        {d.idea?.notes && <Sec t="IDEA NOTES"><p class="notes"><Hl s={d.idea.notes} /></p></Sec>}
        {d.topics.length > 0 && <Topics topics={d.topics} />}
        {d.child_count > 0 && (
          <Sec t="SUB-AGENTS" n={`${d.child_done}/${d.child_count}`}>
            <Kids key={d.id} kids={kids} total={d.child_count} />
          </Sec>
        )}
        <Outputs outputs={d.outputs} />
        {d.first_prompt && <Sec t="FIRST PROMPT"><blockquote class="prompt first"><Hl s={d.first_prompt} /></blockquote></Sec>}
        {session && <Conversation store={store} id={d.id} agent={d.agent_name} fallback={prompts} />}
        {d.tokens && <TokenSec key={d.id} t={d.tokens} />}
        <Where d={d} />
        <Receipt d={d} />
        <Actions d={d} store={store} />
      </div>
    </HlWords.Provider>
  );
}
