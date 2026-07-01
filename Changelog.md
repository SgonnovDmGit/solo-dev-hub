# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/). Russian version: [Changelog.ru.md](Changelog.ru.md).

## [Unreleased]

## [1.6.0] — 2026-07-01

Deploy value persistence (pain №1). Selected deploy secret values can now be kept locally (encrypted at rest) so they pre-fill instead of being retyped every session, and the deploy report gained a database-name line per environment. Originally scoped as a standalone typed secrets vault; corrected during design to a deploy-integrated feature — the DB belongs to deploy.

### Added
- **Opt-in local persistence of deploy secret values.** A per-secret 💾 "save" toggle (placed before the Include column) in the deploy secrets table keeps the value in a new encrypted-at-rest store (`deploy_secret_values`, AES-256-GCM, key in the OS keyring — reuses the secret-bundles crypto, no master password). Persisted values pre-fill the textarea on load instead of being blank; toggling off deletes the stored value. Available only for included + override secrets. Migration v27.
- **Database name in the deploy report.** Each deploy environment row can carry a sub-row showing its database name, drawn from persisted values and plaintext placeholders, with a 💾 (stored locally) / ☁ (only in GitHub) marker. Sensitive values (passwords, keys, tokens) are never displayed; `DATABASE_URL` is redacted to host/db. Host/user and SSH fields are captured by the query but hidden in the UI for now (future filters).

### Tests
- 413 cargo / 86 vitest / 0 svelte issues.

## [1.5.0] — 2026-07-01

Internal refactor milestone. Decomposes the two largest Svelte components into focused sub-components ahead of the typed-secrets-vault work (v1.6.0). No user-facing behavior change; the MINOR bump marks the refactor milestone per the roadmap.

### Changed
- **ProjectDetail and SecretsPanel split into focused components (no behavior change).** `ProjectDetail.svelte` (980 lines) extracts `ProjectHeader.svelte` (title, inline name/description edit, project-type select, edit/delete actions + confirm dialogs) and `ProjectMicroservicesTab.svelte` (connect/disconnect other microservice-projects + connected-parents list, owning its connection state and lazy-loading on mount); `parentsOfMicroservice` stays in the parent and flows to both children as props. `SecretsPanel.svelte` (920 lines) extracts `SecretsList.svelte` (existing-secrets list: checkboxes, per-row autosave, bulk delete) with an exported `reload()` the parent calls after a push for one-fetch refresh-and-verify. The flat flex-shrink cascade (T-000129) is preserved via a self-owned rule on the new list root.

### Removed
- **Dead project-mode in SecretsPanel.** The `mode='project'` project-wide secrets-push flow (prop, state, functions, confirm dialog) had no call site — SecretsPanel was only ever used in repo mode by RepoDetail. Removed along with 4 orphaned i18n keys. Recoverable from git if a project-wide push feature is ever planned.

### Tests
- 400 cargo / 86 vitest / 0 svelte issues (unchanged — pure code movement).

## [1.4.1] — 2026-06-15

Patch release. Continues the v1.4.0 internal-refactor work by decomposing the largest remaining command handler, and adds a developer-workflow self-heal for the local dev server. No user-facing behavior change.

### Changed
- **`sync_project` decomposed out of the command layer (no behavior change).** The ~712-line `sync_project` handler is extracted from `commands/sync.rs` into a new `sync/project_sync.rs` domain module (`run_project_sync`), leaving the Tauri command a 3-line wrapper. The body is split into focused helpers — a `SyncCounters` accumulator, `load_skeleton_templates`, `write_repo_skeletons` (collapsing the two duplicated Phase-0 skeleton loops), and `sync_client_to_server` / `sync_server_to_microservice` / `sync_microservice_to_parents` — preserving every load-bearing edge (B-001 early bail on a moved server folder, rename-replay idempotency, the standard-project "no server / no clients" warning gating, single up-front `project_type` read).
- **dev: auto-free port 1420 before `tauri dev`.** An aborted `tauri dev` orphans its vite child, which keeps listening on port 1420; because vite's `strictPort` is intentional (the Tauri `devUrl` is pinned to that port), the next launch hard-failed with "Port 1420 already in use". A new `predev` step (`scripts/free-port.mjs` — dependency-free, cross-platform, IPv6-aware) clears any stale listener so `npm run dev` / `tauri dev` starts clean every time.

### Tests
- 400 cargo / 86 vitest / 0 svelte issues (unchanged — the decomposition moved code without altering behavior).

## [1.4.0] — 2026-06-14

Internal refactor milestone. The monolithic command layer is split into domain modules on both sides — Rust `lib.rs` (103 Tauri commands, 3346 lines) into `commands/*.rs`, and the TypeScript binding file `tauri-commands.ts` into a `tauri-commands/` directory — and the sidebar resize handle is extracted into its own component. No new user-facing capability; the MINOR bump marks the refactor milestone per the roadmap (v1.4–v1.6). Ships alongside a polish pass on the secret-bundles screen and a broken-theme fix surfaced during dogfooding.

### Changed
- **Internal module refactor (no behavior change).** `lib.rs` split into `commands/{project,repo,bug,dashboard,sync,templates,deploy,timeline,misc}.rs` (handlers registered by defining-module path, `lib.rs` down to 223 lines); `tauri-commands.ts` split into a `tauri-commands/` directory re-exported through an index — the `$lib/api/tauri-commands` specifier resolves to the directory, so no consumer imports changed; the sidebar drag-resize handle extracted into `SidebarResizer.svelte` with a bindable `width` / `collapsed` / `isResizing` / `previewWidth` interface plus an `onCommit` callback.
- **Secret-bundles screen polish (dogfood).** Secret value fields are now masked textareas (mirroring the repo-secrets panel): collapsed to one line, growing on focus, with a reveal toggle and a resizable corner — multi-line keys like `SSH_KEY` are finally readable and editable, where the old single-line input mangled them. The bundle list now puts the name on the left and the secret count on the right of the same row, with per-row dividers and an accent on the active bundle. "New bundle" collapsed into a single button that discloses the create-form inline (Create enabled only with a name, plus Cancel), dropping the duplicate label. The "Add secrets" button is right-aligned.

### Fixed
- **Broken theme on the Secrets and Deploy screens.** `SecretBundles`, `DeploySecretsTable` and `DeployScreen` referenced CSS variables (`--hover-bg`, `--border-light`) that are not defined in the theme and have no fallback, so panel backgrounds, row hovers and borders rendered transparent/invalid. Remapped to the real tokens (`--surface` / `--surface-hover` / `--border`).

### Tests
- 400 cargo / 86 vitest / 0 svelte issues (unchanged — the refactor moved code without altering behavior; `cargo check` clean after gating the test-only event-log read helpers behind `#[cfg(test)]`).

## [1.3.0] — 2026-06-14

Adds reusable, locally-encrypted secret bundles — input the same SSH / DB / npm values once and apply them to any repo's or deploy environment's GitHub secrets, instead of re-typing them per repo. Folds in a deploy repo-config leak fix surfaced during dogfooding. The MINOR bump is driven by the new 🔐 Secrets screen — a new user-facing capability.

