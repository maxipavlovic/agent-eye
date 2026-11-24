import { describe, expect, it } from 'vitest';
import { hlPattern } from './highlight';

describe('hlPattern', () => {
  it('marks every search word, ignoring case and one-letter words', () => {
    const re = hlPattern('Catalog a mig')!;
    expect('Catalog Migration'.split(re)).toEqual(['', 'Catalog', ' ', 'Mig', 'ration']);
    expect(hlPattern('  a  ')).toBeNull();
    expect('x (y) z'.split(hlPattern('(y)')!)).toEqual(['x ', '(y)', ' z']);
  });
});
