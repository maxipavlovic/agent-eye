import type { Agent } from '../../api/types';
import type { Look } from '../../board/look';
import type { Store } from '../../store';
import { agentNames } from '../../board/pieces';
import { irisArt, LaneObject } from './art';
import { Eye } from './Eye';
import { Head } from './Head';
import { CODE, Header, ProjectBar } from './Header';

/** A board with no sessions and no ideas yet: the eye, drawn large, and where sessions come from. */
function Welcome({ agents }: { agents: readonly Agent[] }) {
  return (
    <main class="welcome">
      <div class="weye"><Eye alert={false} /></div>
      <h2>NO SESSIONS YET</h2>
      <p>Start a session in {agentNames(agents)} and it shows up here.</p>
      <p class="hint">Prefer the terminal? Run <code>eye</code> there.</p>
    </main>
  );
}

function Loading() {
  return <div class="loading"><LaneObject id="running" px={48} cls="lic spin" /><span>LOADING</span></div>;
}

/** The drawing's title block, under the board: what is open and done, how much is archived, when it was drawn, the look. */
function TitleBlock({ store }: { store: Store }) {
  const b = store.board.value;
  if (!b) return null;
  const count = (done: boolean) => b.columns.reduce((s, c) => s + ((c.id === 'done') === done ? c.cards.length : 0), 0);
  const at = new Date(b.generated_at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hour12: false });
  return (
    <footer class="tblock">
      <div class="tb-name">Eye · session board</div>
      <div>{count(false)} open · {count(true)} done · {b.archive_count} archived</div>
      <div>Drawn at {at}</div>
      <div class="code">{CODE}</div>
    </footer>
  );
}

/** Iris: the board as a technical drawing in one-pixel lines, with one signal colour for what needs you. */
export const IRIS_LOOK: Look = { name: 'iris', art: irisArt, Header, ProjectBar, Head, Welcome, Loading, Footer: TitleBlock };
