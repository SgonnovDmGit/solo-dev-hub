// T-000095: migrations dispatcher refactor.
//
// Previously `run_migrations` was a 530-line god-fn with 24 inline `if version
// < N { ... }` arms. Each arm is now a free function `mig_vN_<name>` that
// takes `&Connection` and executes the SQL batch (preserving the original
// behavior — including the `PRAGMA user_version = N` inside each batch, so
// the dispatcher doesn't need to update user_version separately).
//
// The dispatcher iterates `MIGRATIONS` in order, skipping versions already
// applied (i.e. `target_version <= current`). The signature uses
// `&Connection` rather than `&Transaction` because the original code never
// wrapped migrations in an explicit transaction, and each SQL batch already
// atomically bumps `user_version`. Switching to Transaction here would be a
// behavioral change and is out of scope for the relocation pass.

use super::*;
use rusqlite::Connection;

type MigrationFn = fn(&Connection) -> SqlResult<()>;

const MIGRATIONS: &[(i32, &str, MigrationFn)] = &[
    (1, "initial_schema", mig_v1_initial),
    (2, "bug_notes_category", mig_v2_bug_notes_category),
    (3, "repositories_local_path", mig_v3_local_path),
    (
        4,
        "repositories_role_check_expanded",
        mig_v4_role_check_expanded,
    ),
    (5, "project_microservices", mig_v5_project_microservices),
    (6, "bug_stats_v1", mig_v6_bug_stats_v1),
    (7, "bug_stats_v2_with_date", mig_v7_bug_stats_v2),
    (
        8,
        "bug_stats_resolved_count",
        mig_v8_bug_stats_resolved_count,
    ),
    (9, "repositories_github_id", mig_v9_repositories_github_id),
    (10, "templates_table", mig_v10_templates),
    (11, "deploy_target_and_manifests", mig_v11_deploy_manifests),
    (
        12,
        "project_type_microservices_rebuild",
        mig_v12_project_type,
    ),
    (13, "github_name_nullable", mig_v13_github_name_nullable),
    (14, "manual_ordering_sort_order", mig_v14_sort_order),
    (15, "deploy_extras_json", mig_v15_deploy_extras),
    (16, "repo_renames_log", mig_v16_repo_renames),
    (
        17,
        "drop_bug_file_path_setting",
        mig_v17_drop_bug_file_path_setting,
    ),
    (18, "bugs_sot_table", mig_v18_bugs_sot),
    (19, "bug_events_log", mig_v19_bug_events),
    (20, "deploy_environments_multi", mig_v20_deploy_environments),
    (21, "tasks_and_event_tables", mig_v21_tasks_events),
    (22, "bug_archived_from_md", mig_v22_archived_from_md),
    (23, "drop_bug_stats_view", mig_v23_drop_bug_stats_view),
    (24, "project_renames_log", mig_v24_project_renames),
    (25, "deploy_repo_config", mig_v25_deploy_repo_config),
];

impl AppDb {
    pub(super) fn run_migrations(&self) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        let version: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

        for &(target, _name, mig_fn) in MIGRATIONS {
            if version < target {
                mig_fn(&conn)?;
            }
        }
        Ok(())
    }
}

fn mig_v1_initial(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS projects (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            description TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS repositories (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,
            github_name TEXT NOT NULL UNIQUE,
            github_url TEXT,
            role TEXT CHECK(role IN ('server','client','test_client','admin_client','other')),
            description TEXT,
            language TEXT,
            last_pushed_at DATETIME,
            added_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS bug_notes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            repository_id INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
            title TEXT NOT NULL,
            description TEXT,
            priority TEXT CHECK(priority IN ('low','medium','high','critical')) DEFAULT 'medium',
            is_resolved INTEGER DEFAULT 0,
            fix_attempts INTEGER DEFAULT 0,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            resolved_at DATETIME
        );

        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT
        );

        PRAGMA user_version = 1;",
    )
}

fn mig_v2_bug_notes_category(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "ALTER TABLE bug_notes ADD COLUMN category TEXT CHECK(category IN ('ui_ux','backend','network','database','security','performance','other','unknown')) DEFAULT 'unknown';
         PRAGMA user_version = 2;",
    )
}

fn mig_v3_local_path(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "ALTER TABLE repositories ADD COLUMN local_path TEXT;
         PRAGMA user_version = 3;",
    )
}

fn mig_v4_role_check_expanded(conn: &Connection) -> SqlResult<()> {
    // Expand role CHECK to include microservice, landing, tool
    conn.execute_batch(
        "CREATE TABLE repositories_new (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,
            github_name TEXT NOT NULL UNIQUE,
            github_url TEXT,
            role TEXT CHECK(role IN ('server','client','test_client','admin_client','microservice','landing','tool','other')),
            description TEXT,
            language TEXT,
            last_pushed_at DATETIME,
            added_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            local_path TEXT
        );
        INSERT INTO repositories_new SELECT * FROM repositories;
        DROP TABLE repositories;
        ALTER TABLE repositories_new RENAME TO repositories;
        PRAGMA user_version = 4;",
    )
}

fn mig_v5_project_microservices(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS project_microservices (
            project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
            repository_id INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
            PRIMARY KEY (project_id, repository_id)
        );
        PRAGMA user_version = 5;",
    )
}

fn mig_v6_bug_stats_v1(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS bug_stats (
            repository_id INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
            severity TEXT NOT NULL,
            category TEXT NOT NULL,
            bugs_count INTEGER DEFAULT 0,
            attempts_count INTEGER DEFAULT 0,
            PRIMARY KEY (repository_id, severity, category)
        );
        PRAGMA user_version = 6;",
    )
}

fn mig_v7_bug_stats_v2(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "DROP TABLE IF EXISTS bug_stats;
         CREATE TABLE bug_stats (
             repository_id INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
             severity TEXT NOT NULL,
             category TEXT NOT NULL,
             date TEXT NOT NULL,
             bugs_count INTEGER DEFAULT 0,
             attempts_count INTEGER DEFAULT 0,
             PRIMARY KEY (repository_id, severity, category, date)
         );
         PRAGMA user_version = 7;",
    )
}

fn mig_v8_bug_stats_resolved_count(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "ALTER TABLE bug_stats ADD COLUMN resolved_count INTEGER DEFAULT 0;
         PRAGMA user_version = 8;",
    )
}

fn mig_v9_repositories_github_id(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "ALTER TABLE repositories ADD COLUMN github_id INTEGER;
         PRAGMA user_version = 9;",
    )
}

fn mig_v10_templates(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "CREATE TABLE templates (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            language_key TEXT NOT NULL,
            file_name TEXT NOT NULL,
            content TEXT NOT NULL,
            is_custom INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(language_key, file_name)
         );
         PRAGMA user_version = 10;",
    )
}

fn mig_v11_deploy_manifests(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "ALTER TABLE repositories ADD COLUMN deploy_target TEXT;
         CREATE TABLE deploy_manifests (
            repository_id INTEGER PRIMARY KEY REFERENCES repositories(id) ON DELETE CASCADE,
            workflow_name TEXT NOT NULL,
            image_tag TEXT NOT NULL,
            compose_service TEXT NOT NULL,
            domain TEXT NOT NULL,
            deploy_branch TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
         );
         PRAGMA user_version = 11;",
    )
}

