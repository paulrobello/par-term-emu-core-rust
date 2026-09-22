/**
 * Tests for parseStoredMacros (QA-118): the localStorage-shape validator
 * used by useStoredMacros. A corrupted or unexpectedly-shaped stored value
 * must yield [] rather than crash the onscreen keyboard.
 */

import { describe, it, expect } from 'vitest';
import { parseStoredMacros } from '@/lib/use-stored-macros';

describe('parseStoredMacros', () => {
  it('returns a well-formed macro array unchanged', () => {
    const raw = JSON.stringify([
      { id: '1', name: 'Greet', script: 'echo hi' },
      { id: '2', name: 'List', script: 'ls -la', sendEnter: false },
    ]);

    expect(parseStoredMacros(raw)).toEqual([
      { id: '1', name: 'Greet', script: 'echo hi' },
      { id: '2', name: 'List', script: 'ls -la', sendEnter: false },
    ]);
  });

  it('returns [] when the stored value is not an array (corrupted entry)', () => {
    expect(parseStoredMacros(JSON.stringify({ id: '1', name: 'x', script: 'y' }))).toEqual([]);
    expect(parseStoredMacros(JSON.stringify('not-an-array'))).toEqual([]);
    expect(parseStoredMacros(JSON.stringify(42))).toEqual([]);
    expect(parseStoredMacros(JSON.stringify(null))).toEqual([]);
  });

  it('drops individual entries missing a string name or script', () => {
    const raw = JSON.stringify([
      { id: '1', name: 'Valid', script: 'echo ok' },
      { id: '2', name: 'Missing script' },
      { id: '3', script: 'missing name' },
      { id: '4', name: 123, script: 'name is not a string' },
      null,
      'not-an-object',
    ]);

    expect(parseStoredMacros(raw)).toEqual([{ id: '1', name: 'Valid', script: 'echo ok' }]);
  });

  it('throws on genuinely malformed JSON (caller\'s try/catch handles it)', () => {
    expect(() => parseStoredMacros('{not json')).toThrow();
  });

  it('returns [] for an empty array', () => {
    expect(parseStoredMacros('[]')).toEqual([]);
  });
});
