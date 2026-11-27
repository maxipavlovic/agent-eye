import { useLayoutEffect } from 'preact/hooks';
import type { Signal } from '@preact/signals';

/**
 * Esc closes a side panel while it is open. It listens from before the paint, so the first Esc closes this panel
 * and not the one under it.
 */
export function usePanelEsc(open: Signal<boolean>): void {
  const on = open.value;
  useLayoutEffect(() => {
    if (!on) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.stopImmediatePropagation();
      open.value = false;
    };
    addEventListener('keydown', onKey, true);
    return () => removeEventListener('keydown', onKey, true);
  }, [on]);
}
