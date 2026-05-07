import { Octokit } from '@octokit/rest';

export interface GitHubRepo {
  id: number;
  full_name: string;
  html_url: string;
  description: string | null;
  language: string | null;
  pushed_at: string | null;
}

export async function fetchAllRepos(token: string): Promise<GitHubRepo[]> {
  const octokit = new Octokit({ auth: token });

  const repos: GitHubRepo[] = [];
  let page = 1;

  while (true) {
    const response = await octokit.repos.listForAuthenticatedUser({
      per_page: 100,
      page,
      sort: 'pushed',
    });

    for (const repo of response.data) {
      repos.push({
        id: repo.id,
        full_name: repo.full_name,
        html_url: repo.html_url,
        description: repo.description ?? null,
        language: repo.language ?? null,
        pushed_at: repo.pushed_at ?? null,
      });
    }

    if (response.data.length < 100) break;
    page++;
  }

  return repos;
}

export function splitRepoFullName(fullName: string): { owner: string; repo: string } {
  const [owner, repo] = fullName.split('/');
  return { owner, repo };
}

export interface RepoSecret {
  name: string;
  created_at: string;
  updated_at: string;
}

export async function listRepoSecrets(token: string, owner: string, repo: string): Promise<RepoSecret[]> {
  const octokit = new Octokit({ auth: token });
  const secrets: RepoSecret[] = [];
  let page = 1;

  while (true) {
    const response = await octokit.actions.listRepoSecrets({ owner, repo, per_page: 100, page });
    for (const s of response.data.secrets) {
      secrets.push({ name: s.name, created_at: s.created_at, updated_at: s.updated_at });
    }
    if (secrets.length >= response.data.total_count) break;
    page++;
  }

  return secrets;
}

export async function getRepoPublicKey(token: string, owner: string, repo: string): Promise<{ key: string; key_id: string }> {
  const octokit = new Octokit({ auth: token });
  const response = await octokit.actions.getRepoPublicKey({ owner, repo });
  return { key: response.data.key, key_id: response.data.key_id };
}

export async function createOrUpdateRepoSecret(
  token: string, owner: string, repo: string,
  secretName: string, encryptedValue: string, keyId: string
): Promise<void> {
  const octokit = new Octokit({ auth: token });
  await octokit.actions.createOrUpdateRepoSecret({
    owner, repo, secret_name: secretName, encrypted_value: encryptedValue, key_id: keyId,
  });
}

export async function deleteRepoSecret(token: string, owner: string, repo: string, secretName: string): Promise<void> {
  const octokit = new Octokit({ auth: token });
  await octokit.actions.deleteRepoSecret({ owner, repo, secret_name: secretName });
}

export async function deleteRepoOnGitHub(token: string, owner: string, repo: string): Promise<void> {
  const octokit = new Octokit({ auth: token });
  await octokit.repos.delete({ owner, repo });
}

export interface BranchInfo { name: string; isDefault: boolean; }

export async function listBranches(token: string, owner: string, repo: string): Promise<BranchInfo[]> {
  const octokit = new Octokit({ auth: token });
  const repoInfo = await octokit.repos.get({ owner, repo });
  const defaultBranch = repoInfo.data.default_branch;
  const branches: BranchInfo[] = [];
  let page = 1;
  while (true) {
    const resp = await octokit.repos.listBranches({ owner, repo, per_page: 100, page });
    for (const b of resp.data) {
      branches.push({ name: b.name, isDefault: b.name === defaultBranch });
    }
    if (resp.data.length < 100) break;
    page++;
  }
  return branches;
}

export async function validateToken(token: string): Promise<boolean> {
  try {
    const octokit = new Octokit({ auth: token });
    await octokit.users.getAuthenticated();
    return true;
  } catch {
    return false;
  }
}

// ── v0.18.0: GitHub Environments + env-scoped secrets ────────────────────────

/** Create/idempotent-ensure a repo environment. PUT is no-op if env already exists. */
export async function createEnvironment(
  token: string, owner: string, repo: string, envName: string,
): Promise<void> {
  const octokit = new Octokit({ auth: token });
  await octokit.repos.createOrUpdateEnvironment({
    owner, repo, environment_name: envName,
  });
}

/** Delete a repo environment. Cascades all env-scoped secrets on the GitHub side. */
export async function deleteEnvironment(
  token: string, owner: string, repo: string, envName: string,
): Promise<void> {
  const octokit = new Octokit({ auth: token });
  await octokit.repos.deleteAnEnvironment({
    owner, repo, environment_name: envName,
  });
}

export interface EnvironmentSecret {
  name: string;
  created_at: string;
  updated_at: string;
}

/** List secrets in a specific environment. */
export async function listEnvironmentSecrets(
  token: string, owner: string, repo: string, envName: string,
): Promise<EnvironmentSecret[]> {
  const octokit = new Octokit({ auth: token });
  const result: EnvironmentSecret[] = [];
  let page = 1;
  while (true) {
    const resp = await octokit.actions.listEnvironmentSecrets({
      owner, repo, environment_name: envName, per_page: 100, page,
    });
    for (const s of resp.data.secrets) {
      result.push({ name: s.name, created_at: s.created_at, updated_at: s.updated_at });
    }
    if (result.length >= resp.data.total_count) break;
    page++;
  }
  return result;
}

export async function getEnvironmentPublicKey(
  token: string, owner: string, repo: string, envName: string,
): Promise<{ key: string; key_id: string }> {
  const octokit = new Octokit({ auth: token });
  const resp = await octokit.actions.getEnvironmentPublicKey({
    owner, repo, environment_name: envName,
  });
  return { key: resp.data.key, key_id: resp.data.key_id };
}

export async function createOrUpdateEnvironmentSecret(
  token: string, owner: string, repo: string, envName: string,
  secretName: string, encryptedValue: string, keyId: string,
): Promise<void> {
  const octokit = new Octokit({ auth: token });
  await octokit.actions.createOrUpdateEnvironmentSecret({
    owner, repo, environment_name: envName,
    secret_name: secretName, encrypted_value: encryptedValue, key_id: keyId,
  });
}

export async function deleteEnvironmentSecret(
  token: string, owner: string, repo: string, envName: string, secretName: string,
): Promise<void> {
  const octokit = new Octokit({ auth: token });
  await octokit.actions.deleteEnvironmentSecret({
    owner, repo, environment_name: envName, secret_name: secretName,
  });
}

/** Get repo numeric id (kept for callers that need it for other purposes). */
export async function getRepoId(token: string, owner: string, repo: string): Promise<number> {
  const octokit = new Octokit({ auth: token });
  const resp = await octokit.repos.get({ owner, repo });
  return resp.data.id;
}
