# Flow: contributor mode (shared repos)

**Introduced in:** v1.20.0 (F-000044; T-000216 .. T-000225)
**Related files:**
- [src-tauri/src/sync/shared.rs](../../src-tauri/src/sync/shared.rs) — the sync of a shared repo: Phase A (`run_phase_a`), Phase B (`SharedSync::finish`), import (`import_incoming`), statuses, the Merge button, switching on and off
- [src-tauri/src/sync/shared_structure.rs](../../src-tauri/src/sync/shared_structure.rs) — `.sdh/project.json` and the join requests: validation, trust anchor, pin, roles, bands
- [src-tauri/src/sync/shared_canon.rs](../../src-tauri/src/sync/shared_canon.rs) — the owner's rule canon at a contributor (`adopt_main_canon`, `followed_rules`)
- [src-tauri/src/sync/merge_by_id.rs](../../src-tauri/src/sync/merge_by_id.rs) and [src-tauri/src/merge_driver.rs](../../src-tauri/src/merge_driver.rs) — the merge by id and the `--merge-driver` mode of the executable
- [src-tauri/src/git_sync.rs](../../src-tauri/src/git_sync.rs) — fetch, fast-forward, push, compare-and-swap `update-ref`, plumbing commits
- [src-tauri/src/sync/claude_md.rs](../../src-tauri/src/sync/claude_md.rs) — `shared_block_for`, the "Shared repo" block of the CLAUDE.md section
- [src-tauri/src/sync/skills.rs](../../src-tauri/src/sync/skills.rs) — `skips_rule_copies`: a contributor renders and collects nothing in a shared repo
- [src-tauri/src/sync/shared_e2e_tests.rs](../../src-tauri/src/sync/shared_e2e_tests.rs) — two installs, two clones, one bare remote in one process
- Schema: [repositories](../schema/repositories.md) (`shared_mode`, `shared_role`, `shared_status`, `shared_import`), [task_events](../schema/task_events.md) and [bug_events](../schema/bug_events.md) (`author`), [settings](../schema/settings.md) (`github_login`, `rules_from_main`), [rule_events](../schema/rule_events.md)
- Related flows: [rules.md](./rules.md), [bug-tracking.md](./bug-tracking.md), [repo-auto-files.md](./repo-auto-files.md)
- Spec (local, gitignored): `docs/superpowers/specs/2026-10-19-v1.20.0-contributor-mode.md`

## What it is

One person (the **owner**, role `main`) shares a repo with a helper (a **contributor**). Both run their own copy of the app on their own clone of the same GitHub repo. The transport is git itself: the shared files travel as commits, so the two installs never talk to each other and the app needs no server.

Shared files: `docs/todo.md`, `docs/done.md`, `docs/bug-reports.md`, the REQ folders, the announcement folders, `docs/InOutBox/`, `docs/server-api/`, `docs/microservice-api/`, `.sdh/` and — owner only — `docs/sdh_skills/`. Everything else (`CLAUDE.md`, `docs/project.md`, `docs/codex/`, `docs/superpowers/`) stays local to each install.

## Roles

The role is **derived, never set by hand**, and cached per repo in `repositories.shared_role`:

| Role | When | Commits |
|---|---|---|
| `main` | the owner switched shared mode on (`repositories.shared_mode`) and the GitHub login of this install is the repo owner | the shared files and `.sdh/`, to his dev branch and to every participant branch |
| `contributor` | `.sdh/project.json` is trusted and lists this install's login among `participants` | the shared files, to his own branch and to the owner's dev branch |
| `pending` | the file is trusted but does not list this install: a join request is waiting | only `.sdh/requests/<login>.json` |
| none | no structure file | nothing special: the repo is ordinary |

The login of the install is stored in the `github_login` setting when a token is validated. Without it nothing shared is processed (`shared_no_login`).

## Structure and requests

`.sdh/project.json` (format 1) holds the project (`name`, `type`), the owner (`login`, `dev_branch`), the repos with their roles, the connected microservices and `participants` (`login`, `branch`, `band_start`). Only the owner's install writes it, on every sync.

A helper asks to join with `.sdh/requests/<login>.json` (`login`, `branch`). His install plumbs only that file into the owner's dev branch and pushes. The owner approves in the "Participants" tab (`approve_join_request`): the participant appears in the structure with a number band, the request file is deleted by the owner's commit, and the helper becomes a `contributor` on his next sync. There is no "reject" button in this release.

## Trust anchor and pin

The file comes from a repo that may be written by the helper, so it is trusted only when `main.login` is the **owner of the repo on GitHub** (`validate_structure`). A helper cannot promote himself by committing his own `project.json`.

The first accepted structure pins the owner's dev branch (`pin_after_accept`, in `shared_import`). From then on only `origin/<pinned branch>` is consulted, and a structure naming another dev branch is `shared_untrusted`. Two candidate structures on different refs are also untrusted: none is ever picked. The owner's switch-off is read from the pinned branch too: its tip no longer carrying the file means the repo is ordinary again, whatever the contributor's own branch still holds.

Organisation repos are not supported: the owner of a repo in an organisation is the organisation, not a person (T-000227).

## One sync, Phase A then Phase B

`run_project_sync_in` runs the shared path around the ordinary steps:

**Phase A** (`run_phase_a`, before anything is written) — for every shared repo of the synced project, plus a throttled probe (every 10 minutes) of the role-less foreign repos:
1. register the merge driver in the clone's `.git/config` (idempotent, every sync);
2. `git fetch origin` (the system git credentials; the token is never given to git);
3. settle the role from the structure and the login;
4. fast-forward the recipient branches: the current branch with `git merge --ff-only` (only when no shared file is modified in the work tree), the others with `update-ref` and the old value as a compare-and-swap. Diverged means the status `needs_manual_merge`, never a force;
5. **import** what arrived by commit (below);
6. contributor only: take the owner's rule canon (below).

