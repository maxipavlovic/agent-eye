// Shapes of the eye board API (docs/api.md). Times are Unix milliseconds.

export type ColumnId = 'ideas' | 'running' | 'needs_you' | 'paused' | 'waiting' | 'done';
export type Where = ColumnId | 'archive';
/** Body of POST /api/cards/:id/column. */
export type MoveTarget = Exclude<ColumnId, 'ideas'> | 'archive' | 'auto';

export type CardKind = 'interactive' | 'bg' | 'history_only' | 'idea';
export type StateSource = 'summary' | 'recap' | 'idea' | 'none';
export type AiStatus = 'needs_you' | 'waiting_on_others' | 'done' | 'paused';
export type LiveStatus = 'busy' | 'idle';
export type TopicStatus = 'done' | 'active' | 'paused' | 'waiting_on_others' | 'needs_you';

/** topic: a part of the work the agent split out; child: a sub-agent or background child;
 *  output: something the session produced. */
export type StreamKind = 'topic' | 'child' | 'output';
/** topic: TopicStatus. child: running | done. output: opened (pr) | published (artifact) |
 *  mentioned (jira) | committed (commit) | written (folder). */
export type StreamStatus = TopicStatus | 'running' | 'opened' | 'published' | 'mentioned' | 'committed' | 'written';

export interface Stream {
  kind: StreamKind;
  key: string;
  title: string;
  status: StreamStatus;
  detail: string;
  link: string | null;
}

export type OutputKind = 'pr' | 'artifact' | 'jira' | 'commit' | 'folder';

export interface Output {
  kind: OutputKind;
  value: string;
  url: string | null;
  /** Artifacts: the title when one is known. */
  title?: string | null;
}

export interface Topic {
  key: string;
  title: string;
  status: TopicStatus;
  next: string;
}

/** Token use of a session; `total` already includes `subagents_total`. */
export interface Tokens {
  input: number;
  output: number;
  cache_write: number;
  cache_read: number;
  total: number;
  subagents_total: number;
}

/** A task an open session started and left running. */
export interface BackgroundTask {
  id: string;
  kind: 'shell' | 'agent' | 'monitor';
  /** The command or description, one line. */
  label: string;
  started_at: number;
  /** The line as the terminal shows it: `npm run dev · 4 min`. */
  line: string;
}

/** A loop an open session runs: a schedule, a self-paced wake-up or a goal. */
export interface LoopItem {
  kind: 'interval' | 'self_paced' | 'goal';
  schedule: string;
  next_at: number | null;
  prompt: string;
  /** The line as the terminal shows it: `↻ every 10 min · next 14:20`. */
  line: string;
}

/** Where a session was started; `null` when unknown. The terminal is the normal case. */
export type Origin = 'terminal' | 'headless' | 'app' | 'desktop' | 'ide' | 'web' | 'night';

export interface Card {
  id: string;
  /** Agent id, e.g. "claude". */
  agent: string;
  /** The agent's short display name, e.g. "Claude". */
  agent_name: string;
  title: string;
  folder: string | null;
  repo: string | null;
  branch: string | null;
  /** Where the session was started (see `Origin`). */
  origin?: Origin | null;
  /** What the agent wrote that `origin` was mapped from (`sdk-ts`). */
  origin_raw?: string | null;
  column: Where;
  auto_column: Where;
  manual: boolean;
  last_activity_at: number | null;
  started_at: number | null;
  /** One line: where the work stands. */
  state: string | null;
  state_source: StateSource;
  /** Who acts next and what: "You: review the PR", "<agent short name>: …" or another person's name. */
  next: string | null;
  /** The open session has stopped and waits for your input to go on (`next` then says what it asks). */
  blocked?: boolean;
  ai_status: AiStatus | null;
  /** The agent's own recap ("Goal… Next…"), when it keeps one. */
  recap: string | null;
  live_status: LiveStatus | null;
  /** One line of what an open session does on its own: its background tasks, else its loop. */
  activity?: string | null;
  /** Background tasks of an open session. */
  background?: BackgroundTask[];
  /** Loops of an open session. */
  loops?: LoopItem[];
  /** The web link of the session while its remote control is on. */
  remote_url?: string | null;
  kind: CardKind;
  /** Board cards: topics only. Detail cards: topics, children and outputs. */
  streams: Stream[];
  /** Board cards: at most 3 picked outputs plus one folder. Detail cards: all. */
  outputs: Output[];
  output_counts: Partial<Record<OutputKind, number>>;
  topics: Topic[];
  topics_more: number;
  resume_cmd: string | null;
  resumable: boolean;
  parent_id: string | null;
  child_count: number;
  child_done: number;
  prompt_count: number;
  /** Sessions: token use, or null when unknown. */
  tokens: Tokens | null;
  /** Idea cards only (id is "idea:<n>"). */
  idea_id?: number;
  /** Idea cards only: a start sends a first message (else the agent opens and waits). */
  has_prompt?: boolean;
  /** When you asked to check on this (a reminder that is set for later or due); null or missing: none. */
  remind_at?: number | null;
  remind_note?: string | null;
  /** The reminder's time has come: the card is in Needs you (an idea stays an idea card). */
  remind_due?: boolean;
  /** What the session left behind (servers still running, commits on no remote, ...), not counting what was ignored. */
  loose?: number;
  /** The session was made by a night-shift run. */
  night?: { id: number; status: string };
  /** Idea cards with a folder: what the folder is now. Older engines leave all three out. */
  folder_state?: FolderState;
  /** Idea cards: a missing folder that Start may create. */
  folder_creatable?: boolean;
  /** Idea cards: one plain line about the folder, null when it is fine. */
  folder_note?: string | null;
}

