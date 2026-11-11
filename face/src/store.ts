import { ACCENTS, parseAccent, type AccentId } from './accents';
import { remindWhen } from './format';
import { pushPref } from './prefs';
import { signal, computed, effect, batch, type Signal, type ReadonlySignal } from '@preact/signals';
import type {
  Agent, ArchivePage, Board, Card, CardDetail, ColumnId, Connection, ConversationPage, IdeaInput, MoveTarget,
  FolderCheck, RemindInput, RemindSettings, ResumePlan, SearchHit, Source, StartPlan,
} from './api/types';

export type ThemePref = 'auto' | 'light' | 'dark';
export type Theme = 'light' | 'dark';

export interface Toast {
  id: number;
  text: string;
  tone: 'info' | 'error';
  /** A button on the toast, e.g. Undo. */
  action?: { label: string; run: () => void };
}

/** Everything a look needs. Looks read signals and call actions; they never fetch. */
export interface Store {
  readonly source: Source['name'];
  board: ReadonlySignal<Board | null>;
  /** id of the card whose detail is open, or null. */
  selected: ReadonlySignal<string | null>;
  detail: ReadonlySignal<CardDetail | null>;
  query: ReadonlySignal<string>;
  /** null when the query is empty. */
  results: ReadonlySignal<SearchHit[] | null>;
  searching: ReadonlySignal<boolean>;
  connection: ReadonlySignal<Connection>;
  themePref: ReadonlySignal<ThemePref>;
  /** The accent colour; also set as <html data-accent="..."> (not for the default, vermilion). */
  accent: ReadonlySignal<AccentId>;
  /** Resolved theme; also set as <html data-theme="light|dark">. */
  theme: ReadonlySignal<Theme>;
  toasts: ReadonlySignal<Toast[]>;
  /** The enabled agents; empty until known (or when the engine has no agent list). */
  agents: ReadonlySignal<Agent[]>;
  /** The desktop notice for due reminders; null until loaded, and when the engine has no such setting (the switch is hidden). */
  notice: ReadonlySignal<RemindSettings | null>;
  /** Number of cards in Needs you. */
  needsYou: ReadonlySignal<number>;
  error: ReadonlySignal<string | null>;

  open(id: string): void;
  close(): void;
  search(q: string): void;
  move(id: string, to: MoveTarget): Promise<void>;
  /** Hides (or shows again) a session; hidden ones are left out of the board and search. */
  setHidden(id: string, hidden: boolean): Promise<void>;
  /** Runs the resume plan for real on the live source (opens iTerm); returns the plan. */
  resume(id: string, fork?: boolean): Promise<ResumePlan | null>;
  /** Messages of a session, newest page first; null (quietly) when they cannot be had. */
  conversation(id: string, before?: number | null, limit?: number): Promise<ConversationPage | null>;
  /** True when saved; on failure a toast says why and the caller keeps the form open. */
  addIdea(input: IdeaInput): Promise<boolean>;
  editIdea(ideaId: number, input: Partial<IdeaInput>): Promise<boolean>;
  startIdea(card: Card, background: boolean): Promise<StartPlan | null>;
  /** What a typed folder is (the idea form's hint); rejects when the engine cannot say. */
  folderCheck(path: string): Promise<FolderCheck>;
  /** Sets (`when`, in the words of `eye remind`) or clears a reminder; true when saved, else a toast says why. */
  remind(id: string, input: RemindInput): Promise<boolean>;
  /** Reads the desktop notice setting into `notice`; an engine without it leaves `notice` null, quietly. */
  loadNotice(): void;
  /** Turns the desktop notice for every reminder on or off; a toast says why when it cannot. */
  setNotice(on: boolean): Promise<void>;
  archive(offset: number, limit?: number): Promise<ArchivePage>;
  setTheme(pref: ThemePref): void;
  setAccent(id: AccentId): void;
  toast(text: string, tone?: Toast['tone'], action?: Toast['action']): number;
  dismissToast(id: number): void;
  refresh(): Promise<void>;
}

const THEME_KEY = 'eye.theme';
const ACCENT_KEY = 'eye.accent';

/** Puts text on the clipboard from the page; false when the page may not (an insecure context, no permission). */
async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

/**
 * What a resume or a start did, as one sentence. A plan in copy mode is also copied here, in the
 * browser: the daemon often cannot reach the clipboard (Linux without a session clipboard), the page can.
 */
async function planNote(plan: ResumePlan, doing: string): Promise<string> {
  if (plan.mode === 'copy') return (await copyText(plan.command)) ? 'Command copied' : plan.note || 'Copy the command from the panel';
  return plan.note || `${doing} in a new terminal`;
}

