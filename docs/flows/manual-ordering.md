# Flow: Manual ordering of projects and repos (F-025)

**Introduced in:** v0.13.0

## Concept

`sort_order INTEGER` column (migration v14) on both `projects` and `repositories` tables is the authoritative position source. **User controls the order**; the app provides only:

1. **Initial defaults** when a record is created.
2. An **Auto-sort button** (destructive) to restore algorithmic ordering.

No "smart" re-sorting — user intent wins. `sort_order` is local to SQLite (not git-synced between machines).

## Initial defaults (on insert)

- **New project**: `sort_order = MIN(existing) - 10` → lands at the **top** of the sidebar. Replaces the session-only `freshProjectIds` logic from v0.10.0 (now persisted).
- **New repo** inserted into a project: `sort_order = MAX(existing in group) + 10` → lands at the **bottom** of the group.
- **Cross-project D&D move**: repo's `sort_order` is reset to `MAX(target group) + 10` → appears at the bottom of the destination.

## Populate on migration v14

```sql
UPDATE projects     SET sort_order = id * 10;
UPDATE repositories SET sort_order = role_priority_of(role) * 1000 + id * 10;
```

Where `role_priority_of` is the existing mapping: server=0, admin_client=1, client=2, test_client=3, microservice=4, landing=5, tool=6, other/NULL=99. Result: existing users get a visually familiar starting order.

## Operations

### ▲▼ arrows at hover

Each project row and repo row shows compact `▲▼` buttons when hovered. Click swaps `sort_order` with the immediate neighbour. **Wrap-around**:

- `▲` on the **first** item → moves to the end (`sort_order = MAX + 10`).
- `▼` on the **last** item → moves to the start (`sort_order = MIN - 10`).
- Buttons are **never disabled**.

Contextual tooltips: normal positions show "Move up"/"Move down"; edges show "Move to end"/"Move to start".

### D&D reorder within a group

Existing cross-project D&D (introduced in v0.4.x) is extended: drop target in the **same group** triggers `rebalance_repo_group(ordered_ids)`. This re-numbers all repos in the group as `10, 20, 30, ...` via a single `UPDATE ... CASE id WHEN ... END` query — atomic, ~5 ms for 30 items, ~20 ms for 200.

### Auto-sort button 🔤

In the sidebar header (next to ⊞/⊟). Click → `ConfirmDialog` → on confirm, `auto_sort_all()` resets every `sort_order` to the initial formula. Destructive — overrides all manual arrangements.

## Tauri commands

| Command | Purpose |
|---------|---------|
| `reorder_project(id, "up"\|"down")` | Swap with neighbour project, wrap at boundaries |
| `reorder_repo(repo_id, "up"\|"down")` | Swap with neighbour repo within project_id, wrap |
| `rebalance_repo_group(ordered_ids)` | CASE-update all ids in group to 10, 20, 30, ... |
| `rebalance_projects(ordered_ids)` | Same for the projects table |
| `auto_sort_all()` | Reset to initial role_priority formula |

## Frontend notes

- `Sidebar.svelte` uses `$projects` / `$allRepos` **directly** — no frontend derived sort. Rust `ORDER BY sort_order ASC, name ASC` is the single source of truth.
- `sortReposByRole` and the `freshProjectIds` Set were removed.
- `RepoDetail.svelte` tabs, BugNotes, Stats remain unrelated to sort_order.

## Rationale

Before v0.13.0, Sidebar ordering was a hardcoded sort by role + alphabetical. Users with non-trivial project structures (mixed roles, priority-based grouping preferences) couldn't express their mental model. Moving to user-controlled `sort_order` restores flexibility while keeping a sensible default via the initial formula + Auto-sort escape hatch.
