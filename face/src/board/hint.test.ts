import { describe, expect, it } from 'vitest';
import { hintFor, tonightWhy } from './hint';

const live = { id: 'a', kind: 'interactive', column: 'needs_you', live_status: 'idle' } as Parameters<typeof hintFor>[0];

describe('the hint on a card', () => {
  it('is for the open card that is a live session waiting on you', () => {
    expect(hintFor(live, 'a', 'Your corrections usually rise now')).toBe('Your corrections usually rise now');
  });

  it('is not for another card, an idea, a card that is not live or not waiting on you, or when there is no hint', () => {
    expect(hintFor(live, 'b', 'x')).toBeNull();
    expect(hintFor({ ...live, kind: 'idea' }, 'a', 'x')).toBeNull();
    expect(hintFor({ ...live, live_status: null }, 'a', 'x')).toBeNull();
    expect(hintFor({ ...live, column: 'paused' }, 'a', 'x')).toBeNull();
    expect(hintFor(live, 'a', null)).toBeNull();
  });
});

describe('why Continue tonight comes first', () => {
  const card = { ...live, resumable: true } as Parameters<typeof tonightWhy>[0];
  const full = 'Evening: your corrections usually run 1.8x now. Hand this to the night shift? (T)';

  it('is the hint without the offer, for a live session waiting on you that the night shift can take', () => {
    expect(tonightWhy(card, full, true)).toBe('Evening: your corrections usually run 1.8x now.');
  });

  it('is nothing without a hint, without the night shift, for a card that cannot be resumed or is not waiting on you', () => {
    expect(tonightWhy(card, null, true)).toBeNull();
    expect(tonightWhy(card, full, false)).toBeNull();
    expect(tonightWhy({ ...card, resumable: false }, full, true)).toBeNull();
    expect(tonightWhy({ ...card, column: 'running' }, full, true)).toBeNull();
  });
});
