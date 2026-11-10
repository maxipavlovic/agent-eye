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
});