export type FolderState = 'ok' | 'missing' | 'not_a_folder' | 'unusable';

/** GET /api/folder: what a typed folder is. */
export interface FolderCheck {
  state: FolderState;
  creatable: boolean;
  note: string | null;
}

/** One local day a session was worked on. */
export interface WorkedDay {
  /** YYYY-MM-DD. */
  day: string;
  prompts: number;
  active_ms: number;
}

/** The days a session was worked on (detail cards): absent or null while they are not counted yet. */
export interface Worked {
  /** The first and last calendar day of the strip, at most the last 60. */
  first_day: string;
  last_day: string;
  /** The days with work in them inside the strip, oldest first. */
  days: WorkedDay[];
  /** Days with work over the whole session, and calendar days from its first to its last. */
  days_count: number;
  span_days: number;
  active_ms: number;
  last_at: number;
  /** Calendar days before the strip. */
  more_days: number;
}

/** Tokens and the estimated cost of a group of messages (API-equivalent, not a bill). */
export interface UsageSum {
  input: number;
  output: number;
  cache_write: number;
  cache_read: number;
  total: number;
  /** null: no token of the group has a known price. */
  cost_usd: number | null;
  /** Tokens on a model without a price: counted in the totals, not in the cost. */
  unpriced_tokens: number;
}

export interface UsageRow extends UsageSum {
  name: string;
  /** Session rows only. */
  id?: string;
  agent?: string;
}

export interface UsageWeek extends UsageSum {
  from: string;
  to: string;
}

export interface UsageLimitWindow {
  label: string;
  used_percent: number;
  window_minutes: number | null;
  /** Unix ms. */
  resets_at: number | null;
  /** The window has reset since it was seen. */
  reset: boolean;
}

/** The plan limits one agent reported, as the newest seen. */
export interface UsageLimits {
  agent: string;
  name: string;
  /** Unix ms. */
  seen_at: number;
  plan: string | null;
  windows: UsageLimitWindow[];
  /** One sentence per window, with its age. */
  lines: string[];
}

/** GET /api/usage. */
export interface UsageReport {
  /** "API-equivalent estimate, not your bill". */
  estimate: string;
  days: number;
  from: string;
  to: string;
  by: 'project' | 'agent' | 'model' | 'session';
  rows: UsageRow[];
  totals: UsageSum;
  by_project: UsageRow[];
  by_agent: UsageRow[];
  by_model: UsageRow[];
  top_sessions: UsageRow[];
  /** The last four weeks, oldest first. */
  weeks: UsageWeek[];
  limits: UsageLimits[];
  /** Sessions whose usage is still being counted in the background. */
  backfill_left: number;
}

/** A session's usage over its whole life (detail cards); null while it is not counted. */
export interface CardUsage extends UsageSum {
  estimate: string;
}

/** One thing a session left behind. */
export interface LooseEnd {
  /** Stable across scans; what the actions send. */
  key: string;
  session_id: string;
  kind: 'process' | 'container' | 'worktree' | 'unpushed' | 'uncommitted';
  /** One plain sentence. */
  line: string;
  /** What it is, in a few words: the confirm question names it. */
  what: string;
  detail: string;
  since_ms: number;
  /** What eye can do: stop, remove, or (git work it never touches) copy a command. */
  action: 'stop' | 'remove' | 'push' | 'commit';
  /** The question to ask before the action (stop and remove only). */
  confirm: string | null;
  /** The git command to copy (push and commit only); eye never runs it. */
  command: string | null;
}

export type LooseAct = 'stop' | 'remove' | 'ignore';

export type NightIsolation = 'worktree' | 'copy';

