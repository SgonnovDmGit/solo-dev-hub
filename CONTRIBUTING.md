# Contributing to Solo Dev Hub

Thanks for considering a contribution. This document covers what you need to build the app locally, how the code is organised, and how to send a PR.

## Build prerequisites

- **Node.js v18+** — frontend toolchain (SvelteKit + Svelte 5 + Vite)
- **Rust** — install via [rustup](https://rustup.rs); Tauri builds against the stable channel
- **Microsoft C++ Build Tools** — required by Tauri's Windows build (install via the Visual Studio Installer, "Desktop development with C++" workload)
- **WebView2 Runtime** — pre-installed on Windows 11; on Windows 10 install from Microsoft

The current build pipeline targets Windows x64. Tauri supports macOS and Linux architecturally, but no non-Windows CI is wired up yet.

## Getting started

```bash
git clone https://github.com/SgonnovDmGit/solo-dev-hub.git
cd solo-dev-hub
git checkout dev          # PRs target dev, not master
npm install
npm run tauri dev         # opens the app with hot reload
```

## Project layout

| Path | Purpose |
|---|---|
| `src/` | SvelteKit frontend — components, stores, i18n, Tauri command bindings |
| `src-tauri/src/` | Rust backend — Tauri commands, SQLite, file sync, keyring, MD parsers |
| `src-tauri/templates/` | Bundled language/global templates (.gitignore, deploy YAMLs, CLAUDE.md sections) |
| `docs/` | Developer documentation (`RELEASING.md`, `flows/`, `formats/`, etc.) |
| `CLAUDE.md` | AI rules for the project (read this if you use an AI agent) |

A deeper architecture map lives in [CLAUDE.md](CLAUDE.md) — the project-level AI instructions double as the most up-to-date inventory of components, tables, and key decisions.

## Code style

### Rust (`src-tauri/`)

- `snake_case` for functions and variables, `PascalCase` for types — standard Rust convention
- Run `cargo fmt` before every commit (no opinion knobs — defaults only)
- Comments in **English** (code) — UI-facing strings live in i18n, not in comments
- Tests next to the code in `_test`-style modules; integration via `cargo test --lib`

### TypeScript / Svelte (`src/`)

- `camelCase` for variables and functions, `PascalCase` for component files (`RepoDetail.svelte`)
- `npm run check` (svelte-check) must pass — type errors are not landed
- Svelte 5 runes (`$state`, `$derived`, `$effect`) are the default; legacy reactive `$:` only when working in a file that already uses it
- Comments in **English**, UI strings in `src/lib/i18n/translations.ts` (both `ru` and `en` keys required)

### Commits

- **Conventional Commits** — `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`, `perf:`, `build:`, `ci:`
- Subject in imperative mood, ≤ 70 characters, no trailing period
- Body explains *why*, not *what* (the diff already shows what)

The commit-message contract is documented in detail in the global AI rules section of [CLAUDE.md](CLAUDE.md) (search for `# Commit messages`).

## Tests

```bash
cd src-tauri && cargo test --lib    # ~294 Rust tests — required before any backend PR
npm test                             # vitest frontend (~40 tests)
npm run check                        # svelte-check — required before any frontend PR
```

Backend changes without test coverage will be asked to add tests. Frontend tests are encouraged where there is logic worth covering (date math, parsers, store derivations) but not required for purely presentational changes.

## Pull requests

- **Target branch is `dev`**, not `master`. `master` is fast-forwarded only at release-tag time.
- One feature or fix per PR. Multi-feature bundles are fine when they form one logical change (e.g. a coordinated UI + backend rename).
- The PR description should answer: what changed, why, what was tested. Reference task IDs from `docs/todo.md` (`T-NNNNNN` / `F-NNNNNN`) or bug IDs from `docs/bug-reports.md` (`B-NNNNNN`) where applicable.
- Run `cargo test --lib` and `npm run check` locally before pushing — CI runs the same checks, but a failing CI is a slow feedback loop.
- For UI changes, walk the affected screens by hand in `npm run tauri dev` and call out in the PR what you smoke-tested.

## Working with an AI agent

The project is built with AI agents in mind. If you use Claude Code, ChatGPT, Copilot, or similar:

- **Read [CLAUDE.md](CLAUDE.md) first** — it's the canonical AI rules file with file formats, contracts, workflows, and per-project conventions
- The bug status workflow (`created` → `in-progress` → `testing` → `confirmed` / `rejected`) is *enforced by the app*; AI agents can only edit `status` and `comment`, never `description` / `severity` / `category` / `fix_attempts`
- Per-task workflow uses the global "Phase work workflow" section of CLAUDE.md (chat → spec → self-review → user OK → impl) for non-trivial scope; one-line fixes go via the fast lane

## Releases

Maintainer-only — see [docs/RELEASING.md](docs/RELEASING.md) for the runbook (branch flow, tagging, CI signing keys, hotfix flow). Contributors do not need to touch any of this.

## License

By contributing, you agree your contribution is licensed under the [MIT License](LICENSE).
