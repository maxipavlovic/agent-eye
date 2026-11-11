import type {
  Agent, ArchivePage, Board, Card, CardDetail, ColumnId, ConversationPage, FolderCheck, Idea, Message, SearchHit, Source, StartPlan, Tokens,
} from '../api/types';

// fixtures/sample/ is synthetic: every build, test, screenshot and the site use it. fixtures/real/
// is a private snapshot of a real board (git-ignored), read only when a build asks for it with
// VITE_FIXTURE=real; otherwise it is not even bundled.
const real = import.meta.env.VITE_FIXTURE === 'real' && import.meta.env.MODE !== 'test'
  ? import.meta.glob<{ default: unknown }>('../../fixtures/real/*.json', { eager: true })
  : {};
const sample = import.meta.glob<{ default: unknown }>('../../fixtures/sample/*.json', { eager: true });
const files = Object.keys(real).length > 0 ? real : sample;

function file<T>(name: string, fallback: T): T {
  const key = Object.keys(files).find((p) => p.endsWith('/' + name));
  return key ? (structuredClone(files[key]!.default) as T) : fallback;
}

/** The fixture's one agent; cards and ideas without agent fields (an old snapshot) get it. */
const AGENT: Agent = { id: 'claude', name: 'Claude Code', short: 'Claude', default: true };

const delay = <T>(v: T, ms = 60) => new Promise<T>((r) => setTimeout(() => r(v), ms));

/** Same words in, same "[match]" snippet style out as the server's FTS search. */
function snippetOf(text: string, words: string[]): string | null {
  const lower = text.toLowerCase();
  const at = words.map((w) => lower.indexOf(w)).filter((i) => i >= 0).sort((a, b) => a - b)[0];
  if (at === undefined) return null;
  const start = Math.max(0, at - 40);
  let s = text.slice(start, start + 110);
  for (const w of words) s = s.replace(new RegExp(w.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), 'gi'), (m) => `[${m}]`);
  return (start > 0 ? '…' : '') + s.replace(/\s+/g, ' ') + (start + 110 < text.length ? '…' : '');
}

function hash(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
  return h >>> 0;
}

/** Plausible, stable token numbers for a session (the snapshot predates the field). */
function tokensFor(c: Card): Tokens | null {
  if (c.kind === 'idea' || c.kind === 'history_only') return null;
  const h = hash(c.id);
  const total = Math.round(2e4 * 1500 ** ((h % 1000) / 1000));
  const sub = c.child_count > 0 ? Math.round(total * (0.1 + ((h >>> 10) % 30) / 100)) : 0;
  const main = total - sub;
  const output = Math.round(main * 0.012);
  const input = Math.round(main * 0.004);
  const cache_write = Math.round(main * 0.06);
  return { input, output, cache_write, cache_read: main - input - output - cache_write, total, subagents_total: sub };
}

const REPLIES = [
  'Done. I made the change and ran the tests; they pass.\n\nNext: look at the diff and tell me if the names read well.',
  'I read the module first. The function already exists; only the caller needs the new argument.',
  'The check failed because one fixture was missing a field. I added it and the type check is green now.',
  'Two options:\n- keep the old name and add an alias\n- rename it everywhere\n\nI would rename it, the old name is used in three places only.',
  'Looks right to me. One thing to decide: should an empty section fold by itself?',
];

/** Moves every time in the sample so the board looks as fresh as when it was written: "now" is the moment it loads. */
function shiftTimes(v: unknown, by: number): void {
  if (Array.isArray(v)) { v.forEach((x) => shiftTimes(x, by)); return; }
  if (!v || typeof v !== 'object') return;
  const o = v as Record<string, unknown>;
  for (const k of Object.keys(o)) {
    const x = o[k];
    if (typeof x === 'number' && /_at$|^ts$/.test(k)) o[k] = x + by;
    else shiftTimes(x, by);
  }
}

