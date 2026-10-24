<script lang="ts">
  // F-000044 / T-000223 (frame 2B): the "Участники" tab of a shared repo —
  // owner, participants with their number bands, and join requests. The owner's
  // install can approve a request; a contributor sees the same table read-only.
  import { tStore, locale } from '$lib/i18n';
  import type { ParticipantRow } from '$lib/utils/shared-mode';
  import { formatRelativeTime, nowTick } from '$lib/utils/time-format';

  interface Props {
    rows: ParticipantRow[];
    /** Offer "Подтвердить" (the owner's install). */
    isOwnerInstall: boolean;
    /** Login of the request being approved right now. */
    approving: string | null;
    onApprove: (login: string) => void;
    /** Last push per participant, keyed by lower-cased login (owner's install only). */
    activity?: Map<string, string | null>;
  }

  let { rows, isOwnerInstall, approving, onApprove, activity = new Map() }: Props = $props();

  // The column is the owner's view: a contributor's install has no such data.
  const lastPush = (row: ParticipantRow): string | null =>
    row.kind === 'participant' ? (activity.get(row.login.trim().toLowerCase()) ?? null) : null;

  function roleKey(kind: ParticipantRow['kind']): string {
    return kind === 'owner' ? 'shared.roleOwner' : kind === 'participant' ? 'shared.roleParticipant' : 'shared.roleRequest';
  }
</script>

<div class="participants">
  {#if rows.length === 0}
    <div class="empty">{$tStore('shared.participantsEmpty' as any)}</div>
  {:else}
    <table>
      <thead>
        <tr>
          <th>{$tStore('shared.colLogin' as any)}</th>
          <th>{$tStore('shared.colBranch' as any)}</th>
          <th>{$tStore('shared.colNumbers' as any)}</th>
          <th>{$tStore('shared.colRole' as any)}</th>
          {#if isOwnerInstall}
            <th title={$tStore('shared.colLastPushHint' as any)}>{$tStore('shared.colLastPush' as any)}</th>
          {/if}
          <th class="action-col"></th>
        </tr>
      </thead>
      <tbody>
        {#each rows as row (row.kind + ':' + row.login)}
          <tr class:request={row.kind === 'request'}>
            <td class="login">
              {row.login}{#if row.isMe}{' '}<span class="you">{$tStore('shared.you' as any)}</span>{/if}
            </td>
            <td class="mono">{row.branch}</td>
            <td class="mono">
              {#if row.kind === 'owner' && row.ownerCeiling !== null}
                {$tStore('shared.bandUpTo' as any).replace('{0}', String(row.ownerCeiling))}
              {:else if row.band}
                {row.band}
              {:else}
                {$tStore('shared.bandNone' as any)}
              {/if}
            </td>
            <td><span class="role-badge {row.kind}">{$tStore(roleKey(row.kind) as any)}</span></td>
            {#if isOwnerInstall}
              {@const at = lastPush(row)}
              <td class="last-push" title={at ? new Date(at).toLocaleString() : ''}>
                {#if at}
                  {formatRelativeTime(at, $nowTick, $locale === 'en' ? 'en' : 'ru')}
                {:else}
                  {$tStore('shared.bandNone' as any)}
                {/if}
              </td>
            {/if}
            <td class="action-col">
              {#if row.canApprove && isOwnerInstall}
                <button
                  type="button"
                  class="approve"
                  onclick={() => onApprove(row.login)}
                  disabled={approving !== null}
                  title={$tStore('shared.approveHint' as any)}
                >{$tStore('shared.approve' as any)}</button>
              {/if}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
    {#if !isOwnerInstall}
      <p class="note">{$tStore('shared.participantsReadOnly' as any)}</p>
    {/if}
  {/if}
</div>

<style>
  .participants {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 12px 24px;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }

  th {
    text-align: left;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-muted);
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
  }

  td {
    padding: 7px 10px;
    border-bottom: 1px solid var(--border);
    color: var(--text);
  }

  tr.request td {
    background: color-mix(in srgb, var(--toast-warning) 8%, transparent);
  }

  .login {
    font-weight: 600;
  }

  .you {
    font-weight: 400;
    color: var(--text-muted);
  }

  .mono {
    font-family: monospace;
    font-size: 12px;
  }

  .last-push {
    white-space: nowrap;
    color: var(--text-muted);
  }

  .action-col {
    width: 1%;
    white-space: nowrap;
    text-align: right;
  }

  .role-badge {
    font-size: 11px;
    padding: 1px 8px;
    border-radius: 10px;
    border: 1px solid var(--border);
    color: var(--text-muted);
  }

  .role-badge.owner {
    color: var(--accent);
    border-color: var(--accent);
  }

  .role-badge.request {
    color: var(--toast-warning);
    border-color: var(--toast-warning);
  }

  .approve {
    font-size: 11px;
    padding: 2px 12px;
    border-radius: 4px;
    border: 1px solid var(--accent);
    background: transparent;
    color: var(--accent);
    cursor: pointer;
  }

  .approve:hover:not(:disabled) {
    background: var(--accent);
    color: white;
  }

  .approve:disabled {
    opacity: 0.5;
    cursor: wait;
  }

  .empty {
    padding: 16px;
    text-align: center;
    color: var(--text-muted);
    font-size: 13px;
    border: 1px dashed var(--border);
    border-radius: 6px;
  }

  .note {
    margin: 10px 0 0;
    font-size: 12px;
    color: var(--text-muted);
  }
</style>
