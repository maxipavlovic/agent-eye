import { render } from 'preact';
import { createStore } from './store';
import { liveSource } from './data/live';
import { looks, pickLook } from './looks';
import { pullPrefs } from './prefs';

// Saved settings first, so the theme and layout are right on the first frame.
void pullPrefs().then(async () => {
  const store = createStore(liveSource());
  const name = pickLook();
  document.documentElement.dataset.look = name;
  const Root = await looks[name]!();
  render(<Root store={store} />, document.getElementById('app')!);
});
