import type { Worked } from '../../api/types';
import { stripCells, workedText } from './worked';

/** The WORKED ON rows of the WHERE section: a line of text and, for a session worked on over several days, a strip of days. */
export function WorkedRows({ w }: { w: Worked | null | undefined }) {
  if (!w) return null;
  return (
    <>
      <dt>WORKED ON</dt>
      <dd>{workedText(w)}</dd>
      {w.span_days > 1 && (
        <>
          <dt></dt>
          <dd class="wstrip">
            {w.more_days > 0 && <small>+{w.more_days} earlier days</small>}
            {stripCells(w).map((c) => <i key={c.day} class={`wcell l${c.level}`} title={c.title} />)}
          </dd>
        </>
      )}
    </>
  );
}
