<script lang="ts">
  import { tStore, type TranslationKey } from '$lib/i18n';
  import type { RuleEvent } from '$lib/types';
  import { formatRuleDate, noteKeyOf } from '$lib/utils/rules';

  let { events }: { events: RuleEvent[] } = $props();

  function kindLabel(kind: string): string {
    const key = `rules.event.${kind}` as TranslationKey;
    const label = $tStore(key);
    return label === key ? kind : label;
  }

  function noteText(note: string): string {
    const key = noteKeyOf(note);
    return key ? $tStore(key) : note;
  }
</script>

{#if events.length === 0}
  <div class="empty">{$tStore('rules.historyEmpty')}</div>
{:else}
  <table>
    <thead>
      <tr>
        <th>{$tStore('rules.historyWhen')}</th>
        <th>{$tStore('rules.historyWhat')}</th>
        <th>{$tStore('rules.historyWhere')}</th>
      </tr>
    </thead>
    <tbody>
      {#each events as ev (ev.id)}
        <tr>
          <td class="when">{formatRuleDate(ev.created_at)}</td>
          <td>{kindLabel(ev.kind)}</td>
          <td>
            {#if ev.repo}<code>{ev.repo}</code>{/if}
            {#if ev.repo && ev.note} · {/if}
            {#if ev.note}<span class="note">{noteText(ev.note)}</span>{/if}
          </td>
        </tr>
      {/each}
    </tbody>
  </table>
{/if}
<div class="footer">{$tStore('rules.historySince')}</div>

<style>
  .empty { font-size: 12px; color: var(--text-muted); padding: 12px 0; }
  table { width: 100%; border-collapse: collapse; font-size: 12px; }
  th { text-align: left; font-size: 10px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.05em; color: var(--text-muted); padding: 4px 8px; border-bottom: 1px solid var(--border); }
  td { padding: 6px 8px; border-bottom: 1px solid var(--border); vertical-align: top; }
  .when { white-space: nowrap; color: var(--text-muted); font-variant-numeric: tabular-nums; }
  code { font-family: ui-monospace, "SF Mono", Menlo, monospace; font-size: 11px; }
  .note { color: var(--text-muted); }
  .footer { font-size: 11px; color: var(--text-muted); margin-top: 10px; line-height: 1.4; }
</style>
