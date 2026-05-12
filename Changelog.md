# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/). Russian version: [Changelog.ru.md](Changelog.ru.md).

## [Unreleased]

## [0.27.0] — 2026-05-12

### Changed
- **T-000090 | Template: explicit reverse-direction disclaimer for REQ** — hardening of the `# Cross-repo requirements` section in the global CLAUDE.md template after a live-session misuse: a server-LLM wrote a "REQ for the admin" and placed it in `docs/client-requirements/<client>/` inversely — what was actually an announcement. Changes: (1) in `## Folders`, after the flat/nested explainer, an explicit note was added: "Reverse directions (server → client, microservice → parent server) do not exist as REQ — sender-initiated changes flow through announcements". Mirrors the disclaimer already present in `## Directions` of the announcement section. (2) In `## LLM policy > LLM must NOT:` a new bullet was added — "author `REQ-*.md` in own recipient-folder (server's `client-requirements/`, microservice's `server-requirements/`)" — those folders are populated by Solo Dev Hub from the sender's outgoing folder, not written by hand; a REQ written there will not propagate and will be mistaken for an incoming request. Server→client and microservice→parent-server initiatives travel via announcements, not REQ.
- **T-000081 | Project CLAUDE.md full refactor** — compressed the inline version-history blob (v0.16→v0.25 inside "Ключевые решения") into a compact "Эволюция" section with one-liners per version (details live in Changelog.md anyway). Stale link cleanup: product display name "GitHub Repo Manager" → "Solo Dev Hub", autoupdate endpoint URL updated to `solo-dev-hub` repo, stale `docs/doc1_global_rules.md` / `doc2` / `doc3` references replaced with current paths (`docs/flows/`, `docs/formats/`, global template). Test count re-baselined to 298. Components table consolidated (Dashboard sub-components grouped into one row). Aligned with global template section taxonomy.
- **T-000089 | Changelog EN + RU split** — renamed `Changelog.md` (Russian) to `Changelog.ru.md`, created English-primary `Changelog.md` as the public-facing changelog. Pre-v0.16 versions condensed to one-liners (historical). The English version will be kept primary going forward; Russian mirror maintained in parallel.

### Added
- **T-000084 | GitHub repo description + topics** — public-launch SEO/discovery preparation. Applied via GitHub web UI on 2026-05-12 (Settings → About). Description: "Solo developer's portfolio cockpit. Bugs, requirements, deploy — all in markdown." (matches README hero, 80 chars, well under the 350-char limit). Topics (12): `tauri`, `svelte`, `sveltekit`, `rust`, `project-management`, `github`, `solo-developer`, `indie-dev`, `bug-tracker`, `deploy-automation`, `developer-tools`, `desktop-app`. Last two added on second-agent review for broader discoverability. Topics work even on a private repo; they become visible only after the public flip (T-000064 in v1.0.0).

## [0.26.1] — 2026-05-12

