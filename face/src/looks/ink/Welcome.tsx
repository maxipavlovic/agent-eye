import { Pix } from '../../pixel/Pix';
import type { Agent } from '../../api/types';
import { EYE_STILL, INK } from './art';

/** "Claude Code", "Claude Code or Codex CLI", "A, B or C"; "your coding agent" while no agent is known. */
export function agentNames(agents: readonly Agent[]): string {
  const names = agents.map((a) => a.name);
  if (names.length === 0) return 'your coding agent';
  if (names.length === 1) return names[0]!;
  return `${names.slice(0, -1).join(', ')} or ${names[names.length - 1]}`;
}

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
