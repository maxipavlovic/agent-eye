import type { Agent, ColumnId, OutputKind, Tokens, Topic } from '../api/types';
import { tok, tokTitle } from '../format';
import { useArt } from './look';
import { LANE_LABEL } from './columns';

/** The small stamp that tells sessions apart; nothing in a look that draws none. */
export function Sigil({ id, big }: { id: string; big?: boolean }) {
  const s = useArt().sigil(id);
  return s && <span class={`sigil${big ? ' big' : ''}`}>{s}</span>;
}

/** Quiet token count; nothing when the card has none. */
export function Tok({ t }: { t: Tokens | null | undefined }) {
  if (!t) return null;
  // "1.2M" cannot be read as an age ("2d"), so a dot is enough to set them apart.
  return <span class="tok" title={tokTitle(t)}>{tok(t.total)}<i aria-hidden="true">·</i></span>;
}

/** The status mark of a topic or sub-agent. */
export function Mark({ s, scale = 2 }: { s: Topic['status']; scale?: number }) {
  return useArt().mark(s, scale, s.replace(/_/g, ' '));
}

export function Empty({ id }: { id: ColumnId }) {
  const art = useArt();
  return (
    <div class="empty">
      {art.empty()}
      <span>{LANE_LABEL[id].empty}</span>
    </div>
  );
}

const ORIGIN_CHIP: Record<string, [string, string]> = {
  headless: ['Headless', 'a headless run (claude -p, codex exec)'],
  app: ['App', 'started from an app that drives the agent'],
  desktop: ['Desktop', 'started from the desktop app'],
  ide: ['IDE', 'started from an editor'],
  web: ['Web', 'started from the web or a remote'],
  night: ['Night', "one of eye's own night runs"],
};

/** The chip text of an origin; the terminal is the normal case and has none. */
export function originChip(origin: string | null | undefined): string | null {
  return origin ? ORIGIN_CHIP[origin]?.[0] ?? null : null;
}

/** Remote Control is on for a session. A plain chip on a card; with `url`, the panel's link to it, opened in a new tab. */
export function RemoteMark({ url }: { url?: string }) {
  const body = <>{useArt().icon('signal')}<span>Remote</span></>;
  return url
    ? <a class="mk rc" href={url} target="_blank" rel="noopener" title="Remote Control is on: open this session in the browser">{body}</a>
    : <span class="mk rc" title="Remote Control is on for this session">{body}</span>;
}


/** The short tag of an output kind, in the panel's outputs. */
export const OUT_TAG: Record<OutputKind, string> = { pr: 'PR', artifact: 'ART', jira: 'JIRA', commit: 'CMT', folder: 'DIR' };

/** "Claude Code", "Claude Code or Codex CLI", "A, B or C"; "your coding agent" while no agent is known. */
export function agentNames(agents: readonly Agent[]): string {
  const names = agents.map((a) => a.name);
  if (names.length === 0) return 'your coding agent';
  if (names.length === 1) return names[0]!;
  return `${names.slice(0, -1).join(', ')} or ${names[names.length - 1]}`;
}
