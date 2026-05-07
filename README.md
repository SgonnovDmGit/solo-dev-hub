# Solo Dev Hub

> 🇷🇺 Русская версия — [README.ru.md](README.ru.md)

**Solo developer's portfolio cockpit. Bugs, requirements, deploy — all in markdown.**

Managing 10 GitHub repos as one person comes with friction GitHub itself doesn't solve: bugs scattered across per-repo Issues, no portfolio-wide overview, multi-repo features (client + server + microservice) demand mental coordination, deploy automation gets hand-rolled in every new project — and the moment you delegate a bug fix to an AI agent, "the agent said it's done, I forgot to verify" silently becomes the failure mode. Solo Dev Hub is a single-window desktop app that organizes your portfolio, locks every bug into a verifiable AI-agent workflow, tracks tasks in commit-able Markdown, syncs requirements between repos, and bundles deploy automation — all under one roof.

<!-- TODO screenshot: hero — main app window with sidebar (project tree expanded), Dashboard tab open, period filter set to Quarter, KPI tiles + top-3 hot projects + daily flow chart visible. Width ~1200px, dark theme. -->

## Why?

Built for solo developers, indie hackers, and freelancers running 5+ active GitHub repos who don't want to:

- Pay for team-tier project-management SaaS (Linear / Jira) for one person
- Use GitHub Issues across N repos and lose any portfolio-wide view
- Hand-roll deploy YAML / Dockerfile in every new project
- Lose track of which repo has the most active bugs this week

**AI-ready by design.** Bugs, tasks, requirements, project metadata, and CLAUDE.md sections all live as Markdown inside your repos. Every AI assistant (Claude, ChatGPT, Copilot) reads your entire portfolio context without an API integration — `git clone` is the integration.

## Features

- **AI-agent bug closure with a safety net** — the bug status workflow (`created` → `in-progress` → `testing` → `confirmed` / `rejected`) splits roles cleanly: the AI agent takes a bug, applies a fix, moves it to `testing` with a comment describing what it did; **you** verify and click ✓ or ✗. The agent **cannot** edit `description`, `severity`, `category`, or `fix_attempts` — only `status` and `comment` are AI-writable. The attempt counter auto-increments on every `testing` transition, so "how many tries did this take" is honest history, not a self-report. Net effect: no bug falls through the gap between "agent said it's fixed" and "I forgot to check".
- **Portfolio dashboard** — period-filtered KPIs (open / closed / fix rate / attempts per period), top-3 hot projects, daily bugs/tasks flow charts, category efficiency bars. See your portfolio at a glance.
- **Markdown bugs** — every bug lives in `docs/bug-reports.md` of the affected repo. SQLite is the source-of-truth, the MD file is a 2-way-synced LLM-readable view. Severity, category, append-only event log per bug.
- **Cross-repo requirements** — `REQ-NNN.md` exchange between client ↔ server ↔ microservice. Sender writes the ask, recipient writes the receipt, the app handles file movement between repos. No GitHub Issues, no email threads.
- **Project graph** — visualize a project as a 1-hop graph: server in the center, repos and connected microservices around it. Click any node to navigate. Built on Cytoscape.
- **Multi-environment deploy** — generate Docker + GitHub Actions deploy pipelines per environment (prod / staging / test / any custom name) with native GitHub Environments integration and per-secret role/scope flags.
- **Tasks (todo.md / done.md)** — each repo has an append-only completion log auto-tagged with versions. Universal data grid: filter, sort, persist preferences per tab.
- **Activity timeline** — multi-source events (bugs, tasks, syncs, deploys, repo renames) across the entire portfolio. Date-range / kind / repo / search filters.
- **Templates** — per-language seeds for `.gitignore`, deploy YAML, CLAUDE.md sections. Customize once in the app, sync into every project.
- **PAT in OS keyring** — your GitHub token goes into Windows Credential Manager (OS-level), never SQLite, never `.env`, never a plaintext file.
- **Single .exe, ~11 MB** — Tauri v2 + WebView2. No Electron bloat. No daemon. No telemetry. The only background network call is the update-checker pinging GitHub Releases once on startup; everything else is on your explicit action.

<!-- TODO screenshot: RepoDetail with Bugs tab open, sidebar collapsed to icons, 4-5 sample bugs visible with different severities and statuses. -->

<!-- TODO screenshot: ProjectGraph for a project with a server repo (center) + 3-4 connected microservices (ring), dashed lines showing cross-project ms connections. -->

<!-- TODO screenshot: DeployScreen master view (table of deploy environments) + a drill-down DeployDetail for one environment showing per-secret role/scope flags. -->

## Tech Stack

