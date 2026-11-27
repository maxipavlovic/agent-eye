import type { ClockHour, ClockReport } from '../api/types';

/** Every kind of turn event, as the agents that can report them all list them. */
const SIGNALS = ['prompt', 'end', 'interrupt', 'tool', 'tool_error', 'refusal', 'think', 'compact'] as const;

/** Prompts per local hour of the demo's four weeks: a working day with an evening tail. */
const PROMPTS = [10, 4, 0, 0, 0, 0, 6, 24, 58, 96, 112, 104, 62, 88, 108, 100, 84, 70, 82, 96, 90, 66, 38, 22];
/** Corrections per hour: about 7 in 100 by day, about 13 in 100 from 18:00 on. */
const CORRECTIONS = [1, 0, 0, 0, 0, 0, 0, 2, 4, 7, 8, 7, 4, 6, 8, 7, 6, 6, 11, 13, 12, 9, 5, 3];

function hours(weeks: number): ClockHour[] {
  const k = weeks / 4;
  return PROMPTS.map((p, h) => {
    const prompts = Math.round(p * k);
    const evening = h >= 18 || h < 6;
    return {
      h,
      prompts,
      corrections: Math.round(CORRECTIONS[h]! * k),
      interrupts: Math.round(prompts / 30),
      tool_calls: prompts * 11,
      tool_errors: Math.round(prompts * 11 * 0.025),
      think: prompts > 0 ? (evening ? 1_480 : 1_590) : null,
      think_n: prompts * 7,
    };
  });
}

/** A new install: nothing counted yet, so no verdict. */
export function emptyClock(weeks: number): ClockReport {
  const zero = (h: number): ClockHour => ({ h, prompts: 0, corrections: 0, interrupts: 0, tool_calls: 0, tool_errors: 0, think: null, think_n: 0 });
  return {
    enabled: true, llm: true, judging: 'on', judging_reason: null, hints: false, idle: false,
    from: '2026-10-29', to: '2026-11-26', prompts: 0, corrections: 0, pending: 0,
    hours: Array.from({ length: 24 }, (_, h) => zero(h)),
    since_break: ['<1h', '1-2h', '2-4h', '4h+'].map((band) => ({ band, prompts: 0, corrections: 0 })),
    bad: [],
    think_agent: null,
    tool_agent: null,
    verdict: { kind: 'too_few', you_ratio: null, you_n: null, think_ratio: null, tool_error_ratio: null, text: 'Too few to tell yet: 0 prompts and 0 corrections (it needs 300 and 30).' },
    now: { in_bad: false, since_break_min: null, hint: null },
    agents: [],
  };
}

/** The demo board's clock: evenings run worse, and the model's side stays flat. */
export function demoClock(weeks: number): ClockReport {
  const hs = hours(weeks);
  const prompts = hs.reduce((n, h) => n + h.prompts, 0);
  const corrections = hs.reduce((n, h) => n + h.corrections, 0);
  const k = weeks / 4;
  const band = (band: string, p: number, c: number) => ({ band, prompts: Math.round(p * k), corrections: Math.round(c * k) });
  return {
    enabled: true, llm: true, judging: 'on', judging_reason: null, hints: false, idle: false,
    from: '2026-10-29', to: '2026-11-26', prompts, corrections, pending: 12,
    hours: hs,
    since_break: [band('<1h', 700, 48), band('1-2h', 330, 31), band('2-4h', 190, 22), band('4h+', 100, 18)],
    bad: [{ from_h: 18, to_h: 0, label: 'evening' }],
    think_agent: 'claude',
    tool_agent: 'claude',
    verdict: {
      kind: 'you', you_ratio: 1.9, you_n: 53, think_ratio: 0.93, tool_error_ratio: 0.95,
      text: 'Evenings: 1.9x more corrections (53). Model thinking −7%. Tool errors flat. Mostly you.',
    },
    now: { in_bad: false, since_break_min: 35, hint: null },
    agents: [
      { agent: 'claude', prompts: Math.round(prompts * 0.94), corrections: Math.round(corrections * 0.95), think: 'signature', think_unit: 'signature', think_ratio: 0.93, tool_error_ratio: 0.95, think_note: null, tool_error_note: null, signals: [...SIGNALS] },
      { agent: 'codex', prompts: Math.round(prompts * 0.06), corrections: Math.round(corrections * 0.05), think: null, think_unit: 'reasoning_tokens', think_ratio: null, tool_error_ratio: null, think_note: null, tool_error_note: null, signals: [...SIGNALS] },
    ],
  };
}