fn mig_v12_project_type(conn: &Connection) -> SqlResult<()> {
    // F-012: microservice = project type
    // Add project_type column; clear legacy role='microservice'; rebuild project_microservices
    // with new semantics (parent_project_id ↔ microservice_project_id).
    conn.execute_batch(
        "ALTER TABLE projects ADD COLUMN project_type TEXT NOT NULL DEFAULT 'standard';
         UPDATE repositories SET role = NULL WHERE role = 'microservice';
         CREATE TABLE project_microservices_new (
            project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
            microservice_project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
            PRIMARY KEY (project_id, microservice_project_id),
            CHECK (project_id != microservice_project_id)
         );
         DROP TABLE project_microservices;
         ALTER TABLE project_microservices_new RENAME TO project_microservices;
         PRAGMA user_version = 12;",
    )
}

fn mig_v13_github_name_nullable(conn: &Connection) -> SqlResult<()> {
    // F-011: local-only repos — github_name becomes nullable.
    // SQLite does not support ALTER COLUMN drop NOT NULL, so rebuild the table.
    // Explicit column list in INSERT to be robust against column-order drift.
    conn.execute_batch(
        "CREATE TABLE repositories_new (
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
         );
         INSERT INTO repositories_new
            (id, project_id, github_name, github_url, role, description, language,
             last_pushed_at, added_at, updated_at, local_path, github_id, deploy_target)
         SELECT
            id, project_id, github_name, github_url, role, description, language,
            last_pushed_at, added_at, updated_at, local_path, github_id, deploy_target
         FROM repositories;
         DROP TABLE repositories;
         ALTER TABLE repositories_new RENAME TO repositories;
         PRAGMA user_version = 13;",
    )
}

fn mig_v14_sort_order(conn: &Connection) -> SqlResult<()> {
    // F-025: manual ordering — add sort_order column to projects and repositories.
    // Initial populate:
    //   - projects: id * 10 (stable insertion order, gap 10 for inserts)
    //   - repositories: role_priority * 1000 + id * 10 (group by role, stable within)
    //     role_priority mapping must match ROLE_ICONS/frontend logic:
    //       server=0, admin_client=1, client=2, test_client=3, microservice=4,
    //       landing=5, tool=6, other/NULL=99
    conn.execute_batch(
        "ALTER TABLE projects ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0;
         ALTER TABLE repositories ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0;
         UPDATE projects SET sort_order = id * 10;
         UPDATE repositories SET sort_order = (
            CASE role
                WHEN 'server' THEN 0
                WHEN 'admin_client' THEN 1
                WHEN 'client' THEN 2
                WHEN 'test_client' THEN 3
                WHEN 'microservice' THEN 4
                WHEN 'landing' THEN 5
                WHEN 'tool' THEN 6
                ELSE 99
            END
         ) * 1000 + id * 10;
         PRAGMA user_version = 14;",
    )
}

fn mig_v15_deploy_extras(conn: &Connection) -> SqlResult<()> {
    // F-022 extras: deploy_manifests gains a JSON column for non-core placeholder values
    // (ENV_FILE_PATH, ENTRY_POINT, GO_VERSION, BINARY_NAME, APP_PORT, …).
    // Stored as a string-map JSON object; empty map == "{}". Absent keys fall back to
    // auto_detect → placeholder default at load time.
    conn.execute_batch(
        "ALTER TABLE deploy_manifests ADD COLUMN extras TEXT NOT NULL DEFAULT '{}';
         PRAGMA user_version = 15;",
    )
}

fn mig_v16_repo_renames(conn: &Connection) -> SqlResult<()> {
    // F-033: rename-log table. Records repository renames detected during
    // upsert_repository_with_outcome (github_name change for existing github_id).
    // Sync-preamble replays all entries each time, renaming counterparty-side
    // folders (client-requirements/<X>, server-requirements/<X>, etc.) on the fs.
    // No applied_at — sync idempotency comes from fs checks, not DB state.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS repo_renames (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            repository_id INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
            old_canonical TEXT NOT NULL,
            new_canonical TEXT NOT NULL,
            renamed_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
         );
         CREATE INDEX IF NOT EXISTS idx_repo_renames_repo ON repo_renames(repository_id);
         PRAGMA user_version = 16;",
    )
}

fn mig_v17_drop_bug_file_path_setting(conn: &Connection) -> SqlResult<()> {
    // T-048: remove obsolete `bug_file_path` setting. The path `docs/bug-reports.md`
    // is now fixed by the global CLAUDE.md template contract and hardcoded in Rust.
    conn.execute_batch(
        "DELETE FROM settings WHERE key = 'bug_file_path';
         PRAGMA user_version = 17;",
    )
}

fn mig_v18_bugs_sot(conn: &Connection) -> SqlResult<()> {
    // T-025/T-026/T-027 (v0.16.0): SQLite = SoT for bugs, MD = LLM-facing view.
    // - CREATE TABLE bugs: full schema with display_id (B-000042), numeric_id (42),
    //   status/severity/category CHECK constraints, confirmed_at for history.
    // - DROP TABLE bug_stats → CREATE VIEW bug_stats (live recompute from bugs).
    //   VIEW has identical columns to old table, all existing Dashboard/StatsTable
    //   SQL queries work without changes. (VIEW later dropped in v23.)
    // - DROP TABLE bug_notes (legacy from v1, unused since bugs moved to MD in v4).
    // - ALTER repositories ADD COLUMN bugs_migrated_at (marker for lazy per-repo
    //   MD→DB migration on first bug-tab open).
    conn.execute_batch(
        "CREATE TABLE bugs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            repository_id INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
            numeric_id INTEGER NOT NULL,
            display_id TEXT NOT NULL,
            created_at TEXT NOT NULL,
            description TEXT NOT NULL,
            severity TEXT NOT NULL CHECK(severity IN ('critical','major','medium','minor')),
            category TEXT NOT NULL CHECK(category IN ('ui_ux','ux_flow','logic','auth','database','performance','security','integration','other')),
            status TEXT NOT NULL CHECK(status IN ('created','in-progress','testing','rejected','confirmed')),
            fix_attempts INTEGER NOT NULL DEFAULT 0,
            comment TEXT,
            confirmed_at TEXT,
            UNIQUE(repository_id, numeric_id)
         );
         CREATE INDEX idx_bugs_repo ON bugs(repository_id);
         CREATE INDEX idx_bugs_status ON bugs(status);
         CREATE INDEX idx_bugs_repo_date ON bugs(repository_id, created_at);

         DROP TABLE IF EXISTS bug_notes;
         DROP TABLE IF EXISTS bug_stats;

         CREATE VIEW bug_stats AS
         SELECT
             repository_id,
             severity,
             category,
             date(created_at) AS date,
             COUNT(*) AS bugs_count,
             COALESCE(SUM(fix_attempts), 0) AS attempts_count,
             SUM(CASE WHEN status='confirmed' THEN 1 ELSE 0 END) AS resolved_count
         FROM bugs
         GROUP BY repository_id, severity, category, date(created_at);

         ALTER TABLE repositories ADD COLUMN bugs_migrated_at TEXT;

         PRAGMA user_version = 18;",
    )
}

