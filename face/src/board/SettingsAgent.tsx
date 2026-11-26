import { useSignal } from '@preact/signals';
import type { SettingsInput, SettingsState } from '../api/types';
import type { Store } from '../store';
import { Sec } from './Sections';
import { Row, Seg } from './SettingsParts';

const ON_OFF: ReadonlyArray<readonly ['on' | 'off', string]> = [['on', 'ON'], ['off', 'OFF']];
const ISOLATION: ReadonlyArray<readonly ['worktree' | 'copy', string]> = [['worktree', 'WORKTREE'], ['copy', 'COPY']];

const when = (restart?: boolean) => (restart ? ' Takes effect when the background service restarts.' : '');

/** ON or OFF for one on/off key. */
function Switch({ label, on, disabled, onPick }: { label: string; on: boolean; disabled: boolean; onPick: (on: boolean) => void }) {
  return <Seg label={label} value={on ? 'on' : 'off'} options={ON_OFF} disabled={disabled} onPick={(v) => onPick(v === 'on')} />;
}

/**
 * The settings that live in `config.toml`: summaries, the night shift, reminders and which agents eye follows. Each one saves
 * at once through POST /api/settings. The sample board shows them but cannot save.
 */
export function AgentSettings({ store, state, save }: { store: Store; state: SettingsState; save: (input: SettingsInput) => Promise<boolean> }) {
  const s = state.settings;
  const sample = store.source === 'fixture';
  const model = useSignal(s['summary.model'].value);
  const win = useSignal(s['night.window'].value);
  const notice = store.notice.value;
  const agents = s['agents.enabled'];
  const saveModel = async (e: Event) => {
    e.preventDefault();
    const m = model.value.trim();
    if (m === s['summary.model'].value) return;
    model.value = (await save({ 'summary.model': m })) ? m : s['summary.model'].value;
  };
  const saveWindow = async (e: Event) => {
    e.preventDefault();
    const w = win.value.trim();
    if (w === s['night.window'].value) return;
    win.value = (await save({ 'night.window': w })) ? w : s['night.window'].value;
  };
  return (
    <>
      {sample && <p class="umeta">This is the sample board: these settings are shown, but nothing is saved.</p>}
      <Sec t="SUMMARIES">
        <Row label="Summaries" help={`Write the state and next step of quiet sessions with an agent, so the card says where you stopped.${when(s['summary.enabled'].restart)}`}>
          <Switch label="Summaries" on={s['summary.enabled'].value} disabled={sample} onPick={(on) => void save({ 'summary.enabled': on })} />
        </Row>
        <form class="nset" onSubmit={saveModel}>
          <label>MODEL
            <input value={model.value} maxLength={s['summary.model'].max} disabled={sample} onInput={(e) => { model.value = e.currentTarget.value; }} placeholder="the agent's own cheap model" autocomplete="off" spellcheck={false} />
          </label>
          <button class="btn sm" type="submit" disabled={sample}>SET MODEL</button>
          <span class="hint">The model name handed to the summary agent; empty uses its own default.{when(s['summary.model'].restart)}</span>
        </form>
      </Sec>
      <Sec t="NIGHT SHIFT">
        <Row label="Night shift" help="Planned runs start by themselves inside the window, when nothing you work in is live.">
          <Switch label="Night shift" on={s['night.enabled'].value} disabled={sample} onPick={(on) => void save({ 'night.enabled': on })} />
        </Row>
        <form class="nset" onSubmit={saveWindow}>
          <label>WINDOW (LOCAL TIME)
            <input value={win.value} disabled={sample} onInput={(e) => { win.value = e.currentTarget.value; }} placeholder="01:00-07:00" autocomplete="off" spellcheck={false} />
          </label>
          <button class="btn sm" type="submit" disabled={sample}>SET WINDOW</button>
          <span class="hint">New runs start only inside it; an end before the start crosses midnight.</span>
        </form>
        <div class="nset">
          <label>AT ONCE
            <select value={String(s['night.parallel'].value)} disabled={sample} onChange={async (e) => { const pick = e.currentTarget; if (!(await save({ 'night.parallel': Number(pick.value) }))) pick.value = String(s['night.parallel'].value); }}>
              {Array.from({ length: s['night.parallel'].max - s['night.parallel'].min + 1 }, (_, i) => String(s['night.parallel'].min + i)).map((k) => <option key={k} value={k}>{k}</option>)}
            </select>
          </label>
          <span class="hint">How many runs go at the same time.</span>
        </div>
        <Row label="Work in" help={s['night.isolation'].value === 'copy' ? 'A separate copy of your repo: your own folder is fully locked.' : 'A worktree of your repo, on a new branch: fast and small.'}>
          <Seg label="Work in" value={s['night.isolation'].value as 'worktree' | 'copy'} options={ISOLATION} disabled={sample} onPick={(v) => void save({ 'night.isolation': v })} />
        </Row>
      </Sec>
      <Sec t="REMINDERS">
        <Row label="Desktop notices" help={notice && !notice.available ? 'This computer has no program to show a notice.' : 'One desktop notice when a reminder you set falls due. It never leaves this computer.'}>
          <Switch label="Desktop notices" on={s['remind.notify'].value} disabled={sample || (!!notice && !notice.available)} onPick={(on) => void save({ 'remind.notify': on })} />
        </Row>
      </Sec>
      <Sec t="AGENTS">
        <Row label="Agents" help={`The agents eye follows; the first one is the default for new ideas and summaries. A switched-off agent's sessions stay and come back when it is on again.${when(agents.restart)}`}>
          <div class="fchips" role="group" aria-label="Agents">
            {agents.choices.map((a) => {
              const on = agents.value.includes(a.id);
              return (
                <button key={a.id} type="button" aria-pressed={on} disabled={sample} onClick={() => void save({ 'agents.enabled': on ? agents.value.filter((x) => x !== a.id) : [...agents.value, a.id] })}>{a.name}</button>
              );
            })}
          </div>
        </Row>
      </Sec>
    </>
  );
}
