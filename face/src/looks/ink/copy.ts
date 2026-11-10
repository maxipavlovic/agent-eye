import type { Store } from '../../store';

/** Copies a resume command to the clipboard and says so in a toast. */
export function copy(text: string, store: Store) {
  const done = () => store.toast('Command copied');
  const fallback = () => {
    try {
      const ta = document.createElement('textarea');
      ta.value = text;
      document.body.appendChild(ta);
      ta.select();
      document.execCommand('copy');
      ta.remove();
      done();
    } catch { store.toast('Copy failed', 'error'); }
  };
  if (navigator.clipboard) navigator.clipboard.writeText(text).then(done, fallback);
  else fallback();
}