/** One night-shift run (GET /api/night, a card's `night`). The report is untrusted output: render it as text. */
export interface NightRun {
  id: number;
  kind: 'idea' | 'session';
  idea_id: number | null;
  session_id: string | null;
  agent: string;
  switch: boolean;
  folder: string;
  prompt: string;
  /** The local date (YYYY-MM-DD) of the night it is planned for. */
  night: string;
  status: 'planned' | 'running' | 'done' | 'failed' | 'stopped' | 'skipped';
  why: string | null;
  limit: { kind: 'weekly' | 'tokens' | 'minutes'; value: number; text: string } | null;
  run_session_id: string | null;
  worktree: string | null;
  branch: string | null;
  base_ref: string | null;
  started_at: number | null;
  ended_at: number | null;
  duration_ms: number | null;
  report: string | null;
  diff_stat: string | null;
  created_at: number;
  /** How the run keeps the folder safe: a git worktree of the repo, or a separate copy (the original fully locked). */
  isolation: NightIsolation;
  /** Detail cards only: tokens the run's session used, and whether its worktree has uncommitted changes. */
  tokens?: number;
  uncommitted?: boolean;
}

/** GET /api/night: the switch, the window, and the runs. */
export interface NightState {
  enabled: boolean;
  window: string;
  parallel: number;
  /** `[night] isolation`: what a new plan starts with. */
  isolation: NightIsolation;
  quiet_minutes: number;
  in_window: boolean;
  planned: number;
  open: NightRun[];
  recent: NightRun[];
}

/** POST /api/night/plan: an idea (`idea_id`) or a session to continue (`session_id`, with `prompt`). */
export interface NightPlanInput {
  idea_id?: number;
  session_id?: string;
  prompt?: string;
  /** `30%`, `2M`, `500k`, `90m`, `2h`; empty: none. */
  limit?: string;
  agent?: string;
  switch?: boolean;
  isolation?: NightIsolation;
}

export interface CardDetail extends Card {
  usage?: CardUsage | null;
  /** What the session left behind, ignored items left out. */
  loose_ends?: LooseEnd[];
  /** The night run that made this session; null (or missing) for any other. */
  night_run?: NightRun | null;
  /** Up to the last 10 prompts, oldest first. */
  prompts: string[];
  first_prompt: string | null;
  last_assistant_text: string | null;
  worked?: Worked | null;
  /** What Resume does on this machine: 'iterm', 'terminal' or 'copy'. */
  resume_mode: 'iterm' | 'terminal' | 'copy' | string;
  /** The Linux terminal program that Resume opens, when known. */
  resume_terminal?: string | null;
  /** Idea detail only: yours, never sent to the agent. */
  notes?: string | null;
  /** Idea detail only: the first message a start sends. */
  prompt?: string | null;
  /** Session detail: the idea it was started from. */
  idea?: { id: number; title: string; notes: string | null; prompt: string | null } | null;
}

export interface Message {
  role: 'user' | 'assistant';
  text: string;
  ts: number | null;
  /** Tool calls the agent made in this turn (0 for user messages). */
  tools: number;
  truncated: boolean;
}

/** GET /api/cards/:id/conversation: oldest first; ask again with `before=next_before` for older ones. */
export interface ConversationPage {
  messages: Message[];
  more: boolean;
  next_before: number | null;
}

export interface Column {
  id: ColumnId;
  title: string;
  cards: Card[];
}

export interface Board {
  columns: Column[];
  archive_count: number;
  generated_at: number;
}

export interface ArchivePage {
  cards: Card[];
  total: number;
  offset: number;
}

export interface SearchHit {
  card: Card;
  snippet: string;
  /** Lower is better (kept from 0.2). */
  score: number;
  /** Higher is better: the same ranking as `score`, turned around. */
  relevance: number;
}

/** An enabled agent: `short` is the name shown on cards and labels. */
export interface Agent {
  id: string;
  name: string;
  short: string;
  default: boolean;
}

export interface Idea {
  id: number;
  title: string;
  notes: string;
  prompt?: string | null;
  folder: string | null;
  /** Agent id, or null for the default agent. */
  agent: string | null;
  created_at: number;
  from_session_id: string | null;
  session_id: string | null;
  sort: number;
  archived: boolean | number;
}

export interface IdeaInput {
  title: string;
  /** Yours; never sent to the agent. */
  notes?: string;
  /** The first message when the idea starts. */
  prompt?: string;
  folder?: string;
  /** Agent id; left out = the default agent. */
  agent?: string;
}

/** Body of POST /api/cards/:id/remind. `when` is read by the engine: tomorrow, mon..sun, 30m, 4h, 3d, 1w, 2026-11-20, 2026-11-20 14:00. */
export interface RemindInput {
  when?: string;
  note?: string;
  clear?: boolean;
}

/** GET and POST /api/remind/settings: the desktop notice when a reminder falls due, for every reminder on this machine. */
export interface RemindSettings {
  notify: boolean;
  /** This machine can show a desktop notice at all; when false the board offers no switch. */
  available: boolean;
}

