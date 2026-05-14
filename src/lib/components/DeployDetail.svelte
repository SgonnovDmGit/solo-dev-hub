<script lang="ts">
  import { onMount } from 'svelte';
  import { allRepos } from '$lib/stores/repos';
  import { pat } from '$lib/stores/settings';
  import { tStore, locale } from '$lib/i18n';
  import { addToast } from '$lib/stores/ui';
  import {
    getDeployEnvironment, updateDeployEnvironment,
    renderDeployFilesForEnv,
    getTemplateFile, readRepoFiles, writeDeployFiles,
  } from '$lib/api/tauri-commands';
  import { listBranches, splitRepoFullName, createEnvironment, type BranchInfo } from '$lib/api/github';
  import type { DeployEnvironment, RenderedFile } from '$lib/types';
  import DeploySecretsTable from './DeploySecretsTable.svelte';
  import DiffDialog from './DiffDialog.svelte';

  interface Props {
    deployEnvId: number;
    onBack: () => void;
    // T-000103 Task 4 → Task 5: parent (DeployScreen) owns repo-wide config
    // and passes it down for cross-source empty-required validation +
    // filtering repo-scope placeholders out of the env-specific placeholder
    // loop. Task 4 only wires the prop; Task 5 consumes it.
    repoConfig?: Record<string, string>;
  }
  let { deployEnvId, onBack, repoConfig: _repoConfig = {} }: Props = $props();

  interface PlaceholderSpec {
    key: string;
    label: string;
    description: string;
    default: string;
    type: string;
    // B-000010a: `optional: true` in meta.json marks a placeholder where empty
    // value is intentional (e.g. ENV_FILE_PATH — the template's bash conditional
    // handles empty correctly). Without this flag, an empty value is treated as
    // a required-but-unset field and blocks Generate.
    optional: boolean;
  }

  // Core 5 — fields with dedicated DB columns in deploy_environments. Stays at 5
  // (DB schema-tied). CONTAINER_NAME is a regular placeholder living in extras
  // since v0.25.0 (was a GitHub secret before that).
  const CORE_KEYS = ['WORKFLOW_NAME', 'IMAGE_TAG', 'COMPOSE_SERVICE', 'DOMAIN', 'DEPLOY_BRANCH'];

  let env = $state<DeployEnvironment | null>(null);
  let placeholders = $state<PlaceholderSpec[]>([]);
  let formValues = $state<Record<string, string>>({});
  let saving = $state(false);
  let generating = $state(false);
  // M9 review-fix: secret role changes (build/deploy/runtime) flip which
  // YAML section the secret renders into; without a regenerate the
  // workflow files on disk fall out of sync with the user's intent.
  // Cleared when handleGenerate completes successfully.
  let workflowStale = $state(false);
  let diffFiles = $state<Array<{ path: string; content: string; existingContent: string | null; shouldWrite: boolean }> | null>(null);
  let branches = $state<BranchInfo[]>([]);

  const repo = $derived(env ? ($allRepos.find((r) => r.id === env!.repository_id) ?? null) : null);

  // B-000010a: a placeholder is "required" unless meta.json marks it `optional`.
  // List of currently-empty required placeholders blocks Generate.
  const requiredKeys = $derived(
    placeholders.filter((p) => !p.optional).map((p) => p.key),
  );
  const missingRequired = $derived(
    requiredKeys.filter((k) => (formValues[k] ?? '').trim() === ''),
  );
  const coreComplete = $derived(missingRequired.length === 0);

  // M10 review-fix: placeholder values are substituted into generated YAML
  // via `@@VAR@@` without context-aware escaping. Values containing chars
  // that break YAML in unquoted scalar positions (`:`, `#`, leading `>`,
  // `|`, etc.) silently corrupt the rendered workflow. Flag suspicious
  // values so the user knows before clicking Generate.
  const YAML_UNSAFE_RE = /[:#"'`\\\n\r]|^[\s>|*&!\[\]{}?\-=<]/;
  const yamlWarnings = $derived(
    Object.entries(formValues)
      .filter(([_, v]) => YAML_UNSAFE_RE.test((v ?? '').trim()))
      .map(([k]) => k)
  );

  function extractLocalized(v: any, fallback: string): string {
    if (v == null) return fallback;
    if (typeof v === 'string') return v;
    if (typeof v === 'object') {
      const loc = $locale;
      return v[loc] ?? v.en ?? v.ru ?? fallback;
    }
    return fallback;
  }

  function normalisePlaceholders(raw: any): PlaceholderSpec[] {
    if (!raw || typeof raw !== 'object') return [];
    return Object.entries(raw).map(([key, spec]: [string, any]) => ({
      key,
      label: extractLocalized(spec?.label, key),
      description: extractLocalized(spec?.description, ''),
      default: typeof spec?.default === 'string' ? spec.default : '',
      type: typeof spec?.type === 'string' ? spec.type : 'string',
      optional: spec?.optional === true,
    }));
  }

  // C3 review-fix: split into env-fetch (runs once on mount) and the
  // repo-dependent GitHub-Environments + branches fetch (runs via $effect
  // when both env and repo are resolved). Previously the entire load()
  // early-returned when `repo` was null at mount time — `$allRepos` is
  // loaded in parallel in +page.svelte and may not be resolved yet, so the
  // GH-side createEnvironment and branches fetch were silently skipped
  // and never retriggered, leaving DeployDetail stuck on "Loading…".
  async function load() {
    env = await getDeployEnvironment(deployEnvId);
    if (!env) return;

    if (repo?.deploy_target) {
      const metaFile = await getTemplateFile(repo.deploy_target, 'meta.json');
      if (metaFile) {
        const meta = JSON.parse(metaFile.content);
        placeholders = normalisePlaceholders(meta.placeholders);
      }
    }

    // Populate formValues from env.extras + env core-5 + meta defaults fallback
    const next: Record<string, string> = {
      WORKFLOW_NAME: env.workflow_name,
      IMAGE_TAG: env.image_tag,
      COMPOSE_SERVICE: env.compose_service,
      DOMAIN: env.domain,
      DEPLOY_BRANCH: env.deploy_branch,
    };
    for (const spec of placeholders) {
      if (CORE_KEYS.includes(spec.key)) continue;
      next[spec.key] = env.extras[spec.key] ?? spec.default;
    }
    formValues = next;

  }

  // Idempotent GitHub-side bootstrap that depends on `repo` being resolved.
  // Tracks both `env` and `repo` reactively so a late-arriving `$allRepos`
  // doesn't leave the deploy detail stuck without an Environment object.
  let ghBootstrapped = $state(false);
  $effect(() => {
    if (!env || !repo || ghBootstrapped) return;
    if (!$pat || !repo.github_name || !repo.github_name.includes('/')) return;
    ghBootstrapped = true;
    void bootstrapGitHubSide(env, repo.github_name);
  });

  async function bootstrapGitHubSide(envSnapshot: DeployEnvironment, ghName: string) {
    const { owner, repo: name } = splitRepoFullName(ghName);
    // Ensure GitHub Environment exists for this deploy slot. Idempotent PUT —
    // no-op when already exists, creates it when missing. Critical for the
    // workflow's `environment: <name>` directive to validate.
    try {
      await createEnvironment($pat!, owner, name, envSnapshot.name);
    } catch (e: any) {
      const msg = String(e?.message ?? e);
      addToast(
        ($tStore('deploy.envCreateFailed' as any) || 'Could not create GitHub Environment "{0}": {1}')
          .replace('{0}', envSnapshot.name)
          .replace('{1}', msg),
        'warning',
      );
    }
    // Fetch branches for DEPLOY_BRANCH datalist.
    try {
      branches = await listBranches($pat!, owner, name);
    } catch {
      // Offline or no access — datalist stays empty, free-text still works.
    }
  }

  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  function scheduleSave() {
    if (!env) return;
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = setTimeout(async () => {
      if (!env) return;
      saving = true;
      try {
        const extras: Record<string, string> = {};
        for (const spec of placeholders) {
          if (CORE_KEYS.includes(spec.key)) continue;
          const v = (formValues[spec.key] ?? '').trim();
          if (v !== '') extras[spec.key] = v;
        }
        await updateDeployEnvironment({
          id: env.id,
          workflow_name: (formValues.WORKFLOW_NAME ?? '').trim(),
          image_tag: (formValues.IMAGE_TAG ?? '').trim(),
          compose_service: (formValues.COMPOSE_SERVICE ?? '').trim(),
          domain: (formValues.DOMAIN ?? '').trim(),
          deploy_branch: (formValues.DEPLOY_BRANCH ?? '').trim(),
          extras,
        });
      } catch (err) {
        addToast(String(err), 'error');
      } finally {
        saving = false;
      }
    }, 400);
  }

  async function handleGenerate() {
    if (!env || !repo || !repo.local_path) {
      addToast($tStore('deploy.noLocalPath' as any), 'error');
      return;
    }
    if (!coreComplete) return;
    generating = true;
    try {
      // Note: GitHub Environment creation is handled in `load()` on mount —
      // covers all entry paths (open existing env, just-cloned, just-created)
      // without coupling it to the Generate flow. Generate just needs to render
      // and write files.
      const rendered: RenderedFile[] = await renderDeployFilesForEnv(env.id);
      const existing = await readRepoFiles(repo.id, rendered.map((r) => r.path));
      diffFiles = rendered.map((r, i) => ({
        path: r.path,
        content: r.content,
        existingContent: existing[i],
        shouldWrite: existing[i] !== r.content,
      }));
    } catch (err) {
      addToast(String(err), 'error');
    } finally {
      generating = false;
    }
  }

  async function handleDiffConfirm(toWrite: RenderedFile[]) {
    if (!repo?.local_path || !env) return;
    if (toWrite.length === 0) {
      diffFiles = null;
      addToast($tStore('deploy.nothingToWrite' as any), 'info');
      return;
    }
    try {
      const result = await writeDeployFiles(env.id, env.repository_id, repo.local_path, toWrite);
      diffFiles = null;
      workflowStale = false;
      addToast(
        $tStore('toast.deployWritten' as any).replace('{0}', String(result.written.length)),
        'success',
      );
    } catch (err) {
      addToast(String(err), 'error');
    }
  }

  onMount(load);
</script>

{#if !env || !repo}
  <p>{$tStore('common.loading' as any)}</p>
{:else}
  <div class="detail">
    <div class="header">
      <button class="ghost" onclick={onBack}>{$tStore('deploy.backToList' as any) || '← Back to list'}</button>
      <h3>{($tStore('deploy.editDeployment' as any) || 'Edit deployment: {0}').replace('{0}', env.name)}</h3>
    </div>

    <section>
      {#each placeholders as spec (spec.key)}
        {@const inputId = `placeholder-${env.id}-${spec.key}`}
        {@const isMissing = !spec.optional && (formValues[spec.key] ?? '').trim() === ''}
        <div class="field" class:missing-required={isMissing} title={spec.description}>
          <label for={inputId}>{spec.label}:</label>
          {#if spec.key === 'DEPLOY_BRANCH' && branches.length > 0}
            <input id={inputId} type="text"
                   list="branches-{env.id}"
                   value={formValues[spec.key] ?? ''}
                   oninput={(e) => { formValues[spec.key] = (e.currentTarget as HTMLInputElement).value; scheduleSave(); }} />
            <datalist id="branches-{env.id}">
              {#each branches as b (b.name)}
                <option value={b.name}>{b.isDefault ? `${b.name} (default)` : ''}</option>
              {/each}
            </datalist>
          {:else if spec.key === 'COMPOSE_SERVICE'}
            <input id={inputId} type="text"
                   value={formValues[spec.key] ?? ''}
                   placeholder={spec.default}
                   oninput={(e) => { formValues[spec.key] = (e.currentTarget as HTMLInputElement).value; scheduleSave(); }} />
            <button type="button"
                    class="ghost copy-btn"
                    title={$tStore('deploy.copyFromContainerName' as any) || 'Copy from container name'}
                    disabled={!(formValues.CONTAINER_NAME ?? '').trim()}
                    onclick={() => { formValues.COMPOSE_SERVICE = (formValues.CONTAINER_NAME ?? '').trim(); scheduleSave(); }}>
              ↩
            </button>
          {:else}
            <input id={inputId} type="text"
                   value={formValues[spec.key] ?? ''}
                   placeholder={spec.default}
                   oninput={(e) => { formValues[spec.key] = (e.currentTarget as HTMLInputElement).value; scheduleSave(); }} />
          {/if}
        </div>
      {/each}
    </section>

    <section>
      <h4>{$tStore('deploy.secretsSection' as any) || 'Secrets'}</h4>
      <DeploySecretsTable
        deployEnvId={env.id}
        envName={env.name}
        repoId={repo.id}
        onRoleChange={() => { workflowStale = true; }}
      />
    </section>

    {#if missingRequired.length > 0}
      <section class="missing-warn">
        ⚠ {$tStore('deploy.missingRequired' as any) || 'Required fields are empty — fill them before generating'}: {missingRequired.join(', ')}
      </section>
    {/if}

    {#if yamlWarnings.length > 0}
      <section class="yaml-warn">
        ⚠ {$tStore('deploy.yamlUnsafeWarning' as any) || 'Values may break YAML — review before generating'}: {yamlWarnings.join(', ')}
      </section>
    {/if}

    <section class="generate-row">
      <button
        class="primary"
        class:stale={workflowStale}
        disabled={generating || !coreComplete}
        onclick={handleGenerate}
        title={!coreComplete
          ? (($tStore('deploy.missingRequired' as any) || 'Required fields are empty') + ': ' + missingRequired.join(', '))
          : workflowStale
            ? ($tStore('deploy.regenerateNeeded' as any) || 'Workflow files are stale — regenerate to apply role changes')
            : ''}
      >
        {workflowStale
          ? ($tStore('deploy.regenerateWorkflowFiles' as any) || 'Regenerate workflow files')
          : ($tStore('deploy.generateWorkflowFiles' as any) || 'Generate workflow files')}
      </button>
    </section>
  </div>
{/if}

{#if diffFiles}
  <DiffDialog
    files={diffFiles}
    onConfirm={handleDiffConfirm}
    onCancel={() => diffFiles = null}
  />
{/if}

<style>
  .detail { padding: 1rem; }
  .header { display: flex; align-items: center; gap: 1rem; }
  section { margin: 1.5rem 0; }
  .field {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin: 0.4rem 0;
  }
  .field label {
    min-width: 11rem;
    text-align: right;
    flex-shrink: 0;
    font-size: 0.9em;
    color: var(--text);
  }
  .field input {
    flex: 1;
    padding: 0.4rem;
    box-sizing: border-box;
  }
  /* B-000010a: visual marker for empty required placeholders. Red border on
     the input so the user can spot the missing field without scanning the
     full form against the summary list at the bottom. */
  .field.missing-required input {
    border-color: rgb(220, 38, 38);
    box-shadow: 0 0 0 1px rgba(220, 38, 38, 0.25);
  }
  .copy-btn {
    flex-shrink: 0;
    padding: 0.25rem 0.55rem;
    font-size: 0.95em;
    line-height: 1.2;
    cursor: pointer;
  }
  .copy-btn:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .generate-row {
    display: flex;
    justify-content: flex-end;
  }
  .yaml-warn {
    background: rgba(234, 179, 8, 0.1);
    border: 1px solid rgba(234, 179, 8, 0.35);
    border-radius: 4px;
    padding: 0.5rem 0.75rem;
    color: var(--text);
    font-size: 0.85rem;
  }
  /* B-000010a: red-tinted warning matching .yaml-warn's structure but with
     "blocker" semantics (Generate is disabled while this is shown). */
  .missing-warn {
    background: rgba(220, 38, 38, 0.1);
    border: 1px solid rgba(220, 38, 38, 0.35);
    border-radius: 4px;
    padding: 0.5rem 0.75rem;
    color: var(--text);
    font-size: 0.85rem;
  }
  /* M9 review-fix: amber-tinted button when workflow files are stale due
     to role changes. Stays disabled if !coreComplete (semantic priority:
     "fix the form first"); enables once form is complete to draw the
     user toward regenerate. */
  button.primary.stale {
    background-color: rgb(234, 179, 8);
    border-color: rgb(234, 179, 8);
  }
  button.primary.stale:hover:not(:disabled) {
    background-color: rgb(202, 138, 4);
  }
</style>
