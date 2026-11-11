import type { Card, Output, OutputKind, Tokens } from './api/types';

/** "now", "5m", "3h", "2d", "3w", then "12 Oct". */
export function ago(ms: number | null | undefined, now = Date.now()): string {
  if (!ms) return '';
  const s = Math.max(0, (now - ms) / 1000);
  if (s < 60) return 'now';
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  if (s < 86400 * 14) return `${Math.floor(s / 86400)}d`;
  if (s < 86400 * 60) return `${Math.floor(s / 86400 / 7)}w`;
  return new Date(ms).toLocaleDateString('en-GB', { day: 'numeric', month: 'short' });
}

/** What Resume does, as the receipt says it: ITERM, TERMINAL or COPY, and the Linux terminal's name when it is known. */
export function resumeLabel(mode: string | null | undefined, terminal?: string | null): { label: string; via: string } {
  const label = mode === 'iterm' || mode === 'terminal' ? mode.toUpperCase() : 'COPY';
  return { label, via: label === 'TERMINAL' && terminal ? terminal : '' };
}

/** Who a "next" line is for: the user, the card's agent, or another person. */
export type Who = 'you' | 'agent' | 'others';

/**
 * "You: review the PR" → {who:'you', name:'You', text:'review the PR'}. The owner is You, the card's
 * agent by its short name ("Claude: fix the test"; a plain "Agent" and, for older lines, "Claude" are
 * accepted too), or another person's name.
 */
export function nextParts(next: string | null | undefined, agentName?: string): { who: Who | null; name: string | null; text: string } {
  if (!next) return { who: null, name: null, text: '' };
  const m = /^([^:\n]{1,30}):/.exec(next);
  if (!m) return { who: null, name: null, text: next };
  const name = m[1]!.trim();
  const isAgent = /^(agent|claude)$/i.test(name) || (!!agentName && name.toLowerCase() === agentName.toLowerCase());
  const who: Who = /^you$/i.test(name) ? 'you' : isAgent ? 'agent' : 'others';
  return { who, name, text: next.slice(m[0].length).trim() };
}

/** Last `n` path segments: "~/code/projects/agent-eye" → "projects/agent-eye". */
export function shortPath(p: string | null | undefined, n = 2): string {
  if (!p) return '';
  return p.split('/').filter(Boolean).slice(-n).join('/');
}

