/**
 * T-000207 / T-000211: a rule refresh names what it took up, lost or set aside.
 * Taken and superseded toasts persist until closed; a lost one is a warning.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { toastRulesFindings } from '../rules-report';

const addToast = vi.fn();
vi.mock('$lib/stores/ui', () => ({
  addToast: (...args: unknown[]) => addToast(...args),
}));

vi.mock('$lib/i18n', () => ({
  tf: (key: string, ...args: unknown[]) => `${key}: ${args.join(', ')}`,
}));

describe('toastRulesFindings', () => {
  beforeEach(() => {
    addToast.mockReset();
  });

  it('persists the taken toast', () => {
    toastRulesFindings({ taken: ['sdh-retro'], lost: [], superseded: [] });
    expect(addToast).toHaveBeenCalledWith('toast.rulesTaken: sdh-retro', 'info', { persist: true });
  });

  it('persists the superseded toast', () => {
    toastRulesFindings({ taken: [], lost: [], superseded: ['sdh-debugging'] });
    expect(addToast).toHaveBeenCalledWith('toast.rulesSuperseded: sdh-debugging', 'info', { persist: true });
  });

  it('does not persist the lost toast', () => {
    toastRulesFindings({ taken: [], lost: ['sdh-retro'], superseded: [] });
    expect(addToast).toHaveBeenCalledTimes(1);
    expect(addToast).toHaveBeenCalledWith('toast.rulesLost: sdh-retro', 'warning');
    expect(addToast.mock.calls[0]).toHaveLength(2);
  });

  it('stays silent when nothing was found', () => {
    toastRulesFindings({ taken: [], lost: [], superseded: [] });
    expect(addToast).not.toHaveBeenCalled();
  });
});
