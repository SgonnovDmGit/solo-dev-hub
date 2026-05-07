# Format: `docs/todo.md`

## Line format

```
- [ ] <id> | <description> | <effort> | <priority> | <status>
```

Alternative: `- <id> | ...` (without checkbox prefix) — accepted too, provided `<id>` starts with `T` or `F`.

## Fields (5)

| # | Field | Type | Required | Enum / example |
|---|-------|------|:--------:|----------------|
| 1 | `id` | String, typically `T-NNN` (task) or `F-NNN` (feature) | ✓ | `T-042`, `F-022` |
| 2 | `description` | String (may contain escaped `|` and `\n`) | ✓ | `Add Go deploy template` |
| 3 | `effort` | Hours (integer or decimal) | ✓ | `2`, `0.5`, `8` |
| 4 | `priority` | Enum | ✓ | `critical` / `high` / `medium` / `low` |
| 5 | `status` | Enum | ✓ | `open` / `in-progress` |

Parser is enum-agnostic (accepts any string values) — the enums above are the **recommended** values for consistency across projects.

## LLM / AI policy

Currently user-owned. LLM may propose additions or status flips via user approval, but direct edits are out of scope. May evolve in v0.14.0 as app grows task-management features.

## Escape rules

Same as `bug-reports.md`: `|` → `\|`, newline → `\n`.

## Example

```
- [ ] T-042 | Add Go deploy template | 4 | high | open
- T-043 | Refactor sidebar | 2 | medium | in-progress
```