/** After `shiftTimes`: moves the days of each `worked` by whole days so its last day is the local day of its last message again. */
function shiftWorked(details: Record<string, CardDetail>): void {
  const day = (s: string) => Date.parse(`${s}T00:00:00Z`);
  for (const d of Object.values(details)) {
    const w = d.worked;
    if (!w) continue;
    const at = new Date(w.last_at);
    const by = Date.UTC(at.getFullYear(), at.getMonth(), at.getDate()) - day(w.last_day);
    const move = (s: string) => new Date(day(s) + by).toISOString().slice(0, 10);
    w.first_day = move(w.first_day);
    w.last_day = move(w.last_day);
    w.days.forEach((x) => { x.day = move(x.day); });
  }
}

/** The few time words the board sends, read like `eye remind` reads them (the real reading is the engine's). */
function whenOf(text: string, now = Date.now()): number {
  const t = text.trim().toLowerCase();
  const day = (n: number, h = 9, m = 0) => { const d = new Date(now); d.setDate(d.getDate() + n); d.setHours(h, m, 0, 0); return d.getTime(); };
  const names = ['sunday', 'monday', 'tuesday', 'wednesday', 'thursday', 'friday', 'saturday'];
  const weekday = t.length >= 3 ? names.findIndex((n) => n.startsWith(t)) : -1;
  const rel = /^([1-9]\d*)([mhdw])$/.exec(t);
  const abs = /^(\d{4})-(\d\d)-(\d\d)(?:[ t](\d\d):(\d\d))?$/.exec(t);
  let at = NaN;
  if (t === 'tomorrow') at = day(1);
  else if (weekday >= 0) at = day(((weekday - new Date(now).getDay() + 6) % 7) + 1);
  else if (rel) at = rel[2] === 'm' ? now + +rel[1]! * 60000 : rel[2] === 'h' ? now + +rel[1]! * 3600000 : day(+rel[1]! * (rel[2] === 'w' ? 7 : 1), new Date(now).getHours(), new Date(now).getMinutes());
  else if (abs) at = new Date(+abs[1]!, +abs[2]! - 1, +abs[3]!, +(abs[4] ?? 9), +(abs[5] ?? 0)).getTime();
  if (Number.isNaN(at)) throw new Error(`cannot read '${text.trim()}': say when as tomorrow, mon..sun, 30m, 4h, 3d, 1w, 2026-11-20 or 2026-11-20 14:00`);
  if (at <= now) throw new Error(`${text.trim()} has already passed`);
  return at;
}

/** A believable conversation from the stored prompts, long enough to page through. */
function conversationOf(d: CardDetail): Message[] {
  const base = d.last_assistant_text ? 1 : 0;
  const prompts = d.prompts.length ? d.prompts : [d.first_prompt ?? d.title];
  const out: Message[] = [];
  const h = hash(d.id);
  const n = Math.max(prompts.length, 8 + (h % 9));
  const t0 = (d.last_activity_at ?? Date.now()) - n * 7 * 60000;
  for (let i = 0; i < n; i++) {
    const ts = t0 + i * 7 * 60000;
    const text = prompts[i % prompts.length]!;
    out.push({ role: 'user', text: i < prompts.length ? text : `${text} (again, shorter)`, ts, tools: 0, truncated: false });
    const last = i === n - 1 && base;
    out.push({
      role: 'assistant',
      text: last ? d.last_assistant_text! : REPLIES[(h + i) % REPLIES.length]!,
      ts: ts + 90000,
      tools: (h + i) % 3 === 0 ? 0 : 2 + ((h + i) % 9),
      truncated: i === 2 && h % 4 === 0,
    });
  }
  return out;
}

