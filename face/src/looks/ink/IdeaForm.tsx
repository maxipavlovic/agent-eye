import { useEffect, useRef } from 'preact/hooks';
import { signal, useSignal } from '@preact/signals';
import { Pix } from '../../pixel/Pix';
import type { Store } from '../../store';
import type { Card, CardDetail } from '../../api/types';
import { homePath } from '../../format';
import { ICONS, INK } from './art';

/** The idea form: a new idea, or the idea card `edit` (its detail is loaded first); `focus` picks the first field. */
export const ideaForm = signal<{ edit?: string; focus?: 'prompt' } | null>(null);

/** Folders of the sessions on the board, most recently active first, each once. */
export function recentFolders(cards: Card[]): string[] {
  const seen = new Set<string>();
  return [...cards]
    .filter((c) => c.kind !== 'idea' && c.folder)
    .sort((a, b) => (b.last_activity_at ?? 0) - (a.last_activity_at ?? 0))
    .map((c) => c.folder!)
    .filter((f) => !seen.has(f) && !!seen.add(f));
}

/** Recent folders offered under the folder field, at most six: all of them while the field is empty or
 *  holds one of them (so another is one click away), else the ones matching what is typed. */
export function folderChoices(folders: string[], typed: string): string[] {
  const q = typed.trim().toLowerCase();
  const hit = (f: string) => [f.toLowerCase(), homePath(f).toLowerCase()];
  if (!q || folders.some((f) => hit(f).includes(q))) return folders.slice(0, 6);
  return folders.filter((f) => hit(f).some((p) => p.includes(q))).slice(0, 6);
}

/** New idea, or (with `idea`) an idea being changed. Notes are yours; only the prompt goes to the agent. */
function IdeaForm({ store, idea, focus, onClose }: { store: Store; idea?: CardDetail; focus?: 'prompt'; onClose: () => void }) {
  const title = useRef<HTMLInputElement>(null);
  const prompt = useRef<HTMLTextAreaElement>(null);
  const folder = useSignal(idea?.folder ?? '');
  const busy = useSignal(false);
  // The agent picker shows only when there is a choice; the default agent stands in until one is picked.
  const agents = store.agents.value;
  const chosen = useSignal(idea?.agent ?? '');
  const agent = agents.find((a) => a.id === chosen.value) ?? agents.find((a) => a.default) ?? agents[0];
  const who = agent?.short ?? 'the agent';
  const folders = recentFolders(store.board.value?.columns.flatMap((c) => c.cards) ?? []);
  const shown = folderChoices(folders, folder.value);
  const picked = homePath(folder.value.trim());
  useEffect(() => { (focus === 'prompt' ? prompt : title).current?.focus(); }, []);
  const submit = async (e: Event) => {
    e.preventDefault();
    if (busy.value) return;
    const f = e.currentTarget as HTMLFormElement;
    const v = (n: string) => (f.elements.namedItem(n) as HTMLInputElement).value.trim();
    if (!v('title')) { title.current?.focus(); return; }
    const input = { title: v('title'), notes: v('notes'), prompt: v('prompt'), folder: folder.value.trim(), agent: agents.length > 1 ? agent?.id : undefined };
    busy.value = true;
    const ok = idea?.idea_id != null
      ? await store.editIdea(idea.idea_id, input)
      : await store.addIdea({ title: input.title, notes: input.notes || undefined, prompt: input.prompt || undefined, folder: input.folder || undefined, agent: input.agent });
    busy.value = false;
    if (ok) onClose();
  };
  return (
    <>
      <div class="scrim" onClick={onClose} />
      <form class="modal" onSubmit={submit} aria-label={idea ? 'Edit idea' : 'New idea'}>
        <div class="dis" />
        <h2><Pix rows={ICONS.ideas} pal={INK} scale={4} /> {idea ? 'EDIT IDEA' : 'NEW IDEA'}</h2>
        <label>TITLE<input ref={title} name="title" defaultValue={idea?.title ?? ''} placeholder="What is the idea?" autocomplete="off" /></label>
        <label>NOTES<span class="hint">For you. Never sent to {who}.</span>
          <textarea name="notes" rows={4} defaultValue={idea?.notes ?? ''} placeholder="Lists, links, anything to keep" />
        </label>
        {agents.length > 1 && (
          <fieldset class="pick">
            <legend>AGENT</legend>
            <div class="fchips">
              {agents.map((a) => (
                <button type="button" key={a.id} title={a.name} aria-pressed={a.id === agent?.id} onClick={() => { chosen.value = a.id; }}>{a.short}</button>
              ))}
            </div>
          </fieldset>
        )}
        <label>PROMPT FOR {who.toUpperCase()}<span class="hint">Sent when you start it. Empty: {who} opens and waits for you.</span>
          <textarea ref={prompt} name="prompt" rows={3} defaultValue={idea?.prompt ?? ''} placeholder={`What should ${who} do first?`} />
        </label>
        <label>FOLDER<input name="folder" value={folder.value} onInput={(e) => { folder.value = e.currentTarget.value; }} placeholder="~/code/…" autocomplete="off" spellcheck={false} /></label>
        {shown.length > 0 && (
          <div class="fchips" role="group" aria-label="Recent folders">
            {shown.map((f) => (
              <button type="button" key={f} title={f} aria-pressed={homePath(f) === picked} onClick={() => { folder.value = homePath(f); }}>{homePath(f)}</button>
            ))}
          </div>
        )}
        <div class="acts end">
          <button type="button" class="btn ghost" onClick={onClose}>CANCEL</button>
          <button type="submit" class="btn big" disabled={busy.value}>SAVE IDEA</button>
        </div>
      </form>
    </>
  );
}

/** The idea form when asked for; editing waits until the idea's detail (with notes and prompt) is loaded. */
export function IdeaFormHost({ store }: { store: Store }) {
  const f = ideaForm.value;
  const d = store.detail.value;
  if (!f) return null;
  const close = () => { ideaForm.value = null; };
  if (!f.edit) return <IdeaForm key="new" store={store} onClose={close} />;
  if (!d || d.id !== f.edit || d.prompt === undefined) return null;
  return <IdeaForm key={f.edit} store={store} idea={d} focus={f.focus} onClose={close} />;
}
