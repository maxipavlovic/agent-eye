import { describe, expect, it } from 'vitest';
import { folderHint, folderMissing, startButton } from './folder';

describe('startButton', () => {
  it('is plain START for a folder that is there, and for an engine that does not say', () => {
    expect(startButton({ folder_state: 'ok' })).toEqual({ label: 'START', blocked: null });
    expect(startButton({})).toEqual({ label: 'START', blocked: null });
  });

  it('says it creates the folder when the folder is missing and may be made', () => {
    expect(startButton({ folder_state: 'missing', folder_creatable: true })).toEqual({ label: 'CREATE FOLDER + START', blocked: null });
  });

  it('is blocked, with the reason, when the folder cannot be made or used', () => {
    const note = 'Folder does not exist, and eye only creates folders inside your home folder';
    expect(startButton({ folder_state: 'missing', folder_creatable: false, folder_note: note })).toEqual({ label: 'START', blocked: note });
    expect(startButton({ folder_state: 'not_a_folder', folder_note: 'Not a folder' }).blocked).toBe('Not a folder');
    expect(startButton({ folder_state: 'unusable', folder_note: null }).blocked).toBe('The folder cannot be used');
  });
});

describe('folderHint', () => {
  it('shows nothing for a fine or unchecked folder, and the note otherwise', () => {
    expect(folderHint(null)).toBeNull();
    expect(folderHint({ state: 'ok', creatable: false, note: null })).toBeNull();
    expect(folderHint({ state: 'missing', creatable: true, note: 'Folder does not exist yet; Start will create it' })).toBe('Folder does not exist yet; Start will create it');
    expect(folderHint({ state: 'not_a_folder', creatable: false, note: 'Not a folder' })).toBe('Not a folder');
  });
});

describe('folderMissing', () => {
  it('marks only a missing folder', () => {
    expect(folderMissing({ folder_state: 'missing' })).toBe(true);
    expect(folderMissing({ folder_state: 'ok' })).toBe(false);
    expect(folderMissing({})).toBe(false);
  });
});
