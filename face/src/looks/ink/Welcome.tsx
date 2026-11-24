import { Pix } from '../../pixel/Pix';
import type { Agent } from '../../api/types';
import { agentNames } from '../../board/pieces';
import { EYE_STILL, INK } from './art';

/** What a board with no sessions and no ideas shows instead of empty lanes. */
export function Welcome({ agents }: { agents: readonly Agent[] }) {
  return (
    <main class="welcome">
      <Pix rows={EYE_STILL} pal={INK} scale={8} />
      <h2>NO SESSIONS YET</h2>
      <p>Start a session in {agentNames(agents)} and it shows up here.</p>
      <p class="hint">Prefer the terminal? Run <code>eye</code> there.</p>
    </main>
  );
}
