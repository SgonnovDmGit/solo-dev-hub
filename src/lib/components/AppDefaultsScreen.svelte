<script lang="ts">
  import { currentScreen } from '$lib/stores/ui';
  import { tStore } from '$lib/i18n';
  import TemplateEditor from './TemplateEditor.svelte';
  import { isRuleFile } from '$lib/utils/rules';

  // T-000174: rule files (skills + the global section) live on the Rules screen.
  // P9 review-fix: the predicate is a module-level reference, so TemplateEditor's
  // $effect tracking `exclude` doesn't re-fire on every parent render (theme
  // toggle, i18n change) — an inline arrow would get a new identity each time.
  const exclude = isRuleFile;

  function back() {
    currentScreen.set({ name: 'settings' });
  }
</script>

<div class="app-defaults-screen">
  <div class="header">
    <button class="ghost back-btn" onclick={back} type="button">
      {$tStore('settings.back' as any)}
    </button>
    <h2>{$tStore('appDefaults.title' as any)}</h2>
  </div>

  <div class="body">
    <TemplateEditor languageKey="_global" {exclude} />
  </div>
</div>

<style>
  .app-defaults-screen { height: 100%; display: flex; flex-direction: column; overflow: hidden; }
  .header { flex-shrink: 0; padding: 12px 24px; border-bottom: 1px solid var(--border); display: flex; align-items: center; gap: 16px; }
  .back-btn { font-size: 12px; padding: 3px 8px; color: var(--text-muted); }
  .back-btn:hover { color: var(--accent); }
  h2 { font-size: 18px; font-weight: 700; margin: 0; }
  .body { flex: 1; overflow: hidden; display: flex; }
</style>
