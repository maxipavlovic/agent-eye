import type { JSX } from 'preact';
import type { ColumnId, TopicStatus } from '../../api/types';
import type { Art, IconName } from '../../board/look';

/*
 * Iris draws everything with one line: a path of class `s` is stroked in currentColor at one device
 * pixel whatever the size (iris.css). Fills are kept for the few solid things: a pupil, a clapper,
 * the dot of a `!`.
 */

/**
 * The lane objects, each drawn with one line on a 32-unit grid. Their parts move in iris.css, gently and only
 * when it means something: the reel turns while a session works, the bell swings while something waits on
 * you, the hourglass turns over while you wait on others, the moon's star twinkles, the bulb lights and the
 * flag waves under the pointer. With reduced motion they keep still.
 */
const LANE: Record<ColumnId, string> = {
  ideas: '<g class="rays"><path class="s" d="M16 1.75v2M4.75 6.5l1.5 1.4M27.25 6.5l-1.5 1.4M2.5 14.5h2M27.5 14.5h2"/></g><path class="s" d="M12.5 21.5c0-2.8-4.6-4.6-4.6-9.6a8.1 8.1 0 0 1 16.2 0c0 5-4.6 6.8-4.6 9.6Z"/><path class="s" d="M12.5 24.5h7M13.75 27.25h4.5"/><path class="s fil" d="M14 21.5v-5.25l2-2.5 2 2.5v5.25"/>',
  running: '<g class="reel"><circle class="s" cx="16" cy="16" r="11.5"/><circle class="s" cx="16" cy="16" r="2.25"/><circle class="s" cx="16" cy="9.25" r="2.75"/><circle class="s" cx="21.85" cy="19.375" r="2.75"/><circle class="s" cx="10.15" cy="19.375" r="2.75"/></g>',
  needs_you: '<g class="ring"><path class="s" d="M5.75 11.5a10.5 10.5 0 0 1 2.75-4.75M26.25 11.5a10.5 10.5 0 0 0-2.75-4.75"/></g><g class="swing"><path class="s" d="M8.75 22.5h14.5l-2-2.75v-6a5.25 5.25 0 0 0-10.5 0v6Z"/><path class="s" d="M16 6.25v2.25"/><circle class="clap" cx="16" cy="25.5" r="1.9"/></g>',
  waiting: '<g class="flip"><path class="s" d="M9.25 5.25h13.5M9.25 26.75h13.5M11 5.25c0 6.25 4.75 7.5 4.75 10.75S11 20.5 11 26.75M21 5.25c0 6.25-4.75 7.5-4.75 10.75S21 20.5 21 26.75M13 25l3-3.75L19 25Z"/></g>',
  paused: '<path class="s" d="M21.6 6.1A10.5 10.5 0 1 0 26.4 21a8.6 8.6 0 0 1-4.8-14.9Z"/><g class="star"><path class="s" d="M24.75 5.5v3.5M23 7.25h3.5"/></g>',
  done: '<path class="s" d="M9.25 27.5V4.75"/><g class="cloth"><path class="s" d="M9.25 6h15l-3.25 4.75 3.25 4.75h-15"/></g>',
};

/** A lane's object at `px` CSS pixels. */
export function LaneObject({ id, px, cls = 'lic' }: { id: ColumnId; px: number; cls?: string }) {
  return <svg class={`${cls} o-${id}`} width={px} height={px} viewBox="0 0 32 32" aria-hidden="true" dangerouslySetInnerHTML={{ __html: LANE[id] }} />;
}

const svg = (w: number, h: number, body: JSX.Element, cls = 'ico') => (
  <svg class={cls} width={w} height={h} viewBox={`0 0 ${w} ${h}`} aria-hidden="true">{body}</svg>
);

/** The header's Lanes | Grid switch. */
export const LAYOUT_ICON = {
  lanes: () => svg(20, 20, <path class="s" d="M2.25 3.25h4v13.5h-4zM8 3.25h4v13.5H8zM13.75 3.25h4v13.5h-4z" />),
  grid: () => svg(20, 20, <path class="s" d="M2.75 2.75h6v6h-6zM11.25 2.75h6v6h-6zM2.75 11.25h6v6h-6zM11.25 11.25h6v6h-6z" />),
};

