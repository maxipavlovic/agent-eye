import { describe, expect, it } from 'vitest';
import { foldedFrom } from './layout';

describe('foldedFrom', () => {
  it('folds Done by default and keeps an old "Done open" choice', () => {
    expect(foldedFrom('', 'folded')).toEqual(['done']);
    expect(foldedFrom('', 'open')).toEqual([]);
  });
  it('reads a saved list and drops lanes that cannot fold', () => {
    expect(foldedFrom('["waiting","paused","needs_you","nope"]', 'open')).toEqual(['waiting', 'paused']);
    expect(foldedFrom('not json', 'folded')).toEqual(['done']);
  });
});
