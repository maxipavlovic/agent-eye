import type { JSX } from 'preact';
import { Pix } from '../../pixel/Pix';
import type { Art, IconName, Look } from '../../board/look';
import { ARROW, BANG, CLOCK, CRESCENT, DOWN, EMPTY, ICONS, INK, INK_CC, MARKS, SIGNAL, UP, sigil } from './art';
import { Head } from './Head';
import { Header, ProjectBar } from './Header';
import { Welcome } from './Welcome';

const ON_ACCENT = { '#': 'var(--on-accent)' };
const ACCENT = { '#': 'var(--accent)' };

const ICON: Record<IconName, () => JSX.Element> = {
  arrow: () => <Pix rows={ARROW} pal={INK_CC} scale={2} />,
  bang: () => <Pix rows={BANG} pal={ON_ACCENT} scale={2} />,
  clock: () => <Pix rows={CLOCK} pal={ON_ACCENT} scale={2} />,
  'clock-accent': () => <Pix rows={CLOCK} pal={ACCENT} scale={2} />,
  crescent: () => <Pix rows={CRESCENT} pal={INK_CC} scale={2} />,
  signal: () => <Pix rows={SIGNAL} pal={INK_CC} scale={2} />,
  up: () => <Pix rows={UP} pal={INK_CC} scale={2} />,
  down: () => <Pix rows={DOWN} pal={INK_CC} scale={2} />,
};

/** Ink's drawings: one-bit pixel sprites, 2 CSS px per art pixel unless a size says otherwise. */
export const inkArt: Art = {
  lane: (id, size) => <Pix rows={ICONS[id]} pal={INK} scale={size === 'sm' ? 3 : 4} />,
  icon: (name) => ICON[name](),
  mark: (s, scale = 2, label) => <Pix rows={MARKS[s]} pal={INK} scale={scale} label={label} />,
  sigil: (id) => <Pix rows={sigil(id)} pal={INK} scale={2} />,
  empty: () => <Pix rows={EMPTY} pal={INK} scale={4} />,
};

function Loading() {
  return <div class="loading"><Pix rows={EMPTY} pal={INK} scale={8} /><span>LOADING<i class="dots" /></span></div>;
}

/** Ink: one-bit paper, black ink, dithered greys, one signal colour for what needs you. */
export const INK_LOOK: Look = { name: 'ink', art: inkArt, Header, ProjectBar, Head, Welcome, Loading };
