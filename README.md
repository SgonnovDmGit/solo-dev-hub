# Solo Dev Hub

Personal Windows desktop application for managing GitHub repositories. Connects to a GitHub account, allows organizing repositories into projects with assigned roles, tracks bugs in Markdown files, syncs requirements between repos, and provides dashboard/statistics.

## Tech Stack

- **Framework:** Tauri v2 (Rust backend + WebView2 frontend)
- **Frontend:** SvelteKit + Svelte 5 + TypeScript
- **Backend:** Rust (SQLite, file I/O, keyring, sync)
- **Database:** SQLite (single local file, migrations v1–v20)
- **GitHub API:** `@octokit/rest` (TypeScript)
- **i18n:** Russian (default) + English, ~390 type-safe keys
- **Autoupdate:** `tauri-plugin-updater` with Ed25519 signing, GitHub Releases as update endpoint
- **Build output:** Single .exe, ~11 MB (production builds via GitHub Actions on `v*` tag push)

## Features

- Connect to GitHub via Personal Access Token (PAT), stored in Windows Credential Manager
- Fetch and cache user-owned repositories with pagination
- Organize repos into projects with roles (server, admin_client, client, test_client, microservice, landing, tool, other)
- Microservices: connect to multiple projects (many-to-many), not included in project
- **Bugs (v0.16.0+):** SQLite = source of truth; `docs/bug-reports.md` is a 2-way-synced LLM-facing view. Event log in `bug_events` table (v0.17.0+) captures each status transition with RFC3339 timestamps. 6-digit IDs (`B-000001`, cap ~1M), history preserved (confirmed bugs stay in DB, drop from MD view only). Severity (critical/major/medium/minor), category, status workflow (created → in-progress → testing → confirmed/rejected)
- Requirements sync: local file copying of REQ-*.md between repo folders (client↔server, server↔microservice)
- **Dashboard (v0.17.0+):** portfolio-level view with period (Week/Month/Quarter/Custom) + project filters; 5 KPI tiles with comparison to previous period; top-3 hot projects; per-day bugs (opened/closed) and tasks (done) charts; category efficiency bars
- Bug statistics per repo: KPI tiles + category efficiency bars (StatsSummary tab), computed via direct queries on `bugs` + `bug_events` tables
- Dark/light theme toggle
- Localization: Russian (default) + English
- Settings: PAT, workspace root, language, theme, rename history viewer
- Custom titlebar with window controls
- Drag-and-drop repos between projects
- Markdown export/import with legacy format support
- Confirmation dialogs, toast notifications, empty states with hints
- **Autoupdate** (v0.15.0+): silent check on startup + green CTA button in titlebar when update available, one-click install-and-relaunch, Ed25519-signed updates from GitHub Releases
- **Multi-environment deploy** (v0.18.0+): one repo → multiple deploy environments (prod/test/staging/any name). meta.json v4 role/scope secrets (build/deploy/runtime × repo/environment). Native GitHub Environments integration. Clone deployment with placeholder copy
- **Support development**: Boosty (RUB/cards/СБП) + TON wallet, in-app via About screen

## Project Structure

```
solo-dev-hub/
├── src-tauri/              # Rust backend
│   ├── src/
│   │   ├── lib.rs          # Tauri entry point, 70+ commands
│   │   ├── db.rs           # SQLite CRUD, migrations v1-v20
│   │   ├── models.rs       # Data structures (incl. Period, DashboardFilter, KpiCard, DailyFlowDay, TopHotProject, CategoryEfficiencyRow)
│   │   ├── export.rs       # Markdown bug report generation/parsing
│   │   ├── sync.rs         # Local file sync for requirements + rename replay
│   │   ├── template_render.rs # Deploy template renderer (@@VAR@@)
│   │   ├── template_seeder.rs # Bundle-seed templates
│   │   └── keyring_store.rs # PAT storage
│   └── Cargo.toml
│
├── src/                    # SvelteKit + TypeScript frontend
│   ├── lib/
│   │   ├── components/     # 31 Svelte components
│   │   ├── stores/         # 6 Svelte stores (ui, projects, repos, bugs, settings, updater)
│   │   ├── api/            # GitHub API client + Tauri command bindings
│   │   ├── i18n/           # Localization (ru/en, ~390 keys)
│   │   └── types.ts        # TypeScript interfaces
│   └── routes/
│       └── +page.svelte    # Root layout
│
├── .github/workflows/      # CI (release.yml: v* tag → build + sign + publish)
│
├── scripts/                # Build/release helpers
│   └── extract-changelog.mjs
│
├── docs/                   # Documentation
│   ├── doc1_global_rules.md
│   ├── doc2_claude_md_template.md
│   ├── doc3_github_manager_spec.md
│   └── RELEASING.md        # Release runbook (key rotation, CI troubleshooting)
│
├── Changelog.md
└── package.json
```

## Architecture

- **Rust backend:** SQLite CRUD, file I/O (sync, MD export), PAT storage (keyring), data models, dashboard queries
- **TypeScript frontend:** GitHub API calls, all UI rendering, state management via Svelte stores
- **Separation:** GitHub API is called from JS side (not proxied through Rust). PAT stored in OS keyring, not SQLite. Bugs stored in SQLite as source of truth (v0.16.0+); `docs/bug-reports.md` is a 2-way-synced LLM-facing view. Event log in `bug_events` table (v0.17.0+) captures each status transition.

## Development

### Prerequisites

- Node.js v18+
- Rust (via rustup)
- Microsoft C++ Build Tools
- WebView2 Runtime (pre-installed on Windows 11)

### Run

```bash
npm install
npm run tauri dev
```

### Build (local)

```bash
npm run tauri build
```

### Release (production)

Production releases are built by GitHub Actions on `v*` tag push — never build locally for distribution (unsigned, no `latest.json`).

```bash
git tag -a v0.15.1 -m "v0.15.1"
git push origin v0.15.1
```

See [docs/RELEASING.md](docs/RELEASING.md) for the full runbook including signing-key rotation.

### Test

```bash
# Rust tests (204 tests)
cd src-tauri && cargo test --lib

# Frontend checks
npm run check
```

## Scripts

| Command | Description |
|---------|------------|
| `npm run tauri dev` | Run dev server with hot reload |
| `npm run tauri build` | Build production .exe |
| `npm run check` | Run svelte-check |
| `cd src-tauri && cargo test` | Run Rust tests |
| `cd src-tauri && cargo check` | Check Rust compilation |
