import type { Store } from '../../store';
import { Board } from '../../board/Board';
import { LookProvider } from '../../board/look';
import { INK_LOOK } from './look';

/** The board in the ink look. */
export function Root({ store }: { store: Store }) {
  return <LookProvider look={INK_LOOK}><Board store={store} /></LookProvider>;
}