/** An artifact's title, else "artifact 7fK2aB9x…" from the end of its URL. */
export function artifactName(o: Output): string {
  const t = o.title?.trim();
  if (t) return t;
  const id = (o.url ?? o.value).split(/[?#]/)[0]!.split('/').filter(Boolean).pop() ?? '';
  return id ? `artifact ${id.slice(0, 8)}${id.length > 8 ? '…' : ''}` : 'artifact';
}

/** Home-relative path: "/Users/you/code/x" → "~/code/x". */
export function homePath(p: string): string {
  return p.replace(/^\/Users\/[^/]+(?=\/|$)/, '~').replace(/^\/home\/[^/]+(?=\/|$)/, '~');
}

/** Folders shown short: with 2+ folders the shared parent becomes `base` and `rels` are the rest. */
export function folderGroup(values: string[]): { base: string | null; rels: string[] } {
  const paths = [...new Set(values.map(homePath))];
  if (paths.length < 2) return { base: null, rels: paths };
  const segs = paths.map((p) => p.split('/'));
  let n = 0;
  const max = Math.min(...segs.map((x) => x.length)) - 1;
  while (n < max && segs.every((x) => x[n] === segs[0]![n])) n++;
  if (n === 0) return { base: null, rels: paths };
  return { base: segs[0]!.slice(0, n).join('/') + '/', rels: segs.map((x) => x.slice(n).join('/')) };
}

/** Short chip text for an output. */
export function outputLabel(o: Output): string {
  switch (o.kind) {
    case 'pr': {
      const m = o.value.match(/([^/]+)\/(?:pull-requests|pull|merge_requests)\/(\d+)/);
      return m ? `${m[1]}#${m[2]}` : 'PR';
    }
    case 'artifact': return artifactName(o);
    case 'jira': return o.value;
    case 'commit':
      if (o.value === 'commit') return 'commit (message not recorded)';
      return o.value.length > 48 ? o.value.slice(0, 47) + '…' : o.value;
    case 'folder': return homePath(o.value);
  }
}

export const OUTPUT_NAMES: Record<OutputKind, string> = {
  pr: 'Pull requests', artifact: 'Artifacts', jira: 'Jira', commit: 'Commits', folder: 'Folders',
};

export function isBusy(c: Card): boolean {
  return c.live_status === 'busy';
}

/** Split "[match]" snippets from search into plain/marked parts. */
export function snippetParts(s: string): Array<{ text: string; hit: boolean }> {
  return s.split(/(\[[^\]]*\])/).filter(Boolean).map((t) =>
    t.startsWith('[') && t.endsWith(']') ? { text: t.slice(1, -1), hit: true } : { text: t, hit: false });
}

/** Token counts for a card: 840, 1.2k, 12k, 340k, 1.2M, 12M. */
export function tok(n: number): string {
  if (n < 1000) return String(Math.max(0, Math.round(n)));
  const one = (v: number, u: string) => `${(Math.round(v * 10) / 10).toString()}${u}`;
  if (n < 9950) return one(n / 1000, 'k');
  if (n < 999500) return `${Math.round(n / 1000)}k`;
  if (n < 9.95e6) return one(n / 1e6, 'M');
  return `${Math.round(n / 1e6)}M`;
}

/** Tooltip for a card's token count. */
export function tokTitle(t: Tokens): string {
  const n = (v: number) => v.toLocaleString('en-GB');
  const parts = [`input ${n(t.input)}`, `output ${n(t.output)}`, `cache write ${n(t.cache_write)}`, `cache read ${n(t.cache_read)}`];
  if (t.subagents_total > 0) parts.push(`sub-agents ${n(t.subagents_total)} (included)`);
  return `${n(t.total)} tokens: ${parts.join(', ')}`;
}

/** Message time: "14:32" today, else "8 Nov 14:32". */
export function msgTime(ms: number | null | undefined, now = Date.now()): string {
  if (!ms) return '';
  const d = new Date(ms);
  const time = d.toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' });
  return new Date(now).toDateString() === d.toDateString() ? time : `${d.toLocaleDateString('en-GB', { day: 'numeric', month: 'short' })} ${time}`;
}

/** Splits text on `inline code` spans; no other markdown. */
export function codeParts(s: string): Array<{ text: string; code: boolean }> {
  return s.split(/(`[^`\n]+`)/).filter(Boolean).map((p) => (p.length > 2 && p.startsWith('`') && p.endsWith('`') ? { text: p.slice(1, -1), code: true } : { text: p, code: false }));
}

const DAY = 86400000;

const clock = (ms: number) => new Date(ms).toLocaleTimeString('en-GB', { hour: 'numeric', minute: '2-digit' });

function daysFrom(ms: number, now: number): number {
  const day = (t: number) => { const d = new Date(t); d.setHours(0, 0, 0, 0); return d.getTime(); };
  return Math.round((day(ms) - day(now)) / DAY);
}

/** When a reminder is: "today 14:00", "tomorrow 9:00", "yesterday 9:00", "Fri 14:00" within the week ahead, else "20 Nov 14:00". */
export function remindWhen(at: number, now = Date.now()): string {
  const days = daysFrom(at, now);
  if (days === 0) return `today ${clock(at)}`;
  if (days === 1) return `tomorrow ${clock(at)}`;
  if (days === -1) return `yesterday ${clock(at)}`;
  if (days > 0 && days < 7) return `${new Date(at).toLocaleDateString('en-GB', { weekday: 'short' })} ${clock(at)}`;
  return `${new Date(at).toLocaleDateString('en-GB', { day: 'numeric', month: 'short' })} ${clock(at)}`;
}

/** A set reminder in a few words: "tomorrow 9:00 · Ana's reply"; null for none. */
export function remindText(c: Card, now = Date.now()): string | null {
  if (!c.remind_at) return null;
  const when = remindWhen(c.remind_at, now);
  return c.remind_note ? `${when} · ${c.remind_note}` : when;
}

/** The tooltip of a card's clock mark: "Reminder: tomorrow 9:00 · Ana's reply"; null for none. */
export function remindTitle(c: Card, now = Date.now()): string | null {
  const text = remindText(c, now);
  if (!text) return null;
  return `${c.remind_due ? 'Reminder due' : 'Reminder'}: ${text}`;
}

/** The next line of a due card, after → YOU: why it is in Needs you. */
export function remindReason(c: Card, now = Date.now()): string | null {
  if (!c.remind_due) return null;
  if (c.remind_note) return `check on this: ${c.remind_note}`;
  return c.remind_at ? `you asked to check on this ${remindWhen(c.remind_at, now)}` : 'you asked to check on this';
}