### Added
- **F-000040 | Cross-repo announcements (proactive push)** — new section in the global CLAUDE.md template for unsolicited information that does not fit the REQ/receipt pattern. Two directions: server→client (sender writes directly to `<client-path>/docs/server-announcements/<sender-canonical>/ANNOUNCE-NNN_<slug>.md`) and microservice→parent-server (mirror, into `docs/microservice-announcements/<ms-project>/`). Recipient reads + deletes = implicit acknowledgement (audit trail in recipient's git history). No app-side sync, no receipts, no confirm-✓. Sender obtains recipient's local path from its own `docs/project.md` — either the `## Repositories` Path column (for clients) or `## Parent projects` (for parent servers, now with path after extending `generate_project_md`). If path is missing ("no local path configured"), the announcement is not deliverable and the sender surfaces a gap in its own todo. Explicit carve-out from the rule "LLM never copies across repo boundaries" — announcements are the one allowed exception; REQ is not. Use cases: server-initiated change affecting client (e.g. new admin endpoint requires client integration); side-effect change affecting other clients; post-internal-review rework affecting client; MS-side change affecting parent. **NOT** for "client asked → server did → integration notes" — those go into the REQ receipt's `## Comment:`.

### Changed
- **`docs/project.md` Parent projects section** now includes the local path of the parent server-repo: `- **<parent-name>** — server repo: <name> (path: <local-path>)` or `(no local path configured)`. Sourced from `db.server_repo_of_microservice(parent_id)` + `.local_path`. Required by F-000040 for MS→server announcement push: the MS-LLM derives the target filesystem path from its own project.md without cross-repo sync infrastructure. +2 unit tests (`test_generate_project_md_microservice_parent_includes_server_path`, `test_generate_project_md_microservice_parent_without_server_path`) → 298 total.
- **T-000086 | F-000040 template clarifications** — two refinements to the `# Cross-repo announcements` section in the global CLAUDE.md template, surfaced during a pilot review by a subagent. (1) **NNN counter behavior** — an explicit rule for non-empty folders: use `max(existing NNN) + 1`; NNN is a monotonic counter, not a slot allocator. Closes the case where some entries were previously acknowledged and removed — new numbers still go after the maximum used, never reusing freed slots. (2) **Threshold: actionable impact required** — a new `### Threshold` subsection after the main "When to use" table. Strict criterion: an announcement is appropriate only when the recipient would need to take action (change code / config / behavior). Mere existence of a sender-side change (e.g. a new admin-only endpoint) is not enough; pure surface additions flow through `docs/api.md` sync. Positive criterion (helper for unsure-senders): if the recipient must change code/config/behavior to keep working — write an announcement; if they can continue and adopt later via api.md — don't. Rationale: excessive announcements devalue the channel.
- **T-000085 | docs/flows/cross-repo-announcements.md** — new flow doc for the F-000040 channel, mirroring the structure of `microservice-server-sync.md`. Contents: model (one-way push, two directions, no app-side sync), "When to use vs REQ receipt" table + threshold/positive criterion summary, lifecycle (server→client example with rate-limit header), microservice→parent-server flow (mentioning project.md path lookup and the v0.26.0 extension), "when appropriate" scenarios (sender-initiated / side-effect / post-review-rework / deprecation) vs "when not" (admin-only surface / silent fix / reactive case / reverse REQ / undefined impact), "Where Solo Dev Hub helps / does not" table (app participates only in `generate_project_md`), carve-out from the no-cross-repo-writes rule with justification via flow asymmetry, cross-reference to the normative H1/H2/H3 in the global CLAUDE.md template and to `microservice-server-sync.md` § Triangular flow for REQ-based flows.
- **T-000088 | RepoDetail header — 2-row chip layout** — header rewritten after iterative design review with the ui-ux-pro-max skill (variant C). Replaces the original 5-row layout with a compact 2-row chip-based version: (1) `[lang] last-pushed · 📁 path [Specify folder]` with `[📚 Update repo docs]` on the right; (2) editable chips `[Project: ▼] [Role: ▼] [Deploy template: ▼]` with `🗑 Delete` on the right. Removed: back button "Back to Dashboard" (Dashboard is available via top-bar + sidebar; the button was redundant and mislabeled — always navigated to Dashboard, not "back"), the header-top cluster with role-badge/project-tag (values duplicated by the chips below), `repo.description` (low signal, noisy), derived `roleLabel`/`roleIcon`/`projectName` (no longer used), `ROLE_ICONS` import. Editable controls styled as pill chips (radius 14px, surface bg, accent border on hover) — native `<select>` styled transparent. Delete button — ghost (transparent border at rest, danger-border on hover, no "repository" word — context is the header). New CSS classes: `.chip`, `.chip-label`, `.chip-select`, `.row-action`, `.meta-dot`. Removed CSS: `.header-top`, `.header-right`, `.back-btn`, `.repo-desc`, `.settings-row`, `.actions-row`, `.meta-pair`, `.meta-label`, `.role-badge`, `.project-tag`, `.inline-select`. HTML preview of iterations at `docs/superpowers/plans/2026-05-12-repo-detail-header-variants.html`. The same approach was applied to **ProjectDetail** in parallel — back button "← Back" removed (hardcoded to Dashboard; redundant with top-bar + sidebar), including the non-found state; `header-top-row` div, `goBack()` function, CSS `.header-top-row` / `.back-btn` removed. Dead i18n keys cleanup: `repoDetail.backToRepos`, `repoDetail.backToReposTooltip`, `project.backToRepos` (ru + en). Adaptive narrow-window behavior: chips got `white-space: nowrap` + `flex-shrink: 0` (the "Deploy template:" label no longer wraps to two lines in a narrow window). Action buttons collapse to icon-only via a container query `@container repo-header (max-width: 760px)` — `.sticky-header` is declared a named size-container, below the threshold `.row-action .btn-label` is hidden and only icon + tooltip remain. The labels "Update repo docs" and "Delete" are wrapped in `<span class="btn-icon">` + `<span class="btn-label">`. The `📚` emoji was removed from the i18n value `repo.initDocsButton` (now lives in the template as an icon-span).
- **Dark theme contrast in native `<select>` dropdowns** — added `color-scheme: dark` on `:root` / `[data-theme="dark"]` and `color-scheme: light` on `[data-theme="light"]` in `app.css`. This is the standard hint asking the browser to use a dark native UI for scrollbars, dropdowns, datepickers. Previously the native `<option>` popup was rendered by WebView2 with OS-default white background → gray text became unreadable. Additional fallback `option { background: var(--bg); color: var(--text); }` for platforms that don't honor color-scheme.
- **T-000080 / T-000079 | Deploy moved from a top-level screen into a RepoDetail tab** — previously Deploy opened as a separate screen route via the 🚀 button in the RepoDetail header (`currentScreen.set({name: 'deploy'})`). Architecturally this is a master-detail inside a single repo (deploy-instance list → drill-down per env), so it logically belongs next to Bugs/Tasks/Done/Changelog/Secrets/Stats. The tab is inserted between Changelog and Secrets. Symmetric with other tabs: state is local (`$state<Tab>`), drill-down (selected env) lives inside DeployScreen rather than ui-store, and resets on tab switch. Closes T-000079 (repo context is obvious from tabs-nav + RepoDetail header). Changes: drop `'deploy'` from the `ScreenName` union in ui-store, drop the route in `+page.svelte`, drop back-button + H2 header in `DeployScreen.svelte`, drop `openDeploy()` + 🚀 button + `.deploy-btn` styles in RepoDetail, drop dead i18n keys (`deploy.back`, `deploy.deploymentsTitle`, `repo.deployButton`), add new i18n keys (`repo.tabDeploy`, `repo.deployBlocked`) × ru/en. Empty-state if the repo has no `github_name` or `deploy_target` (with instructions to set them in the header).

## [0.25.0] — 2026-05-12

### Added
- **T-000078 | Triangular REQ-flow rules in the global CLAUDE.md template** — extends the `# Cross-repo requirements` section for the client → server → microservice case. Two new H2 sections: `## Receipt format` (4 hard-enforced status values: `implemented` / `partially` / `declined` / `clarification-needed` + rewrite workflow on the clarification-loop) and `## Forwarding (triangular flow)` (Server-side responsibility — classify/forward/wait/resume; linkage header `Forwarded-from: <client>/REQ-NNN` for chain-tracing; MS-side responsibility — ignore client identity, audience-via-body). LLM policy extended with a rule to ignore the `Forwarded-from:` header as server-side metadata. Sync flow disambiguation: a final receipt + unhappy sender writes REQ-N+1; a `clarification-needed` receipt → sender updates the original REQ inline. The old H3 `### Receipt content (convention, not enforced)` removed — clashed with the hard-enforce Rule 4. `### Sync state` promoted from H3 to H2 (lost its parent). Behavioral validation via 5 fresh-subagent scenarios on pre-impl drafts (all PASS, coverage: 4/4 rules, 4/4 status values, 4 edge cases — multi-MS, clarification-loop, audience leak, resuming across sessions). The flow doc `docs/flows/microservice-server-sync.md` got a Triangular flow section (lifecycle + Solo Dev Hub vs LLM responsibility table).
- **T-000075 | `CONTRIBUTING.md`** — pre-public-launch artifact: build prerequisites (Node v18+, Rust, MSVC Build Tools, WebView2), getting started, project layout, code style (Rust + TS/Svelte), commits (Conventional Commits), tests, PR rules (target `dev`, not `master`), AI-agent section (link to CLAUDE.md), releases (link to RELEASING.md).
- **T-000076 | `.github/FUNDING.yml`** — `custom: [boosty.to/sgonnovdm/donate]` so the Sponsor button appears in the repo header after the public flip. The TON wallet stays in README + About (FUNDING.yml does not support crypto).
- **T-000062 | README RU+EN drafts (public-launch quality)** — text-only pass: marketing-tone preamble (3 paragraphs: tagline / problem+AI-failure-mode kicker / solution), AI-bug-closure with safety net as feature #1 (4 guarantees: protected fields, auto-attempts counter, explicit user confirm, full event log), Why / Features / Tech Stack / Getting started / Development / Roadmap / Support / License, link to Russian version `README.ru.md` at the top of English. RU tagline — «личный пульт управления» (EN — "cockpit"). Screenshots split out into T-000073 (placeholders with captions in both files).
- `LICENSE` — MIT, previously only in `package.json` without a file.

### Changed
- **T-000063 | Technical identifier rebrand**: Cargo `[package].name` (`github-repo-manager` → `solo-dev-hub`) + `[lib].name` (`github_repo_manager_lib` → `solo_dev_hub_lib`) + `main.rs` call site, `tauri.conf.json` identifier (`com.user2.github-repo-manager` → `com.solodevhub.app`), `package.json` name + regenerated `package-lock.json`. DB path migration `%LOCALAPPDATA%\github-repo-manager\data.db` → `%LOCALAPPDATA%\solo-dev-hub\data.db` via copy-once on first start (idempotent, legacy stays as a recovery breadcrumb). Keyring service rename `github-repo-manager` → `solo-dev-hub` via `migrate_legacy_pat()` (read legacy → write new → delete legacy, idempotent, best-effort). Autoupdate break: v0.24.x → v0.25.x is a fresh install (new identifier = new Windows app entry); v0.25.x → v1.0.0 will work cleanly.
- **T-000061 | Display-name rebrand "GitHub Repo Manager" → "Solo Dev Hub"**: `productName` in `tauri.conf.json` + window title + Cargo description + auto-generated MD footers + i18n strings `appDefaults.syncGlobalConfirm` (ru+en) + About `githubUrl` + README title + RELEASING.md / formats / deploy_template_spec / release.yml releaseName.
- **Autoupdate endpoint** → `https://github.com/SgonnovDmGit/solo-dev-hub/releases/latest/download/latest.json`. The repo stays private until the v1.0.0 public flip — `latest.json` is not served without auth, so autoupdate is paused on v0.25.x. The pubkey is already fresh (regenerated in T-000059).
- **T-000060 | Release flow on master/dev split**: all release-cycle commits land on `dev`, release = fast-forward into `master` + tag. Hotfixes directly on `master`, then `git rebase master` on `dev`. Documented in `docs/RELEASING.md` under "Branches" + suggested git aliases.
- **CI lint**: removed the `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` env line in `.github/workflows/release.yml` (the current solo-dev-hub key has no password). Marked optional in `docs/RELEASING.md`.
- **F-000037 | Deploy: CONTAINER_NAME secret → placeholder** + UI rework. Previously CONTAINER_NAME was stored in GitHub Environment secrets — but that's not secret data, just a per-env container name. Moved to a placeholder (`extras` JSON in SQLite per deploy_env). `${{ secrets.CONTAINER_NAME }}` in `deploy.yml.tmpl` (3 places × 2 templates) → `@@CONTAINER_NAME@@`. After the next "Generate workflow files", the value is baked directly into `.github/workflows/deploy-{env}.yml` (visible in the repo — but the container name is already public via domain/labels, no risk uplift). Logical placeholder reorder in `meta.json`: WORKFLOW → IMAGE_TAG → DOMAIN → DEPLOY_BRANCH → NETWORK_NAME → CONTAINER_NAME → COMPOSE_PROJECT → COMPOSE_SERVICE → (language-specific). UI: copy ↩ button to the right of Service Label (99% of cases = container name). REQUIRED_KEYS gate (Generate button) now includes CONTAINER_NAME. Migration: legacy `CONTAINER_NAME` secret in GitHub Environment remains orphaned (harmless), user fills the placeholder in DeployDetail. Stale `CONTAINER_NAME_PROD` references in `deploy_template_spec.md` + `flows/deploy-flow.md` removed.

### Fixed
- **GitHub Environment auto-ensure on DeployDetail open** — the GH Actions linter complained `Value '<env-name>' is not valid` for a workflow with `environment: <name>` if no matching Environment object existed in `Settings → Environments`. Several cases where the GH-side env was missing: (1) legacy envs from migration v20 (`deploy_manifests` → `deploy_environments`) auto-created with `name='prod'` in DB, but `createEnvironment` PUT was never called; (2) if an env has no override-secret, the implicit chain via `createOrUpdateEnvironmentSecret` also didn't fire, env stayed DB-only. Fix: `createEnvironment(owner, repo, env.name)` is now idempotently called in `DeployDetail.load()` (on mount) — covers all entry paths (open existing / clone / fresh create). PUT is a no-op if it already exists. API errors are surfaced via warning toast (i18n `deploy.envCreateFailed`) so PAT permission issues / fine-grained PAT without "Environments: write" are diagnosable.
- **Deploy YAML build-args indent (10→12 spaces)** — `render_build_args` in `template_render.rs` joined multi-secret lines with `\n          ` (10 spaces). In the template `@@BUILD_ARGS@@` sits at 12 spaces under `build-args: |`. With >1 secret, the second and following landed at 10 spaces — becoming siblings of `build-args` rather than continuations, breaking the YAML: `APP_API_KEY: "${{ secrets.APP_API_KEY }}"` was interpreted as a separate key in the neighboring map. Pre-fix code comment was lying ("Indent = 10 spaces … matches template"). Fix: `\n            ` (12 spaces) + regression test that renders the real flutter_web template with 3 secrets and asserts column 12 for each. 296 cargo tests pass.
- **B-000005 (critical) | Deploy files were not written to the folder** — TS↔Rust API mismatch in `write_deploy_files`. TS in `DeployDetail.svelte:178` mapped `RenderedFile[]` via `(f) => ({rel_path: f.path, content: f.content})`, renaming the field `path` to a non-existent `rel_path`. The Rust struct `RenderedFile` expects `path` without rename — serde failed on missing field, the command returned Err, files were not written. Silent across prior releases (the catch showed a toast, but the user might not have seen it). Fix: pass `toWrite` directly (already a correct `RenderedFile[]`). In parallel, the `writeDeployFiles` type in `tauri-commands.ts:452` was fixed — it was an anonymous `{rel_path; content}[]` + `{written; skipped}` shape; `skipped` also does not exist (Rust returns `errors`). Now references shared `RenderedFile` and `WriteResult` from `types.ts` — guarantees a single contract between the two sides.
- **B-000004 | DeployScreen secrets refresh button** — added ↻ "Refresh from GitHub" in `DeploySecretsTable.svelte` header-row, mirroring the SecretsPanel pattern. Reuses i18n key `secrets.refresh`.
- **Deploy_secrets orphan cleanup on meta.json change**: `ensure_deploy_secrets_populated` now additionally DELETEs rows whose `secret_name` is in NEITHER current GitHub repo secrets NOR `meta.json` required_secrets. Previously the row lived in `deploy_secrets` forever — after F-000037 moved CONTAINER_NAME from secret to placeholder this left an orphan row in DeployDetail. The caller must invoke only with a successfully-fetched `repo_secret_names` (empty-due-to-failure would prune legitimate rows). +1 cargo test → 295 total.
- **B-000003 | Deleted repo secrets did not disappear from the UI until restart**: the GitHub `list secrets` endpoint has eventual consistency — a refetch immediately after DELETE may still return the deleted secret for several seconds, and the old code (`loadSecrets()` after delete) re-showed it. Fix in `SecretsPanel.svelte`: (1) optimistic update — `existingSecrets` is filtered locally immediately by `succeeded` deletes; (2) filtered refetch — after refresh, the fresh response from GitHub is additionally filtered by `deletedSet` (denylist) so stale responses cannot resurrect a deleted entry; (3) ↻ "Refresh from GitHub" button in the "Current secrets" header for manual reload in any staleness scenario.
- **B-000001 | SyncScreen showed `owner/repo` instead of `repo`**: backend `Repository::display_name()` returned the full `github_name`, while frontend `getDisplayName()` already returned the last segment — Rust↔TS asymmetry leaked through `RequirementInfo.source_repo / target_repo` (18 sites) in SyncScreen. Rewrote `display_name()` symmetric to the frontend (`gh.rsplit('/').next()`). All 18 RequirementInfo sites + sync-error logs cleared automatically. `canonical_folder_name()` stays separate — it's the SoT for filesystem folder names with a different fallback (`local-<id>`). +4 unit tests.
- **B-000002 | "Update repo docs" now covers project.md and CLAUDE.md**: the `init_docs_for_repo` command previously only touched user-ownable skeletons (`todo.md`, `bug-reports.md`, `.gitignore`); app-owned files (`project.md`, the `CLAUDE.md` section) were updated only from `sync_project`. Now it idempotently regenerates both — the button mirrors the pre-phase of Sync for one repo. For orphan repos without `project_id` the app-owned part is skipped (nothing to render the project context from).
- **B-000002 (part 2) | Silent skip in `sync_project` now reported in errors[]**: if a repo has no `local_path` or the folder is missing on disk, `sync_project` used to silently skip (project.md/CLAUDE.md/.gitignore not written, no warning in toast). Now an explicit error is pushed — the user sees the reason. Applied to both loops (own repos + microservice server-repos).
- The button "📚 Initialize docs" → "📚 Update repo docs" / "📚 Обновить документацию репозитория". "Init" implied one-time, while the button is idempotent and now overwrites app-owned files every time.

## [0.24.2] — 2026-05-07

Diagnostics + microservice reverse-lookup patch.

### Added
- **B-000018 | Microservice → reverse-lookup to parent servers** (T-000070): when opening an MS project, `list_project_requirements` additionally collects requirements and api/handlers from connected parent servers. From a microservice, you can see all requirements targeting it. New flag `RequirementInfo.is_reverse_lookup` distinguishes rows on the MS side — UI hides the ✓ button (confirm should be done by the sender = parent server from its SyncScreen) and shows ↩ hint.
- `docs/known-issues/B-000017-flicker-multi-monitor.md` — detailed investigation of B-000017: repro, root cause (WebView2 mixed-DPI multi-monitor bug in Chromium's DPI pipeline), 3 attempted fixes with reasons they didn't work, applied side-fixes, workaround, re-evaluation triggers.

### Changed
- **B-000017 | SvelteKit preload-on-hover disabled** (T-000069 side-fix): `data-sveltekit-preload-data="off"` + `data-sveltekit-preload-code="off"` in `src/app.html`. A Tauri SPA with store-based screen switching doesn't use SvelteKit nav — handlers were pure dead overhead, generating noise in the Performance trace on every pointermove (20ms timer install/remove).
- `initUiScale` defensive cleanup: strips inline `style.zoom` on `documentElement` on init in case of a leftover from a dev experiment (CSS zoom stacks on top of WebView setZoom, breaking viewport math).

### Fixed
- **T-000072 | Settings PAT card "Delete token" overflow**: at zoom ≥125% the button overflowed the card's right edge. `flex-wrap: wrap` on `.pat-row-2` + `min-width: 0` on `.pat-row-2-left` — the button gently wraps to the next line when space is tight.

### Known limitations
- **B-000017 | Subpixel flicker on a secondary monitor at zoom ≥125%** (T-000069): Chromium/WebView2 mixed-DPI multi-monitor bug. Not solvable on our side (CSS layer promotion didn't help; CSS zoom instead of setZoom breaks viewport math). **Workaround:** on the affected monitor go to Settings → Appearance → Scale → manual <125% (100/110%) or ≥150%. Full breakdown and re-evaluation triggers in `docs/known-issues/B-000017-flicker-multi-monitor.md`.

## [0.24.1] — 2026-05-07

UX patch — i18n cleanup in Dashboard/Timeline and SyncScreen polish.

### Added
- 22 new i18n keys × ru/en for timeline kinds + Dashboard hardcoded strings: `timeline.kind.{bug_event,task_event,repo_rename,sync_event,deploy_event}`, `dashboard.{deltaToPrev,bugsAbbrev,attemptsAbbrev,outOfFmt,dow.0..6}`, `common.{selectAll,clearAll}`. Previously these were hardcoded in Russian.
- Locale param in `formatRelativeTime(iso, nowMs?, locale?)` — previously ru-only. The default reads the current `$locale` via `get(locale)`; explicit param is used in reactive context (Dashboard activity feed).
- +1 vitest case for en-locale `formatRelativeTime` (40 frontend tests).

### Changed
- **B-000015 | Dashboard + Timeline i18n cleanup** (T-000066): 7 hardcoded Russian sites moved to `$tStore`. Additionally (round 2 after dogfood): finalized KPI hints where English terms (`confirmed`, `fix_attempts`, `closed/created`, `critical`) remained inside Russian phrases — replaced with Russian terminology ("закрытые", "до закрытия", "критичных").
- **Timeline + Dashboard activity event semantics**: `bug.confirmed/rejected` switched from "подтверждён/отклонён" (about the bug) to "решение принято/отклонено" (about the fix). EN side: `'fix accepted/rejected'`. More accurately reflects the workflow.
- **B-000019 | SyncScreen api.md/handlers.md dedupe** (T-000068): direction `server_to_client` created a separate `RequirementInfo` per client for shared files (api.md, handlers.md). 5 clients → 5 "api.md" rows. Frontend now aggregates by `(filename, status)` within each source group: same-status clients collapse into one row with a counter `×N` and a list of targets joined by ", ". Mixed statuses (3 sent / 2 new) remain separate rows.
- **B-000020 | SyncScreen Microservice → Server merged** (T-000071): directions `microservice_to_server_api` and `microservice_to_server_handlers` merged into one "Microservice → Server" section (symmetric with "Server → Client" after B-000019). `aggregateServerToClient` renamed to `aggregateByFilename` — a generic helper reused for both shared-file directions. Removed old i18n keys `sync.microserviceToServer{Api,Handlers}`, added unified `sync.microserviceToServer`.
- `DashboardActivityFeed` migrated from inline-relative-time (which duplicated logic from `time-format.ts` plus `$locale` switches) to shared `formatRelativeTime` + `nowTick` + `$locale` param.

### Fixed
- **B-000016 | SyncScreen scroll-jump after confirm** (T-000067): `handleConfirm` called `loadRequirements()` which recreated the entire `requirements` array → keyed `{#each}` re-keyed the whole DOM → `scrollTop` reset to 0. Now after a successful `confirmRequirement` we locally filter the array (Svelte surgically removes only the confirmed row, surrounding nodes stay stable). Backend `confirm_requirement` physically deletes the file pair, so filtering is the correct reflection of the new state without a round-trip.

### Tests
- 40/40 vitest passing (39 → 40 for the en-locale case)
- 290/290 cargo passing (no new backend tests — fixes were frontend + i18n only)
- svelte-check 444 files, 0 errors / 0 warnings

## [0.24.0] — 2026-05-04

### Added
- **F-000036 | Templates UX rework** — Settings reorganized into a 3-bucket model:
  - Card "Repo templates" with inline buttons `📋 Starter files` (`AppDefaultsScreen` for `_global` files: `.gitignore`, project CLAUDE.md section, `docs/todo.md` / `bug-reports.md` skeletons) and `📤 Deploy templates` (`TemplatesScreen` per-language).
  - New card "AI global rules" with buttons `📝 Open template` (opens GlobalClaudeEditor directly) + `⟳ Sync` (Settings level — previously Sync was hidden inside the editor), plus inline-status `Last: <time>` via the relative-time formatter.
  - New component `GlobalClaudeEditor.svelte` — dedicated editor for `claude.md.global.tmpl` (single-file, no list view), with a Sync button in the header (disabled when the editor is dirty + tooltip "Save first") + last-sync timestamp.
  - Sync timestamp persisted in the settings table key `ai_rules_last_sync_at` (RFC3339), updated only on successful sync.
- `formatRelativeTime(iso, nowMs?)` helper in `src/lib/utils/time-format.ts` with thresholds `<1min` → "just now", `<60min` → "{N} min ago", `<24h` → "{N} h ago", `≥24h` → "{N} d ago". Plus `nowTick` readable store on a 60-second interval — auto-refreshes relative-time displays without user interaction. Russian-only formatting (i18n by locale — next iteration).
- 9 new i18n keys × ru/en for cards 4–5 + AI rules editor (`templatesRepoCard`, `aiRulesCard`, `bucketRepoInit`, `bucketDeploy`, `aiRulesOpenTemplate`, `aiRulesSync`, `aiRulesSyncTooltipDirty`, `aiRulesLastSync`, `aiRulesNeverSynced`).

### Changed
- **Routing refactor** (D12): `currentScreen` Svelte store migrated from `writable<Screen>` (string union) to `writable<ScreenState>` (`{ name: ScreenName; params?: Record<string, unknown> }`). All ~21 `.set('xxx')` callers updated to `.set({ name: 'xxx' })`; all ~18 reactive reads `$currentScreen === 'xxx'` updated to `$currentScreen.name === 'xxx'`. Atomic single-commit, `npm run check` clean. Future-proofs routing for routes with params; immediately unblocks the `'global_claude_editor'` route. Helper `navigateTo(screen)` accepts string-or-ScreenState (backward-compat).
- **AppDefaultsScreen** became bucket-B-only: always excludes `claude.md.global.tmpl` from the displayed `_global` list via the `excludeFiles` prop on `TemplateEditor`. Footer Sync button removed entirely (~38 lines): button + ConfirmDialog block + `handleSyncGlobal` handler + 3 imports + 3 CSS rules. Sync moved to the Settings card and into the editor header.
- `sync_global_claude_md` Tauri command: now returns `SyncGlobalClaudeResult { path, synced_at }` (previously a plain string), writes setting `ai_rules_last_sync_at = NOW()` after a successful `sync::update_claude_md_global`. snake_case at the Tauri boundary (project convention; in TS — `result.synced_at`).
- Settings card 4 renamed "Templates" → "Repo templates", layout changed from two row-label rows to a single row with two inline buttons.

### Removed
- **T-000058 | Migration v23** — `DROP VIEW IF EXISTS bug_stats`. The legacy VIEW had not been used since v0.22.0 (T-000054 stats redesign moved per-repo StatsTable to new queries `get_repo_stats_summary` / `get_project_stats_summary`; Dashboard moved to its own queries in v0.17.0). Two minor versions of dead schema → finally clean. +1 cargo test `test_db_migration_v23_drops_bug_stats_view`, 4 existing tests updated for v23, 1 obsolete `test_bug_stats_view_from_bugs` removed. Companion cleanup of stale references in `README.md`, `docs/flows/dashboard.md`, `docs/flows/bug-tracking.md`, `docs/flows/repository-deletion.md` (the last one also got an updated cascade-list — the doc was 9 cascades behind vs the 3 mentioned).
- Orphan i18n keys `settings.templatesCard` / `templatesRepoLabel` / `templatesGlobalLabel` / `templatesOpenEditor` (replaced by new card-specific keys in Task 4).
- Unused `previousScreen` import in `src/routes/+page.svelte`.

### Fixed
- TemplateEditor `excludeFiles` prop now reactive (`$effect` correctly tracks post-mount changes via an explicit `void excludeFiles` inside the body). Previously read-once-at-mount.
- GlobalClaudeEditor `handleSave` now calls `loadContent()` after a successful `saveTemplateFile` — authoritative refresh from DB instead of optimistic-only local update; mirrors the `TemplateEditor.handleSave` pattern and protects against latent staleness if the `is_custom` flag ever surfaces in this UI.
- Settings cards 4–5 grid layout: `.row-control` without a sibling `.row-label` previously landed in the 130px label column due to grid auto-placement, squeezing the buttons. Inline `grid-column: 1 / -1;` restores full width.

### Tests
- +1 vitest test file (5 cases) for `formatRelativeTime` thresholds.
- +3 cargo tests: `test_sync_global_claude_md_sets_last_sync_at`, `test_sync_global_claude_md_does_not_set_on_failure` (Task 5), `test_db_migration_v23_drops_bug_stats_view` (T-000058).
- Cargo suite: 290 tests passing (288 baseline + 3 new − 1 obsolete = 290).

## [0.23.0] — 2026-05-04

### Added
- **Global CLAUDE.md template** enriched with two new H1 sections: **"Phase work workflow"** (trivial vs non-trivial trigger on three axes: ≥2 non-obvious decisions / cross-boundary / ≥3 sub-tasks; mechanical mass-changes carve-out; user-override clause; 5 steps chat → spec → self-review → OK → impl; project-bindings placeholder) and **"Manual-smoke verification in every spec"** (required content for a `## Verification` section in any spec). Project addendum in our CLAUDE.md (`docs/superpowers/specs/<YYYY-MM-DD>-<phase-name>.md`, subagent-driven implementation).
- **B-000009 / B-000012** Pre-release dogfood patch (see Fixed below).
- **UI scale auto + manual** (B-000009) — Settings → Appearance → "Scale": Auto (default) picks the zoom by the effective logical width of the current monitor (`monitor.size.width / scaleFactor`), recomputes on `window.onMoved` with 300ms debounce when dragging the window between monitors; presets 80% / 90% / 100% / 110% / 125% / 150% for manual override. Heuristic: ≥3500px → 1.5×, ≥2500px → 1.25×, ≥1900px → 1.1×, otherwise 1.0×. Applied via the native `getCurrentWebview().setZoom(scale)` (Tauri v2 webview API), requires permission `core:webview:allow-set-webview-zoom`. Persisted in the `settings` table under keys `ui_scale_mode` (`auto` / `manual`) and `ui_scale_manual` (number). New module `src/lib/ui-scale.ts` encapsulates stores, heuristic, init with `onMoved` listener; settings in `src/lib/stores/settings.ts` via `saveUiScaleMode` / `saveUiScaleManual`. +2 i18n keys (`uiScaleLabel`, `uiScaleAuto`) × ru/en. Round 2: split `uiScaleApplied` (what's applied) and `uiScaleAutoComputed` (what auto would compute for the current monitor regardless of mode) — the dropdown label "Auto (NN%)" now reads the second, so in Manual mode it shows the true auto value rather than the chosen manual. `onMoved` recomputes both; `applyZoom` in manual is a no-op (same manual), `autoComputed` updates for UI consistency when the window moves between monitors.
- **T-000057** About window redesign: hero becomes 2-column (logo + tagline + GitHub link), donate is prominent right after the hero (featured pink-gradient styling), new "What it does" section with 6 features in a 2-col grid (fully localized), update moves down as a minor utility, devs-row becomes a compact one-liner ("Author: Sgonnov D.A. · AI assistant: Claude (Anthropic) · License: MIT").
- **Adaptive About layout** — content stretches to the full window width with adaptive padding `clamp(32px, 4%, 80px)`; hero (logo + title) and features-grid scale via `clamp(180px, 12vw, 280px)` logo, `clamp(26px, 2vw, 36px)` title; features-grid 2-col (≤1100px) → 3-col (≥1100px) → 1-col (≤720px); donate rows aligned via CSS grid + pixel-precise text alignment between the Boosty link and the TON wallet address.
- **Vertical centering** in About via auto-margin trick on `:first-child` / `:last-child` (centers when content fits; scrolls normally without clipping when it doesn't).

### Changed
- About window: GitHub from button to text link; logo 256px → responsive `clamp(180px, 12vw, 280px)`; devs-card vertical → one-liner.
- Update error state in the About card: removed the duplicating "Try again" and "Hide" buttons under the error text — the `↻ Check` button in the card header already performs the same function.

### Removed
- i18n keys `about.developers`, `about.developersValue`, `about.githubRepo` — replaced by the new `about.devs.*` and `about.tagline` structure.
- Unused `dismissUpdateStatus` import from About.svelte (after simplifying the error state).

### Fixed
- **B-000012** Global CLAUDE.md template (`src-tauri/templates/_global/claude.md.global.tmpl`) gained an H1 section **"Feature flow docs (`docs/flows/`)"** with the key disambiguation rule current-vs-planned: present tense describes the current behavior in HEAD; planned is marked explicitly (inline `(planned)` tag, `> 🚧 Not implemented yet` block, or a separate `## Planned changes` section); past tense is forbidden in flow docs (that's changelog material). Also update-policy (the flow is updated in the same commit as the code, not in a follow-up; the `(planned)` marker is removed on implementation; file path references must point to live code), boundaries vs Changelog.md / docs/todo.md / REQ pairs / design-memo folders. As a bonus (separate from B-000012), the H1 **"Release closure checklist"** was added (9-step checklist with conditional steps: REQ receipts only if applicable, schema regen only for server projects with a DB) and the H1 **"Commit messages"** (Conventional Commits format with trailers `Refs T-NNNNNN` / `REF: REQ-NNN` / `B-NNNNNN`). The "Release workflow:" paragraph in the Versioning section is replaced by a forward link to the new checklist.
- **B-000014** When maximizing a window with a custom titlebar (`decorations: false`) on Win11, Windows extends the window ~8px past each screen edge (for the invisible resize border) → content with `height: 100vh` ran 8px below the visible area, the scrollbar and bottom border "went off screen". In `+page.svelte` we added `class:maximized={isMaximized}` to the `.app` div (the state already existed for the maximize/restore icon swap). The CSS rule `.app.maximized { box-sizing: border-box; padding: 0 8px 8px 8px }` compensates for the overhang only in maximize state. Round 2: an additional fix for horizontal scroll — `.comment-btn` in `BugItem.svelte` did not have `white-space: pre-wrap; word-break: break-word` (only `.text-btn` for description did). Long comments overflowed the content area, forcing `<main>` to show a horizontal scrollbar. Wrapping CSS is now uniform across description and comment.
- **B-000013** Stats tabs (RepoDetail / ProjectDetail) did not count `rejected` bugs as active in the "Active" / "Critical active" KPIs. In `stats_summary_for_repo` and `stats_summary_for_project` the whitelist `status IN ('created','in-progress','testing')` was replaced with `status != 'confirmed'` (as in `count_active_bugs`, which Dashboard uses — and which was correct from the start). Logic: rejected is NOT a closed state — the user didn't accept the fix and the bug went back into work. 4 queries changed in `db.rs`. +1 unit test (`test_stats_summary_includes_rejected_in_active`, 288 total). Round 2: the hint `stats.summary.kpiActiveHint` was updated from the stale "open + in-progress + testing" to the localized "all except closed" / "все кроме закрытых" — doesn't enumerate statuses, more flexible, and won't break when new workflow states are added.
- **B-000004** Tasks tab duplicated rows during ID rewrite in todo.md (T-034 and T-000034 simultaneously, or a placeholder F-NNN next to a real F-000035). `sync_tasks_for_repo` now cleans up orphan todo rows: any DB row with `source='todo'` whose task_id is missing from the current todo.md is deleted (task_events cascade via FK). After todo.md normalization and Refresh, the duplicate disappears automatically. Done-rows are append-only and untouched. +1 unit test (`test_sync_tasks_cleans_up_orphan_todo_rows`).
- **B-000011** Tasks/Done tabs stuck with previous-repo data when switching (RepoDetail does not re-mount the tab, only updates the prop — `onMount` fired once). `TasksTab` and `DoneTab` now re-read data via `$effect(() => { void repoId; load(); })` — pattern already used by `RepoChangelogTab`. Also `<DataGrid>` inside both is wrapped in `{#key repoId}` so persisted sort/filters don't leak across repos when `persistKey` changes. Other tabs were checked — all reactive correctly.
- **B-000010** The app icon in the taskbar is no longer blurry and shows the SDH crop. Root cause: Tauri/tao loads ONE icon frame for the running-window (default 256×256 = full logo), and at 200% DPI Windows downscaled 256→64 → blur + not the SDH design. Fix in `lib.rs` — `tauri::Builder::setup` callback explicitly sets the window icon from `64x64.png` (SDH crop, exact 1:1 at 200% DPI taskbar) via `Image::from_bytes(include_bytes!(...))`. Enabled the Tauri Cargo feature `image-png` for PNG decoding. The .exe file icon (RT_GROUP_ICON for Explorer) is not affected — it still has all 10 sharp frames via `embed_resource`. `icon.ico` was rebuilt from the right per-size sources: 16/20/24 — Lanczos from `32x32.png` (previously over-downsampled 4× from 64), 32/64 — exact, 40/48 — Lanczos from `64x64.png`, 96/128/256 — Lanczos from `icon.png` (512×512 full logo). The 64→96 boundary matches the Windows transition to full-size rendering at ≥175% DPI.
- **B-000007** The default WebView2 menu (Inspect / Reload / "Other tools" / "Text direction") no longer appears on right-click in release builds — neither in the main UI nor in input fields. In `+page.svelte` a global contextmenu handler guarded by `import.meta.env.PROD` suppresses the native menu everywhere; for `<input>` / `<textarea>` a new custom `InputContextMenu.svelte` is rendered — a fixed-position 4-item menu (Cut / Copy / Paste / Select All) with position-clamp in the viewport, closing on outside-click / Esc. Cut/Copy disabled when there's no selection; Cut/Paste disabled on readOnly/disabled fields. Clipboard via `navigator.clipboard` (works in WebView2 privileged context). `ctxMenu` is stored via `$state.raw` (Svelte 5's deep-proxy doesn't play well with a DOM element inside — assignment silently failed in builds). In dev the menu stays available for debugging. The Ctrl+C/V/X/A shortcuts are unaffected. New i18n keys `ctx.cut/copy/paste/selectAll`.
- **B-000008** In About, the standalone "AI assistant: Claude (Anthropic)" item was removed from the devs one-liner — it sounded like product placement. AI tooling is now mentioned inline after the author: "Author: Sgonnov D.A., with AI assistants · License: MIT". i18n: removed `about.devs.aiAssistant` + `about.devs.aiValue`, added `about.devs.aiHint`.
- **B-000006** Custom titlebar icons (minimize / maximize / close) replaced from Unicode glyphs (─ □ ✕) to clean 12×12 SVG-stroke icons. The maximize button now swaps to the restore-down icon (two overlapping frames, like the native Windows one) when the window is maximized, with the tooltip toggling between "Maximize" / "Restore". State syncs via `appWindow.onResized()` + `isMaximized()` after toggle — correctly reflects snap-resize, double-click on titlebar, and OS shortcuts. New i18n key `app.restore`.
- **B-000005** Sorting by priority and status in the Tasks tab is now by workflow weight, not alphabetical: priority `critical → high → medium → low`, status `open → in-progress → review`. DataGrid `ColumnDef` got optional `sortWeight: Record<string,number>` (workflow-order weight) and `labelMap: Record<string,string>` (localized cell label + filter dropdown + chips, raw value for match logic unchanged — values outside the format are passed through unchanged and grouped at the end of the sort). Labels translated: critical/high/medium/low, open/in-progress/in-review. `TasksTab.columns` rewritten from `const` to `$derived` — labels rebuild on locale change (previously captured once at mount).

## [0.22.0] — 2026-04-28

### Added
- **T-000054** Stats tab redesign: new `StatsSummary.svelte` with lifetime-only KPI(4: Active / Closed total / Avg attempts / Fix rate) + (project-only) 🔥 Top-3 hot repos within project + Category efficiency bars (sort by % closed DESC). Lifetime banner shows the scope creation date and days_history.
- 2 new Tauri commands: `get_repo_stats_summary`, `get_project_stats_summary`.
- 3 new DB queries: `top_hot_repos_in_project`, `stats_summary_for_repo`, `stats_summary_for_project`.
- 4 new DTOs: `StatsSummary`, `StatsKpi`, `CategoryBar`, `HotRepo`.
- 10 new unit tests (top hot ordering / confirmed excluded / zero-active filter, basic stats / empty / categories sorted / avg+median, project aggregate + top hot / empty / repos no bugs).
- 30 new i18n keys `stats.summary.*` × ru + en.
- **T-000056** Recent Activity Feed: compact 10-event timeline embedded in Stats tabs of both RepoDetail and ProjectDetail. Per-day grouping, deep-link "All events →" navigates to the top-level Timeline screen with pre-filled scope filter. Backend reuses existing `read_timeline` (no new queries / DTOs / migrations).

### Changed
- Stats tab in `RepoDetail` and `ProjectDetail` renders the new `<StatsSummary>` instead of the old `<StatsTable>` pivot (severity × category × date).

### Removed
- `StatsTable.svelte` component.
- Tauri commands: `get_repo_stats`, `get_project_stats`, `get_global_stats`, `get_all_stats`. DB methods and `BugStatRow` DTO/interface — removed alongside.
- 3 legacy db.rs tests, replaced by the new `stats_summary_*` tests.
- VIEW `bug_stats` stays in the schema as dead code (no migration). Cleanup deferred to v0.23.0 if needed.

## [0.21.1] — 2026-04-27

### Fixed
- **B-000002** Timeline filter by repo showed `owner/repo` instead of the short name; now `getDisplayName(r)` (last segment github_name or description), matching Sidebar and the graph.
- **B-000003** Double arrow `← ←` on the "Back" button in AppDefaultsScreen — the template had a hardcoded `← {settings.back}`, while the i18n value `'settings.back'` already contained `← Back`. Removed the hardcoded prefix; the arrow comes from i18n only.
- **BugNotes UX:** after a ✓ click, a confirmed bug disappeared from the list immediately (re-fetch with a non-confirmed filter). Now optimistic local mutation — the row stays visible with confirmed styling (gray background, ✓ marker, disabled controls) until manual Refresh. The user sees "click registered" feedback instead of an instant disappearance.
- **Bug LLM-acknowledgement workflow restored** — confirmed bugs remain in `docs/bug-reports.md` after the ✓ click in the app, so the LLM in the next session sees the confirmation and removes the row as cleanup. Previously (v0.16.0..v0.21.0) `regenerate_bugs_md` filtered confirmed on the write side, and the LLM never saw the confirmation. Now:
  - App sets `status='confirmed'` + `confirmed_at`, MD is regenerated with the row visible
  - LLM on the next session edit removes the confirmed row (per global spec)
  - `reconcile_bugs_for_repo` detects the removal and sets `archived_from_md_at = NOW`
  - Subsequent regens permanently exclude the row (DB row stays for history)

### Added
- **DB schema migration v22** — new column `bugs.archived_from_md_at TEXT` (NULL by default). LLM-acknowledgement marker.
- **`db.list_bugs_for_md(repo_id)`** — returns active rows + non-archived confirmed rows for MD regeneration.
- **`db.mark_bug_archived_from_md(bug_id)`** — idempotent helper (does not overwrite an existing timestamp).
- **2 new Rust tests** (277 → 279):
  - `test_regenerate_bugs_md_excludes_archived_confirmed`
  - `test_reconcile_marks_confirmed_archived_when_llm_removes_from_md`
- Test `test_regenerate_bugs_md_includes_only_non_confirmed` renamed → `_includes_unacknowledged_confirmed` with updated assertions.

### Notes
- Existing confirmed bugs in legacy DBs are treated as `archived_from_md_at = NOW` on import via `migrate_bugs_for_repo` — preserves the UX expectation "confirmed-from-MD-import → drops from MD". Only fresh confirmations via v0.21.1+ go through the new workflow.
- DB-side bug history (the "Show closed" toggle in BugNotes) is unchanged — reads from DB directly via `count_confirmed_bugs` / `read_bugs_from_db`, sees ALL confirmed independently of MD state.

## [0.21.0] — 2026-04-27

### Added
- **F-000013 Project graph** — new "Graph" tab in ProjectDetail with an interactive project map (cytoscape.js): server in the center, clients + microservices around. Pan/zoom (Ctrl+scroll) + click → navigate to RepoDetail / another ProjectDetail. Theme switching via CSS vars + `cy.style().update()`. Concentric layout, role coloring (server=blue, client/landing=green, ms=purple, tool=gray), dashed edges for cross-project ms.
- **T-000055 Settings UX redesign** — Settings reorganized into 4 thematic cards (GitHub PAT / Appearance / Workspace / Templates), compact rows instead of one card per setting. ~50% vertical savings. PAT tooltip "Token needed to sync repositories".
- **T-000050 Local-only repo rename detection** — new function `update_repo_description` + Tauri command, hook on description change logs the event into `repo_renames` (only for local-only repos where canonical = description). UI: click-to-edit name in RepoDetail + inline list "↳ previously: <old> (date)" under the title.
- **ProjectDetail tabs** — reorganized into 4 tabs (Repositories / Microservices / Graph / Stats) following the RepoDetail pattern; the header gained [✏ edit] + [⌫ delete] action buttons in the top-right.
- Tauri command `list_renames_for_repo` (per-repo rename history fetch).

### Changed
- ProjectDetail header: the delete button moved from the bottom of the page to the top-right corner (pattern from RepoDetail).
- StatsTable in the ProjectDetail tab is no longer collapsible — once it's in a tab, it's always expanded.
- In the tab header the textual section label was removed, leaving only the counter badge `(N)` — the title is already reflected in the active tab button.

### Removed
- `SettingsRenameLog.svelte` component — moved to RepoDetail header as an inline list (per-repo).
- Settings "History" card — nothing to configure; history is visible per-repo.
- **Project-level Secrets tab** — secrets are set per-repo in RepoDetail; duplicating at the project level is redundant. The SecretsPanel mode="project" path is no longer used in ProjectDetail.

### Fixed
- ProjectGraph rendered into a 0×0 container (the parent `.project-detail` uses block+overflow-y, not flex). Added explicit `min-height: 600px` + `height: calc(100vh - 280px)` to `.graph-wrapper`.
- Node labels in the graph showed the full github_name (`owner/repo`) instead of the short name. Backend now uses `canonical_folder_name()` (last segment github_name or description) — matches the frontend `getDisplayName`.
- Columns in the repositories table inside ProjectDetail were hardcoded English (`Repo` / `Lang`); now localized via `project.colRepo` / `project.colLang`.

### Notes
- Bundle size impact: cytoscape.js adds ~200KB to the app bundle. Acceptable for a desktop app.
- Test count: 270 → 277 (+3 for T-050 + 4 for F-013).
- No migration required — all changes are SoT/schema-compatible with existing DBs.

## [0.20.2] — 2026-04-26

### Fixed
- DataGrid: long task/done descriptions no longer stretch into one super-wide row — wrapped to 3 lines with ellipsis, variable row height (`white-space: normal` + `-webkit-line-clamp: 3`).
- Grid search now covers not only description but also the task ID (any monospace column). Previously a search for `T-000042` returned nothing.

### Changed
- The filter button in grid column headers is more visible: `▾` instead of `⚙`, a border on hover, accent border + a counter badge when the filter is active. Discoverability improved — previously the 10px icon blended into the text.

## [0.20.1] — 2026-04-26

### Changed
- Task ID format unified: `T-NNN` / `F-NNN` / `D-NNN` → `T-NNNNNN` / `F-NNNNNN` / `D-NNNNNN` (6-digit zero-padded, matching bugs since v0.16.0). The LLM writes new tasks in 6-digit form; the parser reads both (legacy 3-digit + new 6-digit) — backwards-compatible.
- `parse_done_tasks` synthetic counter `D-{:03}` → `D-{:06}` (e.g., for rows with an empty id slot).
- Global CLAUDE.md template (`claude.md.global.tmpl`) updated: the format spec for todo.md / done.md / bug-reports.md now explicitly states the 6-digit format + parser leniency. Users need to click "Sync global CLAUDE.md" in Settings — the updated spec is pushed into `~/.claude/CLAUDE.md`.

### Notes
- Existing legacy IDs in todo.md / done.md of existing repos are NOT rewritten — the parser keeps reading them. On write (a new task via the LLM) the 6-digit form is used. A gradual transition without forced file migration.

## [0.20.0] — 2026-04-26

### Added
- Universal `<DataGrid />` Svelte component for filter/sort/search/persist (text + select filter types).
- Separate **Tasks** and **Done** tabs in RepoDetail (replacing the single Tasks tab) with the new DataGrid.
- New top-level **Timeline** screen (📅 in the titlebar): multi-source timeline with date-range / event-kinds / repos / search filters, per-day grouping, pagination.
- todo.md format: 6th field `created_at` (`YYYY-MM-DD`), legacy 5-field is parsed, mtime-backfill on first sync.
- DB mirror of tasks via migration v21 (4 new tables: `tasks`, `task_events`, `sync_events`, `deploy_events`; mirrors the v0.16.0 bugs SoT pattern).
- Event recording hooks: sync_project, write_deploy_files, secret push/delete (via TS-side `record_*_event` thin commands).
- 14 new Rust tests (251 → 264), new i18n keys.

### Changed
- DashboardActivityFeed picked up sync/deploy/task event types via the extended recent_activity (now delegates to `read_timeline_filtered`).
- `ActivityEvent.repo_id` is now Optional (portfolio-wide `sync_events` don't have a specific repo).
- `write_deploy_files` command accepts `deploy_env_id` + `repo_id` for event recording.
- `delete_repository` now cleans up `tasks_grid_state_<id>` and `done_grid_state_<id>` settings keys.

### Removed
- Component `RepoDocsTab.svelte` — replaced by TasksTab + DoneTab (-306 lines).

### Notes
- After installing v0.20.0 it's recommended to click "Sync global CLAUDE.md" in Settings — it pushes the updated todo.md format spec (6th field) into `~/.claude/CLAUDE.md`. The LLM will read the update on the next session.
- Timeline starts filling with events as work happens; a full historical backfill for the pre-v0.20.0 period is not performed (only rename log + bug events since v0.17.0).

## [0.19.0] — 2026-04-26

### Added
- Mini activity feed in Dashboard: last 10 portfolio events (`bug_events` + `repo_renames` via UNION ALL), click → repo-detail.
- Sidebar collapsible (VS Code-style): 52px initials strip colored by project type (standard gray, microservice blue), click on the icon → expand + select project + scroll.
- Sidebar drag-resize: handle on the right border, width 200..500px, snap to collapsed when dragged past 160px threshold, rAF-throttled live preview.
- Persist sidebar layout (`sidebar_width`, `sidebar_collapsed` in settings, 300ms debounced).
- 5 new Rust tests for `recent_activity` (235 → 240).

### Changed
- App default screen at startup: Dashboard (instead of RepoList with unassigned repos).
- RepoDetail back-button text: "Back to Dashboard" (was "Back to repositories").

### Removed
- Component `RepoList.svelte` (-263 lines) and route `'repo-list'` (duplicated Sidebar drag-drop).
- i18n block `repoList.*` (46 keys in ru+en).

## [0.18.0] — 2026-04-25

### Added
- **Multi-environment deploy (T-044):** a single repo can have multiple parallel deploys (prod/test/staging/any name). New table `deploy_environments`, DeployScreen reworked into master-detail (deploys table + drill-down).
- **Clone deployment:** when creating a new deploy you can pick "Copy from: ..." — placeholders + `deploy_secrets` flags are copied (env-scoped secret values are not copied — the GitHub API doesn't expose values).
- **meta.json v4 (T-046):** `role: build/deploy/runtime` + `scope: repo/environment` per required_secret. The generator routes secrets by role: build → `docker build --build-arg`, runtime → `docker run --env`, deploy → workflow context.
- **GitHub Environments integration:** env-scoped secrets are written into GitHub Environments natively. The workflow uses `environment: @@ENV_NAME@@` on jobs — GitHub resolves `${{ secrets.NAME }}` with env-scoped override if present, otherwise inherits from repo level.
- Placeholders `NETWORK_NAME`, `COMPOSE_PROJECT` in the go + flutter_web templates — remove hardcoded `goapp01_prod_proxy-network` / `goapp01_prod`.
- Sync trigger in SecretsPanel: after a successful PUT of a new repo secret the app registers it in `deploy_secrets` for all of the repo's deploys.
- Cascade cleanup when removing a repo secret: its `deploy_secrets` rows in all deploys are removed too.

### Changed
- `deploy_manifests` table renamed to `deploy_environments` (migration v20). Existing manifests migrate as `name='prod'`.
- `CONTAINER_NAME_PROD` → `CONTAINER_NAME` in templates (container name is now a per-env secret).
- The flutter_web Dockerfile uses `@@DOCKERFILE_ARGS@@` + `@@DART_DEFINES@@` instead of hardcoded `ARG API_BASE_URL`/`ARG APP_API_KEY`. Build-args = UNION of all `role=build` secrets across the repo's deploys.
- Go deploy.yml emits `--env KEY=...` for each runtime secret; `ENV_FILE_PATH` remains as an escape hatch for bulk variables.
- flutter_web meta.json target `dockerfile.tmpl` → `Dockerfile` (was lowercase `dockerfile`, broken on Linux CI).
- Bundled meta.json recommends `role` + `scope` as hints — the user is free to override in the UI.

### Removed
- `DeployManifest` struct + `get_deploy_manifest` / `save_deploy_manifest` / `render_deploy_files` Tauri commands. Replaced by `list_deploy_environments` / `get_deploy_environment` / `create_deploy_environment` / `clone_deploy_environment` / `update_deploy_environment` / `delete_deploy_environment` / `render_deploy_files_for_env`.
- The rename-deployment flow — `name` becomes read-only after creation. To rename: clone with a new name + delete the old one.
- Hardcoded `build-args: API_BASE_URL=...` + pre-build ".env from secrets" step from flutter_web deploy.yml.
- Add/remove secret from the deploy itself: adding new secrets and physically removing them from `deploy_secrets` happens via the **repo Secrets tab** (sync trigger propagates to all deploys). The deploy only toggles Include/Override flags.

### Fixed (post-dogfood polish)
- DeploySecretsTable race condition: the child component owns the entire pipeline (list repo secrets → ensure_populated → list deploy_secrets → list env-scoped). Previously the parent seeded in parallel with the child list → the table was empty on first open.
- Toggling `Include` / `Override` no longer resets scroll — optimistic local update in `dbSecrets` without `await load()`.
- `common.cancel` / `common.loading` i18n keys added — the UI previously showed literal `common.cancel` on the cancel button.

### UI polish (post-dogfood)
- DeployScreen master: ⎘ clone icon in the **first** column (always visible, not on hover). Clicking ⎘ goes straight to the clone flow without a dropdown — source from the selected row.
- New-deployment form simplified: only the name field (branch is chosen on the detail screen via the GitHub branches dropdown).
- DeploySecretsTable single-line layout: name → role-chip → Override → input (flex) → Include. The input field is always visible (disabled until override is enabled).
- Role represented as a clickable colored chip: `BUILD` (indigo) → `DEPLOY` (teal) → `RUNTIME` (amber). Tooltip with explanation. Click cycles.
- Inline label/input layout with tooltip description for placeholders (instead of 3-row label/input/desc).
- GitHub branches dropdown for DEPLOY_BRANCH via `<datalist>` — autocomplete + free-text fallback.
- "Generate workflow files" pinned to the right for visual hierarchy.

## [0.17.0] — 2026-04-24

### Added
- Dashboard redesign: period filter (Week/Month/Quarter/Custom), multi-select projects filter, 5 KPI tiles with partial-same-length period comparison, Top-3 hot projects, per-day charts (bugs opened/closed + tasks done), category efficiency bars with 9 correct categories from DB.
- Migration v19: `bug_events` log table with 3 indexes + `idx_bugs_confirmed_at`. Back-fills synthetic `entered_testing` events for pre-v19 bugs, preserving `COUNT(entered_testing) == fix_attempts` invariant.
- `bug_events` recorded on every status transition (in lib.rs commands + sync.rs reconcile).
- `BugCategory` enum as single source of truth in Rust; TS mirror in `types.ts`.
- Dashboard date math helpers (`resolvePeriod` / `resolveComparePeriod`) with 12 unit tests covering Mon-start week, Q1→Q4-prev-year rollover, end-of-month clamp, d<1 edge.

### Changed
- Dashboard.svelte split into 5 sub-components (DashboardFilters, DashboardKpi, DashboardDailyChart, DashboardCategoryBars, DashboardTopHot).
- `BugCategory` TS type cleaned up — removed stale `backend`/`network`/`unknown`, aligned with 9 DB-valid categories.
- `bug_stats` VIEW stays but is used only by per-repo StatsTable. Dashboard uses its own DB queries (no incremental drift possible).

### Removed
- Three legacy dashboard tables (By Category / By Severity / By Status) with raw `bugs/attempts` cells.

## [0.16.0] — 2026-04-24

### Added
- **Bug architecture rework (T-025 / T-026 / T-027)** — SQLite became the source-of-truth for bugs; the MD file `docs/bug-reports.md` remains an LLM-facing view with 2-way sync. New `bugs` table (12 fields: `id`, `repository_id`, `numeric_id`, `display_id`, `created_at`, `description`, `severity`, `category`, `status`, `fix_attempts`, `comment`, `confirmed_at`) + 3 indexes (`idx_bugs_repo`, `idx_bugs_status`, `idx_bugs_repo_date`). DB migration v18.
- **History of closed bugs** — confirmed rows physically remain in DB, only drop out of the MD view. Available via the "Show closed (N)" toggle in BugNotes (gray background + ✓ prefix + closure date). Previously the LLM swept confirmed rows out of MD on the next edit — history was lost.
- **6-digit ID format `B-NNNNNN`** — switched from 3-digit (`B-001`, cap 999) to 6-digit (`B-000001`, cap 999999). The parser stays lenient to any length (`\d+`) — existing MD with `B-042` migrates seamlessly, numeric-id preserved (42 → display `B-000042`).
- **Lazy MD→DB migration** — on first open of the bug tab of a repo in v0.16.0, the contents of `docs/bug-reports.md` are automatically imported into DB (idempotent, subsequent opens — no-op). Pre-check for duplicate ID, transactional INSERT, marker `repositories.bugs_migrated_at`. Toast "Imported N bugs, M into archive".
- **MD ↔ DB reconciliation** — on bug tab open / Refresh / global Sync `reconcile_bugs_for_repo` is called: LLM edits of `status`/`comment` in MD are ingested into DB; protected fields (description/severity/category/fix_attempts/date) are silently restored via regen; orphan rows and illegally deleted rows are silent-remove/silent-restore.
- **New Tauri commands** — `ensure_bugs_migrated`, `reconcile_bugs_for_repo`, `read_bugs_from_db`, `count_confirmed_bugs`, `create_bug`, `update_bug_fields`, `delete_bug`, `resolve_bug`, `reject_bug`. DTO `BugView` (9 fields with `confirmed_at`) separate from the MD format `FileBugNote` (8 fields).
- **33 new Rust tests** (+18 in `sync::tests`, +15 in `db::tests`): migration idempotency, preserve numeric_id, duplicate-id abort, status transitions increment attempts, protected-field restore, orphan removal, deleted-row restore, invalid transition ignored, VIEW correctness on CRUD, per-repo counter independence. Total 183 cargo tests.

### Changed
- **`bug_stats` table → VIEW** — the incremental table with drift-prone write handlers (~150 lines in `db.rs` + `lib.rs`) is removed. Stats are now live-computed from the `bugs` table (`CREATE VIEW bug_stats AS SELECT ... GROUP BY repo, severity, category, date`). Dashboard/StatsTable SQL queries work unchanged — SQLite treats `SELECT FROM bug_stats` transparently. Drift by construction is impossible.
- **Stats recalculate button removed** from Dashboard — the VIEW is always live, manual recompute lost meaning. The same button removed from RepoDetail stats tab.
- **bug_notes table removed** from SQLite (legacy since v1, unused since bugs moved to MD in v4).
- **Delete bug UI** — now only available for `status='created'` (accidental-creation escape hatch). For actually worked bugs the path is via ✓ confirm in testing state (soft archive preserving history).
- **BugItem confirmed styling** — rows with `status='confirmed'` are visually distinct: semi-transparent background, ✓ marker instead of ✓ button, selects/edit disabled, `confirmed_at` date shown in green on the right.
- **Manual fix_attempts -/+ buttons removed** from BugItem — the counter is now fully app-managed, incremented only on a valid transition to `testing`. Manual edit no longer makes sense (will be overwritten by regen).
- **Status badge in BugItem** — a colored pill (created=gray / in-progress=blue / testing=orange / rejected=red / confirmed=green) after the date. Previously status was visible only indirectly via the presence of ✓/✗ buttons; now it reads at a glance. i18n via the `status.*` keys (already existed).
- **onMount reconcile in BugNotes** — on every remount of the component (opening bug-tab after switching from Tasks/Changelog/Stats) `refreshBugs` is called. Previously reconcile triggered only on selectedRepoId change — LLM edits in MD during work on another tab stayed invisible until an explicit Refresh.
- **LLM shortcut transitions allowed** in `valid_transition`: `created → testing` (quick fix without an in-progress marker) and `rejected → testing` (retry after rejection). Both go to `testing` → `fix_attempts +1`. Makes the natural LLM-session flow (created → taken → straight to testing) work without a mandatory intermediate refresh. Invalid transitions (e.g. `created → confirmed`, `confirmed → *`) remain forbidden.
- **i18n +13 keys** in `bugs.*` and `bugItem.*`: showConfirmed, showConfirmedHint, confirmedAt, migrationToast, migrationError, duplicateIdError, confirmTooltip, rejectTooltip, confirmedBadge, attemptsLabel, attemptsTooltip, rejectConfirmTitle, rejectConfirmMessage, addComment, commentPlaceholder (all ru + en).

### Removed
- **8 incremental bug_stats functions** from `db.rs`: increment_bug_stat, decrement_bug_stat, add_attempts_stat, subtract_attempts_stat, transfer_bug_stat, increment_resolved_stat, reset_repo_stats, reset_all_stats. Their write commands in `lib.rs` left as Ok-stubs for backward-compat (frontend will clean up in subsequent patches).
- **Old MD-centric store functions** from `src/lib/stores/bugs.ts`: `loadBugsFromFile`, `flushBugs`, `reloadBugs`, `setStatus`, `incrementAttempts`, `decrementAttempts`. New API: `loadBugsForRepo`, `refreshBugs`, `toggleShowConfirmed`, `rejectBugWithComment`.

### Fixed
- **Stats drift** — `attempts_count` in Dashboard could be off due to incremental UPDATEs which didn't survive manual MD edits outside the app, LLM edits, migrations, and handler bugs. Now the VIEW is recomputed live from the `bugs` table — the truth, the whole truth, and nothing but the truth.

---

## Pre-v0.16.0 — historical

Compact one-liners. For detailed entries see [Changelog.ru.md](Changelog.ru.md).

## [0.15.4] — 2026-04-23
- Added Donate section in About — Boosty (RUB / Russian cards / SBP) + TON wallet with copy-to-clipboard.
- Removed "Technical details" block from the About error state — cleaner UI; raw plugin err goes to `console.warn`.

## [0.15.3] — 2026-04-23
- Test release to verify the new autoupdate UX from an installed 0.15.2: green CTA button in the titlebar, install button promoted above release notes in About, silent-check without 24h cache.

## [0.15.2] — 2026-04-23
- Update CTA button in the titlebar (green `⬇ Update X.Y.Z`) when an update is available — replaces the inconspicuous red dot.
- About update card: install button right after the heading, release notes below.
- Silent-check on every cold start (no 24h cache).

## [0.15.1] — 2026-04-23
- Updater error UX — raw plugin messages replaced with friendly texts (notFound / network / signature / unknown classifications), technical detail in a collapsible `<details>` block.
- Public-launch (T-037 / T-045) returned to v1.0.0 (creating a new public repo from scratch, not renaming the private one). T-051 Windows code signing stays in v2.0.0 as post-launch polish.

## [0.15.0] — 2026-04-23
- **T-038 / F-018 Autoupdate** — `tauri-plugin-updater` + GitHub Actions release pipeline. App silently checks `latest.json` daily; About section "Updates" with Check / Progress / Install-and-relaunch in one click. Red dot badge on the "About" button in the titlebar when an update is available.
- Ed25519 signing via Tauri updater (private key in GitHub Actions secret, public in `tauri.conf.json`). Windows Authenticode intentionally out of scope — **T-051** in v2.0.0.
- Release workflow (`.github/workflows/release.yml`) — triggered by `v*` tag push → `tauri-apps/tauri-action@v0` builds on `windows-latest`, signs, generates `latest.json`, publishes GitHub Release. Release notes auto-extracted from `## [X.Y.Z]` via `scripts/extract-changelog.mjs`.
- Tags like `v*-rc*` / `v*-beta*` / `v*-alpha*` auto-marked as prerelease.
- Dependencies: +`tauri-plugin-updater 2`, +`tauri-plugin-process 2`, +`@tauri-apps/plugin-updater`, +`@tauri-apps/plugin-process`.
- 17 new i18n keys × ru/en (`about.update.*`).
- Release process — local `npm run tauri build` with manual `.exe` distribution replaced by `git tag vX.Y.Z && git push origin vX.Y.Z` → CI build + publication. Runbook in `docs/RELEASING.md`.
- Migration notes: 0.14.0 → 0.15.0 is a one-time manual install (no updater plugin in 0.14.0). Autoupdate endpoint returns 404 on a private repo until v1.0.0 public flip.

## [0.14.0] — 2026-04-23
- **F-033 Cross-repo sync folder naming + rename-log** — sync directories now named by canonical repo name (last segment of `github_name`), unified for client/microservice. `server-requirements/` on the microservice side now **nested per parent-server-repo** — removes collisions for multi-parent microservices.
- Rename log in DB (table `repo_renames`, migration v16): rename detection in `upsert_repository_with_outcome` via github_id match + different github_name. On the next sync the app renames counterparty-side folders on the filesystem. Idempotent — no "applied" state field, filesystem is state.
- Sync-preamble replay across 3 directions (client→server, server→ms on both sides).
- One-time migration for existing installations (Case A/B/C inside sync, idempotent): A — no-op for already-correct client folders; B — rename `microservice-requirements/<project-name>/` → `<ms-canonical>/` on server side; C — flat `server-requirements/*.md` on MS side → nested `<parent-canonical>/*.md` with content-based attribution.
- UI rename-log viewer — `SettingsRenameLog.svelte`, expandable `<details>` in Settings.
- Tauri command `list_rename_history` + frontend wrapper `listRepoRenames`.
- 14 new tests in sync.rs + db.rs (159/159).
- **T-049 RepoDocsTab refresh + reverse done** — 🔄 button in section header, ontoggle re-read on section expand. Done sorted reverse (newest on top).
- **T-047 Settings UI compaction** — Language + Theme combined into a single `.card-row` (horizontal flex).
- Fixed default `bug_file_path` setting (was stale `'docs/bug_list.md'`); now hardcoded `'docs/bug-reports.md'` (T-048).
- Removed: `bug_file_path` setting (path fixed in template contract, migration v17 clears existing entries), `copy_if_missing` helper + 3 tests, `Repository::github_name_or_empty()` helper.

## [0.13.27] — 2026-04-22
- **Cross-repo sync: edits propagate** — `REQ-*.md` / `*.response.md` now sync via `copy_file_if_changed` (was `copy_if_missing`). Sender/recipient edits propagate to the other side.
- **Reject requirement flow removed** — ✗ button and `reject_requirement` command. If sender is unhappy → create a new `REQ-N+1_<slug>.md` with refined ask. Reject carried no data trail.

## [0.13.26] — 2026-04-22
- **Global CLAUDE.md template**: added section `# Cross-repo requirements (Message/Receipt pattern)` between `# File formats` and `# Versioning`. Formalizes what files, what folders, sync flow 1-6, who can edit/delete what.

## [0.13.25] — 2026-04-22
- **B-006 round 6 (confirmed)** — NPM precheck now calls `GET /api/users/me?expand=permissions` (without expand, `permissions` field is missing → precheck always failed for non-admin users). Confirmed via deploy on swan_info_test_app.

## [0.13.24] — 2026-04-22
- **B-006 round 5 (root cause + preventive check)** — "already in use" issue on nginx step was NPM-account-scope: `visibility='user'` users see only their proxy-hosts via API, but POST validation checks the whole DB. Added fail-fast precheck in nginx-step with clear instructions which NPM UI checkbox to tick.

## [0.13.23] — 2026-04-22
- **Flutter build fail: "No file or variants found for asset: .env"** — restored the `.env from secrets` step in deploy.yml (some projects use `flutter_dotenv` which reads `.env` from assets at runtime). Both patterns (`--dart-define` + `.env` file) now coexist.

## [0.13.22] — 2026-04-21
- **B-006 round 3:** aggressive cleanup fallback now scans all 3 host types (proxy-hosts / redirection-hosts / dead-hosts), not only proxy. Diagnostic dump of all domain_names across all hosts.

## [0.13.21] — 2026-04-21
- **B-006 round 2:** case-insensitive domain match + POST fallback that re-scans and DELETEs conflicting proxy-hosts on `400 "already in use"`, retries POST once.
- Template comments translated to English (Russian → English uniformity).

## [0.13.20] — 2026-04-21
- **B-006:** "needs update" branch now DELETE existing proxy-host + POST fresh instead of PUT (PUT with partial payload didn't reliably regenerate nginx config). Brief downtime (~5-10s) acceptable on first-deploy / broken-state.

## [0.13.19] — 2026-04-21
- **Flutter web template: build-args vs runtime-env collision** — Flutter web is a static build; env vars don't survive into runtime. Now baked in via `--dart-define` at `flutter build`. Dockerfile: `ARG API_BASE_URL` + `ARG APP_API_KEY`, base image `flutter:stable`.

## [0.13.18] — 2026-04-21
- **Template review round 5 (final calibration)** — clarifications on todo per-prefix id counter, D-NNN scope (app UI only), release workflow per-project file list, historical entries imperative voice.
- `docs/api.md` section moved out of global template (kept as opt-in addon).
- Template now 145 lines (was 185) — clean universal contract.

## [0.13.17] — 2026-04-21
- **Template review round 4** — bug-reports LLM policy reworked (3 allowed ops + delete priority over edit-two-fields), confirmed cleanup timing, MAJOR/MINOR bump asks dev, git tag/push by dev, `## [Unreleased]` has no date.
- `docs/benchmarks.md` moved out of global template (Go-specific, opt-in addon).

## [0.13.16] — 2026-04-21
- **N2 redone** — removed contradictory "LLM must write only values from this set" for `category` (clashed with edit-only-status/comment policy).

## [0.13.15] — 2026-04-21
- **Template review round 3 — finish** — Changelog `Deprecated` + `Security` categories added (full Keep-a-Changelog set: 6 types). Bug category clarification on LLM enum + app normalization safety-net.

## [0.13.14] — 2026-04-21
- **Template review round 3** — Todo id mandatory, effort fallback `0`, done empty-id slot example, done ordering rule, bug `created → in-progress` LLM-triggered, fix_attempts bump clarity, confirmed cleanup timing, API admin-matrix footnote, escape rules tightened, SemVer dev/LLM unified, Benchmarks per-project, Benchmarks degradation thresholds.

## [0.13.13] — 2026-04-21
- **Template review round 2** — Bug workflow synced across 2 source-of-truth files, `confirmed` workflow clarified, LLM doesn't create bug rows, `created` app-assigned, todo `review` status added, todo id LLM-assigned, done policy rewritten.
- `parse_done_tasks` rejects 2-field lines + new tests.

## [0.13.12] — 2026-04-21
- **Global CLAUDE.md template review** — severity `trivial` removed (UI had 4 grades), `VB-NNN` removed (not generated), `confirmed` removed from enum-values (terminal state), `fix_attempts` wording corrected ("incremented when status enters `testing`"), done format unified, escape rules clarified, todo/done LLM+user policy unified, api.md `in-progress` status added.
- `priorityClass` updated for new todo priority enum.
- `addBug` default category `unknown` → `other`, file loader normalizes unknown categories to `other`.

## [0.13.11] — 2026-04-21
- **Global CLAUDE.md template extended** with 4 new contracts: Versioning (SemVer), Changelog.md (Keep a Changelog), `docs/api.md` (server only), `docs/benchmarks.md` (Go server only). All English (universal contract).

## [0.13.10] — 2026-04-21
- **CLAUDE.md template split into two**: project-context (`claude.md.section.tmpl`) goes into per-repo `<repo>/CLAUDE.md`; rules (`claude.md.global.tmpl`) go into `~/.claude/CLAUDE.md` via the new "Sync global CLAUDE.md" button.
- Rust function `sync::update_claude_md_global` for writing the global block.

## [0.13.9] — 2026-04-21
- **`docs/done.md` v2 format** — 4 fields → 3 fields (`id | desc | version`) + date from `## YYYY-MM-DD` section header. `[x]` checkbox removed. Parser tolerant to legacy 4-field with `[x]`.
- F-028 Legacy v1 bug parser removed (user finished migration via "⬇ v2" button in 0.13.8).

## [0.13.8] — 2026-04-20
- **F-030 Changelog tab** in RepoDetail — 5th tab between Tasks and Secrets, renders `Changelog.md` as preformatted text.
- **"⬇ v2" button in BugNotes** — force-rewrite `bug-reports.md` in v2 format (migration tool for T-041).
- `docs/formats/REVIEW-2026-04-20.md` — unified source-of-truth review for todo/done/bug-reports formats.

## [0.13.7] — 2026-04-20
- **B-002 root cause: `each_key_duplicate`** — done.md rows under the same version+date had composite `t.id + '|' + t.date` collisions, Svelte 5 in production-build threw, render stuck on previous DOM. Fix: index-based `{#each ... as t, i (i)}`.
- 10s timeout kept as safety-net.

## [0.13.6] — 2026-04-20
- **Diagnostic (B-002 retry#4)** — 10s timeout on `readRepoTodo` / `readRepoDone` + console.log on each stage. Regression tests against real done.md + todo.md (`include_str!`).

## [0.13.5] — 2026-04-20
- **B-002 (3rd retry)** Done still showed "Loading..." — split shared `loading` into `todoLoading` / `doneLoading`, loadTodo/loadDone run independently.
- Section counters simplified (total count instead of English-only `status === 'open'` filter).

## [0.13.4] — 2026-04-20
- **B-002 (2nd retry)** Svelte `$state` + custom button → native `<details>`/`<summary>` (removes Svelte reactivity from click path).
- **B-003** removed tab-in-tab collapse in Stats and Secrets — Stats inline, SecretsPanel got `collapsible: boolean` prop.
- **B-001** scroll in Secrets/Stats tabs of RepoDetail (overflow-y:auto + min-height:0).
- **B-004** `ux_flow` category added to `tool` role in BugItem dropdown.

## [0.13.1] — 2026-04-20
- **F-022 DeployManifest extras persisted** (migration v15) — non-core placeholders saved as JSON in `deploy_manifests.extras`. Debounced 400ms on any field.
- **F-022 Go: `ENV_FILE_PATH` placeholder** (no longer secret) — bash conditional adds `--env-file` only when set.
- **Secrets: normalize line endings before encryption** — strips `\r`, trims, PEM-friendly trailing `\n`. Solves SSH_KEY copy-from-Windows issue.
- **Secrets: post-push verification** — list secrets after push, surface "GitHub confirmed but secret missing" silent-fail (usually PAT scope).
- **F-025 Reorder: per-row ▲▼ removed** — one pair in Sidebar header, targets selected item. Post-reorder highlight + scroll-into-view on wrap.
- **Sidebar-width 280 → 320px**.
- Nginx deploy: cert poll until `expires_on != null` (LE async issue). Go deploy: `--env-file` restored. F-021 done.md parser uses date-anchor. F-022 Go Dockerfile WORKDIR/-o collision fixed.

## [0.13.0] — 2026-04-19
- **F-022 Go deploy template** — new bundled language `go`. 9 placeholders, 7 required_secrets. Reference: `swanqu_server`.
- **meta.json schema v3** — rich-object placeholders with localized labels + auto_detect from repo files.
- **DeployScreen dynamic form** — rendered from meta.json placeholders.
- **F-025 Manual ordering** — migration v14 `sort_order INTEGER` on projects + repositories, ▲▼ buttons + D&D + auto-sort.
- **F-021 Docs viewer** — Tabs layout in RepoDetail (Overview/Bugs/Tasks/Stats). RepoDocsTab reads todo.md/done.md as tables.
- **F-026 Bug format v2** — 10 → 8 fields. Pipe-escape, newline-escape. Legacy v1 parser for transparent migration.
- **docs/formats/** — 5 formal specs (bug-reports v2, todo, done, claude-md-section, project-md).
- Tauri command `read_repo_file`.

## [0.12.0] — 2026-04-19
- **F-024 Sidebar UX**: collapse/expand-all buttons in sidebar header, projects collapsed by default on first launch, state persisted in SQLite settings.

## [0.11.0] — 2026-04-17
- Project graph fast prototype (cytoscape exploration), early micro-events, refinements in sync flow.

## [0.10.0] — 2026-04-17
- **F-019 Cross-repo requirements (REQ/Receipt pattern)** — sender writes `REQ-NNN_<slug>.md`, recipient writes `*.response.md`, sync replicates pairs. SyncScreen UI.

## [0.9.0] — 2026-04-17
- **F-017 Microservice connections** — many-to-many between projects and microservices, project_microservices table.

## [0.8.1] — 2026-04-16
- Hot-fix bundle for deploy template rendering.

## [0.8.0] — 2026-04-16
- **F-016 Deploy templates** — bundled flutter_web + go templates, DeployScreen with placeholders + required_secrets, render workflow files.

## [0.7.0] — 2026-04-16
- **F-015 Project & Repository CRUD overhaul** — assign_repository (cross-project move), role-based UI, role-priority sort.

## [0.6.0] — 2026-04-15
- Secrets management v1 — SecretsPanel with libsodium client-side encryption, PUT via GitHub API.

## [0.5.0] — 2026-04-15
- Stats v1 — incremental `bug_stats` table with handlers (later replaced by VIEW in v0.16.0).

## [0.4.0] — 2026-04-08
- BugNotes initial UI — MD-first storage with parsing on load and write on edit.

## [0.3.0] — 2026-04-02
- Multi-project model — projects table, sidebar tree, role-based repo grouping.

## [0.2.0] — 2026-03-31
- GitHub OAuth → PAT auth, listRepos integration, repo metadata caching.

## [0.1.0] — 2026-03-29
- Initial project scaffold: Tauri v2 + SvelteKit + Svelte 5 + TypeScript + Rust + SQLite.