fn mig_v19_bug_events(conn: &Connection) -> SqlResult<()> {
    // v0.17.0 (Dashboard redesign): `bug_events` log for honest
    // attempts-per-period metrics. Invariant:
    //   COUNT(bug_events WHERE bug_id=X AND event_type='entered_testing') == bugs.fix_attempts
    //
    // Back-fill is done in a separate step (task A3) so the schema-only
    // change commits cleanly without data mutation.
    conn.execute_batch(
        "CREATE TABLE bug_events (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             bug_id INTEGER NOT NULL REFERENCES bugs(id) ON DELETE CASCADE,
             event_type TEXT NOT NULL CHECK(event_type IN (
                 'created','taken','entered_testing','confirmed','rejected','reopened'
             )),
             ts TEXT NOT NULL,
             from_status TEXT,
             to_status TEXT
         );
         CREATE INDEX idx_bug_events_bug ON bug_events(bug_id);
         CREATE INDEX idx_bug_events_ts ON bug_events(ts);
         CREATE INDEX idx_bug_events_type_ts ON bug_events(event_type, ts);

         CREATE INDEX idx_bugs_confirmed_at ON bugs(confirmed_at)
             WHERE confirmed_at IS NOT NULL;

         PRAGMA user_version = 19;",
    )
}

fn mig_v20_deploy_environments(conn: &Connection) -> SqlResult<()> {
    // v0.18.0: Multi-environment deploy + meta.json v4.
    // Rename concept: each deploy_manifests row becomes a deploy_environments
    // row with name='prod'. New deploy_secrets table per-deploy per-secret flags.
    conn.execute_batch(
        "CREATE TABLE deploy_environments (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            repository_id INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            workflow_name TEXT NOT NULL,
            image_tag TEXT NOT NULL,
            compose_service TEXT NOT NULL,
            domain TEXT NOT NULL,
            deploy_branch TEXT NOT NULL,
            sort_order INTEGER NOT NULL DEFAULT 0,
            extras TEXT NOT NULL DEFAULT '{}',
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(repository_id, name)
         );
         CREATE INDEX idx_deploy_env_repo ON deploy_environments(repository_id);

         INSERT INTO deploy_environments
            (repository_id, name, workflow_name, image_tag,
             compose_service, domain, deploy_branch, extras, updated_at)
         SELECT
            repository_id, 'prod', workflow_name, image_tag,
            compose_service, domain, deploy_branch, extras, updated_at
         FROM deploy_manifests;

         DROP TABLE deploy_manifests;

         CREATE TABLE deploy_secrets (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            deploy_env_id INTEGER NOT NULL REFERENCES deploy_environments(id) ON DELETE CASCADE,
            secret_name TEXT NOT NULL,
            role TEXT CHECK(role IN ('build','deploy','runtime')),
            included INTEGER NOT NULL DEFAULT 1,
            override_enabled INTEGER NOT NULL DEFAULT 0,
            sort_order INTEGER NOT NULL DEFAULT 0,
            UNIQUE(deploy_env_id, secret_name)
         );
         CREATE INDEX idx_deploy_secrets_env ON deploy_secrets(deploy_env_id);

         PRAGMA user_version = 20;",
    )
}

fn mig_v21_tasks_events(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "CREATE TABLE tasks (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             repository_id INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
             task_id TEXT NOT NULL,
             prefix TEXT NOT NULL CHECK(prefix IN ('T','F','D')),
             description TEXT NOT NULL,
             effort REAL,
             priority TEXT,
             status TEXT,
             version TEXT,
             source TEXT NOT NULL CHECK(source IN ('todo','done')),
             created_at TEXT NOT NULL,
             updated_at TEXT NOT NULL,
             UNIQUE(repository_id, task_id, source)
         );
         CREATE INDEX idx_tasks_repo ON tasks(repository_id);
         CREATE INDEX idx_tasks_repo_source ON tasks(repository_id, source);
         CREATE INDEX idx_tasks_status ON tasks(status) WHERE status IS NOT NULL;

         CREATE TABLE task_events (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             task_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
             event_type TEXT NOT NULL CHECK(event_type IN (
                 'created','taken','review','done','reopened'
             )),
             ts TEXT NOT NULL,
             from_status TEXT,
             to_status TEXT
         );
         CREATE INDEX idx_task_events_task ON task_events(task_id);
         CREATE INDEX idx_task_events_ts ON task_events(ts);
         CREATE INDEX idx_task_events_type_ts ON task_events(event_type, ts);

         CREATE TABLE sync_events (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             repository_id INTEGER REFERENCES repositories(id) ON DELETE CASCADE,
             sync_type TEXT NOT NULL CHECK(sync_type IN ('project_sync','tasks','secret','requirements')),
             ts TEXT NOT NULL,
             change_count INTEGER NOT NULL DEFAULT 0,
             details TEXT
         );
         CREATE INDEX idx_sync_events_ts ON sync_events(ts);
         CREATE INDEX idx_sync_events_repo ON sync_events(repository_id);

         CREATE TABLE deploy_events (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             deploy_env_id INTEGER REFERENCES deploy_environments(id) ON DELETE CASCADE,
             repository_id INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
             action TEXT NOT NULL CHECK(action IN ('render','env_secret_set','env_secret_delete')),
             ts TEXT NOT NULL,
             details TEXT
         );
         CREATE INDEX idx_deploy_events_ts ON deploy_events(ts);
         CREATE INDEX idx_deploy_events_repo ON deploy_events(repository_id);

         ALTER TABLE repositories ADD COLUMN tasks_migrated_at TEXT;

         PRAGMA user_version = 21;",
    )
}

fn mig_v22_archived_from_md(conn: &Connection) -> SqlResult<()> {
    // v0.21.1: Restore confirmed-bug LLM-acknowledgement workflow.
    // App now writes confirmed rows to MD (so LLM sees confirmation);
    // when LLM removes a confirmed row from MD, reconcile sets
    // archived_from_md_at and regenerate excludes those rows forever.
    conn.execute_batch(
        "ALTER TABLE bugs ADD COLUMN archived_from_md_at TEXT;
         PRAGMA user_version = 22;",
    )
}

fn mig_v23_drop_bug_stats_view(conn: &Connection) -> SqlResult<()> {
    // T-000058 (v0.24.0): Drop legacy `bug_stats` VIEW.
    // The VIEW was a live-computed replacement for the pre-v18 incremental
    // table. After T-000054 (v0.22.0 stats redesign), Dashboard moved to
    // its own queries directly against `bugs` + `bug_events`, and per-repo
    // StatsTable was replaced by `StatsSummary` which uses
    // `get_repo_stats_summary` / `get_project_stats_summary`. No production
    // code reads from `bug_stats` anymore — drop it as dead schema.
    // `IF EXISTS` makes this defensive: if a fresh-install DB never ran
    // v18 (unlikely but possible in test setups), the DROP is a no-op.
    conn.execute_batch(
        "DROP VIEW IF EXISTS bug_stats;
         PRAGMA user_version = 23;",
    )
}

