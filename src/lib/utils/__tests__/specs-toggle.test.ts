/**
 * T-000243 review: the core of the specs checkbox, tested at the util level.
 *
 * Why here and not in a component test: the repo has no testing-library / jsdom
 * setup (vitest runs in `node`), so the DOM cannot be driven. The two review
 * findings live in logic that is pure anyway — "a rejected save must not change
 * what the checkbox displays" is exactly "the outcome the component assigns to
 * the bound draft equals the stored value" — so the rollback semantics are
 * pinned here, while the component's part of the contract (binding the input to
 * that draft, so Svelte writes the value back into the DOM) is the documented
 * `bind:checked` mechanism.
 */
import { describe, it, expect, vi } from 'vitest';
import { saveSpecsToggle, type SpecsSaveDeps } from '../specs-toggle';

type VerifyFn = (token: string, repo: string, login: string) => Promise<{ admin: boolean }>;
type SaveFn = (repoId: number, on: boolean) => Promise<unknown>;

function deps(over: { verify?: VerifyFn; save?: SaveFn } = {}): SpecsSaveDeps {
  const verify: VerifyFn =
    over.verify ?? (vi.fn(async () => ({ admin: true, newlyAdmin: false })) as unknown as VerifyFn);
  const save: SaveFn = over.save ?? (vi.fn(async () => null) as unknown as SaveFn);
  return { verifyAdmin: verify, setRepoShareSpecs: save };
}

const ownRepo = { repoId: 7, repoFullName: 'me/api', login: 'me', token: 'tok', stored: true };
const orgRepo = { repoId: 8, repoFullName: 'Acme-Org/web', login: 'me', token: 'tok', stored: true };

describe('saveSpecsToggle', () => {
  it('saves the flip of the stored value and reports the stored value back on success', async () => {
    const d = deps();
    const res = await saveSpecsToggle(ownRepo, d);
    expect(res).toEqual({ ok: true, value: false, note: null });
    expect(d.setRepoShareSpecs).toHaveBeenCalledWith(7, false);
  });

  it('hands the backend warning note through', async () => {
    const d = deps({ save: vi.fn(async () => 'docs/superpowers/ is still ignored by [superpowers/]') });
    const res = await saveSpecsToggle(ownRepo, d);
    expect(res).toEqual({ ok: true, value: false, note: 'docs/superpowers/ is still ignored by [superpowers/]' });
  });

  it('a rejected save leaves the displayed value unchanged (the stored one)', async () => {
    const d = deps({ save: vi.fn(async () => { throw new Error('boom'); }) });
    const res = await saveSpecsToggle({ ...ownRepo, stored: true }, d);
    // The browser already flipped the checkbox to off; the value the component
    // must show again is the stored `true`.
    expect(res).toEqual({ ok: false, value: true, kind: 'failed', error: new Error('boom') });
    expect(d.setRepoShareSpecs).toHaveBeenCalledWith(7, false);
  });

  it('verifies the administrator of an organization repo BEFORE saving', async () => {
    const order: string[] = [];
    const d = deps({
      verify: vi.fn(async () => {
        order.push('verify');
        return { admin: true, newlyAdmin: true };
      }),
      save: vi.fn(async () => {
        order.push('save');
        return null;
      }),
    });
    const res = await saveSpecsToggle(orgRepo, d);
    expect(res).toEqual({ ok: true, value: false, note: null });
    expect(order).toEqual(['verify', 'save']);
    expect(d.verifyAdmin).toHaveBeenCalledWith('tok', 'Acme-Org/web', 'me');
  });

  it('refuses without saving and without enabling anything when the login is not an admin', async () => {
    const d = deps({
      verify: vi.fn(async () => ({ admin: false, newlyAdmin: false })),
      save: vi.fn(async () => null),
    });
    const res = await saveSpecsToggle(orgRepo, d);
    expect(res).toEqual({ ok: false, value: true, kind: 'notAdmin' });
    expect(d.setRepoShareSpecs).not.toHaveBeenCalled();
  });

  it('treats a missing token as "not an admin" for an organization repo, like the shared switch', async () => {
    const d = deps();
    const res = await saveSpecsToggle({ ...orgRepo, token: null }, d);
    expect(res).toEqual({ ok: false, value: true, kind: 'notAdmin' });
    expect(d.verifyAdmin).not.toHaveBeenCalled();
    expect(d.setRepoShareSpecs).not.toHaveBeenCalled();
  });

  it('a failed verification is a failed save, never a silent one', async () => {
    const d = deps({
      verify: vi.fn(async () => { throw new Error('403'); }),
      save: vi.fn(async () => null),
    });
    const res = await saveSpecsToggle(orgRepo, d);
    expect(res).toEqual({ ok: false, value: true, kind: 'failed', error: new Error('403') });
    expect(d.setRepoShareSpecs).not.toHaveBeenCalled();
  });

  it('never asks GitHub about the install\'s own repo', async () => {
    const d = deps();
    await saveSpecsToggle(ownRepo, d);
    expect(d.verifyAdmin).not.toHaveBeenCalled();
    expect(d.setRepoShareSpecs).toHaveBeenCalled();
  });

  it('is harmless for a repo without a GitHub name', async () => {
    const d = deps();
    const res = await saveSpecsToggle({ repoId: 1, repoFullName: null, login: 'me', token: null, stored: false }, d);
    expect(res).toEqual({ ok: true, value: true, note: null });
    expect(d.verifyAdmin).not.toHaveBeenCalled();
  });
});
