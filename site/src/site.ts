import './site.css';
import { trackEye } from '../../face/src/looks/ink/eyeTracker';

type ThemePref = 'light' | 'dark' | 'auto';

// The board uses the same key, so the demo opens in the theme chosen here.
const THEME_KEY = 'eye.theme';
const system = matchMedia('(prefers-color-scheme: dark)');

function savedTheme(): ThemePref {
  try {
    const v = localStorage.getItem(THEME_KEY);
    if (v === 'light' || v === 'dark' || v === 'auto') return v;
  } catch { /* storage blocked */ }
  return 'light';
}

function applyTheme(pref: ThemePref) {
  const theme = pref === 'auto' ? (system.matches ? 'dark' : 'light') : pref;
  document.documentElement.dataset.theme = theme;
  for (const b of document.querySelectorAll<HTMLElement>('[data-theme-set]')) b.setAttribute('aria-pressed', String(b.dataset.themeSet === pref));
  // The demo board is on the same origin: keep it in step.
  try { document.querySelector('iframe')?.contentDocument?.documentElement.setAttribute('data-theme', theme); } catch { /* not loaded yet */ }
}

let pref = savedTheme();
applyTheme(pref);
system.addEventListener('change', () => applyTheme(pref));
document.querySelector('iframe')?.addEventListener('load', () => applyTheme(pref));
for (const b of document.querySelectorAll<HTMLElement>('[data-theme-set]')) {
  b.addEventListener('click', () => {
    pref = b.dataset.themeSet as ThemePref;
    try { localStorage.setItem(THEME_KEY, pref); } catch { /* storage blocked */ }
    applyTheme(pref);
  });
}

// Full screen: the demo frame fills the screen, and the same button (or Esc) brings it back.
// Without the Fullscreen API (iPhone) the link opens the board on its own page, which links back.
const frame = document.querySelector<HTMLElement>('.frame');
const full = document.querySelector<HTMLAnchorElement>('[data-fullscreen]');
if (frame && full && document.fullscreenEnabled) {
  full.addEventListener('click', (e) => {
    e.preventDefault();
    if (document.fullscreenElement) void document.exitFullscreen();
    else void frame.requestFullscreen();
  });
  document.addEventListener('fullscreenchange', () => {
    const on = document.fullscreenElement === frame;
    full.textContent = on ? 'EXIT FULL SCREEN (ESC)' : 'FULL SCREEN';
    full.setAttribute('aria-pressed', String(on));
    if (on) frame.querySelector('iframe')?.focus();
  });
}

// One eye follows the pointer: the big one on the home page, else the one in the header.
const eye = document.querySelector<HTMLElement>('.hero-eye .eye') ?? document.querySelector<HTMLElement>('.brand .eye');
if (eye) trackEye(eye);

// Copy buttons.
document.addEventListener('click', (e) => {
  const button = (e.target as Element).closest<HTMLButtonElement>('[data-copy]');
  if (!button) return;
  const done = () => { button.textContent = 'COPIED'; setTimeout(() => { button.textContent = 'COPY'; }, 1600); };
  navigator.clipboard?.writeText(button.dataset.copy ?? '').then(done, () => { button.textContent = 'PRESS CTRL-C'; });
});

// On a phone the docs menu starts folded.
if (matchMedia('(max-width: 860px)').matches) document.querySelector('.docmenu')?.removeAttribute('open');