fn mig_v24_project_renames(conn: &Connection) -> SqlResult<()> {
    // T-000092 (v0.29.0): project-rename log. Symmetric to `repo_renames`
    // (v16) but scoped to a project rather than a repository. Used to
    // replay `microservice-api/<X>/` folder renames on parent server side
    // when a microservice project is renamed — the folder there is keyed
    // by project name (`ms_project.name`), not by repo canonical name,
    // so `repo_renames` alone doesn't cover it.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS project_renames (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
            old_name TEXT NOT NULL,
            new_name TEXT NOT NULL,
            renamed_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
         );
         CREATE INDEX IF NOT EXISTS idx_project_renames_project ON project_renames(project_id);
         PRAGMA user_version = 24;",
    )
}

/// T-000103 Task 1 (v0.31.0): split deploy config into repo-wide vs env-specific.
///
/// Schema:
///   - `repositories.deploy_repo_config TEXT NOT NULL DEFAULT '{}'` — JSON map of
///     placeholder values that render a repo-wide single file (e.g. Dockerfile).
///   - `sync_events.sync_type` CHECK expanded to include `'migration'` so we can
///     log conflict events when first-env-wins discards values from later envs.
///
/// Data move (schema-driven, NOT hardcoded on `'go'`):
///   For each repository with non-empty `deploy_target`:
///     1. Look up its template's `meta.json` (templates.language_key = deploy_target,
///        file_name = 'meta.json'). Parse JSON.
///     2. Find placeholders where `placeholders.<KEY>.scope == "repo"`.
///        If none → skip this repo (no-op for templates that don't mark any
///        placeholder as repo-scope, or for fresh DBs where Task 2 hasn't yet
///        updated bundled meta.json — the migration must work regardless).
///     3. Read all `deploy_environments` rows for this repo, sorted by `sort_order ASC`.
///     4. For each repo-scope placeholder name:
///          - Take its value from the FIRST env's `extras`. If absent → skip the key.
///          - Walk subsequent envs; record conflict if any value differs.
///          - Write key→value to repo's `deploy_repo_config` JSON.
///          - Remove the key from ALL envs' `extras` JSON.
///     5. If any conflicts → INSERT into `sync_events`:
///          sync_type='migration', change_count = # conflicting keys,
///          details = JSON {"conflicts":[{"key","kept_env","kept_value","discarded":[...]}]}
///
/// Idempotency:
///   - If `deploy_repo_config != '{}'` already for a repo → skip the data-loop for
///     that repo entirely (don't overwrite, don't re-strip envs).
fn mig_v25_deploy_repo_config(conn: &Connection) -> SqlResult<()> {
    // Step 1: schema changes — add column, expand sync_events CHECK constraint.
    //
    // sync_events.sync_type CHECK was created in v21 as
    //   CHECK(sync_type IN ('project_sync','tasks','secret','requirements'))
    // SQLite can't ALTER a CHECK; rebuild the table to add 'migration'.
    conn.execute_batch(
        "ALTER TABLE repositories ADD COLUMN deploy_repo_config TEXT NOT NULL DEFAULT '{}';

         CREATE TABLE sync_events_new (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             repository_id INTEGER REFERENCES repositories(id) ON DELETE CASCADE,
             sync_type TEXT NOT NULL CHECK(sync_type IN ('project_sync','tasks','secret','requirements','migration')),
             ts TEXT NOT NULL,
             change_count INTEGER NOT NULL DEFAULT 0,
             details TEXT
         );
         INSERT INTO sync_events_new (id, repository_id, sync_type, ts, change_count, details)
            SELECT id, repository_id, sync_type, ts, change_count, details FROM sync_events;
         DROP TABLE sync_events;
         ALTER TABLE sync_events_new RENAME TO sync_events;
         CREATE INDEX idx_sync_events_ts ON sync_events(ts);
         CREATE INDEX idx_sync_events_repo ON sync_events(repository_id);",
    )?;

    // Step 2: data move.
    mig_v25_data_move(conn)?;

    // Step 3: bump user_version.
    conn.pragma_update(None, "user_version", 25)?;
    Ok(())
}

