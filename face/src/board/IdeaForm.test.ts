import { describe, expect, it } from 'vitest';
import { folderChoices, recentFolders } from './IdeaForm';

describe('recentFolders', () => {
  it('lists session folders newest first, once each, without ideas', () => {
    const c = (folder: string | null, last: number, kind = 'interactive') => ({ folder, last_activity_at: last, kind }) as never;
    expect(recentFolders([c('/a', 1), c('/b', 3), c('/a', 5), c(null, 9), c('/i', 9, 'idea')])).toEqual(['/a', '/b']);
  });
});

describe('recentFolders, the suggestions', () => {
  it('leaves out temp and scratch folders and folders the engine says are gone', () => {
    const c = (folder: string, last: number, state?: string) => ({ folder, last_activity_at: last, kind: 'interactive', folder_state: state }) as never;
    const cards = [
      c('/private/tmp/claude-501/x/scratchpad/live/eyehome/night/4/work', 9),
      c('/tmp/play', 8),
      c('~/code/gone', 7, 'missing'),
      c('~/code/app', 6, 'ok'),
      c('~/code/lib', 5),
    ];
    expect(recentFolders(cards)).toEqual(['~/code/app', '~/code/lib']);
  });
});

describe('folderChoices', () => {
  it('offers six recent folders, then only the ones matching what is typed', () => {
    const all = ['/Users/you/code/a', '/Users/you/code/eye', '/Users/you/notes', '~/code/b', '/x/5', '/x/6', '/x/7'];
    expect(folderChoices(all, '')).toHaveLength(6);
    expect(folderChoices(all, ' CODE ')).toEqual(['/Users/you/code/a', '/Users/you/code/eye', '~/code/b']);
    expect(folderChoices(all, '~/code/e')).toEqual(['/Users/you/code/eye']);
    // a picked folder keeps the others in reach
    expect(folderChoices(all, '~/code/eye')).toHaveLength(6);
    expect(folderChoices(all, 'nothing')).toEqual([]);
  });
});