export interface ResumePlan {
  dry_run: boolean;
  /** What happened or would: 'iterm', 'terminal' or 'copy'. */
  mode: string;
  /** The Linux terminal program, when `mode` is 'terminal' and it is known. */
  terminal?: string | null;
  command: string;
  applescript?: string | null;
  note?: string;
}

export interface StartPlan extends ResumePlan {
  session_id: string | null;
  job: string | null;
  /** The folder was missing and the start made it (a dry run: would make it). */
  creates_folder?: boolean;
}

/** Where the store gets its data: the live API or a local fixture. */
export interface Source {
  readonly name: 'live' | 'fixture';
  board(): Promise<Board>;
  card(id: string): Promise<CardDetail>;
  archive(offset: number, limit: number): Promise<ArchivePage>;
  search(q: string): Promise<{ results: SearchHit[] }>;
  ideas(): Promise<{ ideas: Idea[] }>;
  /** The enabled agents (GET /api/agents). */
  agents(): Promise<{ agents: Agent[] }>;
  /** Usage over the last `days` days (GET /api/usage); an older engine answers 404. */
  usage(days: number): Promise<UsageReport>;
  move(id: string, column: MoveTarget): Promise<Card>;
  /** Manual hide / unhide of a session (POST /api/cards/:id/hidden). */
  setHidden(id: string, hidden: boolean): Promise<Card>;
  /** Removes a session from eye, and with `transcript` the agent's own copy too (POST /api/sessions/:id/forget, `confirm: true`). Cannot be undone. */
  forget(id: string, transcript: boolean): Promise<void>;
  /** The newest `limit` messages, or the `limit` before index `before`. */
  conversation(id: string, limit: number, before: number | null): Promise<ConversationPage>;
  /** `fork`: resume as a new session branched from this one. */
  resume(id: string, dryRun: boolean, fork?: boolean): Promise<ResumePlan>;
  addIdea(input: IdeaInput): Promise<Card>;
  /** Changes the given fields; an empty string clears notes, prompt or folder. */
  editIdea(ideaId: number, input: Partial<IdeaInput>): Promise<Card>;
  /** `createFolder`: make the idea's folder first when it does not exist. */
  startIdea(ideaId: number, background: boolean, dryRun: boolean, createFolder?: boolean): Promise<StartPlan>;
  /** What a typed folder is (read-only). */
  folderCheck(path: string): Promise<FolderCheck>;
  /** Sets or clears a "check on this" reminder on a session or an idea card (POST /api/cards/:id/remind). */
  remind(id: string, input: RemindInput): Promise<Card>;
  /** Stops, removes or ignores one thing a session left behind (POST /api/loose/<action>); the engine refuses what no longer matches. */
  looseAct(action: LooseAct, key: string): Promise<{ ok: boolean; note?: string }>;
  /** Resumes the session with a first message that lists what it left (POST /api/loose/send). */
  looseSend(session: string, dryRun: boolean): Promise<ResumePlan>;
  /** Whether a due reminder shows a desktop notice (GET /api/remind/settings); an older engine answers 404. */
  remindSettings(): Promise<RemindSettings>;
  /** Turns the desktop notice on or off; it takes effect at once (POST /api/remind/settings). */
  setRemindSettings(input: { notify: boolean }): Promise<RemindSettings>;
  /** The night shift: switch, window, planned and finished runs (GET /api/night); an older engine answers 404. */
  night(): Promise<NightState>;
  /** Changes the switch, the window or `parallel` (POST /api/night/settings). */
  nightSettings(input: { enabled?: boolean; window?: string; parallel?: number }): Promise<NightState>;
  /** Plans a run for tonight (POST /api/night/plan); the engine says why when it refuses. */
  nightPlan(input: NightPlanInput): Promise<NightRun>;
  nightCancel(id: number): Promise<NightState>;
  /** Changes the limit of a planned run; empty removes it. */
  nightLimit(id: number, limit: string): Promise<NightState>;
  /** Removes a run's worktree and branch, or its copy; the page asked first. */
  nightDiscard(id: number, loseChanges: boolean): Promise<{ ok: boolean; note?: string }>;
  nightOpen(id: number): Promise<{ ok: boolean }>;
  /** Fetches a repo copy's night branch into the original repo (POST /api/night/:id/bring); the page asked first. */
  nightBring(id: number): Promise<{ ok: boolean; note?: string }>;
  /** Calls onChange when the data changed; reports connection state. Returns unsubscribe. */
  subscribe(onChange: () => void, onState: (s: Connection) => void): () => void;
}

export type Connection = 'connecting' | 'live' | 'offline' | 'fixture';
