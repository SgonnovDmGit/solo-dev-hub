# Format: managed section in repo `CLAUDE.md`

Auto-generated section inserted into each repo's `CLAUDE.md` at `sync_project` time. Delimited by markers so user-authored content outside survives untouched.

## Markers

```
<!-- manager:begin -->
...rendered section content...
<!-- manager:end -->
```

**Self-heal**: on orphan (only `manager:begin` OR only `manager:end`) — parser strips the orphan line, rest of file kept as user content, then the managed block is rewritten cleanly on next sync.

## Template

Source template: `src-tauri/templates/_global/claude.md.section.tmpl`. Placeholders (Handlebars-like `{{NAME}}`):

| Placeholder | Meaning |
|-------------|---------|
| `{{PROJECT_NAME}}` | Project name from `projects.name` |
| `{{PROJECT_TYPE_DISPLAY}}` | `📁 Standard` or `⚙ Microservice` |
| `{{REPO_ROLE_OR_UNSET}}` | Repo role (`server`/`client`/etc) or `—` |
| `{{PROJECT_DESCRIPTION}}` | `projects.description` or empty |
| `{{REPOS_TABLE}}` | MD table of project repos with roles |
| `{{MICROSERVICES_BLOCK}}` | Bulleted list of connected microservices |
| `{{PARENTS_BLOCK}}` | Parent projects (for microservice type) |

## Rules

- Content **between markers** is fully managed by the app — edits lost on next sync. Document this for LLM so it doesn't try to patch there.
- Content **outside markers** is 100% user-owned.
- If repo has no `CLAUDE.md` on sync — file is created with markers + rendered section only.

## LLM / AI policy

LLM **must not** edit content between `manager:begin` / `manager:end`. Any project-context overrides go outside markers (user-authored section).

Global instructions (`~/.claude/CLAUDE.md`) follow the same marker contract but render `Global AI instructions` placeholder section instead of per-project data.
