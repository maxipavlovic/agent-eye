import type { OriginChoice, OriginsSetting, SettingsInput } from '../api/types';
import { Part, Row, Switch, type Folds } from './SettingsParts';

/** The section's title. */
export const QUIET_PART = 'STARTED BY PROGRAMS';

/** "1 session", "5 sessions". */
const count = (n: number) => `${n} session${n === 1 ? '' : 's'}`;

/** What the folded heading says: how many sessions programs started here while none is quiet, else which are. */
export function quietSum(q: OriginsSetting): string {
  if (q.choices.length === 0) return 'NONE';
  const quiet = q.choices.filter((c) => q.value.includes(c.id));
  if (quiet.length === 0) return `${q.choices.reduce((n, c) => n + c.sessions, 0)} FOUND · OFF`;
  if (quiet.length === q.choices.length) return 'QUIET';
  return `${quiet.map((c) => c.name.toUpperCase()).join(', ')} QUIET`;
}

/**
 * The line under an origin's switch: its sessions and what quiet clears ("5 sessions started by an app through an
 * agent's SDK, 2 in Needs you now"). The engine counts Needs you as if nothing were quiet, so while it is on the line
 * says how many it keeps out.
 */
export function originLine(c: OriginChoice, quiet: boolean): string {
  if (c.sessions === 0) return `No sessions ${c.about} left.`;
  const asks = c.needs_you === 0 ? 'none' : String(c.needs_you);
  return `${count(c.sessions)} ${c.about}, ${asks} ${quiet ? 'kept out of Needs you' : 'in Needs you now'}.`;
}

/** The list with `id` switched on or off, in the order of the choices. */
export function quietList(q: OriginsSetting, id: string, on: boolean): string[] {
  return q.choices.map((c) => c.id).filter((x) => (x === id ? on : q.value.includes(x)));
}

/**
 * STARTED BY PROGRAMS: one switch for each program origin found on this machine (an app on an agent's SDK, a script's
 * headless run), saved at once. A quiet origin keeps its switch when none of its sessions are left.
 */
export function QuietSettings({ q, sample, save, folds }: { q: OriginsSetting; sample: boolean; save: (input: SettingsInput) => Promise<boolean>; folds: Folds }) {
  return (
    <Part t={QUIET_PART} sum={quietSum(q)} folds={folds}>
      {q.choices.length === 0 ? (
        <p class="umeta">No sessions started by a program on this machine.</p>
      ) : (
        <>
          {q.choices.map((c) => {
            const on = q.value.includes(c.id);
            return (
              <Row key={c.id} label={`Quiet ${c.name}`} restart={q.restart} help={originLine(c, on)}>
                <Switch label={`Quiet ${c.name}`} on={on} disabled={sample} onPick={(v) => void save({ 'board.quiet': quietList(q, c.id, v) })} />
              </Row>
            );
          })}
          <p class="umeta">What a program's agent asks is for that program, not for you. A quiet session never moves to Needs you by itself, gets no summary and stays out of HOURS. It is still listed, found and resumable, and a reminder or a move by hand still works.</p>
        </>
      )}
    </Part>
  );
}
