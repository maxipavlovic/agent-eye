import type { Store } from '../../store';
import { Board } from '../../board/Board';
import { LookProvider } from '../../board/look';
import { IRIS_LOOK } from './look';

/** The board in the iris look. */
export function Root({ store }: { store: Store }) {
  return <LookProvider look={IRIS_LOOK}><Board store={store} /></LookProvider>;
}
