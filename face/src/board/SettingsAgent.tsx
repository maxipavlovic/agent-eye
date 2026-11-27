import { useSignal } from '@preact/signals';
import type { SettingsInput, SettingsState } from '../api/types';
import type { Store } from '../store';
import { ModelPick } from './SettingsModel';
import { onOff, Part, restartNote, Row, Seg, Shown, Switch, type Folds } from './SettingsParts';
import { QuietSettings } from './SettingsQuiet';

const ISOLATION: ReadonlyArray<readonly ['worktree' | 'copy', string]> = [['worktree', 'WORKTREE'], ['copy', 'COPY']];

/**
 * The settings that live in `config.toml`: summaries, which programs' sessions are quiet, the night shift, reminders,
 * the steering clock and which agents eye follows. Each one the panel changes saves at once through POST /api/settings; the agents and the clock switch
 * need a restart, so the panel shows them and says how to change them. The sample board shows them but cannot save.
 */
export function AgentSettings({ store, state, save, folds }: { store: Store; state: SettingsState; save: (input: SettingsInput) => Promise<boolean>; folds: Folds }) {
  const s = state.settings;
  const sample = store.source === 'fixture';
  const win = useSignal(s['night.window'].value);
  const notice = store.notice.value;
  const agents = s['agents.enabled'];
  const nameOf = (id: string) => agents.choices.find((a) => a.id === id)?.name ?? id;
  const followed = agents.value.map(nameOf);
  const others = agents.choices.filter((a) => !agents.value.includes(a.id)).map((a) => a.name);
  const brk = s['clock.break_minutes'];
  const gap = useSignal(String(brk.value));
  const idle = s['clock.idle'];
  const clockOff = !s['clock.enabled'].value;
  const model = s['summary.model'];
  const modelNow = model.value || model.default;
  const saveWindow = async (e: Event) => {
    e.preventDefault();
    const w = win.value.trim();
    if (w === s['night.window'].value) return;
    win.value = (await save({ 'night.window': w })) ? w : s['night.window'].value;
  };
  const saveBreak = async (e: Event) => {
    e.preventDefault();
    const n = Number(gap.value.trim());
    if (!Number.isInteger(n) || n === brk.value) { gap.value = String(brk.value); return; }
    gap.value = (await save({ 'clock.break_minutes': n })) ? String(n) : String(brk.value);
  };
  return (
    <>
      {sample && <p class="umeta">This is the sample board: these settings are shown, but nothing is saved.</p>}
      <Part t="SUMMARIES" sum={s['summary.enabled'].value && modelNow ? `ON · ${modelNow}` : onOff(s['summary.enabled'].value)} folds={folds}>
        <Row label="Write summaries" restart={s['summary.enabled'].restart} help="Write the state and next step of quiet sessions with an agent, so the card says where you stopped.">
          <Switch label="Summaries" on={s['summary.enabled'].value} disabled={sample} onPick={(on) => void save({ 'summary.enabled': on })} />
        </Row>
        <ModelPick m={model} sample={sample} save={save} />
      </Part>
      <QuietSettings q={s['board.quiet']} sample={sample} save={save} folds={folds} />
      <Part t="NIGHT SHIFT" sum={s['night.enabled'].value ? `ON · ${s['night.window'].value}` : onOff(false)} folds={folds}>
        <Row label="Start by themselves" restart={s['night.enabled'].restart} help="Planned runs start by themselves inside the window, when nothing you work in is live.">
          <Switch label="Night shift" on={s['night.enabled'].value} disabled={sample} onPick={(on) => void save({ 'night.enabled': on })} />
        </Row>
        <form class="nset" onSubmit={saveWindow}>
          <label>WINDOW (LOCAL TIME)
            <input value={win.value} disabled={sample} onInput={(e) => { win.value = e.currentTarget.value; }} placeholder="01:00-07:00" autocomplete="off" spellcheck={false} />
          </label>
          <button class="btn sm" type="submit" disabled={sample}>SET WINDOW</button>
          <span class="hint">New runs start only inside it; an end before the start crosses midnight.{restartNote(s['night.window'].restart)}</span>
        </form>
        <div class="nset">
          <label>AT ONCE
            <select value={String(s['night.parallel'].value)} disabled={sample} onChange={async (e) => { const pick = e.currentTarget; if (!(await save({ 'night.parallel': Number(pick.value) }))) pick.value = String(s['night.parallel'].value); }}>
              {Array.from({ length: s['night.parallel'].max - s['night.parallel'].min + 1 }, (_, i) => String(s['night.parallel'].min + i)).map((k) => <option key={k} value={k}>{k}</option>)}
            </select>
          </label>
          <span class="hint">How many runs go at the same time.{restartNote(s['night.parallel'].restart)}</span>
        </div>
        <Row label="Work in" restart={s['night.isolation'].restart} help={s['night.isolation'].value === 'copy' ? 'A separate copy of your repo: your own folder is fully locked.' : 'A worktree of your repo, on a new branch: fast and small.'}>
          <Seg label="Work in" value={s['night.isolation'].value as 'worktree' | 'copy'} options={ISOLATION} disabled={sample} onPick={(v) => void save({ 'night.isolation': v })} />
        </Row>
      </Part>
      <Part t="REMINDERS" sum={onOff(s['remind.notify'].value)} folds={folds}>
        <Row label="Desktop notices" restart={s['remind.notify'].restart} help={notice && !notice.available ? 'This computer has no program to show a notice.' : 'One desktop notice when a reminder you set falls due. It never leaves this computer.'}>
          <Switch label="Desktop notices" on={s['remind.notify'].value} disabled={sample || (!!notice && !notice.available)} onPick={(on) => void save({ 'remind.notify': on })} />
        </Row>
      </Part>
      <Part t="HOURS" sum={onOff(s['clock.enabled'].value)} folds={folds}>
        <Shown label="Steering clock" value={onOff(s['clock.enabled'].value)} file="[clock] enabled" restart={s['clock.enabled'].restart} help="Notes when you prompt and when the agent finishes, to show which hours you steer well and which badly. Timings and sizes only, never text." />
        <Row label="Judge corrections" restart={s['clock.llm'].restart} help="Let your own summary agent tell which prompts were corrections, once a night. This sends the prompt and the reply before it to that agent, as summaries do. Off: corrections stay at zero, and HOURS shows only timing and tool errors.">
          <Switch label="Judge corrections" on={s['clock.llm'].value} disabled={sample || clockOff} onPick={(on) => void save({ 'clock.llm': on })} />
        </Row>
        <Row label="Hint" restart={s['clock.hints'].restart} help="A quiet mark beside HOURS while you are in an hour where your corrections usually rise. It never pops up.">
          <Switch label="Hint" on={s['clock.hints'].value} disabled={sample || clockOff} onPick={(on) => void save({ 'clock.hints': on })} />
        </Row>
        <form class="nset" onSubmit={saveBreak}>
          <label>BREAK AFTER (MINUTES)
            <input type="number" min={brk.min} max={brk.max} value={gap.value} disabled={sample || clockOff} onInput={(e) => { gap.value = e.currentTarget.value; }} autocomplete="off" />
          </label>
          <button class="btn sm" type="submit" disabled={sample || clockOff}>SET</button>
          <span class="hint">A gap this long between prompts counts as a break ({brk.min} to {brk.max}).{restartNote(brk.restart)}</span>
        </form>
        <Row label="Idle time" restart={s['clock.idle'].restart} help={idle.available === false ? 'Not available on this system.' : 'Count breaks from keyboard and mouse idle time. Only a number of seconds is read; never what you type.'}>
          <Switch label="Idle time" on={idle.value} disabled={sample || clockOff || idle.available === false} onPick={(on) => void save({ 'clock.idle': on })} />
        </Row>
      </Part>
      <Part t="AGENTS" sum={followed.join(', ') || 'NONE'} folds={folds}>
        <Shown label="Followed" value={followed.join(', ') || 'None'} file="[agents] enabled" restart={agents.restart} help="The first is the default for new ideas and summaries. A switched-off agent's sessions stay and come back when it is on again.">
          {others.length > 0 && <p class="umeta soff">Not followed: {others.join(', ')}.</p>}
        </Shown>
      </Part>
    </>
  );
}
