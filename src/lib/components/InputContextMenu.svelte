<script lang="ts">
  import { tStore } from '$lib/i18n';

  interface Props {
    x: number;
    y: number;
    target: HTMLInputElement | HTMLTextAreaElement;
    onClose: () => void;
  }

  let { x, y, target, onClose }: Props = $props();

  // B-000007: WebView2's native input context menu shows extras like "More
  // tools" / "Writing direction" that look like browser dev cruft inside a
  // desktop app. We suppress the native menu entirely and render this minimal
  // 4-item replacement (Cut / Copy / Paste / Select All) — same actions a user
  // would actually need in a form field.

  const isReadOnly = $derived(target.readOnly || target.disabled);
  const hasSelection = $derived.by(() => {
    const start = target.selectionStart ?? 0;
    const end = target.selectionEnd ?? 0;
    return end > start;
  });

  let menuEl = $state<HTMLDivElement | null>(null);
  let menuPos = $state<{ left: number; top: number }>({ left: 0, top: 0 });

  // Clamp menu inside viewport (right-click near right/bottom edge would
  // otherwise clip the menu). Effect reads x, y, menuEl reactively.
  $effect(() => {
    let left = x;
    let top = y;
    if (menuEl) {
      const rect = menuEl.getBoundingClientRect();
      const vw = window.innerWidth;
      const vh = window.innerHeight;
      if (left + rect.width > vw) left = Math.max(4, vw - rect.width - 4);
      if (top + rect.height > vh) top = Math.max(4, vh - rect.height - 4);
    }
    menuPos = { left, top };
  });

  function getSelectionText(): string {
    const start = target.selectionStart ?? 0;
    const end = target.selectionEnd ?? 0;
    return target.value.slice(start, end);
  }

  async function handleCut() {
    if (isReadOnly) { onClose(); return; }
    const text = getSelectionText();
    if (text) {
      try {
        await navigator.clipboard.writeText(text);
        const start = target.selectionStart ?? 0;
        const end = target.selectionEnd ?? 0;
        target.focus();
        target.setRangeText('', start, end, 'end');
        target.dispatchEvent(new Event('input', { bubbles: true }));
      } catch (e) { console.warn('cut failed', e); }
    }
    onClose();
  }

  async function handleCopy() {
    const text = getSelectionText();
    if (text) {
      try { await navigator.clipboard.writeText(text); }
      catch (e) { console.warn('copy failed', e); }
    }
    onClose();
  }

  async function handlePaste() {
    if (isReadOnly) { onClose(); return; }
    try {
      const text = await navigator.clipboard.readText();
      if (text) {
        const start = target.selectionStart ?? 0;
        const end = target.selectionEnd ?? 0;
        target.focus();
        target.setRangeText(text, start, end, 'end');
        target.dispatchEvent(new Event('input', { bubbles: true }));
      }
    } catch (e) { console.warn('paste failed', e); }
    onClose();
  }

  function handleSelectAll() {
    target.focus();
    target.select();
    onClose();
  }

  function handleOutside(e: MouseEvent) {
    if (menuEl && !menuEl.contains(e.target as Node)) onClose();
  }

  function handleKey(e: KeyboardEvent) {
    if (e.key === 'Escape') onClose();
  }
</script>

<!-- Note: no `oncontextmenu` here — the parent +page document-level listener
     already routes new contextmenu events (replacing or clearing ctxMenu).
     A redundant svelte:window oncontextmenu would risk catching the very
     event that opened this menu and closing it on the same tick. -->
<svelte:window onmousedown={handleOutside} onkeydown={handleKey} />

<div
  bind:this={menuEl}
  class="ctx-menu"
  style="left: {menuPos.left}px; top: {menuPos.top}px"
  onmousedown={(e) => e.stopPropagation()}
  oncontextmenu={(e) => { e.preventDefault(); e.stopPropagation(); }}
  role="menu"
  tabindex="-1"
>
  <button class="item" disabled={!hasSelection || isReadOnly} onclick={handleCut} type="button">
    {$tStore('ctx.cut' as any)}
  </button>
  <button class="item" disabled={!hasSelection} onclick={handleCopy} type="button">
    {$tStore('ctx.copy' as any)}
  </button>
  <button class="item" disabled={isReadOnly} onclick={handlePaste} type="button">
    {$tStore('ctx.paste' as any)}
  </button>
  <div class="sep"></div>
  <button class="item" onclick={handleSelectAll} type="button">
    {$tStore('ctx.selectAll' as any)}
  </button>
</div>

<style>
  .ctx-menu {
    position: fixed;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 4px 0;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.25);
    z-index: 9999;
    min-width: 160px;
    user-select: none;
  }
  .item {
    display: block;
    width: 100%;
    background: transparent;
    border: none;
    text-align: left;
    padding: 6px 14px;
    font-size: 12px;
    color: var(--text);
    cursor: pointer;
  }
  .item:hover:not(:disabled) {
    background: var(--surface-hover, rgba(255, 255, 255, 0.06));
  }
  .item:disabled {
    color: var(--text-muted);
    cursor: not-allowed;
    opacity: 0.5;
  }
  .sep {
    height: 1px;
    background: var(--border);
    margin: 4px 0;
  }
</style>
