import type { Store } from '../store';

/** Save the session as one readable page (E, and EXPORT in MORE): secrets are masked. It is asked for with the token, so the page saves the file itself; a plain link could not send it. */
export async function exportSession(store: Store, id: string): Promise<void> {
  const file = await store.exportSession(id);
  if (!file) return;
  const url = URL.createObjectURL(file.blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = file.name;
  document.body.appendChild(a);
  a.click();
  a.remove();
  // the download has started by the time the click returns; keep the file a little longer anyway
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
}
