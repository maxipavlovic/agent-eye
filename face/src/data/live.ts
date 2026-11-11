import type {
  Agent, ArchivePage, Board, Card, CardDetail, Connection, ConversationPage, Idea, IdeaInput, MoveTarget, ResumePlan,
  FolderCheck, SearchHit, Source, StartPlan,
} from '../api/types';

const UNAUTHORIZED = 'Not signed in: open the link that `eye board` prints.';
/** Consecutive event-stream errors before the badge says offline. */
const OFFLINE_AFTER = 3;

export class ApiError extends Error {
  constructor(message: string, readonly status: number) {
    super(message);
    this.name = 'ApiError';
  }
}

async function call<T>(path: string, init?: { method: 'POST'; body: unknown }): Promise<T> {
  let res: Response;
  try {
    res = await fetch(path, init
      ? { method: 'POST', credentials: 'same-origin', headers: { 'content-type': 'application/json' }, body: JSON.stringify(init.body) }
      : { credentials: 'same-origin' });
  } catch {
    throw new ApiError('Cannot reach eye. Is `eye serve` running?', 0);
  }
  if (res.status === 401) throw new ApiError(UNAUTHORIZED, 401);
  let data: unknown = null;
  try { data = await res.json(); } catch { /* not JSON */ }
  if (!res.ok) {
    const msg = data && typeof data === 'object' && 'error' in data ? String((data as { error: unknown }).error) : `HTTP ${res.status}`;
    throw new ApiError(msg, res.status);
  }
  return data as T;
}

const enc = encodeURIComponent;

/** A Source over the real eye API, same origin. The cookie comes from the `/?t=<token>` redirect. */
export function liveSource(): Source {
  return {
    name: 'live',
    board: () => call<Board>('/api/board'),
    card: (id) => call<CardDetail>(`/api/cards/${enc(id)}`),
    archive: (offset, limit) => call<ArchivePage>(`/api/archive?offset=${offset}&limit=${limit}`),
    search: (q) => call<{ results: SearchHit[] }>(`/api/search?q=${enc(q)}`),
    ideas: () => call<{ ideas: Idea[] }>('/api/ideas'),
    agents: () => call<{ agents: Agent[] }>('/api/agents'),
    move: (id: string, column: MoveTarget) => call<Card>(`/api/cards/${enc(id)}/column`, { method: 'POST', body: { column } }),
    setHidden: (id, hidden) => call<Card>(`/api/cards/${enc(id)}/hidden`, { method: 'POST', body: { hidden } }),
    conversation: (id, limit, before) =>
      call<ConversationPage>(`/api/cards/${enc(id)}/conversation?limit=${limit}${before === null ? '' : `&before=${before}`}`),
    resume: (id, dryRun, fork = false) =>
      call<ResumePlan>(`/api/cards/${enc(id)}/resume`, { method: 'POST', body: fork ? { dry_run: dryRun, fork: true } : { dry_run: dryRun } }),
    addIdea: (input: IdeaInput) => call<Card>('/api/ideas', { method: 'POST', body: input }),
    editIdea: (ideaId, input) => call<Card>(`/api/ideas/${ideaId}`, { method: 'POST', body: input }),
    startIdea: (ideaId, background, dryRun, createFolder = false) =>
      call<StartPlan>(`/api/ideas/${ideaId}/start`, { method: 'POST', body: createFolder ? { background, dry_run: dryRun, create_folder: true } : { background, dry_run: dryRun } }),
    folderCheck: (path) => call<FolderCheck>(`/api/folder?path=${enc(path)}`),
    subscribe(onChange, onState) {
      let errors = 0;
      const set = (s: Connection) => onState(s);
      const es = new EventSource('/api/events');
      es.onopen = () => { errors = 0; set('live'); };
      es.addEventListener('board-changed', () => onChange());
      es.onerror = () => { errors++; set(errors >= OFFLINE_AFTER ? 'offline' : 'connecting'); };
      const onVisible = () => { if (document.visibilityState === 'visible') onChange(); };
      document.addEventListener('visibilitychange', onVisible);
      set('connecting');
      return () => {
        es.close();
        document.removeEventListener('visibilitychange', onVisible);
      };
    },
  };
}
