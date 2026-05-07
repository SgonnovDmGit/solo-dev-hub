# User-only addon: `docs/api.md` spec

**Not bundled in the app's `claude.md.global.tmpl`** — this is a personal convention of the user for server-side projects, not enforced or parsed by Solo Dev Hub. Copy-paste the block below into your `~/.claude/CLAUDE.md` manually if you want it as a global rule.

Reason for exclusion: `docs/api.md` applies only to server-type projects. Bundling it globally would clutter the LLM context in client/tool/microservice projects where there's no API surface to document.

---

## Block to paste

```markdown
# `docs/api.md` (server projects only)

## Access model

### Protection levels
| Level | Guard | Description |
|---|---|---|
| `admin` | Admin key | System management endpoints |
| `auth` | Client auth (JWT / session) | Endpoints for authenticated users |
| `public` | None | Open endpoints |

### Access matrix
| Role | admin-read | admin-write | auth-read | auth-write | public |
|---|:-:|:-:|:-:|:-:|:-:|
| Admin | ✓ | ✓ | ✓ | ✗ | ✓ |
| Client | ✗ | ✗ | ✓ | ✓ | ✓ |
| Unauthenticated | ✗ | ✗ | ✗ | ✗ | ✓ |

**Note:** Admin has `auth-write: ✗` by design — all admin write operations go through dedicated `admin-write` endpoints (clean separation of concerns), not through client `auth-write` endpoints.

## Endpoints (grouped by access level)

Tables with columns: `Method | Path | Description | Status`.

Example:

### auth-read — fetch data (client)

| Method | Path | Description | Status |
|---|---|---|---|
| GET | /api/v1/profile | User profile | implemented |

## Endpoint statuses
- `implemented` — shipped and tested
- `in-progress` — currently being developed
- `planned` — scheduled, not started
- `deprecated` — marked for removal
```
