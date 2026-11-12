import { describe, expect, it } from 'vitest';
import { artifactName, codeParts, folderGroup, homePath, nextParts, outputLabel, remindReason, remindText, remindTitle, remindWhen, resumeLabel, snippetParts, tok } from './format';
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

const pr = (value: string): Output => ({ kind: 'pr', value, url: null });

describe('pull request labels', () => {
  it('reads GitHub, Bitbucket and GitLab links', () => {
    expect(outputLabel(pr('https://github.com/o/agent-eye/pull/12'))).toBe('agent-eye#12');
    expect(outputLabel(pr('https://bitbucket.org/t/connect/pull-requests/7/overview'))).toBe('connect#7');
    expect(outputLabel(pr('https://gitlab.com/g/r/-/merge_requests/5#note_1'))).toBe('-#5');
  });
  it('says PR for anything else, and stays fast on a long link', () => {
    expect(outputLabel(pr('https://example.com/pulls/12'))).toBe('PR');
    expect(outputLabel(pr('a'.repeat(100_000)))).toBe('PR');
  });
});

describe('snippetParts', () => {
  it('marks the [matches]', () => {
    expect(snippetParts('fix [login] then [deploy]')).toEqual([
      { text: 'fix ', hit: false }, { text: 'login', hit: true }, { text: ' then ', hit: false }, { text: 'deploy', hit: true },
    ]);
  });
  it('keeps an unclosed bracket as text, and stays fast on many of them', () => {
    expect(snippetParts('a [b')).toEqual([{ text: 'a [b', hit: false }]);
    expect(snippetParts('['.repeat(100_000))).toHaveLength(1);
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

describe('resumeLabel', () => {
  it('says what Resume does, and which Linux terminal it is when known', () => {
    expect(resumeLabel('iterm')).toEqual({ label: 'ITERM', via: '' });
    expect(resumeLabel('terminal')).toEqual({ label: 'TERMINAL', via: '' });
    expect(resumeLabel('terminal', 'xdg-terminal-exec')).toEqual({ label: 'TERMINAL', via: 'xdg-terminal-exec' });
    expect(resumeLabel('copy', 'ignored')).toEqual({ label: 'COPY', via: '' });
    expect(resumeLabel('', null)).toEqual({ label: 'COPY', via: '' });
    expect(resumeLabel(undefined)).toEqual({ label: 'COPY', via: '' });
  });
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

describe('reminders', () => {
  const at = (d: number, h: number, m = 0) => new Date(2026, 5, 10 + d, h, m).getTime();
  const now = at(0, 8);
  const c = (o: object) => o as Parameters<typeof remindTitle>[0];

  it('says when in words', () => {
    expect(remindWhen(at(0, 14), now)).toBe('today 14:00');
    expect(remindWhen(at(1, 9), now)).toBe('tomorrow 9:00');
    expect(remindWhen(at(3, 14, 30), now)).toBe('Sat 14:30');
    expect(remindWhen(at(10, 9), now)).toBe('20 Jun 9:00');
    // a due one can be from before today
    expect(remindWhen(at(-1, 23, 55), now)).toBe('yesterday 23:55');
    expect(remindWhen(at(-3, 9), now)).toBe('7 Jun 9:00');
  });

  it('says when and the note of a set reminder, and nothing without one', () => {
    expect(remindText(c({ remind_at: at(1, 9), remind_note: "Ana's reply" }), now)).toBe("tomorrow 9:00 · Ana's reply");
    expect(remindText(c({ remind_at: at(10, 9) }), now)).toBe('20 Jun 9:00');
    expect(remindText(c({}), now)).toBeNull();
    expect(remindText(c({ remind_at: null }), now)).toBeNull();
  });

  it('titles the clock mark of any set reminder, near or far, and says when one is due', () => {
    expect(remindTitle(c({ remind_at: at(1, 9), remind_note: "Ana's reply" }), now)).toBe("Reminder: tomorrow 9:00 · Ana's reply");
    expect(remindTitle(c({ remind_at: at(30, 9) }), now)).toBe('Reminder: 10 Jul 9:00');
    expect(remindTitle(c({ remind_at: at(0, 7), remind_due: true }), now)).toBe('Reminder due: today 7:00');
    expect(remindTitle(c({}), now)).toBeNull();
  });

  it('gives a due card the reason for its YOU line, with or without a note', () => {
    expect(remindReason(c({ remind_due: true, remind_at: at(0, 7), remind_note: "Ana's reply" }), now)).toBe("check on this: Ana's reply");
    expect(remindReason(c({ remind_due: true, remind_at: at(0, 7), remind_note: null }), now)).toBe('you asked to check on this today 7:00');
    expect(remindReason(c({ remind_due: false, remind_note: 'x' }), now)).toBeNull();
    expect(remindReason(c({}), now)).toBeNull();
  });
});
