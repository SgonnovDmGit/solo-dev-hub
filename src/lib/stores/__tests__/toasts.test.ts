/**
 * T-000211: a toast stays until closed when it is persistent; plain info and
 * success toasts still auto-dismiss after 5 s.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { get } from 'svelte/store';
import { addToast, dismissToast, toasts } from '../ui';

function messages(): string[] {
  return get(toasts).map((t) => t.message);
}

describe('addToast', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    for (const t of get(toasts)) dismissToast(t.id);
    vi.useRealTimers();
  });

  it('removes a plain info toast after 5 s', () => {
    addToast('plain');
    vi.advanceTimersByTime(4_999);
    expect(messages()).toEqual(['plain']);
    vi.advanceTimersByTime(1);
    expect(messages()).toEqual([]);
  });

  it('keeps a persistent info toast after 5 s, with its type', () => {
    addToast('taken', 'info', { persist: true });
    vi.advanceTimersByTime(60_000);
    expect(get(toasts)).toEqual([expect.objectContaining({ message: 'taken', type: 'info' })]);
  });

  it('keeps a persistent toast of any type', () => {
    addToast('done', 'success', { persist: true });
    vi.advanceTimersByTime(60_000);
    expect(messages()).toEqual(['done']);
  });

  it('keeps errors and warnings until closed, as before', () => {
    addToast('boom', 'error');
    addToast('careful', 'warning');
    vi.advanceTimersByTime(60_000);
    expect(messages()).toEqual(['boom', 'careful']);
  });
});
