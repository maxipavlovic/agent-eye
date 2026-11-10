import type { Store } from '../../store';
import type { ColumnId } from '../../api/types';
import type { Project } from './model';

/** Lane label (shown in the pixel font), the small note beside it, and the text of an empty lane. */
export const LANE_LABEL: Record<ColumnId, { name: string; sub?: string; empty: string }> = {
  ideas: { name: 'IDEAS', empty: 'NO IDEAS YET' },
  running: { name: 'RUNNING', empty: 'NOTHING RUNNING' },
  needs_you: { name: 'NEEDS YOU', empty: 'ALL CLEAR' },
  paused: { name: 'PAUSED', empty: 'NOTHING PAUSED' },
  waiting: { name: 'WAITING', sub: 'on others', empty: 'NOBODY TO WAIT FOR' },
  done: { name: 'DONE', empty: 'NOTHING DONE YET' },
};

/** Board order, whatever order the source sends. */
export const ORDER: ColumnId[] = ['ideas', 'running', 'needs_you', 'waiting', 'paused', 'done'];

/** Every lane but Needs you can be folded to a rail by hand. */
export const FOLDABLE: ReadonlySet<ColumnId> = new Set(['ideas', 'running', 'waiting', 'paused', 'done']);

/** How a card is drawn: the face of each lane's cards. */
export type Kind = 'need' | 'run' | 'wait' | 'idea' | 'row';

export const KIND: Record<ColumnId, Kind> = { ideas: 'idea', running: 'run', needs_you: 'need', paused: 'row', waiting: 'wait', done: 'row' };

/** What the lanes and the grid share: the store, the projects shown, and whether cards carry a project tag. */
export interface BoardCtx { store: Store; ps: Project[]; showTags: boolean }
