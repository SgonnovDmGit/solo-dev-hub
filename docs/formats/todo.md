# Format: `docs/todo.md`

## Line format

```
- [ ] <id> | <description> | <effort> | <priority> | <status>
```

Alternative: `- <id> | ...` (without checkbox prefix) — accepted too, provided `<id>` starts with `T` or `F`.

## Fields (5)

> Note: the current canonical line carries a 6th field `created_at` (`YYYY-MM-DD`) after `status` — see the global `~/.claude/CLAUDE.md` todo spec. The parser is lenient and reads 5-field legacy lines too.

| # | Field | Type | Required | Enum / example |
|---|-------|------|:--------:|----------------|
| 1 | `id` | String: `T-NNN` (task), `F-NNN` (feature), or operations classes `O-NNN` (technical) / `M-NNN` (marketing) | ✓ | `T-042`, `F-022`, `O-000001`, `M-000001` |
| 2 | `description` | String (may contain escaped `|` and `\n`) | ✓ | `Add Go deploy template` |
| 3 | `effort` | Hours (integer or decimal) | ✓ | `2`, `0.5`, `8` |
| 4 | `priority` | Enum | ✓ | `critical` / `high` / `medium` / `low` |
| 5 | `status` | Enum | ✓ | `open` / `in-progress` |

Parser is enum-agnostic (accepts any string values) — the enums above are the **recommended** values for consistency across projects.

## Operations classes (`O-` / `M-`) — non-code, not release blockers

`O-NNNNNN` (technical) and `M-NNNNNN` (marketing) capture **операционка** — work to be *done*, not *coded*, that the AI still needs execution context for. Same line format and independent per-prefix counters as `T-`/`F-`. Three rules set them apart:

- **Never a release blocker** — open `O-`/`M-` tasks do not gate a code release tag.
- **Never release-scoped** — their `version` is always empty; they don't inherit a `## vX.Y.Z` header (the app force-empties it even under one). In the Tasks/Done grids they show a distinct **Тип** icon (⚙ technical / 📣 marketing).
- **Placed first in the file** — under a plain `## Операционка` header above every version section. Belonging to no release, at the bottom they sit below everything planned and stop catching the eye, and these are the tasks nobody but the owner will recall.

## LLM / AI policy

Currently user-owned. LLM may propose additions or status flips via user approval, but direct edits are out of scope. May evolve in v0.14.0 as app grows task-management features.

## Escape rules

Same as `bug-reports.md`: `|` → `\|`, newline → `\n`.

## Example

```
- [ ] T-042 | Add Go deploy template | 4 | high | open
- T-043 | Refactor sidebar | 2 | medium | in-progress
```