/** The header's theme switch: light, dark, as the system is. */
export const THEME_ICON = {
  light: () => svg(20, 20, <><circle class="s" cx="10" cy="10" r="3.75" /><path class="s" d="M10 1.75v2.5M10 15.75v2.5M1.75 10h2.5M15.75 10h2.5M4.15 4.15l1.8 1.8M14.05 14.05l1.8 1.8M4.15 15.85l1.8-1.8M14.05 5.95l1.8-1.8" /></>),
  dark: () => svg(20, 20, <path class="s" d="M16.2 12.6A7 7 0 0 1 7.4 3.8a7 7 0 1 0 8.8 8.8Z" />),
  auto: () => svg(20, 20, <><circle class="s" cx="10" cy="10" r="7.25" /><path d="M10 2.75a7.25 7.25 0 0 0 0 14.5Z" fill="currentColor" /></>),
};

const ICON: Record<IconName, () => JSX.Element> = {
  arrow: () => svg(16, 8, <path class="s" d="M0 4h15M11.5.5 15 4l-3.5 3.5" />, 'ico arrow'),
  bang: () => svg(14, 14, <><rect x="6.35" y="2.4" width="1.3" height="6.2" fill="currentColor" /><circle cx="7" cy="10.9" r="1" fill="currentColor" /></>),
  clock: () => svg(14, 14, <><circle class="s" cx="7" cy="7" r="5.5" /><path class="s" d="M7 3.8V7l2.2 1.4" /></>),
  'clock-accent': () => svg(14, 14, <><circle class="s" cx="7" cy="7" r="5.5" /><path class="s" d="M7 3.8V7l2.2 1.4" /></>, 'ico accent'),
  crescent: () => svg(14, 14, <path class="s" d="M11.6 9.2A5 5 0 0 1 4.8 2.4a5 5 0 1 0 6.8 6.8Z" />),
  signal: () => svg(14, 14, <><circle cx="7" cy="10" r=".9" fill="currentColor" /><path class="s" d="M4.6 7.6a3.4 3.4 0 0 1 4.8 0M2.6 5.6a6.2 6.2 0 0 1 8.8 0" /></>),
  up: () => svg(12, 7, <path class="s" d="M1.5 5.5 6 1l4.5 4.5" />),
  down: () => svg(12, 7, <path class="s" d="M1.5 1.5 6 6l4.5-4.5" />),
};

const MARK: Record<TopicStatus, () => JSX.Element> = {
  done: () => <circle cx="6" cy="6" r="3.2" fill="currentColor" />,
  active: () => <><circle class="s" cx="6" cy="6" r="4.5" /><circle cx="6" cy="6" r="1.4" fill="currentColor" /></>,
  paused: () => <path class="s" d="M4.5 3v6M7.5 3v6" />,
  waiting_on_others: () => <path class="s" d="M2 6h8M7 3l3 3-3 3" />,
  needs_you: () => <circle class="acc-fill" cx="6" cy="6" r="3.6" />,
};

/** Iris's drawings: hairlines, no stamps per session (a card is told apart by its words). */
export const irisArt: Art = {
  lane: (id, size) => <LaneObject id={id} px={size === 'sm' ? 24 : 28} />,
  icon: (name) => ICON[name](),
  mark: (s, scale = 2, label) => {
    const px = 6 * scale;
    return (
      <svg class="mk-s" width={px} height={px} viewBox="0 0 12 12" role={label ? 'img' : undefined} aria-label={label} aria-hidden={label ? undefined : true}>{MARK[s]()}</svg>
    );
  },
  sigil: () => null,
  empty: () => null,
};

/** The fold mark of a lane's header: two short arrows pointing in. */
export const fold = () => svg(14, 10, <path class="s" d="M1 1l4 4-4 4M13 1 9 5l4 4" />);
