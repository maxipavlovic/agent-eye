import { createContext, type ComponentChildren, type ComponentType, type JSX } from 'preact';
import { useContext } from 'preact/hooks';
import type { Agent, Column, ColumnId, TopicStatus } from '../api/types';
import type { Store } from '../store';
import type { Project } from './model';

/**
 * The small drawings the shared board asks for. Each look draws them its own way (ink: pixel
 * sprites, iris: hairlines); the board only says which one and how big.
 */
export interface Art {
  /** A lane's object: `sm` on a folded rail and a grid tile, `lg` in a form's title. */
  lane(id: ColumnId, size: 'sm' | 'lg'): JSX.Element;
  /** A small icon in the text: on a chip, a mark box, a stepper. */
  icon(name: IconName): JSX.Element;
  /** The status mark of a topic or a sub-agent. */
  mark(status: TopicStatus, scale?: number, label?: string): JSX.Element;
  /** The stamp that tells sessions apart; null for a look that draws none. */
  sigil(id: string): JSX.Element | null;
  /** What an empty lane shows above its words. */
  empty(): JSX.Element | null;
}

/**
 * - arrow: who acts next, on a chip.
 * - bang, clock: a mark box in the accent (Needs you, a reminder).
 * - clock-accent: the reminder clock beside the time, in the accent.
 * - crescent, sunset, signal: the calm mark boxes (a night run, the clock's hint, Remote Control).
 * - up, down: the time stepper.
 */
export type IconName = 'arrow' | 'bang' | 'clock' | 'clock-accent' | 'crescent' | 'sunset' | 'signal' | 'up' | 'down';

export interface HeaderProps { store: Store }
export interface ProjectBarProps { store: Store; total: number; ps: Project[]; filter: string | null; setFilter: (key: string | null) => void }
/** A lane's header. `inline` puts the name beside the icon (the grid); `fold` folds the lane. */
export interface HeadProps { col: Column; inline?: boolean; fold?: () => void }
/** A grid lane drawn as one line: folded by hand (`open` unfolds it) or empty. */
export interface TileProps { col: Column; open?: () => void }

/** Everything that makes a look a look; the rest of the board is shared. */
export interface Look {
  /** Names the look's saved settings (`eye.<name>.layout`, `.fold`, `.panel`). */
  name: string;
  art: Art;
  Header: ComponentType<HeaderProps>;
  ProjectBar: ComponentType<ProjectBarProps>;
  Head: ComponentType<HeadProps>;
  /** A board with no sessions and an empty archive. */
  Welcome: ComponentType<{ agents: readonly Agent[] }>;
  /** Before the first board arrives. */
  Loading: ComponentType;
  /** Under the board, when the look has something there. */
  Footer?: ComponentType<{ store: Store }>;
  /** A grid lane folded or empty; without it the grid draws its own plain tile. */
  Tile?: ComponentType<TileProps>;
}

const LookCtx = createContext<Look | null>(null);

export function LookProvider({ look, children }: { look: Look; children?: ComponentChildren }) {
  return <LookCtx.Provider value={look}>{children}</LookCtx.Provider>;
}

/** The look the board is drawn in. */
export function useLook(): Look {
  const look = useContext(LookCtx);
  if (!look) throw new Error('the board needs a look: wrap it in <LookProvider>');
  return look;
}

/** The art of the look the board is drawn in. */
export const useArt = (): Art => useLook().art;