- **Framework** — Tauri v2 (Rust backend + WebView2 frontend, single-binary distribution)
- **Frontend** — SvelteKit + Svelte 5 + TypeScript
- **Backend** — Rust: SQLite via `rusqlite`, file I/O for sync, Windows Credential Manager via `keyring`
- **GitHub API** — `@octokit/rest` (called directly from the JS side, never proxied through Rust)
- **Graph** — Cytoscape.js with concentric layout, theme-aware
- **i18n** — Russian (default) + English, ~390 type-safe keys, no runtime dependency
- **Autoupdate** — `tauri-plugin-updater` with Ed25519 signing; production builds via GitHub Actions on `v*` tag push

## Getting started

### Install

> Current builds are **Windows x64 only**. Tauri supports macOS and Linux architecturally; non-Windows builds may appear in the release pipeline by request.

1. Download `solo-dev-hub_<version>_x64-setup.exe` from the [Releases page](https://github.com/SgonnovDmGit/solo-dev-hub/releases)
2. Run the installer.
3. **First launch may show a Windows SmartScreen warning** ("Unrecognized publisher"). Authenticode code-signing is on the v2.0.0 roadmap. Until then: click "More info" → "Run anyway".

### First-time setup

1. **Generate a GitHub Personal Access Token** at [github.com/settings/tokens](https://github.com/settings/tokens) with these scopes:
   - `repo` — full repository access (read your repos, manage Actions secrets)
   - `workflow` — required for the deploy automation
   - `read:user` — read your profile info
2. Open Solo Dev Hub → **Settings** (cog icon) → paste the PAT → save. The token goes into Windows Credential Manager — never on disk in plaintext.
3. **Set your workspace root** — Settings → Workspace. This is the directory under which the app expects your repos to be cloned (e.g. `C:\Users\You\Development\`).
4. Click **🔄 Sync** in the sidebar. The app fetches your repo list from GitHub.
5. **Organize**: drag repos into projects in the sidebar, or click a repo to assign a role (server / client / microservice / landing / tool / etc.).

<!-- TODO screenshot: Settings screen with the four cards (PAT, Appearance, Workspace, Templates), PAT field obscured but the 👁 reveal toggle visible. -->

### Daily flow

- **Sidebar** shows your projects → repos. Click a repo → tabs for Bugs / Tasks / Done / Changelog / Stats / Secrets.
- **Add a bug** via "+ Add bug" — instantly committable in `docs/bug-reports.md` (the MD is a view; SQLite is the SoT).
- **Dashboard** (📊 in the sidebar) — portfolio-wide KPIs filtered by period and projects.
- **Timeline** (📅) — chronological event feed across the whole portfolio.
- **Deploy** — click on a deploy-capable repo → Deploy tab → set up environments + secrets → generate Dockerfile + workflow with one click.

## Development

### Prerequisites

- Node.js v18+
- Rust (via [rustup](https://rustup.rs))
- Microsoft C++ Build Tools (Tauri requirement)
- WebView2 Runtime (pre-installed on Windows 11)

### Local

```bash
npm install
npm run tauri dev          # local dev with hot reload
```

### Tests

```bash
cd src-tauri && cargo test --lib   # ~290 Rust tests
npm test                            # vitest frontend (~40 tests)
npm run check                       # svelte-check
```

### Production build

Production releases are built by GitHub Actions on `v*` tag push — never build locally for distribution (unsigned, no `latest.json`):

```bash
git tag -a v0.25.0 -m "v0.25.0"
git push origin master v0.25.0
```

The full release runbook (key rotation, CI troubleshooting, hotfix flow) — [docs/RELEASING.md](docs/RELEASING.md).

### AI rules

`CLAUDE.md` (gitignored) carries the in-project AI rules. The app's "Sync to ~/.claude/CLAUDE.md" feature pushes the global section into your user-level Claude Code config. Per-project CLAUDE.md sits in each repo's root.

## Roadmap

- **v0.25.0** *(current cycle)* — pre-rebrand cleanup: display-name flip, branches workflow, README polish.
- **v1.0.0** — public launch. Technical identifier rebrand, repo flips from private to public, README and Releases become visible to the world.
- **v2.0.0** — Windows Authenticode code signing (removes the SmartScreen warning) + read-only API viewer screen.

## Support development

The app is free and ad-free. If it saves you time, consider supporting development:

- **Boosty** — [boosty.to/sgonnovdm/donate](https://boosty.to/sgonnovdm/donate) (RUB / cards / СБП)
- **TON** — `UQA-0I3SN2vw8F2ZzEoOTXT36-ToF0mu4Yp4_6pVmsR_dI0S`

Or use the in-app About screen — one-click links and copy-to-clipboard for the TON address.

## License

[MIT](LICENSE) © 2026 Sgonnov D.A.

Built with [Tauri](https://tauri.app), [SvelteKit](https://kit.svelte.dev), and AI assistants.
