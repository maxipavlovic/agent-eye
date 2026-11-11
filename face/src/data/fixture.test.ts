import { describe, expect, it } from 'vitest';
import { nextParts } from '../format';
import { fixtureSource } from './fixture';

describe('fixtureSource with the sample data', () => {
  it('fills every lane and has a session stopped on a prompt', async () => {
    const b = await fixtureSource().board();
    for (const col of b.columns) expect(col.cards.length, col.id).toBeGreaterThan(0);
    const need = b.columns.find((c) => c.id === 'needs_you')!.cards;
    expect(need.some((c) => c.blocked && nextParts(c.next).who === 'you')).toBe(true);
  });

  it('moves the sample times so the board is as fresh as when it loads', async () => {
    const src = fixtureSource();
    const b = await src.board();
    const newest = Math.max(...b.columns.flatMap((c) => c.cards.map((x) => x.last_activity_at ?? 0)));
    expect(Date.now() - newest).toBeLessThan(60 * 60000);
    expect(b.generated_at).toBeGreaterThan(Date.now() - 5000);
  });

  it('holds nothing personal: the user is You and the home is /Users/you', async () => {
    const src = fixtureSource();
    const b = await src.board();
    const text = JSON.stringify([b, await src.archive(0, 50), ...(await Promise.all(b.columns.flatMap((c) => c.cards.map((x) => src.card(x.id)))))]);
    expect(text).not.toMatch(/\bMax\b/);
    expect(text).not.toMatch(/\/Users\/(?!you\b)/);
  });

  it('offers one agent, Claude, and gives every card and idea its agent', async () => {
    const src = fixtureSource();
    expect((await src.agents()).agents).toEqual([{ id: 'claude', name: 'Claude Code', short: 'Claude', default: true }]);
    const b = await src.board();
    for (const c of b.columns.flatMap((col) => col.cards)) {
      expect(c.agent).toBe('claude');
      expect(c.agent_name).toBe('Claude');
    }
    expect((await src.ideas()).ideas.every((i) => i.agent === null)).toBe(true);
  });

  it('keeps the agent of an idea it adds or edits', async () => {
    const src = fixtureSource();
    const added = await src.addIdea({ title: 'Try another agent', agent: 'claude' });
    expect(added).toMatchObject({ agent: 'claude', agent_name: 'Claude' });
    const edited = await src.editIdea(added.idea_id!, { agent: 'claude' });
    expect(edited.agent).toBe('claude');
    expect((await src.ideas()).ideas.find((i) => i.id === added.idea_id)!.agent).toBe('claude');
  });

  it('serves a written conversation, and makes one up for the rest', async () => {
    const src = fixtureSource();
    const b = await src.board();
    const blocked = b.columns.find((c) => c.id === 'needs_you')!.cards.find((c) => c.blocked)!;
    const written = await src.conversation(blocked.id, 10, null);
    expect(written.more).toBe(false);
    expect(written.messages.at(-1)!.text).toMatch(/install/);
    const paused = b.columns.find((c) => c.id === 'paused')!.cards[0]!;
    expect((await src.conversation(paused.id, 10, null)).messages.length).toBeGreaterThan(0);
  });

  it('finds sessions by what was said in them', async () => {
    const { results } = await fixtureSource().search('retry');
    expect(results.length).toBeGreaterThan(2);
    expect(results.some((r) => /\[retry\]/i.test(r.snippet))).toBe(true);
  });

  it('has one reminder that is due, in Needs you, and one coming', async () => {
    const b = await fixtureSource().board();
    const cards = b.columns.flatMap((c) => c.cards);
    const due = cards.filter((c) => c.remind_due);
    expect(due.map((c) => c.column)).toEqual(['needs_you']);
    expect(due[0]!.remind_note).toBeTruthy();
    expect(b.columns.find((c) => c.id === 'needs_you')!.cards[0]!.id).toBe(due[0]!.id);
    const soon = cards.find((c) => c.remind_at && !c.remind_due && c.kind !== 'idea')!;
    expect(soon.remind_at!).toBeGreaterThan(Date.now());
  });

  it('sets, clears and refuses reminders, and sends a due card back where the rules put it', async () => {
    const src = fixtureSource();
    const due = (await src.board()).columns.flatMap((c) => c.cards).find((c) => c.remind_due)!;
    const set = await src.remind(due.id, { when: 'tomorrow', note: 'again' });
    expect(set).toMatchObject({ remind_note: 'again', remind_due: false, column: due.auto_column });
    expect(set.remind_at!).toBeGreaterThan(Date.now());
    const b = await src.board();
    expect(b.columns.find((c) => c.id === 'needs_you')!.cards.some((c) => c.id === due.id)).toBe(false);
    expect((await src.card(due.id)).remind_note).toBe('again');
    expect(await src.remind(due.id, { clear: true })).toMatchObject({ remind_at: null, remind_note: null });
    await expect(src.remind(due.id, { when: 'soon' })).rejects.toThrow(/cannot read/);
    await expect(src.remind(due.id, { when: '2020-01-01' })).rejects.toThrow(/passed/);
    await expect(src.remind('nope', { when: '1d' })).rejects.toThrow(/No card/);
  });

  it('reads the times the board sends and no look-alikes', async () => {
    const src = fixtureSource();
    const id = (await src.board()).columns.flatMap((c) => c.cards)[0]!.id;
    for (const when of ['tomorrow', 'mon', 'Monday', '1w', '3d']) {
      expect((await src.remind(id, { when })).remind_at!, when).toBeGreaterThan(Date.now());
    }
    for (const when of ['monkey', 'tuesdayfoo', 'mo', '0d', '0w']) {
      await expect(src.remind(id, { when }), when).rejects.toThrow(/cannot read/);
    }
  });
});
