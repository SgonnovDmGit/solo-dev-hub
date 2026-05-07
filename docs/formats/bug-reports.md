# Format: `docs/bug-reports.md`

**Schema version**: v2 (since app v0.13.0). Legacy v1 (10 fields) still parsed but deprecated — removed in v0.14.0 (F-028).

## Line format

```
- <id> | <date> | <description> | <severity> | <category> | <status> | <fix_attempts> | <comment>
```

Alternative prefix `- [ ] ` is also accepted (legacy). All fields are separated by ` | `.

## Fields (8)

| # | Field | Type | Required | Enum / example |
|---|-------|------|:--------:|----------------|
| 1 | `id` | String, `B-NNN` or `VB-NNN` | ✓ | `B-042` |
| 2 | `date` | ISO date `YYYY-MM-DD` | ✓ | `2026-04-19` |
| 3 | `description` | String (may contain escaped `|` and `\n`) | ✓ | `App crashes on login` |
| 4 | `severity` | Enum | ✓ | `critical` / `major` / `medium` / `minor` / `trivial` |
| 5 | `category` | String (free-form) | ✓ | `auth`, `ui_ux`, `logic` |
| 6 | `status` | Enum | ✓ | `created` / `in-progress` / `testing` / `rejected` / `confirmed` |
| 7 | `fix_attempts` | Integer ≥ 0 | ✓ | `0`, `3` |
| 8 | `comment` | String or empty | — (may be empty) | `Fixed in B-007 via upsert_repository_with_outcome` |

## Status workflow

```
created → in-progress → testing → confirmed (row deleted from file)
                                 → rejected → in-progress (attempts +1)
```

## LLM / AI policy

**LLM may edit only `status` and `comment`.** Reasons:
- `description` / `severity` / `category` are user-owned — set on bug creation, reflect user intent.
- `id`, `date` are immutable after creation.
- `fix_attempts` is managed by the app (incremented on `rejected` transition).

## Escape rules

- Literal `|` inside `description` or `comment` → written as `\|`.
- Literal newline inside a field → written as `\n`.
- On parse: regex-based split respecting `\|` (negative lookbehind). Then `\n` → newline, `\|` → `|`.

## Examples

Minimal valid row (no comment):
```
- B-001 | 2026-04-19 | Login crash | major | auth | created | 0 | 
```

Row with pipe + newline escapes:
```
- B-042 | 2026-04-19 | Regex /error\|warning/ breaks parser.\nSteps below. | major | logic | testing | 1 | Fixed via\|escape
```

## Legacy v1 (10 fields) auto-migration

Old files with 10 pipe-separated fields — `id|date|screen|description|reproduction|severity|category|status|fix_attempts|comment` — are parsed by merging `[screen] description \n\n Reproduction: <reproduction>` into the single v2 `description` field. Transparent at first save through the app.
