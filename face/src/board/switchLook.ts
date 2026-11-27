import type { LookName } from '../looks/names';
import { savePref } from '../prefs';
import type { Store } from '../store';

/**
 * The page's look switches by loading the page again (each look brings its own styles), without a `?look=`
 * that would pick the old one. When the engine refuses the setting, the page stays, a toast says so, and the
 * browser's copy goes back to what it was, so the next page does not open in the refused look either.
 */
export async function switchLook(store: Store, id: LookName) {
  let was: string | null = null;
  try { was = localStorage.getItem('eye.look'); } catch { /* storage blocked */ }
  if (!(await savePref('eye.look', id))) {
    try { if (was === null) localStorage.removeItem('eye.look'); else localStorage.setItem('eye.look', was); } catch { /* storage blocked */ }
    store.toast('The look could not be saved: eye refused it. Try again, or run eye doctor.', 'error');
    return;
  }
  const url = new URL(location.href);
  if (url.searchParams.has('look')) {
    url.searchParams.delete('look');
    location.replace(url.toString());
  } else {
    location.reload();
  }
}
