# repositories

## Description

A repository belonging to at most one project. May be GitHub-backed or local-only.

<details>
<summary><strong>Table Definition</strong></summary>

```sql
CREATE TABLE "repositories" (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,
            github_name TEXT UNIQUE,
            github_url TEXT,
            role TEXT CHECK(role IN ('server','client','test_client','admin_client','microservice','landing','tool','other')),
            description TEXT,
            language TEXT,
            last_pushed_at DATETIME,
            added_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            local_path TEXT,
            github_id INTEGER,
            deploy_target TEXT
         , sort_order INTEGER NOT NULL DEFAULT 0, bugs_migrated_at TEXT, tasks_migrated_at TEXT, deploy_repo_config TEXT NOT NULL DEFAULT '{}', autocommit_branch TEXT, codex_source_path TEXT, shared_mode INTEGER NOT NULL DEFAULT 0, shared_role TEXT
             CHECK (shared_role IN ('main','contributor','pending')), shared_status TEXT, shared_import TEXT)
```

</details>

## Columns

| Name               | Type     | Default           | Nullable | Children                                                                                                                                                                                                    | Parents                 | Comment                                                                                                                                                                                           |
| ------------------ | -------- | ----------------- | -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| id                 | INTEGER  |                   | true     | [repo_renames](repo_renames.md) [bugs](bugs.md) [deploy_environments](deploy_environments.md) [deploy_events](deploy_events.md) [sync_events](sync_events.md) [tasks](tasks.md) [peer_links](peer_links.md) |                         |                                                                                                                                                                                                   |
| project_id         | INTEGER  |                   | true     |                                                                                                                                                                                                             | [projects](projects.md) | ON DELETE SET NULL — the only non-cascading FK in the schema. Deleting a project orphans its repositories rather than destroying them.                                                            |
| github_name        | TEXT     |                   | true     |                                                                                                                                                                                                             |                         | UNIQUE, and nullable since v13 so local-only repos (never pushed to GitHub) can exist. The last path segment is the canonical folder name used for all cross-repo doc folders.                    |
| github_url         | TEXT     |                   | true     |                                                                                                                                                                                                             |                         |                                                                                                                                                                                                   |
| role               | TEXT     |                   | true     |                                                                                                                                                                                                             |                         | 'microservice' is still legal here but is legacy: v12 NULLed every such row when microservice became a project type rather than a repo role.                                                      |
| description        | TEXT     |                   | true     |                                                                                                                                                                                                             |                         |                                                                                                                                                                                                   |
| language           | TEXT     |                   | true     |                                                                                                                                                                                                             |                         |                                                                                                                                                                                                   |
| last_pushed_at     | DATETIME |                   | true     |                                                                                                                                                                                                             |                         |                                                                                                                                                                                                   |
| added_at           | DATETIME | CURRENT_TIMESTAMP | true     |                                                                                                                                                                                                             |                         |                                                                                                                                                                                                   |
| updated_at         | DATETIME | CURRENT_TIMESTAMP | true     |                                                                                                                                                                                                             |                         |                                                                                                                                                                                                   |
| local_path         | TEXT     |                   | true     |                                                                                                                                                                                                             |                         |                                                                                                                                                                                                   |
| github_id          | INTEGER  |                   | true     |                                                                                                                                                                                                             |                         | Stable GitHub identifier — the basis of rename detection.                                                                                                                                         |
| deploy_target      | TEXT     |                   | true     |                                                                                                                                                                                                             |                         | Matches templates.language_key.                                                                                                                                                                   |
| sort_order         | INTEGER  | 0                 | false    |                                                                                                                                                                                                             |                         |                                                                                                                                                                                                   |
| bugs_migrated_at   | TEXT     |                   | true     |                                                                                                                                                                                                             |                         | Lazy-migration marker (v18); NULL means the repo's MD has never been imported.                                                                                                                    |
| tasks_migrated_at  | TEXT     |                   | true     |                                                                                                                                                                                                             |                         | Lazy-migration marker (v21). While NULL, the first task sync suppresses 'created' events so backfilling legacy files doesn't flood the timeline.                                                  |
| deploy_repo_config | TEXT     | '{}'              | false    |                                                                                                                                                                                                             |                         | v25 — JSON map of repo-wide (not per-environment) deploy placeholder values.                                                                                                                      |
| autocommit_branch  | TEXT     |                   | true     |                                                                                                                                                                                                             |                         | v28 — NULL means auto-commit is off for this repo.                                                                                                                                                |
| codex_source_path  | TEXT     |                   | true     |                                                                                                                                                                                                             |                         |                                                                                                                                                                                                   |
| shared_mode        | INTEGER  | 0                 | false    |                                                                                                                                                                                                             |                         | v34 — the owner's switch (main side) that puts this repo into contributor mode. 0 = not shared.                                                                                                   |
| shared_role        | TEXT     |                   | true     |                                                                                                                                                                                                             |                         | v34 — cached role of THIS install for the repo: 'main' (owner, publishes the structure), 'contributor', or 'pending' (a contributor who has requested access). NULL means the repo is not shared. |
| shared_status      | TEXT     |                   | true     |                                                                                                                                                                                                             |                         | v34 — status code of the shared-mode sync (e.g. 'needs_manual_merge'); NULL means no pending condition.                                                                                           |
| shared_import      | TEXT     |                   | true     |                                                                                                                                                                                                             |                         | v34 — JSON map of the last imported blob sha per shared file, so an unchanged file is not imported again.                                                                                         |

## Constraints

| Name                            | Type        | Definition                                                                                              |
| ------------------------------- | ----------- | ------------------------------------------------------------------------------------------------------- |
| id                              | PRIMARY KEY | PRIMARY KEY (id)                                                                                        |
| - (Foreign key ID: 0)           | FOREIGN KEY | FOREIGN KEY (project_id) REFERENCES projects (id) ON UPDATE NO ACTION ON DELETE SET NULL MATCH NONE     |
| sqlite_autoindex_repositories_1 | UNIQUE      | UNIQUE (github_name)                                                                                    |
| -                               | CHECK       | CHECK(role IN ('server','client','test_client','admin_client','microservice','landing','tool','other')) |
| -                               | CHECK       | CHECK (shared_role IN ('main','contributor','pending'))                                                 |

## Indexes

| Name                            | Definition           |
| ------------------------------- | -------------------- |
| sqlite_autoindex_repositories_1 | UNIQUE (github_name) |

## Relations

![er](repositories.svg)

---

> Generated by [tbls](https://github.com/k1LoW/tbls)
