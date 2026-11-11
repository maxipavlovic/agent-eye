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

export interface CardDetail extends Card {
  /** Up to the last 10 prompts, oldest first. */
  prompts: string[];
  first_prompt: string | null;
  last_assistant_text: string | null;
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
  score: number;
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
  move(id: string, column: MoveTarget): Promise<Card>;
  /** Manual hide / unhide of a session (POST /api/cards/:id/hidden). */
  setHidden(id: string, hidden: boolean): Promise<Card>;
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
  /** Calls onChange when the data changed; reports connection state. Returns unsubscribe. */
  subscribe(onChange: () => void, onState: (s: Connection) => void): () => void;
}

export type Connection = 'connecting' | 'live' | 'offline' | 'fixture';
