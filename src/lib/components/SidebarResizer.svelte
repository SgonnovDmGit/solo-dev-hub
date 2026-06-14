<script lang="ts">
  // v0.19.0 drag-resize handle for the sidebar, extracted from Sidebar.svelte
  // (T-000099). Owns the pointer-drag mechanics + RAF throttling; commits the
  // result back through bindable props and calls `onCommit` so the parent can
  // persist and react (e.g. close open forms when the drag snaps to collapsed).
  interface Props {
    /** Committed non-collapsed width (px). */
    width: number;
    /** Committed collapsed state. */
    collapsed: boolean;
    /** True while a drag is in flight (parent reads it for layout + drag guards). */
    isResizing: boolean;
    /** Live width during a drag — parent reads it to compute the effective width. */
    previewWidth: number;
    /** Block drag start (e.g. a repo drag is already in flight). */
    disabled?: boolean;
    /** Fired after a drag commits (collapsed/width binds already updated). */
    onCommit?: () => void;
  }
  let {
    width = $bindable(),
    collapsed = $bindable(),
    isResizing = $bindable(),
    previewWidth = $bindable(),
    disabled = false,
    onCommit,
  }: Props = $props();

  let resizeStartX = 0;
  let resizeStartWidth = 0;
  let rafId: number | null = null;

  function clampWidth(w: number): number {
    return Math.max(200, Math.min(500, w));
  }

  function onPointerDown(e: PointerEvent) {
    if (e.button !== 0) return;
    if (disabled) return; // don't start resize while a repo drag is in flight
    e.preventDefault();
    isResizing = true;
    resizeStartX = e.clientX;
    resizeStartWidth = collapsed ? 52 : width;
    previewWidth = resizeStartWidth;
    // Window-level listeners — the pointer may travel outside the handle.
    window.addEventListener('pointermove', onPointerMove);
    window.addEventListener('pointerup', onPointerUp);
    window.addEventListener('pointercancel', onPointerUp);
  }

  function onPointerMove(e: PointerEvent) {
    if (!isResizing) return;
    if (rafId !== null) cancelAnimationFrame(rafId);
    rafId = requestAnimationFrame(() => {
      const delta = e.clientX - resizeStartX;
      previewWidth = Math.max(0, Math.min(500, resizeStartWidth + delta));
      rafId = null;
    });
  }

  function onPointerUp() {
    if (!isResizing) return;
    isResizing = false;
    window.removeEventListener('pointermove', onPointerMove);
    window.removeEventListener('pointerup', onPointerUp);
    window.removeEventListener('pointercancel', onPointerUp);
    if (rafId !== null) {
      cancelAnimationFrame(rafId);
      rafId = null;
    }

    // Commit decision from the final preview width: below 160px snaps to
    // collapsed (width keeps its last non-collapsed value), else commit width.
    if (previewWidth < 160) {
      collapsed = true;
    } else {
      collapsed = false;
      width = clampWidth(previewWidth);
    }
    onCommit?.();
  }
</script>

<div
  class="resize-handle"
  class:active={isResizing}
  onpointerdown={onPointerDown}
  role="separator"
  aria-orientation="vertical"
></div>

<style>
  .resize-handle {
    position: absolute;
    top: 0;
    right: 0;
    width: 4px;
    height: 100%;
    cursor: col-resize;
    z-index: 10;
    user-select: none;
  }
  .resize-handle:hover,
  .resize-handle.active {
    background: var(--accent);
    opacity: 0.5;
  }
</style>