/** A Source over the bundled snapshot. Writes change only the in-memory copy. */
export function fixtureSource(): Source {
  const board = file<Board>('board.json', { columns: [], archive_count: 0, generated_at: Date.now() });
  const details = file<Record<string, CardDetail>>('cards.json', {});
  const archive = file<ArchivePage>('archive.json', { cards: [], total: 0, offset: 0 });
  const complete = (c: Card) => {
    if (c.tokens === undefined) c.tokens = tokensFor(c);
    c.agent ??= AGENT.id;
    c.agent_name ??= AGENT.short;
  };
  board.columns.forEach((col) => col.cards.forEach(complete));
  archive.cards.forEach(complete);
  Object.values(details).forEach(complete);
  const ideas = file<{ ideas: Idea[] }>('ideas.json', { ideas: [] });
  ideas.ideas.forEach((i) => { i.agent ??= null; });
  // Written conversations; sessions without one get a made-up one from their prompts.
  const written = file<Record<string, Message[]>>('conversations.json', {});
  if (files === sample) {
    const by = Date.now() - board.generated_at;
    for (const part of [board, details, archive, ideas, written]) shiftTimes(part, by);
    shiftWorked(details);
  }
  /** What the real engine would say about a folder: the ones on the board exist, a new one under `~` can be made. */
  const exists = (p: string) => p === '~' || allCards().some((c) => c.folder === p && c.folder_state !== 'missing');
  const folderCheck = (path: string): FolderCheck => {
    const p = path.trim().replace(/\/+$/, '');
    if (!p) return { state: 'unusable', creatable: false, note: 'No folder given' };
    if (exists(p)) return { state: 'ok', creatable: false, note: null };
    if (p.startsWith('~/')) return { state: 'missing', creatable: true, note: 'Folder does not exist yet; Start will create it' };
    if (!p.startsWith('/')) return { state: 'unusable', creatable: false, note: 'Give the full path, or start with ~' };
    return { state: 'missing', creatable: false, note: 'Folder does not exist, and eye only creates folders inside your home folder' };
  };
  /** The folder fields of an idea card. */
  const folderFields = (path: string | null): Pick<Card, 'folder_state' | 'folder_creatable' | 'folder_note'> => {
    if (!path) return {};
    const f = folderCheck(path);
    return { folder_state: f.state, folder_creatable: f.creatable, folder_note: f.note };
  };
  const agents: Agent[] = [AGENT];
  const nameOf = (id: string | null | undefined) => (agents.find((a) => a.id === id) ?? agents.find((a) => a.default)!).short;
  let listeners: Array<() => void> = [];
  const changed = () => listeners.forEach((f) => f());

  /** Hidden sessions, with the column to put them back in. */
  const hidden = new Map<string, { card: Card; column: ColumnId }>();

  const allCards = (): Card[] => [...board.columns.flatMap((c) => c.cards), ...archive.cards];
  const find = (id: string) => allCards().find((c) => c.id === id || c.id.startsWith(id));
  const ideaOf = (c: Card) => (c.idea_id == null ? undefined : ideas.ideas.find((i) => i.id === c.idea_id));
  const detailOf = (c: Card): CardDetail => {
    const d: CardDetail = details[c.id] ? { ...details[c.id]!, column: c.column, manual: c.manual } : { ...c, prompts: [], first_prompt: null, last_assistant_text: null, resume_mode: 'iterm' };
    const i = ideaOf(c);
    return i ? { ...d, notes: i.notes || null, prompt: i.prompt ?? null } : d;
  };

  return {
    name: 'fixture',
    board: () => delay(structuredClone(board)),
    card: (id) => {
      const c = find(id);
      return c ? delay(structuredClone(detailOf(c))) : Promise.reject(new Error(`No card ${id}`));
    },
    archive: (offset, limit) => delay({ cards: archive.cards.slice(offset, offset + limit), total: archive.total, offset }),
    search: (q) => {
      const words = q.toLowerCase().split(/\s+/).filter(Boolean);
      const hits: SearchHit[] = [];
      for (const c of allCards()) {
        const d = detailOf(c);
        const fields = [c.title, c.state ?? '', c.next ?? '', c.folder ?? '', c.recap ?? '', ...c.topics.map((t) => t.title), ...d.prompts, d.last_assistant_text ?? ''];
        const hay = fields.join('\n').toLowerCase();
        if (!words.every((w) => hay.includes(w))) continue;
        const snippet = fields.map((f) => snippetOf(f, words)).find(Boolean) ?? c.title;
        hits.push({ card: c, snippet, score: -words.length - (c.title.toLowerCase().includes(words[0]!) ? 5 : 0) });
      }
      hits.sort((a, b) => a.score - b.score);
      return delay({ results: hits.slice(0, 30) }, 120);
    },
    ideas: () => delay(structuredClone(ideas)),
    agents: () => delay({ agents: structuredClone(agents) }),
    move: (id, to) => {
      const c = find(id);
      if (!c) return Promise.reject(new Error(`No card ${id}`));
      for (const col of board.columns) col.cards = col.cards.filter((x) => x.id !== c.id);
      const target = to === 'auto' ? (c.auto_column === 'archive' ? null : c.auto_column) : to === 'archive' ? null : to;
      const moved: Card = { ...c, column: target ?? 'archive', manual: to !== 'auto' };
      if (target) board.columns.find((x) => x.id === target)?.cards.unshift(moved);
      else { archive.cards.unshift(moved); board.archive_count++; }
      changed();
      return delay(moved);
    },
    setHidden: (id, hide) => {
      const stashed = [...hidden.values()].find((h) => h.card.id === id || h.card.id.startsWith(id));
      if (hide) {
        const col = board.columns.find((x) => x.cards.some((c) => c.id === id || c.id.startsWith(id)));
        const c = col?.cards.find((x) => x.id === id || x.id.startsWith(id));
        if (!col || !c) return stashed ? delay(stashed.card) : Promise.reject(new Error(`No card ${id}`));
        col.cards = col.cards.filter((x) => x.id !== c.id);
        hidden.set(c.id, { card: c, column: col.id });
        changed();
        return delay(c);
      }
      if (!stashed) return Promise.reject(new Error(`No card ${id}`));
      hidden.delete(stashed.card.id);
      board.columns.find((x) => x.id === stashed.column)?.cards.unshift(stashed.card);
      changed();
      return delay(stashed.card);
    },
    conversation: (id, limit, before) => {
      const c = find(id);
      if (!c) return Promise.reject(new Error(`No card ${id}`));
      const all = written[c.id] ?? conversationOf(detailOf(c));
      const end = before ?? all.length;
      const start = Math.max(0, end - limit);
      const page: ConversationPage = { messages: all.slice(start, end), more: start > 0, next_before: start > 0 ? start : null };
      return delay(page, 150);
    },
    resume: (id, _dry, fork) => {
      const c = find(id);
      const cmd = c?.resume_cmd ?? `claude --resume ${id}`;
      return delay({
        dry_run: true,
        mode: 'iterm',
        command: fork ? `${cmd} --fork-session` : cmd,
        note: `Fixture: would ${fork ? 'fork' : 'run'} ${c?.resume_cmd ?? id}`,
      });
    },
    addIdea: (input) => {
      const n = Math.max(0, ...ideas.ideas.map((i) => i.id)) + 1;
      const card: Card = {
        id: `idea:${n}`, idea_id: n, agent: input.agent ?? AGENT.id, agent_name: nameOf(input.agent), title: input.title, folder: input.folder ?? null, repo: null, branch: null,
        column: 'ideas', auto_column: 'ideas', manual: false, last_activity_at: Date.now(), started_at: null,
        state: input.notes ?? null, state_source: 'idea', next: null, ai_status: null, recap: null, live_status: null, has_prompt: !!input.prompt,
        kind: 'idea', streams: [], outputs: [], output_counts: {}, topics: [], topics_more: 0, resume_cmd: null,
        resumable: false, parent_id: null, child_count: 0, child_done: 0, prompt_count: 0, tokens: null,
        ...folderFields(input.folder ?? null),
      };
      ideas.ideas.push({ id: n, title: input.title, notes: input.notes ?? '', prompt: input.prompt ?? null, folder: input.folder ?? null, agent: input.agent ?? null, created_at: Date.now(), from_session_id: null, session_id: null, sort: n, archived: false });
      board.columns.find((c) => c.id === 'ideas')?.cards.push(card);
      changed();
      return delay(card);
    },
    editIdea: (ideaId, input) => {
      const i = ideas.ideas.find((x) => x.id === ideaId);
      const c = allCards().find((x) => x.id === `idea:${ideaId}`);
      if (!i || !c) return Promise.reject(new Error('no such idea'));
      if (input.title !== undefined && !input.title.trim()) return Promise.reject(new Error('an idea needs a title'));
      const v = (s: string) => s.trim() || null;
      if (input.title !== undefined) i.title = c.title = input.title.trim();
      if (input.notes !== undefined) { i.notes = input.notes.trim(); c.state = v(input.notes); }
      if (input.prompt !== undefined) { i.prompt = v(input.prompt); c.has_prompt = !!i.prompt; }
      if (input.folder !== undefined) {
        const fields = folderFields(v(input.folder)); // asked before the card holds the new folder
        i.folder = c.folder = v(input.folder);
        delete c.folder_state; delete c.folder_creatable; delete c.folder_note;
        Object.assign(c, fields);
      }
      if (input.agent !== undefined) { i.agent = input.agent; c.agent = input.agent; c.agent_name = nameOf(input.agent); }
      changed();
      return delay(structuredClone(c));
    },
    startIdea: (ideaId, background, _dry, createFolder = false) => {
      const c = allCards().find((x) => x.id === `idea:${ideaId}`);
      const missing = c?.folder_state === 'missing';
      if (missing && !createFolder) return Promise.reject(new Error(`the folder ${c?.folder} does not exist yet`));
      if (c && c.folder_state && c.folder_state !== 'ok' && !(missing && c.folder_creatable)) return Promise.reject(new Error(c.folder_note ?? 'the folder cannot be used'));
      if (missing && c) Object.assign(c, { folder_state: 'ok', folder_creatable: false, folder_note: null });
      const plan: StartPlan = {
        dry_run: true, mode: background ? 'bg' : 'iterm', command: `claude  # idea ${ideaId}`,
        note: `Fixture: would start idea ${ideaId}${background ? ' in the background' : ''}`, session_id: null, job: null, creates_folder: missing,
      };
      return delay(plan);
    },
    folderCheck: (path) => delay(folderCheck(path)),
    remind: (id, input) => {
      const c = find(id);
      if (!c) return Promise.reject(new Error(`No card ${id}`));
      try {
        const at = input.clear ? null : whenOf(input.when ?? '');
        const note = input.clear ? null : input.note?.trim() || null;
        const detail = details[c.id];
        const wasDue = c.remind_due && c.column === 'needs_you';
        for (const x of detail ? [c, detail] : [c]) Object.assign(x, { remind_at: at, remind_note: note, remind_due: false });
        // a card that was in Needs you for its reminder goes back to where the rules put it
        if (wasDue && c.auto_column !== 'needs_you' && c.auto_column !== 'archive') {
          for (const col of board.columns) col.cards = col.cards.filter((x) => x.id !== c.id);
          Object.assign(c, { column: c.auto_column });
          board.columns.find((x) => x.id === c.auto_column)?.cards.unshift(c);
        }
        if (c.id.startsWith('idea:')) {
          const i = ideaOf(c);
          if (i) Object.assign(i, { remind_at: at, remind_note: note });
        }
        changed();
        return delay(structuredClone(c));
      } catch (e) {
        return Promise.reject(e);
      }
    },
    subscribe: (onChange, onState) => {
      listeners.push(onChange);
      onState('fixture');
      return () => { listeners = listeners.filter((f) => f !== onChange); };
    },
  };
}
