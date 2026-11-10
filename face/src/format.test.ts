import { describe, expect, it } from 'vitest';
import { artifactName, codeParts, folderGroup, homePath, nextParts, outputLabel, tok } from './format';
import type { Output } from './api/types';

describe('nextParts', () => {
  it('reads You as the user, whatever the case', () => {
    expect(nextParts('You: review the PR')).toEqual({ who: 'you', name: 'You', text: 'review the PR' });
    expect(nextParts('you: ship it').who).toBe('you');
  });
  it('reads the card\'s agent by its short name', () => {
    expect(nextParts('Codex: fix the failing test', 'Codex')).toEqual({ who: 'agent', name: 'Codex', text: 'fix the failing test' });
    expect(nextParts('codex: go on', 'Codex').who).toBe('agent');
  });
  it('still reads an Agent or Claude owner in older lines', () => {
    expect(nextParts('Agent: run the tests').who).toBe('agent');
    expect(nextParts('Claude: run the tests').who).toBe('agent');
    expect(nextParts('Claude: run the tests', 'Gemini').who).toBe('agent');
  });
  it('reads any other name as another person, and no owner as plain text', () => {
    expect(nextParts('Dana: send the draft', 'Claude')).toEqual({ who: 'others', name: 'Dana', text: 'send the draft' });
    expect(nextParts('Codex: go on', 'Claude').who).toBe('others');
    expect(nextParts('wait for the review')).toEqual({ who: null, name: null, text: 'wait for the review' });
    expect(nextParts(null)).toEqual({ who: null, name: null, text: '' });
  });
});

const art = (o: Partial<Output>): Output => ({ kind: 'artifact', value: 'https://claude.ai/artifact/7fK2aB9x1234abcd', url: null, ...o });

describe('artifactName', () => {
  it('prefers the title', () => expect(artifactName(art({ title: ' Catalog plan ' }))).toBe('Catalog plan'));
  it('falls back to a short id', () => {
    expect(artifactName(art({ url: 'https://claude.ai/artifact/7fK2aB9x1234abcd' }))).toBe('artifact 7fK2aB9x…');
    expect(outputLabel(art({ title: null }))).toBe('artifact 7fK2aB9x…');
  });
});

describe('folders', () => {
  it('makes paths home-relative', () => expect(homePath('/Users/you/code/x')).toBe('~/code/x'));
  it('keeps one folder whole', () => expect(folderGroup(['/Users/you/code/a/site'])).toEqual({ base: null, rels: ['~/code/a/site'] }));
  it('trims the shared parent', () => {
    expect(folderGroup(['/Users/you/code/pc/site', '/Users/you/code/pc/site/src/layouts', '/Users/you/code/pc/scripts'])).toEqual({
      base: '~/code/pc/', rels: ['site', 'site/src/layouts', 'scripts'],
    });
  });
  it('never trims a folder away', () => expect(folderGroup(['/Users/you/a', '/Users/you/a/b']).rels).toEqual(['a', 'a/b']));
});

describe('tok', () => {
  it('formats counts', () => {
    expect([0, 840, 999, 1000, 1234, 9949, 9950, 12000, 340400, 999499, 999500, 1234567, 9949999, 12400000].map(tok)).toEqual(
      ['0', '840', '999', '1k', '1.2k', '9.9k', '10k', '12k', '340k', '999k', '1M', '1.2M', '9.9M', '12M']);
  });
});

describe('codeParts', () => {
  it('marks inline code spans only', () => {
    expect(codeParts('run `npm test` now, `a\nb` stays')).toEqual([
      { text: 'run ', code: false }, { text: 'npm test', code: true }, { text: ' now, `a\nb` stays', code: false }]);
    expect(codeParts('``')).toEqual([{ text: '``', code: false }]);
  });
});