### Added
- **Secret bundles — new 🔐 "Secrets" titlebar screen** (ru locale: «Наборы секретов»). Reusable named bundles of GitHub secret values, **encrypted at rest** (AES-256-GCM, 32-byte data-key in the OS keyring — no master password; same trust model as the PAT). Master-detail editor: create / rename / delete bundles, add secrets via a **bulk `KEY=VALUE` textarea** (same dotenv format + per-line validation as the repo-secrets panel), masked values with per-row reveal toggle, per-secret delete. **Apply a bundle from two surfaces** — SecretsPanel (→ repo-level secrets) and DeploySecretsTable (→ a deploy environment's secrets) — merging decrypted values into the existing push form (bundle wins on name conflict); existing GitHub push paths unchanged. New `secret_bundles` + `secret_bundle_items` tables (migration v26 — values stored only as encrypted BLOB + nonce, never plaintext), `crypto/bundle_cipher` module, keyring data-key helper, 7 Tauri commands, and a pure `bundle-apply` merge helper with round-trip-safe value serialization (triple-quote / escaped-double-quote fallback so a value containing `"""` or newlines survives re-parsing before push).

### Fixed
- **Deploy repo-config leak between repos.** Focusing a "Shared image config" field in repo A and then switching to repo B wrote A's shared config into B — the on-blur save resolved the target repo from the live global selection (`$selectedRepoId`) rather than the repo the edit belonged to. The save now captures the owning repo id at mount (an `untrack` snapshot — the deploy block is keyed by repo, so one repo per instance) and guards the write against the current selection, so a blur firing during the repo-switch teardown is dropped instead of leaked. Normal same-repo saves are unaffected.

### Tests
- 400 cargo (+14: AES-GCM cipher roundtrip / tamper / wrong-key / nonce-length ×7; migration v26 table-presence ×1; bundle db CRUD + CASCADE + UNIQUE + decrypt ×6) / 86 vitest (+14: apply-merge map overlay + dotenv-text merge across 8 round-trip value classes incl. triple-quote-in-multiline and literal-backslash edge cases) / 0 svelte issues.

## [1.2.0] — 2026-06-02

Adds a portfolio-wide deploy report and a `.gitattributes` managed template, alongside two dashboard/secrets fixes from continued dogfooding. The MINOR bump is driven by the new top-level Deploys screen — a new user-facing capability.

### Added
- **Deploy report — new "Деплои" titlebar screen.** Portfolio-wide inventory of every deploy environment across all repos, grouped by project (orphan repos under "Без проекта"). Columns: repository, environment (color badge), domain (click → open in browser), branch, image tag, included-secrets count, config-updated date. Project / environment / free-text filters; columns aligned across sections via a fixed table layout. Clicking a row drills into that repo's Deploy tab with the environment opened in detail, via a one-shot `deployDrillTarget` signal (mirrors the Timeline deep-link). Read-only — live GitHub Actions run status is deferred to a later release. New `list_deploy_report` query (JOIN `deploy_environments` × `repositories` × `projects` + included-secrets count, repo display via `display_name()`) + Tauri command + `DeployReport.svelte`.
- **`.gitattributes` managed template (B-000024).** New `_global/.gitattributes.tmpl` synced into managed repos: `* text=auto eol=lf` line-ending normalization for sources, CRLF for Windows scripts (`*.bat`/`*.cmd`/`*.ps1`), binary markers. Shares the dedup-aware section-merge logic with `.gitignore` — extracted to `sync/managed_block`, with `sync_gitignore_section` now a thin wrapper. Wired into `init_docs_for_repo` + `sync_project`; appears in Settings → default templates automatically (seeder + editor are zero-touch via `include_dir!` + dynamic listing).

### Fixed
- **B-000026 (regression, major).** Dashboard custom date period was unusable — selecting the "Custom" preset only highlighted the button and rendered no date inputs, so no range could be chosen. Added start/end `<input type="date">` (seeded from the current window) wired to the already-present but previously-orphaned `setCustomPeriod` store action, guarding `start ≤ end`.
- **B-000025.** The bulk-paste secrets field now remembers its content per repository and restores it when switching back, instead of wiping on every repo switch. Implemented with a module-level per-repo draft map in `SecretsPanel` (load/save on repo change); the post-push clear is preserved per-repo. (First attempt — a `{#key}` remount that wiped the field — was rejected; this is the rework.)

### Tests
- 386 cargo (+6: deploy-report aggregation query; `.gitattributes` section-merge ×4; bundled-dotfile embed guard) / 72 vitest / 0 svelte issues.

## [1.1.0] — 2026-05-25

First MINOR after the v1.0.0 public launch. Adds a verdict-rollback path for bugs and ships two dogfood-surfaced UX fixes that emerged the same week. The MINOR bump is driven by T-000130 — a new user-facing capability (↩ reopen button) — not by the polish fixes that came along with it.

### Added
- **T-000130 — Reopen bug action (↩ button).** Until now ✓ and ✗ on a `testing` bug were one-way: clicking either by accident or on second thought left the user with no way back. The new ↩ button on `confirmed` and `rejected` rows reopens the bug to `testing` so the verdict can be retaken without losing fix history. Replaces the read-only ✓ mark on `confirmed` rows (the status badge already names the state) and fills the previously-empty action slot on `rejected`. No confirm dialog — the whole point of the button is fast rollback. New Tauri command `reopen_bug(repo_id, display_id)` + DB method `reopen_bug(bug_id)` atomically: `status='testing'`, `confirmed_at = NULL`, `archived_from_md_at = NULL`. `fix_attempts` deliberately preserved — reopen is the undo of a verdict, not a new fix attempt. A `reopened` event is logged to `bug_events` (with `from_status` = original) so the Dashboard activity feed sees the action, but it does NOT contribute to KPI5 (avg attempts per closed in period filters by `event_type='entered_testing'`). The bug_events invariant `COUNT(entered_testing) == bugs.fix_attempts` is preserved through reopen. Amber `#f59e0b` for the button distinguishes "rollback" from finalising ✓ (green) and rejecting ✗ (red).

### Changed
- **`dialog.confirm` i18n key** — `Подтвердить` / `Confirm` → `ОК` / `OK`. Affects every `ConfirmDialog` site (13 usages: bug delete, bug reject, deploy env delete, GlobalClaudeEditor discard, project type change, project delete, repo delete, secrets bulk delete, project secrets push, sidebar project delete, template revert). The header of each dialog already names the action — the button just needs to be a confirmation primitive. Closes B-000022.

### Fixed
- **T-000129 — SecretsPanel bulk-paste textarea now grows vertically** to fill the Секреты tab. Previously pinned at `rows="4"` (~70px) regardless of viewport — on a 1337px-tall window the input occupied 5% of vertical real estate, the rest sat empty. `.secrets-wrapper` in RepoDetail now lays out as `display: flex; flex-direction: column`, and `.secrets-section.flat` cascades `flex: 1 / min-height: 0` through `.secrets-body` → `.new-secrets` → `.secrets-textarea`. `.existing-secrets` carries `flex-shrink: 0` so a long list keeps natural height and the wrapper scrolls instead of compressing it. Minimum height 70px kept as a floor; resize-vertical handle preserved.

### Tests
- 380 cargo (+4 for `reopen_bug`: confirmed → testing clears confirmed_at + keeps attempts; rejected → testing keeps attempts; archived_from_md_at cleared so the bug reappears in MD; bug_events invariant `COUNT(entered_testing) == fix_attempts` holds across reopen) / 72 vitest / 0 svelte issues.

## [1.0.4] — 2026-05-25

Template-only mini-patch — gitignore template for managed repos was missing one folder pattern and had a trailing-slash inconsistency on another. Downstream impact: any managed repo whose `.gitignore` was generated from this template before v1.0.4 may have `docs/microservice-announcements/` untracked-but-visible in git status, plus `docs/server-announcements` matching loose files (not just the folder).

### Fixed
- **`src-tauri/templates/_global/.gitignore.tmpl`** — added `docs/microservice-announcements/` (symmetric to `docs/server-announcements/`; both are recipient-side acknowledgement-by-delete channels per `# Cross-repo announcements` rules — neither belongs in git history). Added trailing `/` to `docs/server-announcements` so the pattern unambiguously matches the folder, not loose files of the same name. Existing managed repos will pick up both patterns on next "Sync global rules" run.

### Tests
- 376 cargo / 72 vitest / 0 svelte issues (unchanged — template-only change).

## [1.0.3] — 2026-05-25

Dogfood patch — five bugs surfaced in daily use of v1.0.2, plus brand-identity unification and a Tauri-runtime bump that fell out of the B-000017 investigation.

### Added
- **T-000127** — Windows-bundle icon PNGs (`32x32.png`, `64x64.png`, `128x128.png`, `128x128@2x.png`) regenerated from the new hex+Y-tree design to match `icon.ico` from B-000017 v8. In-app branding (titlebar `logo.png`, About-hero `logo-large.png` in `src/lib/assets/`) intentionally kept on the older detailed full-logo per user preference. macOS/iOS/Android sets deferred (not built locally; see T-000125).
- **Dashboard refresh** now reconciles MD→DB portfolio-wide before reload — both the manual `↻` button and the initial `onMount` pass call a new `reconcile_all_projects` backend command that walks every repo and runs `reconcile_bugs_for_repo` + `sync_tasks_for_repo`. Previously refresh was DB-read only, so LLM-side MD edits stayed invisible until the user manually synced each project.
- **`scripts/cleanup-target.sh`** — disk-recovery helper for cargo `target/`. Safe default drops `target/debug/incremental/` only (typically frees 5-10GB while preserving compiled deps); `--full` mode runs `cargo clean` for a complete debug+release wipe. Smoke on local repo: 19G → 12G in safe mode.
- **`sync::confirm_pair(source_repo, target_repo, filename)`** — new helper that handles bilateral REQ-pair deletion from sender+recipient repo records directly, without depending on "server in current project". Replaces ~90 lines of branching path-resolution in `confirm_requirement`.

### Changed
- **Tauri 2.10.3 → 2.11.2** (tao 0.34.8 → 0.35.3, wry 0.54.4 → 0.55.1). Originally an investigation step for B-000017; left in place after the icon fix because no regressions surfaced.
- **`icon.ico` regenerated** via Python/PIL to mirror the structure of a known-working sibling Tauri app on the same Windows 11 / high-DPI display setup. New file: 6 frames (16/24/32/48/64/256) all 32bpp PNG-compressed, 22KB. Old file: 10 frames mixed 8/24/32-bpp with uncompressed-BMP large frames, 419KB.

### Fixed
- **B-000016** — Dashboard `↻` refresh button was DB-read only; it didn't catch MD-side LLM edits to bugs / tasks. Added backend command `reconcile_all_projects` that walks every repo and runs `reconcile_bugs_for_repo` + `sync_tasks_for_repo`; both the manual button and the initial Dashboard mount now go through it.
- **B-000017** (after 8 attempts) — Windows-taskbar icon rendered as a blurry smudge on high-DPI displays (3072×1920 with 200% scale → ~32-48 physical px taskbar). The first 4 attempts iterated icon-design choices, the next 3 tried Tauri / Cargo-feature changes (drop `set_icon`, drop `image-png` feature, Tauri version bump) — all dead-ends. **Real fix (v8):** diff against `F:\Development\MySafeSpace` (a working sibling Tauri app on the same machine) revealed `icon.ico` was the structural culprit — our file had mixed bit-depths (16-frame 8-bpp paletted, large frames uncompressed BMP), the sibling's was uniformly 32bpp PNG-compressed. Regenerated via Python/PIL to mirror the sibling. Captured the methodology lesson — "for runtime/render bugs, diff against a known-working sibling FIRST before iterating speculative fixes" — in the assistant's auto-memory for future bug runs.
- **B-000018** — Sidebar `+` button blended with surrounding ASCII text; replaced glyph with ➕ and added chip-style min-width / centred alignment for visual balance with adjacent controls.
- **B-000019 + B-000020** — pressing Sync on a microservice-project was a no-op: `sync_project` iterated `clients` and `microservice_ids` loops which are both empty for a microservice. Added an MS-driven sync block that fans out to each connected parent server via `list_parents_of_microservice` — copies api.md and handlers.md from MS to parent, REQ files from parent to MS, response.md files from MS back to parent. Mirrors the parent-driven block that already existed for server projects.
- **B-000021** — confirm-✓ button on REQ pairs only worked from the server's project SyncScreen. Opening the microservice's own SyncScreen for the same REQ showed it as `is_reverse_lookup` and hid the button entirely, forcing the user to navigate to the parent project just to acknowledge. Root cause: `confirm_requirement` resolved paths via "server in current project", which is the MS itself in reverse-lookup view (not the actual REQ sender). Extracted `sync::confirm_pair(source, target, filename)` that derives paths from sender + recipient repo records directly — confirm now works symmetrically from either project's SyncScreen. UI guard `!req.is_reverse_lookup` dropped along with the now-redundant ↩ hint span; `is_reverse_lookup` field kept on `RequirementInfo` as an informational/audit flag only.
- **T-000128** — pending LLM MD edits on `docs/bug-reports.md` were silently wiped whenever the user clicked any bug-mutation button in the app (`+ Add bug`, ✓ confirm, ✗ reject, edit fields, delete). Root cause: the five Tauri commands `create_bug`, `resolve_bug`, `update_bug_fields`, `delete_bug`, `reject_bug` in `lib.rs` each did `mutate DB → regenerate_bugs_md` with no `reconcile_bugs_for_repo` call first — so the regen wrote the stale DB state, overwriting whatever the LLM had just edited in MD. Surfaced live during the v1.0.3 closure pass for B-000021 (the bug's `testing` status + fix comment kept reverting to `created` / empty after each app interaction). Fix: prepend `let _ = sync::reconcile_bugs_for_repo(&db, repo_id);` to each of the five commands so LLM edits ingest into DB first, then the mutation lands on top, then regen reflects both. Pattern documented + asserted in new regression test `test_t000128_reconcile_before_mutate_preserves_llm_edits` in `sync/bugs.rs`.

### Tests
- 376 cargo (+5 new for `sync::confirm_pair` covering client→server / server→MS happy paths, sibling-NNN disambig invariant from v0.27.1 preserved, unknown source role errors out, missing local_path silent no-op; +1 new T-000128 regression `test_t000128_reconcile_before_mutate_preserves_llm_edits`) / 72 vitest / 0 svelte issues on 495 files.

## [1.0.2] — 2026-05-18

Release-signing infrastructure fix. The first actual end-to-end autoupdate cycle on the public repo (v1.0.0 → v1.0.1) surfaced a long-standing latent mismatch: `tauri.conf.json` carried a pubkey (`7135A97A3C3F89EF`) that did not correspond to the private key in the CI `TAURI_SIGNING_PRIVATE_KEY` secret (`4D58133D6147291E`). The two had drifted apart at a past keypair rotation where the new private key was written into the GH Secret but the matching pubkey was never propagated to `tauri.conf.json`. Every release from that point onward shipped installers whose `.sig` verified against a key the embedded binary did not trust — invisible while the repo was private (autoupdate endpoint required auth), invisible while everyone installed manually from local builds, but it broke the moment a real autoupdate path opened.

### Fixed
- **Autoupdate signature verification end-to-end**. Pubkey in `src-tauri/tauri.conf.json` realigned to `4D58133D6147291E`, matching the actual CI signing key. From v1.0.2 onwards new installs embed the correct pubkey and verify signatures successfully.
- **⚠ One-time manual reinstall required for existing v1.0.0 and v1.0.1 installs.** The bug means those binaries embed the stale pubkey and will continue rejecting v1.0.2 (and any future) signed updates from autoupdate. Workaround: download `Solo.Dev.Hub_1.0.2_x64-setup.exe` from this release page and run it once. After that, autoupdate works automatically for all future releases. New installs from v1.0.2 onwards have no impact.

### Tests
- 370 cargo / 72 vitest / 0 svelte issues on 495 files (no code change beyond the manifest version bump and the pubkey field in `tauri.conf.json` — baseline carried from v1.0.1).

## [1.0.1] — 2026-05-18

First post-launch patch. Two dogfood-surfaced bugs in v1.0.0 — both regressions introduced by F-000041 (the project's first local `git` CLI shellout, shipped in v0.34.0) and the v0.34.0 path-row layout that landed under it.

### Fixed
- **B-000014** (critical) — on Windows release builds a console window flashed every time the user clicked a repository in the sidebar. Root cause: the `$effect` for `canUntrack` in `RepoDetail.svelte` calls `check_git_available_for_repo` → backend spawns `git --version` via bare `std::process::Command::new`, which on Windows-GUI-host inherits `STARTUPINFO` with no `CREATE_NO_WINDOW` flag → cmd.exe pops up for the subprocess lifetime. Added a `spawn_cmd()` helper that sets `CREATE_NO_WINDOW` (`0x08000000`) via `CommandExt::creation_flags` on Windows; applied to all 5 production callsites in `git_ops.rs` (`check_git_available` × 2, `list_gitignored_tracked`, `untrack_files`, `count_other_staged_changes`). `#[cfg(windows)]` — no-op on macOS/Linux. Test callsites left on bare `Command::new` — `cargo test` on Windows runs in a console host where the flag is moot.
- **B-000015** (major) — two-part issue surfaced during smoke. Part 1: a deeply nested `.local-path` (e.g. `📁 F:\Development\some\long\subdir\to\repo`) pushed the `📚 Init docs` and `🧹 Untrack` row-action buttons onto the next line of `meta-row`. Capped `.local-path` with `max-width: 40ch` + `overflow: hidden` + `text-overflow: ellipsis` + `white-space: nowrap` + `min-width: 0` (last required so the flex child actually shrinks); full path available on hover via the `title` attribute. Part 2 (retest finding): on every repo switch, the two row-action buttons flickered. Root cause: the `canUntrack` `$effect` reset to `false` synchronously before kicking off the async backend check, so the `{#if canUntrack}` block tore the untrack button out of the DOM and re-added it ~ms later, with the init-docs button visibly jittering from the flex-row reflow. Dropped the sync reset and added a stale-response guard (`repo?.id === repoId`) so a slower response for repo A cannot overwrite a faster response for repo B when the user clicks A → B in quick succession.

### Tests
- 370 cargo / 72 vitest / 0 svelte issues on 495 files.

## [1.0.0] — 2026-05-18

**Public launch.** Solo Dev Hub goes public and becomes MIT-licensed open source. No breaking API changes from v0.34.0 — this release marks the transition from `0.x` (unstable contract) to `1.x` (frozen contract starts here). Tauri identifier (`com.solodevhub.app`) and lib name (`solo_dev_hub_lib`) have been stable since v0.25.0, so autoupdate `v0.34.x → v1.0.0` runs as a normal in-place update on existing installations.

### Changed
- **T-000064** — `SgonnovDmGit/solo-dev-hub` repository visibility flipped from private to public. The autoupdate endpoint `https://github.com/SgonnovDmGit/solo-dev-hub/releases/latest/download/latest.json` now resolves without GitHub auth — installations on `v0.25.x..v0.34.x` will pick up `v1.0.0` through the in-app updater.
- **T-000064** — Legacy `SgonnovDmGit/github-repo-manager` repo archived (readonly) with a `moved to solo-dev-hub` redirect note. The full pre-rebrand history stays preserved there for posterity.
- **T-000074** — Version bump to `1.0.0` across `package.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/tauri.conf.json`.

### Tests
- 370 cargo / 72 vitest / 0 svelte issues on 495 files (baseline carried from v0.34.0 — no code changes in this release beyond the version bump).

## [0.34.0] — 2026-05-17

Final pre-launch patch before v1.0.0 public flip. Two user-visible streams: a one-click "Untrack gitignored files" workflow that removes files from the git index after `.gitignore` rules change post-commit (F-000041 — the project's first local `git` CLI shellout layer), and a project-name pin in the SyncScreen header so cross-repo flows always show context. Plus tightening of the global AI-rules (retro one-block delivery, allow committing / pushing on integration branches, PowerShell `&&` portability rule) and CRLF-normalization via `.gitattributes` to kill phantom Windows diffs.

### Added
- F-000041 / T-000119 — backend `git_ops` module wrapping the local `git` CLI: binary discovery (PATH + Windows fallback), repo-state detection (clean / mid-merge / mid-rebase via marker files), `git ls-files -ci --exclude-standard -z` listing, chunked `git rm --cached` batching, other-staged count for the UI info-warning. First subprocess shellout in the project; 12 new unit tests.
- F-000041 / T-000120 — three Tauri commands (`check_git_available_for_repo`, `list_gitignored_tracked`, `untrack_files`) + TS wrappers + boundary DTOs (`UntrackReport`, `GitignoredListing`).
- F-000041 / T-000121 — `UntrackGitignoredDialog.svelte` (Svelte 5 runes, modeled on MergeChoiceDialog) with select-all / deselect-all / per-row checkboxes, mid-merge / mid-rebase block, other-staged info-warning, partial-error toast aggregation. 🧹 trigger in RepoDetail header next to 📚 Init docs (housekeeping cluster). 11 i18n keys × ru+en + 2 toast keys.
- T-000123 — current project name now appears in SyncScreen header (`Sync — {project}`) so the user sees scope at a glance.

### Changed
- T-000118 — `.gitattributes` added at repo root: `* text=auto eol=lf` baseline + per-extension overrides for source / data / binary file groups. Kills phantom CRLF modifications on Windows hosts with `core.autocrlf=true`.
- T-000124 — global AI-rules template tightened (propagates to `~/.claude/CLAUDE.md` on next global sync): retro delivered as one block instead of conversation-paced six turns; multi-branch flows (dev → master via merge) — assistant may `git commit` and `git push origin <branch>` on the integration branch without per-action approval (tags + release merges + final master push stay user-only); new section "Shell command portability" — avoid `&&` (fails on Windows PowerShell 5.1 default), prefer one command per invocation or `;` for cross-shell compatibility.
- Internal: cargo fmt baseline (27 files reformatted to rustfmt-clean) — pulled out as a separate commit so feature changes don't carry format noise.

### Fixed
- F-000041 / T-000121 smoke finding — Untrack button rendered in the middle of row 2 because two consecutive `.row-action` (margin-left: auto) flex siblings split the available space. Moved to row 1 next to Init docs with a margin override so Init Docs anchors the pair to the right edge.

### Tests
- 370 cargo / 72 vitest / 0 svelte issues on 495 files.

## [0.33.0] — 2026-05-17

Pre-launch polish before v1.0.0 public flip. Four streams: `docs/project.md` format consistency fix surfaced during dogfood, global AI-rules tightening (`docs/handlers.md` scope + Release lifecycle section), Top-3 hot formula broadening, and README hero/feature screenshots.

### Added
- **T-000112** new `## docs/handlers.md` subsection in `# API contract sync` of the global template — defines optional server-side internal handler notes file (transaction boundaries, side-effect chains, cross-cutting concerns), syncs symmetrically with `api.md` to `docs/server-api/handlers.md` (server → client) and `docs/microservice-api/<ms>/handlers.md` (microservice → parent server). Includes a hard rule forbidding handler-level documentation in `README.md` (cross-repo sync only propagates `api.md` + `handlers.md`; README stays sender-side, invisible downstream).
- **T-000113** new `# Release lifecycle` section in the global template, between `# Phase work workflow` (per-task) and `# Manual-smoke verification`. 11 stages (Request → Analysis → Spec → Spec review → Plan → Plan review → Implementation → Test → Release closure → Next-release plan → Retro) with soft permission-gated transitions and optional loop-backs on user request or forced circumstances. Each review stage is a 3-step procedure (assistant self-review for ambiguities/contradictions/gaps → clarification questions if real → user OK). Mandatory 6-point retro checklist (what worked / what didn't / release + project readiness / LLM session retrospective / user session retrospective / process lessons) stored as `project`-type memory file in auto-memory dir (`retro_v<X_Y_Z>.md`), not committed to docs/.
- **T-000115 + T-000116** Top-3 hot projects formula broadening. New weighted heat-score `critical × 50 + major × 15 + active × 1 + closed_in_period × 2 + tasks_done_in_period × 1` replaces the previous "active bugs only" filter — task-active projects now surface in top-3 when no severity bugs anywhere. Threshold: any non-zero signal qualifies. `top_hot_projects` SQL takes `Option<(period_start, period_end)>` — `Some` for dashboard window, `None` for Stats tab lifetime mode (sentinel `0001-01-01` / `9999-12-31` dates). Frontend chips extend to `N crit / N maj / N act / N closed · N tasks` (slash for bug-domain, middle-dot before task chip); native `title=""` tooltip on section header shows the full formula. Same fix mirrored in `top_hot_repos_in_project` for StatsSummary consistency.
- **T-000073** 8 hero/feature screenshots integrated into `README.md` + `README.ru.md` replacing TODO-placeholder comments: dashboard hero (Quarter period, KPI + top-3 hot + daily flow), repo bugs (severity/status/attempts variety), repo tasks (DataGrid with version column from T-000109), project graph (server centered, dashed cross-project edges), deploy master + deploy drill-down (Flutter, BUILD/DEPLOY/RUNTIME role variety), requirements sync (cross-repo REQ flow across 4 directions), settings (PAT/Appearance/Workspace/Templates/Global AI rules cards).

### Changed
- **`docs/project.md` template gitignored.** Contains user-specific local filesystem paths and regenerates on every sync — same regenerated-view family as `docs/todo.md` / `docs/done.md` / `docs/bug-reports.md`. Added to `.gitignore.tmpl`.

### Fixed
- **B-000013** `docs/project.md` section format consistency. Connected microservices and Parent projects sections were bullet-lists while Repositories was a markdown table — three sections now render as parallel tables with `| Microservice/Parent project | Server repo | Path | GitHub |` columns. Marker strings `_no local path configured_` and `⚠ server repo not resolvable` migrated into table cells; announcement-LLM grep behavior unchanged (matches docstring text inside cells). Global template `claude.md.global.tmpl` and `docs/formats/project-md.md` spec updated to reflect new table format.

### Tests
- 358 cargo (354 → +4 from T-000115: tasks-only-qualifies / closed-in-period-contributes / one-critical-dominates-50-tasks / lifetime-mode), 72 vitest (unchanged from v0.32.0), svelte-check 0/0/0 on 493 files. T-000114 subagent behavioral verification (3 parallel general-purpose agents simulating user prompts → checking template rule routing) — all PASSed, no rule-text refinement needed.

## [0.32.0] — 2026-05-15

Three small targeted tasks closing UX gaps from the v0.31.0 dogfood + a documentation-driven feature for the Tasks view. All from a single afternoon session.

### Added
- **T-000109** SemVer-aware version column in TasksTab + symmetric upgrade in DoneTab. The Rust todo-parser now tracks `## vX.Y.Z — <description>` section headers as a release-grouping signal and applies the inherited version to every task below until the next such header. Non-version `##` headers (`## Format`, `## Backlog`, etc.) are ignored — only `v<digit>...` activates the signal. Tasks above the first version header carry an empty version. `parse_todo_tasks` and `TodoTask` gain a `version: String` field; sync wires it into `tasks.version` on insert AND updates existing rows when a task moves between version sections (no event in `task_events` — metadata shift, not a workflow transition). DataGrid grows a `sortCompare?: (a, b) => number` column hook for custom comparators. New `src/lib/utils/semver.ts` exports `compareSemVer` (parses `MAJOR.MINOR.PATCH`, sorts pre-release tags before the matching release, falls back to `localeCompare` for non-semver values; null/empty sorts to the end). TasksTab gets a new "Version" column (sortable, text-filter, SemVer-aware sort); DoneTab's existing version column upgraded to the same comparator. Convention documented in `_global/claude.md.global.tmpl` line 50 — plain markdown readers still see headers as comments; Solo Dev Hub-style tools can opt into the release-grouping interpretation.
- **T-000110** `auto_detect` runner + `value_if_match` predicate mode. Previously the `auto_detect` block in `meta.json` was dead-spec — declared on `NODE_VERSION` / `GO_VERSION` but never executed, so users always got the static `default`. New pure-function runner `src/lib/api/auto-detect.ts` reads files via injected `readFile` callback (backed by the existing `readRepoFile` Tauri command) and applies the regex. Two modes: capture (existing — group `[1]` becomes the value; example: `NODE_VERSION` from `.nvmrc`) and predicate (new — `value_if_match` static string when regex hits; example: `PRE_BUILD_COMMAND = "npm run paraglide:compile"` when `package.json` contains `"@inlang/paraglide-js"`). `path` may be a single string or an ordered array — runner tries each in turn, stops on the first match. `DeployScreen.loadRepoConfig` calls the runner for every repo-scope placeholder with `auto_detect` AND empty stored value, persists results on first detect via `setRepoDeployConfig`. User overrides preserved (non-empty values are skipped); re-detection only fires when the field is cleared. `vite_static/meta.json` updated: `PRE_BUILD_COMMAND` gets predicate-mode auto-detect for Paraglide; `BUILD_OUTPUT_DIR` gets capture-mode auto-detect from `vite.config.{js,ts}` or `svelte.config.js`.

### Fixed
- **T-000111** Secrets-parser bare-multiline hint. When `parseEnvText` hits `missing '='` and a secret has already been parsed on this run, the error now reads `Line N: looks like a multi-line value for 'SSH_KEY'. Wrap it in triple quotes: SSH_KEY="""<newline>...<newline>"""` instead of the generic message. Generic message preserved when no secret has been parsed yet (orphan-line-first case). Hint fires once per run-on — `prevSecretName` resets after emit. Pattern dogfooded 2026-05-14 from T-000107 deploy where a bare-multiline SSH key paste failed silently with the generic error.

### Tests
- 354 cargo (350 → +4 from T-000109 parser + sync logic), 72 vitest (50 → +3 T-000111, +10 T-000110, +9 T-000109 semver), svelte-check 0/0/0 on 493 files (was 489 — added `auto-detect.ts` + `semver.ts` + the two new test files).

## [0.31.0] — 2026-05-14

Deploy config rework + third built-in template. The Go multi-env dogfood in v0.29.2 surfaced that repo-wide placeholders (`GO_VERSION`, `BINARY_NAME`, `ENTRY_POINT`, `APP_PORT`) lived per-env in `extras{}` — every env duplicated them, and divergent values produced "last Generate wins" surprises since they all render into a single shared `Dockerfile`. v0.31.0 fixes the storage model (T-000103) and ships a third deploy target (T-000107) on top of that fix.

### Added
- **T-000103** Repo-wide vs env-specific placeholder split. `repositories.deploy_repo_config TEXT NOT NULL DEFAULT '{}'` (migration v25) stores repo-scope values once per repo; per-env `extras{}` stays for env-scope only. Migration v25 reads each template's `meta.json` from the `templates` table, identifies placeholders with `"scope": "repo"`, lifts their value from the first env (ordered by `sort_order ASC`), strips them from all envs. First-env-wins on divergence with a `sync_events` row carrying JSON conflict detail (`{"conflicts":[{"key":"GO_VERSION","kept_env":"prod","kept_value":"1.26-alpine","discarded":[{"env":"test","value":"alpine"}]}]}`). Idempotent: skips the data-loop if `deploy_repo_config != '{}'`. DeployScreen gets a collapsible "Shared image settings" section above the env list; DeployDetail filters repo-scope placeholders out of its loop. Per-key autosave on blur (mirrors DeploySecretsTable to avoid B-000009 focus loss). Sticky DeployDetail header (`position: sticky; top: 0; z-index: 10`). Schema-aware render merger `template_render::build_placeholder_vars` sources `scope: "repo"` keys from `deploy_repo_config`, `scope: "environment"` (default) from `env.extras`. Activity feed (Timeline, RecentActivityFeed, DashboardActivityFeed) recognises `sync_type='migration'` with JSON parsing. +32 tests (314 → 346).
- **T-000107** Third built-in deploy template `vite_static` for Vite-based static SPAs (Svelte/React/Vue/Solid + Vite → `nginx:alpine` via SSH push + NPM upsert). Architectural twin of `flutter_web` (same downstream); differs only in build stage: `node:lts-alpine` + `npm ci` instead of Flutter SDK + dart-define. New derived value `@@DOCKERFILE_ENVS@@` (fifth in the family with `BUILD_ARGS` / `RUNTIME_ENV_ARGS` / `DOCKERFILE_ARGS` / `DART_DEFINES`) emits `ENV NAME=$NAME` per build secret — required for Vite because the npm-spawned process reads `VITE_*` from `process.env`, not from Docker ARG scope. Repo-scope placeholders: `NODE_VERSION` (default `lts-alpine`, auto-detects from `.nvmrc`), `BUILD_OUTPUT_DIR` (default `dist`), `PRE_BUILD_COMMAND` (default `true` = shell no-op; set to `npm run paraglide:compile` for Paraglide projects). `deploy.yml.tmpl` is a byte-copy of `flutter_web/deploy.yml.tmpl` (SSH + NPM machinery is shared). Reference target shape: Digital-mech-lab landing. +4 tests (346 → 350).

### Changed
- **Scope vocabulary split (T-000103 Task 2).** Two scope vocabularies now coexist in `meta.json`: `placeholders.<KEY>.scope ∈ {"repo", "environment"}` (default `"environment"`) and `required_secrets[*].scope ∈ {"deploy_repo", "environment"}` (no default — explicit). The `"deploy_repo"` rename (from the pre-v0.31.0 `"repo"`) disambiguates "this is a GH Actions Repository Secret, not an Environment Secret" from "this placeholder renders into a single repo-wide file". Pre-v1.0.0, no shipped users → no back-compat shim. Custom templates carrying the obsolete value fail to load with a human-readable error pointing at the exact field.
- **Strict-mode `meta.json` validation at seed + parse time.** `template_meta::validate_meta_json` runs on every bundled-template seed (`template_seeder.rs:42`); a bundled template carrying an invalid scope value fails app startup with a clear message. `parse_meta_placeholders` and `parse_meta_secret_hints` are the parser equivalents called from `render_files_for_deploy_env` — both reject unknown scope values rather than silently falling back. Frontend keeps reading freeform `label` / `description` / `default` / `type` / `auto_detect` straight from the raw JSON.
- **Go template (`templates/go/meta.json`)** bumped to `version: 5`. Four placeholders (`GO_VERSION`, `BINARY_NAME`, `ENTRY_POINT`, `APP_PORT`) now carry `"scope": "repo"`. Two NPM secrets renamed `"scope": "repo"` → `"scope": "deploy_repo"`.
- **Flutter_web template (`templates/flutter_web/meta.json`)** bumped to `version: 5`. Two NPM secrets renamed `"scope": "repo"` → `"scope": "deploy_repo"`.

### Tests
- 350 cargo (was 314 in v0.30.x — +32 from T-000103, +4 from T-000107), 50 vitest (unchanged), svelte-check 0/0/0 on 489 files (unchanged file count — vite_static template files are bundle assets, not TS sources).

## [0.30.1] — 2026-05-14

Second mechanical refactor pass on top of v0.30.0. Three more splits — `export.rs` by parser domain, `sync/claude_md.rs` un-bundled into three concerns, and `i18n/translations.ts` split by key prefix. Pure lexical move, no behavior change. All test suites unchanged (314 cargo, 50 vitest, svelte-check clean — file count 454 → 489 due to the new `i18n/strings/` files).

### Changed
- **T-000104** Split `src-tauri/src/export.rs` (1123 lines) into `src-tauri/src/export/` directory: `mod.rs` (barrel) + `util.rs` (shared pipe-parser, escape/unescape) + `bugs.rs` (v2 8-field format generate/parse) + `bugs_legacy.rs` (pre-v2 import path: `parse_header`, `parse_bug_entry`, `parse_markdown_legacy`) + `todo_done.rs` (F-021 todo/done parsers). Commit `4ca36e0`.
- **T-000105** Un-bundled `src-tauri/src/sync/claude_md.rs` (872 lines, three concerns) into focused files: `claude_md.rs` (448 lines — CLAUDE.md section rendering only), `project_md.rs` (238 lines, new — `generate_project_md` for cross-repo announcement Path lookups), `gitignore.rs` (221 lines, new — `sync_gitignore_section`). Commit `0684235`.
- **T-000106** Split `src/lib/i18n/translations.ts` (1594 lines, ~727 keys × ru/en flat objects) by key prefix into `src/lib/i18n/strings/<domain>.ts` (35 files, one per top-level prefix). Each file exports `ru` and `en` slices for its domain. Root `translations.ts` (now 116 lines) merges them at module init via spread. `TranslationKey` type-narrowing preserved via `as const`. Commit `336ec38`.

### Tests
- 314 cargo (unchanged), 50 vitest (unchanged), svelte-check 0/0/0 on 489 files (was 454 — added 35 `i18n/strings/*.ts` files).

## [0.30.0] — 2026-05-14

Pre-v1.0.0 mechanical refactor bundle. Six tasks (T-000093/094/095/096/097/102) split the four largest Rust modules and the TypeScript `types.ts` by domain. Pure lexical move + extraction — no behavioral change. All test suites stay green (314 cargo, 50 vitest, svelte-check clean).

### Changed
- **T-000093** Removed 9 no-op `*_stat` Tauri commands (`increment_bug_stat`, `decrement_bug_stat`, `add_attempts_stat`, `subtract_attempts_stat`, `increment_resolved_stat`, `transfer_bug_stat`, `reset_repo_stats`, `reset_all_stats`, `recalculate_all_stats`) — legacy stubs from the v0.16.0 stats-table→VIEW migration with bodies `Ok(())` and no callers. Frontend `tauri-commands.ts` wrappers removed too. Surviving 3 commands (`get_repo_stats_summary`, `get_project_stats_summary`, `get_project_graph`) kept under tightened section header.
- **T-000094** Split `src-tauri/src/db.rs` (7,314 lines, 261 methods) into `src-tauri/src/db/` directory: `mod.rs` (struct + ctor + free fns + `pub mod`) plus 10 domain sub-modules — `migrations` (967 lines), `projects` (978), `repos` (1,290), `bugs` (1,081), `dashboard` (787), `deploy` (805), `tasks_events` (339), `stats` (427), `timeline` (406), `graph` (194). Multiple `impl AppDb` blocks across files; API surface unchanged.
- **T-000095** Refactored `run_migrations` god-fn (~530 lines, 24 inline schema blobs) into a single dispatcher array of `(target_version, name, fn)` tuples calling per-version free fns (`mig_v1_initial` through `mig_v24_project_renames`). Each migration is now a standalone fn that takes `&Connection` — easier to read, easier to test, easier to add new ones. Per-migration tests live as neighbors of their migration fns.
- **T-000096** Extracted `run_count_with_project_filter(&self, base_sql, fixed_params, project_ids)` helper in `db/dashboard.rs`. Four dashboard counter call-sites (`count_active_bugs`, `count_active_bugs_with_severity`, `count_closed_bugs_in_period`, `count_opened_bugs_in_period`) collapsed from ~10 lines of `params_from_iter` + `extend(ids_refs)` boilerplate each to 5–6 lines. More complex queries (avg-attempts, top-hot, bugs-per-day, category-efficiency) kept their own SQL where the helper shape didn't fit.
- **T-000097** Split `src-tauri/src/sync.rs` (3,054 lines, ~30 free fns) into `src-tauri/src/sync/` directory: `mod.rs` (16-line barrel) plus 5 domain sub-modules — `fs` (path safety + file primitives, 334 lines), `requirements` (rename-replay + nested-folder migration, 489 lines), `claude_md` (CLAUDE.md / project.md section rendering, 872 lines), `bugs` (Bug MD↔DB sync, 900 lines), `tasks` (Task MD↔DB sync, 502 lines). No visibility promotions needed.
- **T-000102** Split `src-tauri/src/models.rs` (715 lines, 48 structs) into `src-tauri/src/models/` directory: `mod.rs` (barrel) + 10 sub-modules (`core`, `bugs`, `dashboard`, `deploy`, `graph`, `stats`, `sync`, `tasks`, `templates`, `timeline`). Frontend `src/lib/types.ts` (405 lines) collapsed to a 14-line barrel re-exporting from new `src/lib/types/*.ts` mirroring the Rust split. All 40 `from '$lib/types'` call-sites compile unchanged via flat `export *`.

### Tests
- 314 cargo (unchanged), 50 vitest (unchanged), svelte-check 0/0/0 on 454 files (was 444 — the new `types/*.ts` files counted).

## [0.29.2] — 2026-05-14

Hotfix collected from the multi-deploy Go dogfood session. Six bug-fixes and one template-rule clarification across the sidebar, deploy screen, dashboard, and Go template.

### Fixed
- **B-000006** DeployScreen drill-down state (env detail) survived repo navigation — switching to another repo in the sidebar kept rendering the first repo's env. Wrapped `<DeployScreen />` in `{#key repo.id}` so the component remounts on repo change and the drill-down resets.
- **B-000007** Reorder ▲/▼ buttons targeted a stale selection right after creating a new project. `handleCreateProject` now clears `selectedRepoId`, focuses the new project, and opens its screen — the reorder buttons become immediately actionable on the just-added project.
- **B-000008** Deploy tab content was clipped below the viewport when the env list or secrets table grew long. `RepoDetail` deploy tab was missing the scroll-container wrapper that Secrets and Stats tabs already had; added `.deploy-wrapper` mirroring that pattern.
- **B-000009** Secret value entry: typing-and-tabbing in DeployTable's per-secret input ran `await load()` after every save, full-reloading the list and stealing focus from the next textarea. Replaced with optimistic local update. SecretsPanel per-existing-secret edit moved to the same per-row autosave pattern so both screens share one mechanic.
- **B-000010** Generated workflow had empty `build-args:` and `docker run -e` lines even after marking secrets for inclusion. Root cause: `ensure_deploy_secrets_populated` defaulted unknown secrets to `role="deploy"` (which neither build nor runtime filters pick up). Changed default to `runtime` since meta hints already cover deploy infrastructure (SSH/NPM) and explicit build (Flutter API_BASE_URL); whatever the user adds outside hints is overwhelmingly app config. Also moved the `"-alpine"` suffix out of the Go Dockerfile template into the `GO_VERSION` value (default `"alpine"` = latest stable Go on alpine via Docker Hub auto-track); the prior `golang:@@GO_VERSION@@-alpine` template was brittle on empty values, `"latest"`, and double-suffix from user-entered full tags. Empty-required validation: meta.json now accepts `"optional": true` per placeholder; DeployDetail highlights empty required fields with a red border and blocks Generate. Go template: `migrations` COPY uncommented by default (Go web servers typically embed them), `@@BUILD_ARGS@@` / `@@DOCKERFILE_ARGS@@` placeholders wired through `docker/build-push-action` and Dockerfile builder stage.
- **B-000011** Top-3 hot projects meta line ("0 crit / 0 maj / 2 act") was hardcoded English. Moved to i18n keys (`крит` / `важн` / `актив` in Russian).
- **B-000012** Same stale-selection root as B-000007 but via `openProject` and `clickProjectInCollapsed` — clicking on a sibling project after moving a repo kept ▲/▼ targeting the moved repo. Both functions now clear `selectedRepoId` before setting `selectedProjectId`.

### Changed
- **Template wording** for the confirmed-bug cleanup rule in the global CLAUDE.md template — cleanup now fires on the user signal ("посмотри баги", "I added bugs", etc.) instead of "next time the LLM edits bug-reports.md for any unrelated reason". Confirmed rows no longer linger across sessions.

### Tests
- 314 cargo (was 311) — +1 each for default-role-runtime, GO_VERSION bare-alpine, GO_VERSION 1.26-alpine regression rename.
- 50 vitest, svelte-check clean.

## [0.29.1] — 2026-05-13

Patch release. Secrets input UX fixes across all three entry points so SSH-key-style multi-line values, inline `# comments`, and quoted values work consistently in the bulk `.env` paste, the per-repo-secret edit box, and the per-deploy-secret override box.

### Added
- `secrets-parser` now accepts dotenv-style single-line values: surrounding `"..."` / `'...'` quotes are stripped, `\n \r \t \\ \"` escape sequences are decoded inside double-quoted values (single quotes stay literal), and inline `# comment` after a value is dropped when preceded by whitespace. Triple-quote `"""..."""` block form unchanged. SSH keys can now be entered as a one-row value via `\n` escapes.

### Fixed
- DeploySecretsTable per-env override-value box was an `<input type="password">` and could not accept multi-line paste at all — making SSH_KEY override impossible without first creating an empty secret elsewhere and editing it in the repo-side box. Swapped to a `<textarea>` with `-webkit-text-security: disc` masking, matching the SecretsPanel per-secret-box pattern. The two boxes are now visually consistent.

### Tests
- +11 vitest cases in `secrets-parser.test.ts` covering inline comments, quote stripping, escape decoding, single-vs-double-quote semantics, unclosed-quote errors, and an SSH-key one-row round-trip. 50 vitest total, svelte-check clean, 311 cargo unchanged.

## [0.29.0] — 2026-05-13

Pre-screenshot polish bundle for the public launch. Two deferred review items (P7, KPI/StatsSummary drift) closed; multi-deploy Go isolation pinned by integration tests.

### Added
- **T-000092** `project_renames` table (migration v24) — symmetric to `repo_renames` but scoped to a project. `update_project` logs name changes; sync-preamble replays them as `microservice-api/<old>/ → <new>/` folder renames on parent server side. `repo_renames` did not cover this because the folder is keyed by project name (`projects.name`), not by repo canonical name. Idempotent via fs checks; collision (both `old/` and `new/` exist) surfaces as a manual-intervention warning. Flow doc `docs/flows/api-handlers-sync.md` updated with the rename-replay section.
- Multi-env Go integration coverage — three new tests in `render_deploy_tests` verifying same-repo prod+test renders produce env-isolated `deploy-{name}.yml` (network, branch, domain, container, runtime secrets) and an env-agnostic shared `Dockerfile` when repo-wide placeholders (`GO_VERSION`, `BINARY_NAME`, `ENTRY_POINT`, `APP_PORT`) match.

### Fixed
- **T-000091** Dashboard KPI5 `avg attempts` (reads `bug_events.entered_testing` count) drifted from per-repo / per-project `StatsSummary` (reads `bugs.fix_attempts`) after running `migrate_bugs_for_repo` on a freshly added repo with existing MD content. Cause: `migrate_bugs_transactional` inserted bugs with `fix_attempts > 0` but did not create synthetic `bug_events`, and `backfill_bug_events_for_existing` had a global one-shot guard that skipped subsequent migrations. Fix: synthesize `created` + N×`entered_testing` + optional `confirmed` events inside the migration transaction, mirroring the backfill logic. Invariant `COUNT(entered_testing) == bugs.fix_attempts` now holds at all entry points.

### Tests
- 311 cargo tests (was 308): +1 for T-000091 migration→events synthesis, +3 for T-000092 project_renames, +3 for multi-env Go isolation, +1 for v24 schema migration.

## [0.28.0] — 2026-05-12

Second code-review pass after v0.27.1 — 32 findings across 4 domains (bug/stats/dashboard, tasks/timeline/datagrid, cross-repo sync, deploy/secrets/settings). All addressed across 4 batches: 3 critical + 8 high + 11 medium + 9 polish + 2 deferred (P7 microservice-api rename-replay needs new schema, P10 COMPOSE_SERVICE copy UX is a design call). No new features, no schema migrations.

### Fixed (critical)
- **C1 | `confirm_requirement` deletes wrong microservice pair when NNN collides** — the server→MS branch iterated all connected microservices and deleted from the first whose filename matched, with no target disambiguation. Each MS's REQ folder has its own NNN counter, so two MSes can independently carry `REQ-001.md` — clicking ✓ on one row erased the sibling. Tauri command now takes `target_repo_id`, frontend `SyncScreen.handleConfirm` resolves both source and target via `getDisplayName`. Includes M8 fix: source role other than `client*`/`server` (e.g. `tool`/`landing`) now returns an explicit error instead of silently no-op'ing through the MS iteration path.
- **C2 | `valid_transition` allowed LLM `testing → confirmed`** — the function whitelisted the transition while comments insisted it was UI-only. An LLM writing `status: confirmed` in `bug-reports.md` bypassed the user-verification gate. Removed `testing → confirmed` and `testing → rejected` from the whitelist; `confirmed_at` is now set exclusively via the UI `resolve_bug` command. Test renamed to `test_reconcile_rejects_testing_to_confirmed_from_md` asserting the new guard.
- **C3 | `DeployDetail.load()` early-exited when `repo` not yet reactive** — `repo` is `$derived` from `$allRepos`, which loads in parallel and may not be ready at mount. The previous `if (!env || !repo) return` silently skipped the GitHub Environment auto-ensure and branches fetch with no retrigger. Split into env-fetch on mount + `$effect`-driven GitHub-side bootstrap that fires once when both are resolved.

### Fixed (high)
- **H1** `refreshBugs` now skips reconcile for remote-only repos (no `local_path` → `bugs_migrated_at IS NULL` → error toast on every bug-tab remount). Store tracks `currentRepoHasLocalPath` alongside `currentRepoId`.
- **H2** Removed legacy `backend` / `network` from `BugItem.defaultCategories` (not in the 9-value DB CHECK enum since v0.13.12 — picking them produced raw SQLite error toasts). Added `auth` instead.
- **H3** Timeline `kind` / `repo_ids` / `project_ids` filters pushed into the SQL `WHERE` clause before `LIMIT/OFFSET`. Previously Rust filtered after fetch — when most rows got filtered out the frontend saw `r.length < PAGE_SIZE` and stopped paginating with matching events still on later pages. `search` substring stays in Rust.
- **H4** DoneTab date column + default sort changed from `created_at` (task-creation date, often months before completion) to `updated_at` (set by `update_task_source` on todo→done transition).
- **H5** Historical done.md entries with empty `dt.date` (no section header context) now fallback to `done.md` mtime instead of `todo.md` mtime — the entry originated in done.md, not todo.md.
- **H6** `sync_tasks_for_repo` now resolves split-state where the same `task_id` exists in both `todo` and `done` sources (post-crash or manual MD edit listing in both files) by dropping the `todo` duplicate. `done` wins because it reflects later intent. +1 unit test.
- **H7** `delete_pat` also wipes the legacy keyring entry (`github-repo-manager`). Without this, `migrate_legacy_pat` resurrected the deleted token on the next cold start.
- **H8** `write_deploy_files` Timeline event records `written.len()` rather than `files.len()` — path-rejects and write failures no longer inflate the metric. Migrated event details emission to `serde_json::json!` for consistency with H4 batch from v0.27.1.

### Fixed (medium UX)
- **M2** BugItem comment row now visible whenever a comment exists, not only when `fix_attempts > 0`. Previously a comment set in `created` / `in-progress` state was invisible until the first testing transition.
- **M3** Timeline removed double `loadFirstPage` on deep-link mount — the `$effect` already fires on initial mount.
- **M4** DataGrid filter dropdown closes on outside-click + Esc via `svelte:window` listeners.
- **M5** Server's `docs/api.md` absent during client sync is no longer pushed to `errors` — silent skip, symmetric with `handlers.md`.
- **M6** `init_docs_for_repo` surfaces `"(project.md + CLAUDE.md skipped — repo has no project assigned)"` in the result list for orphan repos so the user sees what was intentionally omitted.
- **M7** `replay_rename_in_dir` returns `RenameOutcome { Renamed, NoOp, Collision }` enum instead of ambiguous `bool`. Callers now surface collisions as explicit warnings.
- **M9** DeployDetail Generate button reflects workflow-stale state after secret role changes (build/deploy/runtime cycle). Amber tint + "Regenerate workflow files" label + tooltip. `DeploySecretsTable` takes `onRoleChange` callback prop.
- **M10** DeployDetail surfaces a YAML-unsafe-value warning before the Generate button when placeholder values contain chars that break YAML in unquoted scalars (`:`, `#`, quotes, backticks, newlines, leading flow-indicator).
- **M11** Updater silent-mode preserves error category for `network` / `signature` / `unknown` — only `notFound` (expected on private repo pre-public-flip) stays quiet so the About card can surface real errors on next user-initiated check.

### Fixed (polish)
- **P1** `addBug` store default severity aligned 'minor' → 'medium' to match the UI call site.
- **P2** Comment on `active_bugs` KpiCard explaining intentional absence of compare-period delta (point-in-time metric).
- **P3** `DashboardTopHot` meta line shows `major` count alongside `critical` and `active` — backend sort weighs critical → major → active.
- **P4** DataGrid search placeholder migrated to i18n key `grid.searchPlaceholder` (ru + en).
- **P5** `parse_done_entries_in_period` accepts legacy `DD.MM.YYYY` / `DD/MM/YYYY` date headers (matching `parse_done_tasks` tolerance). Normalizes to `YYYY-MM-DD` for range comparison.
- **P6** "No clients found" warning suppressed when server is also missing — server-only build-out phase is a legitimate state, no warning spam.
- **P8** `SyncScreen.loadRequirements` migrated from `onMount` to `$effect(projectId)` — reloads on project change without unmount.
- **P9** `AppDefaultsScreen.excludeFiles` moved to a module-level const so `TemplateEditor.$effect` doesn't re-fire on every parent render.

### Deferred
- **M1** (Dashboard KPI5 vs `StatsSummary` avg attempts drift) — theoretical, only after `migrate_bugs_for_repo` imports without backfilling `entered_testing` events. Requires structural rework to unify data sources. → v0.29.0.
- **P7** (`microservice-api/<project-name>/` rename-replay) — needs a new `project_renames` table (`repo_renames` is repo-scoped only). → v0.29.0.
- **P10** (COMPOSE_SERVICE copy-from-CONTAINER_NAME direction) — design call (CONTAINER_NAME often = COMPOSE_SERVICE + env suffix); a better tooltip is the right fix, future Deploy UX pass.

### Tests
- 303 cargo passing (+1 from H6 split-state test, net after C2 test rename)
- svelte-check 444 / 0 errors / 0 warnings

## [0.27.1] — 2026-05-12

Patch release: code review fixes — 2 critical bugs + 5 important issues + 1 cleanup. No new features, no schema changes.

### Fixed
- **Critical | ✓ Confirm button in SyncScreen silently no-ops for all GitHub-backed repos** (regression since v0.25.0). After B-000001 fix `Repository::display_name()` was changed to return the last segment of `github_name` (`web-app-client`, not `owner/web-app-client`), but `findRepoId` in [SyncScreen.svelte:57](src/lib/components/SyncScreen.svelte#L57) was still matching against the full `r.github_name`. Result: every confirm click on a GitHub repo returned `null` and exited silently — backend was never called. Fix: replace the comparison with `getDisplayName(r) === name` so the TS side mirrors Rust's semantics. Local-only repos accidentally worked through the description fallback.
- **Critical | Path traversal in `write_deploy_files`** — `meta.json` `file_targets` was joined onto repo root without validating against `..`-escapes or absolute paths. A user-edited template via TemplatesScreen could write outside the repo root. New helper [sync::is_safe_subpath](src-tauri/src/sync.rs) rejects absolute paths, drive letters, `..`, and root-component paths. Applied as a guard in [write_deploy_files](src-tauri/src/lib.rs#L2553) (rendering loop) and in [read_repo_files](src-tauri/src/lib.rs#L2532) (symmetric read-side). +4 unit tests (`test_is_safe_subpath_accepts_normal` / `_rejects_parent_dir` / `_rejects_absolute` / `_rejects_windows_absolute`) → 302 total.
- **`NaiveDate::succ_opt().unwrap()` panic risk** on Dashboard daily flow loops — replaced with `match ... break` in [lib.rs:1021](src-tauri/src/lib.rs#L1021) and [db.rs:2951](src-tauri/src/db.rs#L2951). A malformed or far-future date filter (`9999-12-31`) would have eventually overflowed and panicked the IPC thread.
- **JSON injection in event details column** — `record_deploy_secret_event` and `record_secret_event` interpolated `secret_name` / `action` directly into a raw JSON string via `format!`. Replaced with `serde_json::json!({...}).to_string()` so quotes and special chars in input no longer corrupt the stored JSON.
- **`SyncResult.errors` false positives for non-standard projects** — `sync_project` unconditionally pushed "No server found" / "No clients found" even when the project was a microservice (intentionally has neither). Now scoped to `project_type == "standard"`. UI warning toasts on every sync of a microservice project stop firing.
- **`migrate_flat_to_nested` partial-copy rollback** (sync.rs Case C, multi-parent same-content branch) — if `fs::copy` failed mid-loop after earlier copies succeeded, the code early-returned leaving ghost files in some parent subfolders. Now rolls back successful copies and emits a warning, leaving the flat source intact for retry on the next sync.
- **`read_repo_files` accepted raw `local_path` from frontend** — refactored to take `repo_id` and look up the local path from DB (mirrors `read_repo_file`'s shape). Closes a wider-than-necessary read surface; the frontend caller in [DeployDetail.svelte:176](src/lib/components/DeployDetail.svelte#L176) updated to pass `repo.id`.

### Removed
- Dead TypeScript interface `DeployManifest` in [types.ts](src/lib/types.ts) — corresponding Rust type was replaced with `DeployEnvironment` in v0.18.0. Nothing imported it anymore.

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
