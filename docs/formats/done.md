# Format: `docs/done.md`

**Schema version**: v2 (since v0.13.9). No `[x]` checkbox — the file itself is the "done" list. Date lives in a section header that groups tasks by completion day.

## Structure

```markdown
# Done

## 2026-04-21
- D-001 | Refactor SecretsPanel | v0.13.9
- T-045 | Fix login regression | v0.13.9
-  | Tiny doc tweak | v0.13.9

## 2026-04-20
- T-040 | Format alignment round | v0.13.8
```

- `## <date>` — section header in `YYYY-MM-DD` format (canonical). Tolerated on parse: `DD.MM.YYYY`, `DD/MM/YYYY`, and "prefix-word + date" combos like `## День 29.03.2026` (for legacy files). The parser extracts the first date-looking word from the header.
- `- …` — task line. Applies to the nearest preceding `## <date>` header; if none, `date` is empty.

## Line format (3 pipe-separated fields)

```
- <id> | <description> | <version>
```

## Fields (3 per line + 1 inherited)

| # | Field | Type | Required | Example |
|---|-------|------|:--------:|---------|
| 1 | `id` | String (task/feature/done id) | — (may be empty) | `T-042`, `F-022`, `D-001`, or `` |
| 2 | `description` | String (may contain escaped `\|` and `\n`) | ✓ | `Refactor SecretsPanel` |
| 3 | `version` | String, SemVer tag or similar | — (may be empty) | `v0.13.9`, `0.13.9`, or `` |
| — | `date` | Inherited from preceding `## <date>` header | — (empty if no header) | `2026-04-21` |

### Auto-id `D-NNN`
If the `id` slot is empty, the parser assigns `D-001`, `D-002`, … **in-memory only** — the file is NOT rewritten. Counter resets per file. Prefix `D-` is reserved for auto-generated done ids (no collision with user's `T-`/`F-`/`B-` series).

### Tolerant 2-field fallback
A line with 2 pipe fields (`- <description> | <version>`) is accepted as "no id" — first field becomes description, id auto-assigned.

## LLM / AI policy

Append-only from the LLM perspective. Tasks move here on completion. Direct edits uncommon. User may reorganise manually.

## Escape rules

- Literal `|` inside description → `\|`
- Literal newline → `\n`

## Migration from old 4-field format (pre-0.13.9)

Old files with `- [x] <id> | <description> | <date> | <commit>` (4 fields, `[x]` checkbox, inline date, commit SHA) are tolerated by parser: the inline `date` is dropped (section-header date wins — or falls back to empty), `commit` is treated as `version`. Checkbox prefix is stripped if present. User is free to rewrite in the new shape at any time; app does not auto-migrate files.
