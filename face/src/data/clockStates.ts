import type { ClockHour, ClockReport } from '../api/types';
import { demoClock } from './clockSample';

// Two more states of the sample clock, for the site demo, the tests and the end-to-end flows.

/** A few days in: too few prompts for a verdict, so the panel says so and draws nothing. */
export function tooFewClock(): ClockReport {
  const zero = (h: number): ClockHour => ({ h, prompts: 0, corrections: 0, interrupts: 0, tool_calls: 0, tool_errors: 0, think: null, think_n: 0 });
  return {
    ...demoClock(4), pending: 0, prompts: 120, corrections: 6,
    hours: Array.from({ length: 24 }, (_, h) => (h >= 9 && h < 17 ? { ...zero(h), prompts: 15, corrections: h === 10 ? 6 : 0, tool_calls: 120, tool_errors: 2 } : zero(h))),
    since_break: [], bad: [],
    verdict: { kind: 'too_few', you_ratio: null, you_n: 6, think_ratio: null, tool_error_ratio: null, text: 'Too few to tell yet.' },
    facts: ['You correct about 1 in 20 prompts: 5 per 100 (6 of 120).'],
  };
}

/** The demo week, but it is evening and hints are on: the header and the card you work in carry a quiet mark. */
export function hintClock(): ClockReport {
  return { ...demoClock(4), hints: true, now: { in_bad: true, since_break_min: 130, hint: { text: 'Your corrections usually rise now', offer: true } } };
}