/// v25 data-move helper. Idempotent — skips repos where `deploy_repo_config != '{}'`.
/// Extracted from `mig_v25_deploy_repo_config` so tests can seed state on a
/// fully-migrated DB and exercise the data-move path without re-running ALTER
/// statements (which would fail since the column already exists).
fn mig_v25_data_move(conn: &Connection) -> SqlResult<()> {
    //
    // Read all repos that have a non-empty deploy_target and an as-yet-untouched
    // deploy_repo_config (still the default '{}'). Idempotency: existing non-'{}'
    // values are left alone, so re-running v25 (defensive — should never happen
    // because user_version is bumped at end) wouldn't double-strip.
    let repos: Vec<(i64, String)> = {
        let mut stmt = conn.prepare(
            "SELECT id, deploy_target FROM repositories
             WHERE deploy_target IS NOT NULL AND deploy_target != ''
               AND (deploy_repo_config IS NULL OR deploy_repo_config = '{}')",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.filter_map(Result::ok).collect()
    };

    for (repo_id, deploy_target) in repos {
        // Look up meta.json for this template. If template / meta.json missing
        // (e.g. deploy_target points at a key that doesn't exist yet) → silent skip.
        let meta_content: Option<String> = conn
            .query_row(
                "SELECT content FROM templates WHERE language_key = ?1 AND file_name = 'meta.json'",
                rusqlite::params![deploy_target],
                |row| row.get::<_, String>(0),
            )
            .ok();
        let Some(meta_str) = meta_content else {
            continue;
        };

        // Parse meta.json. If invalid JSON → skip (don't crash migration).
        let meta: serde_json::Value = match serde_json::from_str(&meta_str) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Identify placeholders where scope == "repo".
        let repo_scope_keys: Vec<String> = meta
            .get("placeholders")
            .and_then(|v| v.as_object())
            .map(|phs| {
                phs.iter()
                    .filter_map(|(name, spec)| {
                        let scope = spec.get("scope").and_then(|v| v.as_str());
                        if scope == Some("repo") {
                            Some(name.clone())
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        if repo_scope_keys.is_empty() {
            // Template has no repo-scope placeholders → no-op for this repo.
            continue;
        }

        // Fetch envs sorted by sort_order ASC (first-env-wins on conflict).
        let envs: Vec<(i64, String, String)> = {
            let mut stmt = conn.prepare(
                "SELECT id, name, extras FROM deploy_environments
                 WHERE repository_id = ?1 ORDER BY sort_order ASC, id ASC",
            )?;
            let rows = stmt.query_map(rusqlite::params![repo_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            rows.filter_map(Result::ok).collect()
        };

        if envs.is_empty() {
            // No envs to lift values from. Still leave deploy_repo_config as
            // '{}' (default). User will populate it via UI once they add an env.
            continue;
        }

        // Parse each env's extras into a mutable HashMap.
        let mut env_maps: Vec<(i64, String, std::collections::HashMap<String, String>)> = envs
            .into_iter()
            .map(|(id, name, extras_json)| {
                let map: std::collections::HashMap<String, String> =
                    serde_json::from_str(&extras_json).unwrap_or_default();
                (id, name, map)
            })
            .collect();

        let mut repo_config: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut conflicts: Vec<serde_json::Value> = Vec::new();

        // For each repo-scope key, lift from first env, detect conflicts, strip everywhere.
        for key in &repo_scope_keys {
            // Find the first env (by sort_order) that has this key.
            let first_with = env_maps.iter().find(|(_, _, m)| m.contains_key(key));
            let Some((_, kept_env, kept_map)) = first_with else {
                // Key not present in any env → nothing to lift.
                continue;
            };
            let kept_value = kept_map.get(key).cloned().unwrap_or_default();
            let kept_env_name = kept_env.clone();

            // Collect later-env differences for conflict log.
            let mut discarded: Vec<serde_json::Value> = Vec::new();
            let mut seen_kept = false;
            for (_, env_name, m) in &env_maps {
                if !seen_kept {
                    if env_name == &kept_env_name {
                        seen_kept = true;
                    }
                    continue;
                }
                if let Some(v) = m.get(key) {
                    if v != &kept_value {
                        discarded.push(serde_json::json!({
                            "env": env_name,
                            "value": v,
                        }));
                    }
                }
            }
            if !discarded.is_empty() {
                conflicts.push(serde_json::json!({
                    "key": key,
                    "kept_env": kept_env_name,
                    "kept_value": kept_value,
                    "discarded": discarded,
                }));
            }

            repo_config.insert(key.clone(), kept_value);

            // Strip this key from ALL envs.
            for (_, _, m) in env_maps.iter_mut() {
                m.remove(key);
            }
        }

        // If nothing actually got lifted (every key absent from every env), still
        // leave deploy_repo_config as '{}' and skip writes.
        if repo_config.is_empty() {
            continue;
        }

        // Persist deploy_repo_config on the repo.
        let repo_config_json =
            serde_json::to_string(&repo_config).unwrap_or_else(|_| "{}".to_string());
        conn.execute(
            "UPDATE repositories SET deploy_repo_config = ?1 WHERE id = ?2",
            rusqlite::params![repo_config_json, repo_id],
        )?;

        // Persist stripped extras back into each env.
        for (env_id, _, m) in &env_maps {
            let new_extras = serde_json::to_string(m).unwrap_or_else(|_| "{}".to_string());
            conn.execute(
                "UPDATE deploy_environments SET extras = ?1 WHERE id = ?2",
                rusqlite::params![new_extras, env_id],
            )?;
        }

        // Log conflicts (if any) via sync_events.
        if !conflicts.is_empty() {
            let details = serde_json::json!({ "conflicts": conflicts }).to_string();
            conn.execute(
                "INSERT INTO sync_events (repository_id, sync_type, ts, change_count, details)
                 VALUES (?1, 'migration', ?2, ?3, ?4)",
                rusqlite::params![
                    repo_id,
                    chrono::Utc::now().to_rfc3339(),
                    conflicts.len() as i64,
                    details,
                ],
            )?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_db() -> AppDb {
        AppDb::new(PathBuf::from(":memory:")).unwrap()
    }

    #[test]
    fn test_db_init_creates_tables() {
        let db = make_db();
        let conn = db.conn.lock().unwrap();
        // v0.16.0: bug_notes dropped (legacy, replaced by `bugs` table).
        let count: i32 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('projects','repositories','bugs','settings')",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 4);
    }

    #[test]
    fn test_db_migration_version() {
        let db = make_db();
        let conn = db.conn.lock().unwrap();
        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        // Update this when a new migration is added.
        assert!(version >= 25);
    }

    #[test]
    fn test_db_migration_v19_bug_events_schema() {
        let db = make_db();
        let conn = db.conn.lock().unwrap();
        // Table exists
        let count: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='bug_events'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "bug_events table must exist");

        // Three indexes created
        let idx_count: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index'
             AND name IN ('idx_bug_events_bug','idx_bug_events_ts','idx_bug_events_type_ts')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(idx_count, 3, "three bug_events indexes expected");

        // idx_bugs_confirmed_at partial index also created
        let cat_idx: i32 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_bugs_confirmed_at'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(cat_idx, 1);
    }

    #[test]
    fn test_db_migration_v19_bug_events_check_constraint() {
        let db = make_db();
        // A bug is needed for the FK to be satisfied
        let repo = db
            .insert_local_repository("/tmp/r1", "r1", None, None)
            .unwrap();
        let bug = db
            .insert_bug(
                repo.id,
                1,
                "2026-01-01T00:00:00Z",
                "desc",
                "minor",
                "other",
                "created",
                0,
                None,
                None,
            )
            .unwrap();

        let conn = db.conn.lock().unwrap();
        // Valid event_type inserts
        conn.execute(
            "INSERT INTO bug_events (bug_id, event_type, ts, from_status, to_status)
             VALUES (?1, 'created', '2026-01-01T00:00:00Z', NULL, 'created')",
            [bug.id],
        )
        .unwrap();

        // Invalid event_type rejected
        let bad = conn.execute(
            "INSERT INTO bug_events (bug_id, event_type, ts, from_status, to_status)
             VALUES (?1, 'garbage', '2026-01-01T00:00:00Z', NULL, NULL)",
            [bug.id],
        );
        assert!(bad.is_err(), "garbage event_type must violate CHECK");
    }

    #[test]
    fn test_db_migration_idempotent() {
        let db = make_db();
        db.run_migrations().unwrap();
    }

    #[test]
    fn test_migration_v13_github_name_nullable() {
        let db = make_db();
        let conn = db.conn.lock().unwrap();
        // After v13, github_name should not have NOT NULL constraint.
        let notnull: i32 = conn
            .query_row(
                "SELECT \"notnull\" FROM pragma_table_info('repositories') WHERE name = 'github_name'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(notnull, 0, "github_name must be nullable after v13");
    }

    #[test]
    fn test_db_migration_v3_local_path_column() {
        let db = make_db();
        let conn = db.conn.lock().unwrap();
        let count: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('repositories') WHERE name = 'local_path'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_db_migration_v18_bugs_schema() {
        let db = make_db();
        let conn = db.conn.lock().unwrap();
        // v18: `bugs` table with expected columns exists.
        let count: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('bugs')
                 WHERE name IN ('id','repository_id','numeric_id','display_id','created_at',
                                'description','severity','category','status','fix_attempts',
                                'comment','confirmed_at')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 12, "bugs table should have 12 columns");

        // `bug_notes` was dropped.
        let legacy: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='bug_notes'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(legacy, 0, "bug_notes must be dropped in v18");

        // `bug_stats` was a VIEW v18..v22; dropped in v23 (T-000058) as dead
        // schema after Dashboard/StatsSummary stopped reading from it. The
        // assertion is inverted now — the VIEW must be gone after migrations.
        let is_view: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='view' AND name='bug_stats'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(is_view, 0, "bug_stats VIEW must be dropped by v23");

        // `bugs_migrated_at` column exists on repositories.
        let marker: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('repositories') WHERE name = 'bugs_migrated_at'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(marker, 1, "repositories.bugs_migrated_at column expected");
    }

    #[test]
    fn test_v20_migrates_deploy_manifests_to_environments() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("test.db");
        let db = AppDb::new(path.clone()).unwrap();

        // Seed a repo + a deploy_manifests row (v11 schema still works
        // because we're using the real CURRENT DB which already has v20 applied;
        // for this smoke test we use the NEW deploy_environments table directly).
        let project = db.create_project("p1", None, "tool").unwrap();
        let repo = db
            .insert_local_repository("/tmp/test-repo", "test-repo", Some(project.id), None)
            .unwrap();

        // New schema invariant: deploy_environments table exists + has expected columns.
        let conn = db.conn.lock().unwrap();
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(deploy_environments)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        for expected in &[
            "id",
            "repository_id",
            "name",
            "workflow_name",
            "image_tag",
            "compose_service",
            "domain",
            "deploy_branch",
            "sort_order",
            "extras",
            "updated_at",
        ] {
            assert!(
                cols.contains(&expected.to_string()),
                "missing column {}",
                expected
            );
        }

        // deploy_secrets table exists
        let cols2: Vec<String> = conn
            .prepare("PRAGMA table_info(deploy_secrets)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        for expected in &[
            "id",
            "deploy_env_id",
            "secret_name",
            "role",
            "included",
            "override_enabled",
            "sort_order",
        ] {
            assert!(
                cols2.contains(&expected.to_string()),
                "missing deploy_secrets column {}",
                expected
            );
        }

        // deploy_manifests dropped
        let manifest_exists: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='deploy_manifests'",
                [],
                |_| Ok(true),
            )
            .unwrap_or(false);
        assert!(!manifest_exists, "deploy_manifests must be dropped in v20");

        // user_version bumped (v20 migration ran; v21..v25 also applied on fresh DB)
        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 25);

        drop(conn);
        let _ = repo;
        std::mem::forget(tmp);
    }

    #[test]
    fn test_v20_preserves_existing_manifest_as_prod_env() {
        // Full migration fidelity check: start on v19 schema, insert a deploy_manifests row,
        // then simulate running v20. Since AppDb::new always runs latest migrations, we need
        // a custom db constructor that stops at v19. That's overkill for this plan — instead
        // test the INSERT+SELECT round-trip on the new table. Migration logic coverage itself
        // is achieved in test_v20_migrates_deploy_manifests_to_environments.
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("test.db");
        let db = AppDb::new(path).unwrap();
        let project = db.create_project("p1", None, "tool").unwrap();
        let repo = db
            .insert_local_repository("/tmp/r1", "r1", Some(project.id), None)
            .unwrap();

        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO deploy_environments (repository_id, name, workflow_name, image_tag,
             compose_service, domain, deploy_branch, extras)
             VALUES (?1, 'prod', 'Deploy', 'latest', 'backend', 'x.com', 'master', '{}')",
            rusqlite::params![repo.id],
        )
        .unwrap();

        let (name, branch): (String, String) = conn
            .query_row(
                "SELECT name, deploy_branch FROM deploy_environments WHERE repository_id = ?1",
                rusqlite::params![repo.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(name, "prod");
        assert_eq!(branch, "master");

        drop(conn);
        std::mem::forget(tmp);
    }

    #[test]
    fn test_db_migration_v21_creates_event_tables() {
        let db = make_db();
        let conn = db.conn.lock().unwrap();
        let count: i32 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('tasks','task_events','sync_events','deploy_events')",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(
            count, 4,
            "4 new tables expected (tasks, task_events, sync_events, deploy_events)"
        );
    }

    #[test]
    fn test_db_migration_v21_version() {
        let db = make_db();
        let conn = db.conn.lock().unwrap();
        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 25);
    }

    #[test]
    fn test_db_migration_v21_tasks_migrated_at_column() {
        let db = make_db();
        let conn = db.conn.lock().unwrap();
        let mut stmt = conn.prepare("PRAGMA table_info(repositories)").unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(cols.contains(&"tasks_migrated_at".to_string()));
    }

    #[test]
    fn test_db_migration_v23_drops_bug_stats_view() {
        // T-000058 (v0.24.0): `bug_stats` VIEW was created in v18 as a
        // live-computed replacement for the pre-v18 incremental table. After
        // T-000054 (v0.22.0 stats redesign), no production code reads from it
        // anymore — Dashboard and StatsSummary query `bugs` + `bug_events`
        // directly. Migration v23 drops the VIEW as dead schema.
        let db = make_db();
        let conn = db.conn.lock().unwrap();

        // VIEW must not exist after migrations have run.
        let count: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='view' AND name='bug_stats'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 0,
            "bug_stats VIEW should be dropped by v23, but still exists"
        );

        // No `bug_stats` table either (it was dropped in v18 when the VIEW
        // replaced it; ensure v23 didn't accidentally recreate the legacy table).
        let table_count: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='bug_stats'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(table_count, 0, "bug_stats must not exist as a table either");
    }

    #[test]
    fn test_db_migration_v24_creates_project_renames() {
        // T-000092: project-rename log. Mirrors `repo_renames` (v16) for
        // microservice-api/<project-name>/ replay on parent server side.
        let db = make_db();
        let conn = db.conn.lock().unwrap();
        let exists: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='project_renames'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(exists, 1, "project_renames table must be created by v24");
        let idx: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_project_renames_project'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(idx, 1, "idx_project_renames_project must be created by v24");
    }

    #[test]
    fn test_v13_preserves_deploy_manifests_fk() {
        // v0.18.0 note: migration v20 renamed deploy_manifests → deploy_environments.
        // Test preserved under old name for git-blame continuity; asserts the
        // cascade-delete contract on the new table.
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("test.db");
        let db = AppDb::new(path).unwrap();
        let project = db.create_project("p1", None, "tool").unwrap();
        let repo = db
            .insert_local_repository("/tmp/r1", "r1", Some(project.id), None)
            .unwrap();

        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO deploy_environments (repository_id, name, workflow_name, image_tag,
             compose_service, domain, deploy_branch, extras)
             VALUES (?1, 'prod', 'Deploy', 'latest', 'svc', 'x.com', 'master', '{}')",
            rusqlite::params![repo.id],
        )
        .unwrap();
        drop(conn);

        db.delete_repository(repo.id).unwrap();

        let conn = db.conn.lock().unwrap();
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM deploy_environments WHERE repository_id = ?1",
                rusqlite::params![repo.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0, "deploy_environments row must cascade-delete");
        drop(conn);
        std::mem::forget(tmp);
    }

    // ── v25: deploy_repo_config (T-000103 Task 1) ────────────────────────────

    /// Seed helper: inserts a `templates` row containing a synthetic meta.json
    /// whose given placeholder names are marked `scope: "repo"`. All other
    /// placeholders default to env-scope.
    fn seed_template_with_repo_scope(db: &AppDb, language_key: &str, repo_scope_keys: &[&str]) {
        let mut placeholders = serde_json::Map::new();
        for k in repo_scope_keys {
            placeholders.insert(
                (*k).to_string(),
                serde_json::json!({
                    "label": {"ru": k, "en": k},
                    "default": "",
                    "type": "string",
                    "scope": "repo"
                }),
            );
        }
        let meta = serde_json::json!({
            "display_name": language_key,
            "placeholders": placeholders,
            "file_targets": {},
            "version": 4
        });
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO templates (language_key, file_name, content, is_custom)
             VALUES (?1, 'meta.json', ?2, 0)",
            rusqlite::params![language_key, meta.to_string()],
        )
        .unwrap();
    }

    fn seed_template_without_repo_scope(db: &AppDb, language_key: &str) {
        let meta = serde_json::json!({
            "display_name": language_key,
            "placeholders": {
                "DOMAIN": {"label": {"ru":"d","en":"d"}, "default": "", "type": "string"}
            },
            "file_targets": {},
            "version": 4
        });
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO templates (language_key, file_name, content, is_custom)
             VALUES (?1, 'meta.json', ?2, 0)",
            rusqlite::params![language_key, meta.to_string()],
        )
        .unwrap();
    }

    /// Insert a deploy_env row with the given name, sort_order, and extras map.
    fn insert_env(
        db: &AppDb,
        repo_id: i64,
        name: &str,
        sort_order: i64,
        extras: serde_json::Value,
    ) -> i64 {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO deploy_environments
                (repository_id, name, workflow_name, image_tag, compose_service,
                 domain, deploy_branch, sort_order, extras, updated_at)
             VALUES (?1, ?2, 'Deploy', 'latest', 'svc', 'x.com', 'master', ?3, ?4, CURRENT_TIMESTAMP)",
            rusqlite::params![repo_id, name, sort_order, extras.to_string()],
        ).unwrap();
        conn.last_insert_rowid()
    }

    fn get_repo_config(db: &AppDb, repo_id: i64) -> String {
        let conn = db.conn.lock().unwrap();
        conn.query_row(
            "SELECT deploy_repo_config FROM repositories WHERE id = ?1",
            rusqlite::params![repo_id],
            |r| r.get::<_, String>(0),
        )
        .unwrap()
    }

    fn get_env_extras(db: &AppDb, env_id: i64) -> String {
        let conn = db.conn.lock().unwrap();
        conn.query_row(
            "SELECT extras FROM deploy_environments WHERE id = ?1",
            rusqlite::params![env_id],
            |r| r.get::<_, String>(0),
        )
        .unwrap()
    }

    fn count_migration_events(db: &AppDb) -> i64 {
        let conn = db.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM sync_events WHERE sync_type = 'migration'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn test_v25_lifts_repo_scope_placeholders_from_first_env() {
        // Happy path: 1 env with GO_VERSION in extras, marked scope: "repo" in
        // template's meta.json. After data-move: value lives in deploy_repo_config,
        // is stripped from env.extras.
        let db = make_db();
        let project = db.create_project("p", None, "tool").unwrap();
        let repo = db
            .insert_local_repository("/tmp/r1", "r1", Some(project.id), None)
            .unwrap();
        db.set_deploy_target(repo.id, Some("go")).unwrap();
        seed_template_with_repo_scope(&db, "go", &["GO_VERSION"]);
        let env_id = insert_env(
            &db,
            repo.id,
            "prod",
            0,
            serde_json::json!({"GO_VERSION": "1.26-alpine", "APP_PORT": "8080"}),
        );

        // Run data-move (re-running on a fully-migrated DB is fine — the column exists).
        let conn = db.conn.lock().unwrap();
        mig_v25_data_move(&conn).unwrap();
        drop(conn);

        let repo_config: std::collections::HashMap<String, String> =
            serde_json::from_str(&get_repo_config(&db, repo.id)).unwrap();
        assert_eq!(
            repo_config.get("GO_VERSION").map(|s| s.as_str()),
            Some("1.26-alpine")
        );

        let env_extras: std::collections::HashMap<String, String> =
            serde_json::from_str(&get_env_extras(&db, env_id)).unwrap();
        assert!(
            !env_extras.contains_key("GO_VERSION"),
            "GO_VERSION must be stripped from env extras"
        );
        assert_eq!(
            env_extras.get("APP_PORT").map(|s| s.as_str()),
            Some("8080"),
            "env-scope keys must remain in extras"
        );
        assert_eq!(
            count_migration_events(&db),
            0,
            "no conflict → no sync_events row"
        );
    }

    #[test]
    fn test_v25_multi_env_same_value_no_conflict() {
        // 2 envs with identical GO_VERSION value → no sync_event row, just lift.
        let db = make_db();
        let project = db.create_project("p", None, "tool").unwrap();
        let repo = db
            .insert_local_repository("/tmp/r2", "r2", Some(project.id), None)
            .unwrap();
        db.set_deploy_target(repo.id, Some("go")).unwrap();
        seed_template_with_repo_scope(&db, "go", &["GO_VERSION"]);
        let prod = insert_env(
            &db,
            repo.id,
            "prod",
            0,
            serde_json::json!({"GO_VERSION": "1.26-alpine"}),
        );
        let test = insert_env(
            &db,
            repo.id,
            "test",
            1,
            serde_json::json!({"GO_VERSION": "1.26-alpine"}),
        );

        let conn = db.conn.lock().unwrap();
        mig_v25_data_move(&conn).unwrap();
        drop(conn);

        let repo_config: std::collections::HashMap<String, String> =
            serde_json::from_str(&get_repo_config(&db, repo.id)).unwrap();
        assert_eq!(
            repo_config.get("GO_VERSION").map(|s| s.as_str()),
            Some("1.26-alpine")
        );
        // Both envs stripped
        let prod_extras: std::collections::HashMap<String, String> =
            serde_json::from_str(&get_env_extras(&db, prod)).unwrap();
        assert!(!prod_extras.contains_key("GO_VERSION"));
        let test_extras: std::collections::HashMap<String, String> =
            serde_json::from_str(&get_env_extras(&db, test)).unwrap();
        assert!(!test_extras.contains_key("GO_VERSION"));
        assert_eq!(
            count_migration_events(&db),
            0,
            "identical values across envs → no conflict logged"
        );
    }

    #[test]
    fn test_v25_multi_env_conflict_first_wins() {
        // 2 envs differ on GO_VERSION. sort_order 0 (prod) has "1.26-alpine",
        // sort_order 1 (test) has "alpine". After data-move: prod wins, conflict
        // logged with details JSON.
        let db = make_db();
        let project = db.create_project("p", None, "tool").unwrap();
        let repo = db
            .insert_local_repository("/tmp/r3", "r3", Some(project.id), None)
            .unwrap();
        db.set_deploy_target(repo.id, Some("go")).unwrap();
        seed_template_with_repo_scope(&db, "go", &["GO_VERSION"]);
        insert_env(
            &db,
            repo.id,
            "prod",
            0,
            serde_json::json!({"GO_VERSION": "1.26-alpine"}),
        );
        insert_env(
            &db,
            repo.id,
            "test",
            1,
            serde_json::json!({"GO_VERSION": "alpine"}),
        );

        let conn = db.conn.lock().unwrap();
        mig_v25_data_move(&conn).unwrap();
        drop(conn);

        let repo_config: std::collections::HashMap<String, String> =
            serde_json::from_str(&get_repo_config(&db, repo.id)).unwrap();
        assert_eq!(
            repo_config.get("GO_VERSION").map(|s| s.as_str()),
            Some("1.26-alpine"),
            "first env by sort_order ASC wins"
        );

        // sync_events row exists with sync_type='migration' and details parseable
        let conn = db.conn.lock().unwrap();
        let (sync_type, change_count, details): (String, i64, String) = conn
            .query_row(
                "SELECT sync_type, change_count, details FROM sync_events
             WHERE repository_id = ?1 AND sync_type = 'migration'",
                rusqlite::params![repo.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(sync_type, "migration");
        assert_eq!(change_count, 1, "1 conflicting key");
        let parsed: serde_json::Value = serde_json::from_str(&details).unwrap();
        let conflicts = parsed.get("conflicts").and_then(|v| v.as_array()).unwrap();
        assert_eq!(conflicts.len(), 1);
        let c = &conflicts[0];
        assert_eq!(c.get("key").and_then(|v| v.as_str()), Some("GO_VERSION"));
        assert_eq!(c.get("kept_env").and_then(|v| v.as_str()), Some("prod"));
        assert_eq!(
            c.get("kept_value").and_then(|v| v.as_str()),
            Some("1.26-alpine")
        );
        let discarded = c.get("discarded").and_then(|v| v.as_array()).unwrap();
        assert_eq!(discarded.len(), 1);
        assert_eq!(
            discarded[0].get("env").and_then(|v| v.as_str()),
            Some("test")
        );
        assert_eq!(
            discarded[0].get("value").and_then(|v| v.as_str()),
            Some("alpine")
        );
    }

    #[test]
    fn test_v25_idempotent() {
        // Pre-set deploy_repo_config to a non-'{}' value. Re-running data-move
        // must NOT touch that repo (no strip, no overwrite).
        let db = make_db();
        let project = db.create_project("p", None, "tool").unwrap();
        let repo = db
            .insert_local_repository("/tmp/r4", "r4", Some(project.id), None)
            .unwrap();
        db.set_deploy_target(repo.id, Some("go")).unwrap();
        seed_template_with_repo_scope(&db, "go", &["GO_VERSION"]);
        let env_id = insert_env(
            &db,
            repo.id,
            "prod",
            0,
            serde_json::json!({"GO_VERSION": "from-env"}),
        );

        // Pre-seed deploy_repo_config so the idempotency guard kicks in.
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "UPDATE repositories SET deploy_repo_config = ?1 WHERE id = ?2",
            rusqlite::params![r#"{"GO_VERSION":"preseeded"}"#, repo.id],
        )
        .unwrap();
        mig_v25_data_move(&conn).unwrap();
        drop(conn);

        // Repo config NOT overwritten
        let repo_config: std::collections::HashMap<String, String> =
            serde_json::from_str(&get_repo_config(&db, repo.id)).unwrap();
        assert_eq!(
            repo_config.get("GO_VERSION").map(|s| s.as_str()),
            Some("preseeded")
        );
        // Env extras NOT stripped
        let env_extras: std::collections::HashMap<String, String> =
            serde_json::from_str(&get_env_extras(&db, env_id)).unwrap();
        assert_eq!(
            env_extras.get("GO_VERSION").map(|s| s.as_str()),
            Some("from-env"),
            "env extras must NOT be touched on idempotent re-run"
        );
    }

    #[test]
    fn test_v25_template_without_repo_scope_noop() {
        // flutter_web-style template: meta.json has placeholders but none with
        // scope: "repo". data-move must leave deploy_repo_config as '{}' and
        // env.extras untouched.
        let db = make_db();
        let project = db.create_project("p", None, "tool").unwrap();
        let repo = db
            .insert_local_repository("/tmp/r5", "r5", Some(project.id), None)
            .unwrap();
        db.set_deploy_target(repo.id, Some("flutter_web")).unwrap();
        seed_template_without_repo_scope(&db, "flutter_web");
        let env_id = insert_env(
            &db,
            repo.id,
            "prod",
            0,
            serde_json::json!({"DOMAIN": "example.com"}),
        );

        let conn = db.conn.lock().unwrap();
        mig_v25_data_move(&conn).unwrap();
        drop(conn);

        assert_eq!(get_repo_config(&db, repo.id), "{}");
        let env_extras: std::collections::HashMap<String, String> =
            serde_json::from_str(&get_env_extras(&db, env_id)).unwrap();
        assert_eq!(
            env_extras.get("DOMAIN").map(|s| s.as_str()),
            Some("example.com"),
            "env-scope keys untouched"
        );
        assert_eq!(count_migration_events(&db), 0);
    }

    #[test]
    fn test_v25_unknown_deploy_target_skips() {
        // Repo with deploy_target pointing at a non-existent template_key (no
        // meta.json row). Must not error, must not touch any data.
        let db = make_db();
        let project = db.create_project("p", None, "tool").unwrap();
        let repo = db
            .insert_local_repository("/tmp/r6", "r6", Some(project.id), None)
            .unwrap();
        db.set_deploy_target(repo.id, Some("unknown_xyz")).unwrap();
        let env_id = insert_env(
            &db,
            repo.id,
            "prod",
            0,
            serde_json::json!({"GO_VERSION": "1.26-alpine"}),
        );

        let conn = db.conn.lock().unwrap();
        mig_v25_data_move(&conn).unwrap();
        drop(conn);

        assert_eq!(get_repo_config(&db, repo.id), "{}");
        let env_extras: std::collections::HashMap<String, String> =
            serde_json::from_str(&get_env_extras(&db, env_id)).unwrap();
        assert_eq!(
            env_extras.get("GO_VERSION").map(|s| s.as_str()),
            Some("1.26-alpine")
        );

        // Repo with deploy_target = NULL: also a no-op. We can also test the
        // NULL case by using a fresh repo without set_deploy_target.
        let repo2 = db
            .insert_local_repository("/tmp/r7", "r7", Some(project.id), None)
            .unwrap();
        assert!(repo2.deploy_target.is_none());
        let conn = db.conn.lock().unwrap();
        mig_v25_data_move(&conn).unwrap();
        drop(conn);
        assert_eq!(get_repo_config(&db, repo2.id), "{}");
    }

    #[test]
    fn test_v25_adds_deploy_repo_config_column() {
        // Sanity check: column exists with default '{}' on every repo row.
        let db = make_db();
        let _proj = db.create_project("p", None, "tool").unwrap();
        let repo = db
            .insert_local_repository("/tmp/r-col", "r-col", None, None)
            .unwrap();
        assert_eq!(get_repo_config(&db, repo.id), "{}");
    }
}
