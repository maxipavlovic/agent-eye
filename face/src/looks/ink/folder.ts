import type { Card, FolderCheck } from '../../api/types';

type Folder = Pick<Card, 'folder_state' | 'folder_creatable' | 'folder_note'>;

/** The start button of an idea: its label, and the reason it cannot be pressed (null when it can).
 *  A missing folder that eye may make is created by the start, so the label says so. */
export function startButton(c: Folder): { label: string; blocked: string | null } {
  if (c.folder_state === 'missing' && c.folder_creatable) return { label: 'CREATE FOLDER + START', blocked: null };
  if (c.folder_state && c.folder_state !== 'ok') return { label: 'START', blocked: c.folder_note || 'The folder cannot be used' };
  return { label: 'START', blocked: null };
}

/** The line under the folder field: nothing while the folder is fine or not checked yet. */
export function folderHint(check: FolderCheck | null): string | null {
  return check && check.state !== 'ok' ? check.note : null;
}

/** Does this idea card's folder not exist yet (so the card carries the quiet mark)? */
export const folderMissing = (c: Folder) => c.folder_state === 'missing';
