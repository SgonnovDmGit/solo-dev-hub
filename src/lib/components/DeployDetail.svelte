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
  import { listBranches, splitRepoFullName, type BranchInfo } from '$lib/api/github';
  import type { DeployEnvironment, RenderedFile } from '$lib/types';
  import DeploySecretsTable from './DeploySecretsTable.svelte';
  import DiffDialog from './DiffDialog.svelte';

  interface Props {
    deployEnvId: number;
    onBack: () => void;
  }
  let { deployEnvId, onBack }: Props = $props();

  interface PlaceholderSpec {
    key: string;
    label: string;
    description: string;
    default: string;
    type: string;
  }

  const CORE_KEYS = ['WORKFLOW_NAME', 'IMAGE_TAG', 'COMPOSE_SERVICE', 'DOMAIN', 'DEPLOY_BRANCH'];

  let env = $state<DeployEnvironment | null>(null);
  let placeholders = $state<PlaceholderSpec[]>([]);
  let formValues = $state<Record<string, string>>({});
  let saving = $state(false);
  let generating = $state(false);
  let diffFiles = $state<Array<{ path: string; content: string; existingContent: string | null; shouldWrite: boolean }> | null>(null);
  let branches = $state<BranchInfo[]>([]);

  const repo = $derived(env ? ($allRepos.find((r) => r.id === env!.repository_id) ?? null) : null);
  const coreComplete = $derived(CORE_KEYS.every((k) => (formValues[k] ?? '').trim() !== ''));

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
    }));
  }

  async function load() {
    env = await getDeployEnvironment(deployEnvId);
    if (!env || !repo) return;

    if (repo.deploy_target) {
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

    // Fetch branches for DEPLOY_BRANCH datalist
    if ($pat && repo.github_name && repo.github_name.includes('/')) {
      try {
        const { owner, repo: name } = splitRepoFullName(repo.github_name);
        branches = await listBranches($pat, owner, name);
      } catch {
        // Offline or no access — datalist stays empty, free-text still works
      }
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
      const rendered: RenderedFile[] = await renderDeployFilesForEnv(env.id);
      // v0.18.0 scope cut: legacy deploy.yml detection → toast-warning only.
      // Full DiffDialog-integrated delete flow deferred to v0.18.1 (requires
      // DiffDialog `isDelete` semantics + delete_repo_file Tauri command).
      const legacyProbe = await readRepoFiles(repo.local_path, ['.github/workflows/deploy.yml']);
      if (legacyProbe[0] !== null) {
        addToast($tStore('deploy.legacyDeployYmlWarning' as any) || 'Legacy .github/workflows/deploy.yml detected. After generating new deploy-{name}.yml files, remove it manually (git rm).', 'warning');
      }
      const existing = await readRepoFiles(repo.local_path, rendered.map((r) => r.path));
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
      const files = toWrite.map((f) => ({ rel_path: f.path, content: f.content }));
      const result = await writeDeployFiles(env.id, env.repository_id, repo.local_path, files);
      diffFiles = null;
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
        <div class="field" title={spec.description}>
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
      <DeploySecretsTable deployEnvId={env.id} envName={env.name} repoId={repo.id} />
    </section>

    <section class="generate-row">
      <button class="primary" disabled={generating || !coreComplete} onclick={handleGenerate}>
        {$tStore('deploy.generateWorkflowFiles' as any) || 'Generate workflow files'}
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
  .generate-row {
    display: flex;
    justify-content: flex-end;
  }
</style>
