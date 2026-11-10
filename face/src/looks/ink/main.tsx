import { render } from 'preact';
import { createStore } from '../../store';
import { fixtureSource } from '../../data/fixture';
import { keepPrefsLocal } from '../../prefs';
import { Root } from './Root';
import './ink.css';

keepPrefsLocal();
const store = createStore(fixtureSource(), { defaultTheme: 'light' });
render(<Root store={store} />, document.getElementById('app')!);
