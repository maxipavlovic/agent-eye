import type { Card, ColumnId } from '../../api/types';
import { ago, isBusy, nextParts } from '../../format';

const DAY = 86_400_000;

/**
 * The one thing worth knowing about a lane, under its name: how long the oldest ask has waited,
 * who you are waiting on, how many are working right now.
 */
export function laneFact(id: ColumnId, cards: readonly Card[], now = Date.now()): string {
  const times = cards.map((c) => c.last_activity_at).filter((t): t is number => !!t);
  switch (id) {
    case 'needs_you': {
      if (!cards.length) return 'nothing waits on you';
      if (!times.length) return `${cards.length} waiting on you`;
      const oldest = ago(Math.min(...times), now);
      return oldest === 'now' ? 'just asked' : `oldest ask ${oldest}`;
    }
    case 'waiting': {
      if (!cards.length) return 'nobody to wait for';
      const names = [...new Set(cards.map((c) => nextParts(c.next, c.agent_name)).filter((n) => n.who === 'others' && n.name).map((n) => n.name!))];
      if (!names.length) return 'on others';
      // two names fit a lane's width; the rest is a count
      return `on ${names.slice(0, 2).join(', ')}${names.length > 2 ? ` +${names.length - 2}` : ''}`;
    }
    case 'running': {
      const busy = cards.filter(isBusy).length;
      return busy ? `${busy} working now` : 'none working right now';
    }
    case 'paused': {
      const week = times.filter((t) => now - t < 7 * DAY).length;
      return week ? `${week} this week` : 'none this week';
    }
    case 'ideas':
    case 'done': {
      if (!cards.length) return id === 'ideas' ? 'no ideas yet' : 'nothing done yet';
      if (!times.length) return id === 'ideas' ? `${cards.length} parked` : `${cards.length} finished`;
      const newest = ago(Math.max(...times), now);
      return newest === 'now' ? 'newest just now' : `newest ${newest}`;
    }
  }
}

/** Two weeks, the span of a lane's timeline. */
export const SPAN_MS = 14 * DAY;

/**
 * Where a session's tick sits on a lane's timeline, 0 (two weeks ago or earlier) to 1 (now): a log
 * scale in hours, so the last day takes room and last week does not crowd it.
 */
export function tickAt(t: number | null | undefined, now = Date.now()): number | null {
  if (!t) return null;
  const hours = Math.max(0, (now - t) / 3_600_000);
  return Math.max(0, 1 - Math.log1p(hours) / Math.log1p(SPAN_MS / 3_600_000));
}
