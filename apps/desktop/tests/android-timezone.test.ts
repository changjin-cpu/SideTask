import { expect, it, vi } from 'vitest';
import { currentTimeZone, validTimeZone } from '../src/lib/deadline';

it('normalizes Android WebView numeric UTC without inventing a regional timezone', () => {
  const original = Intl.DateTimeFormat;
  const spy = vi.spyOn(Intl, 'DateTimeFormat');
  try {
    for (const offset of ['+00:00', '-00:00']) {
      spy.mockImplementation(() => ({ resolvedOptions: () => ({ timeZone: offset }) }) as Intl.DateTimeFormat);
      expect(currentTimeZone()).toBe('UTC');
    }
    spy.mockRestore();
    expect(validTimeZone(currentTimeZone())).toBe(true);
    expect(Intl.DateTimeFormat).toBe(original);
  } finally { spy.mockRestore(); }
});