**The ordinary steps** — skeleton files, REQ copies, codex, peer channels — see the other flows.

**Phase B** (`SharedSync::finish`): write the structure or the join request, commit the shared files into the current branch (a pathspec commit) and into the other recipient branches (plumbing, per-file 3-way merge by id), `git push` the recipient branches in one command without `--force`, settle the status. `master` and `main` are refused at every layer (`git_sync`), and a remote hook in the e2e tests records any update of them or any non-fast-forward.

## Merge by id

`todo.md`, `done.md` and `bug-reports.md` are merged by entity, never as text: the same function serves the git merge driver (`merge=sdh` in `.gitattributes`, executable started as `app --merge-driver %O %A %B %P`) and the plumbing commit into a branch that is not checked out. For three versions of a row with an id:

- present on one side only: added;
- deleted on one side and unchanged on the other: deleted; deleted on one side and changed on the other: the changed row stays;
- a field changed on one side only: that change;
- a field changed on both sides differently: **status** goes to the further one along the work (task `open < in-progress < review`, a row in `done.md` beats one in `todo.md`; bug `created < in-progress < testing`, `rejected` after `testing`); every other field goes to the side doing the merge (`%A`). A conflict marker never appears.

An unrecognised line is kept verbatim from `%A`. Known limitation: the base of the plumbing merge is chosen per whole file by an age heuristic over both histories, which picks a wrong base when an identical whole file repeats three or more times in both histories (T-000226).

## Import as truth

The files that arrived by commit are the truth, the database follows (`import_incoming`). `repositories.shared_import` keeps, per shared file, the blob last imported or committed by this install. When HEAD's blob differs, the difference "stored to HEAD" is applied to the database: protected bug fields and new rows included, with the author in `task_events.author` / `bug_events.author` (the `SDH-Participant:` trailer of the commits, else the owner of the number band). Then the work tree is reconciled as always, so an uncommitted local edit of a protected field is restored while a committed one sticks. The first sync of a repo is a baseline and applies nothing.

## Number bands

Two installs must never allocate the same id. The owner has everything below the first participant band; a participant gets `500000 + k * 100000` for `k = 0..4` (up to five participants), one band for all prefixes (`T`, `F`, `O`, `M`, `B`), allocation takes the maximum inside the band (`band_for`, `next_numeric_id_in_band`). A pending helper cannot allocate. Every install's band is written into the CLAUDE.md section of the shared repo, as six digits without separators (for example `500000`–`599999`; the owner `000001`–`499999`).

## Statuses

`repositories.shared_status`, shown in the repo header and as a badge in the sidebar; a toast appears only when the status appears, not on every sync. The declaration order of `SharedStatus` is the priority when several apply:

`shared_untrusted` > `fetch_failed` > `push_failed` > `push_rejected` (a protected branch, `GH006`) > `needs_manual_merge` (offers the Merge button: `git merge origin/<branch>` with the driver, aborted on a conflict in code) > `no_own_branch` > `pending_approval`. `shared_no_login` stands alone.

## Rules: the owner's canon

The rule copies in `docs/sdh_skills/` of a shared repo are the owner's.

- **Owner:** renders them as in any repo and commits them (the markers `*.approved` and the reason files `*.conflict` are excluded from the pathspec).
- **Contributor and pending helper:** `skips_rule_copies` is true. The install renders nothing into `docs/sdh_skills/` of that repo, prunes nothing and does not collect hand-back markers from it. Copies an earlier version rendered untracked there are deleted when the role is settled (`drop_untracked_rule_copies`: `sdh-*.md` that git does not track), because git refuses a fast-forward over untracked files that the incoming commit would create.
- **Canon.** After the import, `adopt_main_canon` reads the tracked `docs/sdh_skills/<name>.md` at HEAD. For a name this install knows whose text differs from its canon, the text is stored as this install's custom canon (`templates`, `_global`, `skill.<name>.md.tmpl`), a `taken` event with the note `canon of main <login>` goes into [rule_events](../schema/rule_events.md), and the sync renders everywhere, so the contributor's other repos and `~/.claude/skills` carry it. When two shared repos carry different texts of one rule, the copy whose last commit touching it is newest wins and stays.
- **Locked.** The rule is remembered as followed in the setting `rules_from_main` (rule -> login, repo, commit time). The Rules screen marks it "owner's canon" and disables Save and Reset; the backend refuses with `rule_follows_main: <login>`; a hand-back marker of it in an ordinary repo is set aside as superseded. To change it the contributor asks the owner or hands the rule back from his own repo.
- **Released.** An entry is valid only while its repo is a contributor repo of this install. When the role is cleared or the repo is deleted, the entry is dropped on the next read and the rule is editable again; its text stays as the install's custom canon.

## Switching off

The owner switches the mode off (`disable_shared_mode`): `.sdh/` is deleted by a commit on his dev branch and on every participant branch, the managed `.gitignore` / `.gitattributes` blocks return to the ordinary form, and files already committed stay tracked (the existing "untrack gitignored files" dialog offers `git rm --cached`). A contributor's next sync finds no structure on the pinned branch, clears his role and status, deletes the stale `.sdh/` from his own branch and pushes; the repo is ordinary again and the local database is not cleaned. A deletion that did not reach the remote is retried on later syncs.