function readPref(fallback: ThemePref): ThemePref {
  try {
    const v = localStorage.getItem(THEME_KEY);
    if (v === 'light' || v === 'dark' || v === 'auto') return v;
  } catch { /* storage blocked */ }
  return fallback;
}

function readAccent(): AccentId {
  try { return parseAccent(localStorage.getItem(ACCENT_KEY)); } catch { /* storage blocked */ }
  return parseAccent(null);
}

/** `defaultTheme`: the theme until one is chosen (the offline board opens in daylight). */
export function createStore(src: Source, { defaultTheme = 'auto' }: { defaultTheme?: ThemePref } = {}): Store {
  const board = signal<Board | null>(null);
  const selected = signal<string | null>(null);
  const detail = signal<CardDetail | null>(null);
  const query = signal('');
  const results = signal<SearchHit[] | null>(null);
  const searching = signal(false);
  const connection = signal<Connection>(src.name === 'fixture' ? 'fixture' : 'connecting');
  const toasts = signal<Toast[]>([]);
  const error = signal<string | null>(null);
  const themePref = signal<ThemePref>(readPref(defaultTheme));
  const agents = signal<Agent[]>([]);
  const notice = signal<RemindSettings | null>(null);

  const mq = typeof matchMedia === 'function' ? matchMedia('(prefers-color-scheme: dark)') : null;
  const systemDark = signal(mq?.matches ?? false);
  mq?.addEventListener('change', (e) => { systemDark.value = e.matches; });
  const theme = computed<Theme>(() =>
    themePref.value === 'auto' ? (systemDark.value ? 'dark' : 'light') : themePref.value);
  effect(() => { document.documentElement.dataset.theme = theme.value; });
  const accent = signal<AccentId>(readAccent());
  effect(() => {
    // vermilion is what the style sheet says without an attribute
    if (accent.value === ACCENTS[0].id) delete document.documentElement.dataset.accent;
    else document.documentElement.dataset.accent = accent.value;
  });

  const needsYou = computed(() =>
    board.value?.columns.find((c) => c.id === 'needs_you')?.cards.length ?? 0);

  let toastSeq = 0;
  function dismissToast(id: number) {
    toasts.value = toasts.value.filter((x) => x.id !== id);
  }
  function toast(text: string, tone: Toast['tone'] = 'info', action?: Toast['action']) {
    const t: Toast = { id: ++toastSeq, text, tone, action };
    toasts.value = [...toasts.value, t];
    setTimeout(() => dismissToast(t.id), tone === 'error' || action ? 8000 : 4000);
    return t.id;
  }

  function fail(e: unknown): null {
    toast(e instanceof Error ? e.message : String(e), 'error');
    return null;
  }

  async function refresh() {
    try {
      const b = await src.board();
      batch(() => { board.value = b; error.value = null; });
      const id = selected.value;
      if (id) detail.value = await src.card(id);
    } catch (e) {
      error.value = e instanceof Error ? e.message : String(e);
    }
  }

  let openSeq = 0;
  function open(id: string) {
    const seq = ++openSeq;
    selected.value = id;
    const known = board.value?.columns.flatMap((c) => c.cards).find((c) => c.id === id);
    detail.value = known ? { prompts: [], first_prompt: null, last_assistant_text: null, resume_mode: '', ...known } : null;
    src.card(id).then((d) => { if (seq === openSeq) detail.value = d; }, fail);
  }

  function close() {
    openSeq++;
    batch(() => { selected.value = null; detail.value = null; });
  }

  let searchTimer: ReturnType<typeof setTimeout> | undefined;
  let searchSeq = 0;
  function search(q: string) {
    query.value = q;
    clearTimeout(searchTimer);
    if (!q.trim()) {
      searchSeq++;
      batch(() => { results.value = null; searching.value = false; });
      return;
    }
    searching.value = true;
    searchTimer = setTimeout(() => {
      const seq = ++searchSeq;
      src.search(q.trim()).then((r) => {
        if (seq !== searchSeq) return;
        batch(() => { results.value = r.results; searching.value = false; });
      }, (e) => { searching.value = false; fail(e); });
    }, 150);
  }

  /** Moves the card locally right away; the server's answer (or the next refresh) corrects it. */
  function placeLocally(id: string, to: ColumnId) {
    const b = board.value;
    if (!b) return;
    let moved: Card | undefined;
    const columns = b.columns.map((c) => {
      const keep = c.cards.filter((x) => (x.id === id ? ((moved = x), false) : true));
      return keep.length === c.cards.length ? c : { ...c, cards: keep };
    });
    if (!moved) return;
    const card: Card = { ...moved, column: to, manual: true };
    board.value = {
      ...b,
      columns: columns.map((c) => (c.id === to ? { ...c, cards: [card, ...c.cards] } : c)),
    };
  }

  async function move(id: string, to: MoveTarget) {
    const before = board.value;
    if (to !== 'auto' && to !== 'archive') {
      placeLocally(id, to);
      // The open panel follows at once, before the server answers.
      const d = detail.value;
      if (d && d.id === id) detail.value = { ...d, column: to, manual: true };
    }
    try {
      await src.move(id, to);
      await refresh();
    } catch (e) {
      board.value = before;
      fail(e);
    }
  }

  async function setHidden(id: string, hide: boolean) {
    const before = { board: board.value, results: results.value };
    if (hide) {
      batch(() => {
        const b = board.value;
        if (b) board.value = { ...b, columns: b.columns.map((c) => ({ ...c, cards: c.cards.filter((x) => x.id !== id) })) };
        if (results.value) results.value = results.value.filter((h) => h.card.id !== id);
      });
    }
    try {
      await src.setHidden(id, hide);
      await refresh();
      if (!hide && query.value.trim()) search(query.value);
    } catch (e) {
      batch(() => { board.value = before.board; results.value = before.results; });
      fail(e);
    }
  }

  async function conversation(id: string, before: number | null = null, limit = 10) {
    try {
      return await src.conversation(id, limit, before);
    } catch {
      return null;
    }
  }

  async function resume(id: string, fork = false) {
    try {
      const plan = await src.resume(id, false, fork);
      toast(await planNote(plan, fork ? 'Forking' : 'Opening'));
      return plan;
    } catch (e) {
      return fail(e);
    }
  }

  async function addIdea(input: IdeaInput) {
    try {
      await src.addIdea(input);
      toast('Idea saved');
      await refresh();
      return true;
    } catch (e) {
      fail(e);
      return false;
    }
  }

  async function editIdea(ideaId: number, input: Partial<IdeaInput>) {
    try {
      await src.editIdea(ideaId, input);
      toast('Idea saved');
      await refresh();
      return true;
    } catch (e) {
      fail(e);
      return false;
    }
  }

  async function startIdea(card: Card, background: boolean) {
    if (card.idea_id == null) return fail(new Error('Not an idea'));
    try {
      // a folder that is missing and may be made is made by the start (the button says so)
      const plan = await src.startIdea(card.idea_id, background, false, card.folder_state === 'missing' && card.folder_creatable === true);
      // a background start ran its command: there is nothing to copy
      toast(background ? plan.note || `Started in the background (${plan.job ?? 'job'})` : await planNote(plan, 'Starting'));
      await refresh();
      return plan;
    } catch (e) {
      return fail(e);
    }
  }

  function setAccent(id: AccentId) {
    accent.value = id;
    pushPref(ACCENT_KEY, id);
  }

  async function remind(id: string, input: RemindInput) {
    try {
      const c = await src.remind(id, input);
      toast(input.clear ? 'Reminder cleared' : `Reminder set: ${remindWhen(c.remind_at ?? Date.now())}`);
      await refresh();
      return true;
    } catch (e) {
      fail(e);
      return false;
    }
  }

  function loadNotice() {
    src.remindSettings().then((s) => { notice.value = s; }, () => { notice.value = null; });
  }

  async function setNotice(on: boolean) {
    try {
      notice.value = await src.setRemindSettings({ notify: on });
    } catch (e) {
      fail(e);
    }
  }

  function setTheme(pref: ThemePref) {
    themePref.value = pref;
    pushPref(THEME_KEY, pref);
  }

  src.subscribe(() => { void refresh(); }, (s) => { connection.value = s; });
  void refresh();
  src.agents().then((r) => { agents.value = r.agents; }, () => { /* an older engine has no agent list: one agent */ });

  return {
    source: src.name,
    board, selected, detail, query, results, searching, connection, themePref, accent, theme, toasts, agents, notice, needsYou, error,
    open, close, search, move, setHidden, resume, conversation, addIdea, editIdea, startIdea, folderCheck: (path) => src.folderCheck(path), remind, loadNotice, setNotice,
    archive: (offset, limit = 50) => src.archive(offset, limit),
    setTheme, setAccent, toast, dismissToast, refresh,
  };
}

export type { Signal };
