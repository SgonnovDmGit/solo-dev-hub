// REQ-pair-specific operations: rename-replay, nested-folder migration,
// and bilateral pair-deletion (confirm). Called from `sync_project` and
// `confirm_requirement` in lib.rs.

use std::fs;
use std::path::Path;

use crate::models::Repository;
use crate::sync::fs::remove_file_if_exists;

/// B-000021: bilateral delete of a REQ pair across sender and recipient sides.
/// Resolves filesystem paths from the sender and recipient `Repository`
/// records directly — no dependency on "current project" — so the confirm
/// flow works symmetrically from either project's SyncScreen (server's
/// own view OR microservice's reverse-lookup view).
///
/// Path layout per direction (source_repo.role drives selection):
/// - `client | admin_client | test_client` → sender (client)
///   `docs/backend-requirements/` + recipient (server)
///   `docs/client-requirements/<client-canonical>/`.
/// - `server` → sender (server)
///   `docs/microservice-requirements/<ms-canonical>/` + recipient (ms-server)
///   `docs/server-requirements/<server-canonical>/`.
///
/// Returns `Err` for unsupported source roles (`tool` / `landing` / null) so
/// callers don't silently no-op on bad input — same contract as the previous
/// in-place implementation.
pub fn confirm_pair(
    source_repo: &Repository,
    target_repo: &Repository,
    filename: &str,
) -> Result<(), String> {
    let (Some(source_path), Some(target_path)) =
        (source_repo.local_path.as_ref(), target_repo.local_path.as_ref())
    else {
        return Ok(());
    };
    let source_base = Path::new(source_path);
    let target_base = Path::new(target_path);
    let response_name = filename.replace(".md", ".response.md");
    let source_role = source_repo.role.as_deref().unwrap_or("");

    let (sender_dir, recipient_dir) = match source_role {
        "client" | "admin_client" | "test_client" => {
            let sender = source_base.join("docs").join("backend-requirements");
            let recipient = target_base
                .join("docs")
                .join("client-requirements")
                .join(source_repo.canonical_folder_name());
            (sender, recipient)
        }
        "server" => {
            let sender = source_base
                .join("docs")
                .join("microservice-requirements")
                .join(target_repo.canonical_folder_name());
            let recipient = target_base
                .join("docs")
                .join("server-requirements")
                .join(source_repo.canonical_folder_name());
            (sender, recipient)
        }
        _ => {
            return Err(format!(
                "Unexpected source repo role for REQ confirm: {:?}",
                source_repo.role
            ));
        }
    };

    remove_file_if_exists(&sender_dir.join(filename))?;
    remove_file_if_exists(&sender_dir.join(&response_name))?;
    remove_file_if_exists(&recipient_dir.join(filename))?;
    remove_file_if_exists(&recipient_dir.join(&response_name))?;
    Ok(())
}

/// F-033 Stage 1e: replay a pending repo rename on the recipient side.
/// Idempotent: returns Ok(false) when there's nothing to do (old dir missing,
/// or new dir already exists — the latter counts as "migrated earlier, or
/// operator created the new folder manually"). Returns Ok(true) when the
/// filesystem was actually mutated.
///
/// Callers pass the **parent** directory (e.g. `srv/docs/client-requirements/`)
/// and the `old`/`new` canonical folder names. No DB state is updated — idempotency
/// comes from fs checks, not from a persistent "applied" flag.
/// M7 review-fix: distinguishable outcomes so callers can surface
/// "collision" as a warning instead of swallowing it as no-op. Previously
/// the function returned `Ok(false)` for both "nothing to do" and "new dir
/// already exists alongside old" — the latter is a manual-intervention
/// signal, the former is normal.
#[derive(Debug, PartialEq, Eq)]
pub enum RenameOutcome {
    /// Successfully renamed `old` → `new`.
    Renamed,
    /// No rename needed (old missing, identical names, or empty inputs).
    NoOp,
    /// Both `old` and `new` directories exist — ambiguous state, left as-is.
    Collision,
}

pub fn replay_rename_in_dir(parent: &Path, old: &str, new: &str) -> Result<RenameOutcome, String> {
    if old == new || old.is_empty() || new.is_empty() {
        return Ok(RenameOutcome::NoOp);
    }
    let old_dir = parent.join(old);
    let new_dir = parent.join(new);
    if !old_dir.exists() {
        return Ok(RenameOutcome::NoOp);
    }
    if new_dir.exists() {
        return Ok(RenameOutcome::Collision);
    }
    fs::rename(&old_dir, &new_dir).map_err(|e| {
        format!(
            "fs::rename {} -> {}: {}",
            old_dir.display(),
            new_dir.display(),
            e
        )
    })?;
    Ok(RenameOutcome::Renamed)
}

/// F-033 Stage 1f Case B: one-time migration of a subfolder under a parent dir,
/// renaming `old_name/` to `new_name/` if and only if:
///   - `old_name` != `new_name`
///   - `parent/old_name/` exists
///   - `parent/new_name/` does NOT exist
/// Returns Ok(true) on actual rename, Ok(false) on no-op, and pushes a warning
/// to `warnings` if both dirs coexist (ambiguous — operator must resolve).
pub fn migrate_subfolder_rename(
    parent: &Path,
    old_name: &str,
    new_name: &str,
    warnings: &mut Vec<String>,
) -> Result<bool, String> {
    if old_name == new_name || old_name.is_empty() || new_name.is_empty() {
        return Ok(false);
    }
    let old_dir = parent.join(old_name);
    let new_dir = parent.join(new_name);
    if !old_dir.exists() {
        return Ok(false);
    }
    if new_dir.exists() {
        warnings.push(format!(
            "Migration ambiguous: both {} and {} exist — leaving alone (user should merge or remove one)",
            old_dir.display(),
            new_dir.display()
        ));
        return Ok(false);
    }
    fs::rename(&old_dir, &new_dir).map_err(|e| {
        format!(
            "fs::rename {} -> {}: {}",
            old_dir.display(),
            new_dir.display(),
            e
        )
    })?;
    Ok(true)
}

/// F-033 Stage 1f Case C: migrate files from the flat `server-requirements/`
/// on the microservice side into per-parent subfolders `server-requirements/<parent>/`.
///
/// Attribution algorithm per flat REQ file:
/// - Collect matching parents (byte-equal copy at `<parent-side>/<filename>`).
/// - Collect conflicting parents (copy exists but content differs).
/// - If any conflicts → leave file in flat root + push error (unsafe).
/// - Else if 0 matches → leave + warn (orphan).
/// - Else if 1 match → `fs::rename` into that parent's subfolder.
/// - Else (>= 2 same-content) → `fs::copy` into each matching parent's subfolder,
///   then `fs::remove_file` the source.
///
/// `parent_lookup` is a callable `fn(filename) -> Vec<(parent_canonical, existing_path_on_parent)>`.
/// Passing it as a closure keeps this helper unit-testable without DB / repo deps.
///
/// Returns number of flat files migrated (moved or copied-N-and-removed). Errors and
/// orphan warnings are appended to `warnings`.
pub fn migrate_flat_to_nested<F>(
    flat_root: &Path,
    mut parent_lookup: F,
    warnings: &mut Vec<String>,
) -> Result<usize, String>
where
    F: FnMut(&str) -> Vec<(String, std::path::PathBuf)>,
{
    if !flat_root.exists() || !flat_root.is_dir() {
        return Ok(0);
    }
    // Collect flat REQ/.response files (skip subdirs and non-matching names).
    let mut flat_files: Vec<std::path::PathBuf> = Vec::new();
    for entry in fs::read_dir(flat_root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with("REQ-") {
            continue;
        }
        if !name.ends_with(".md") {
            continue;
        }
        flat_files.push(p);
    }

    let mut migrated: usize = 0;
    for flat in flat_files {
        let fname = flat
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let flat_content = match fs::read(&flat) {
            Ok(c) => c,
            Err(e) => {
                warnings.push(format!("Case C: read {}: {}", flat.display(), e));
                continue;
            }
        };
        let candidates = parent_lookup(&fname);
        let mut matches: Vec<String> = Vec::new();
        let mut conflicts: Vec<String> = Vec::new();
        for (parent_canonical, candidate_path) in candidates {
            match fs::read(&candidate_path) {
                Ok(c) if c == flat_content => matches.push(parent_canonical),
                Ok(_) => conflicts.push(parent_canonical),
                Err(_) => { /* parent-side file missing or unreadable — ignore */ }
            }
        }
        if !conflicts.is_empty() {
            warnings.push(format!(
                "Case C: {} has diverged copies on parent(s) [{}] — manual resolution needed, left in flat root",
                fname,
                conflicts.join(", ")
            ));
            continue;
        }
        match matches.len() {
            0 => {
                warnings.push(format!(
                    "Case C: {} has no matching parent copy — possibly stale, left in flat root",
                    fname
                ));
            }
            1 => {
                let dst = flat_root.join(&matches[0]).join(&fname);
                if let Some(dst_dir) = dst.parent() {
                    fs::create_dir_all(dst_dir).map_err(|e| e.to_string())?;
                }
                fs::rename(&flat, &dst).map_err(|e| {
                    format!(
                        "Case C: rename {} -> {}: {}",
                        flat.display(),
                        dst.display(),
                        e
                    )
                })?;
                migrated += 1;
            }
            _ => {
                // Multi-parent same-content: duplicate into each matching parent's
                // subfolder. If any copy fails mid-loop, roll back the successful
                // ones and leave the flat source intact — preserves "all or nothing"
                // for the parent set so the next sync pass can retry cleanly without
                // ghost partial copies in some parents.
                let mut created: Vec<std::path::PathBuf> = Vec::new();
                let mut copy_error: Option<String> = None;
                for parent_canonical in &matches {
                    let dst = flat_root.join(parent_canonical).join(&fname);
                    if let Some(dst_dir) = dst.parent() {
                        if let Err(e) = fs::create_dir_all(dst_dir) {
                            copy_error = Some(e.to_string());
                            break;
                        }
                    }
                    if let Err(e) = fs::copy(&flat, &dst) {
                        copy_error = Some(format!(
                            "Case C: copy {} -> {}: {}",
                            flat.display(),
                            dst.display(),
                            e
                        ));
                        break;
                    }
                    created.push(dst);
                }
                if let Some(err) = copy_error {
                    for dst in &created {
                        let _ = fs::remove_file(dst);
                    }
                    warnings.push(format!(
                        "Case C: {} partial-copy failed, rolled back ({}); left in flat root for retry",
                        fname, err
                    ));
                    continue;
                }
                fs::remove_file(&flat)
                    .map_err(|e| format!("Case C: remove flat source {}: {}", flat.display(), e))?;
                migrated += 1;
            }
        }
    }
    Ok(migrated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // ── F-033 Stage 1e: replay_rename_in_dir ─────────────────────────────────

    #[test]
    fn test_replay_rename_noop_when_old_missing() {
        let tmp = TempDir::new().unwrap();
        let parent = tmp.path();
        // new exists but no old
        fs::create_dir_all(parent.join("new-name")).unwrap();
        let result = replay_rename_in_dir(parent, "old-name", "new-name").unwrap();
        assert_eq!(result, RenameOutcome::NoOp, "no-op when old_dir missing");
        assert!(parent.join("new-name").exists());
    }

    #[test]
    fn test_replay_rename_success() {
        let tmp = TempDir::new().unwrap();
        let parent = tmp.path();
        fs::create_dir_all(parent.join("old-name")).unwrap();
        fs::write(parent.join("old-name/REQ-001.md"), "x").unwrap();

        let result = replay_rename_in_dir(parent, "old-name", "new-name").unwrap();
        assert_eq!(result, RenameOutcome::Renamed);
        assert!(!parent.join("old-name").exists());
        assert!(parent.join("new-name").exists());
        assert!(parent.join("new-name/REQ-001.md").exists());
    }

    #[test]
    fn test_replay_rename_collision_when_both_exist() {
        let tmp = TempDir::new().unwrap();
        let parent = tmp.path();
        fs::create_dir_all(parent.join("old-name")).unwrap();
        fs::create_dir_all(parent.join("new-name")).unwrap();

        let result = replay_rename_in_dir(parent, "old-name", "new-name").unwrap();
        assert_eq!(
            result,
            RenameOutcome::Collision,
            "both dirs present → collision"
        );
        assert!(parent.join("old-name").exists(), "old preserved");
        assert!(parent.join("new-name").exists(), "new preserved");
    }

    #[test]
    fn test_replay_rename_same_name_noop() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("same")).unwrap();
        let result = replay_rename_in_dir(tmp.path(), "same", "same").unwrap();
        assert_eq!(result, RenameOutcome::NoOp);
    }

    // ── F-033 Stage 1f Case B: migrate_subfolder_rename ──────────────────────

    #[test]
    fn test_migrate_subfolder_rename_basic() {
        let tmp = TempDir::new().unwrap();
        let parent = tmp.path();
        fs::create_dir_all(parent.join("ProjectName")).unwrap();
        fs::write(parent.join("ProjectName/REQ-001.md"), "data").unwrap();

        let mut warnings = Vec::new();
        let result =
            migrate_subfolder_rename(parent, "ProjectName", "repo-canonical", &mut warnings)
                .unwrap();
        assert!(result);
        assert!(!parent.join("ProjectName").exists());
        assert!(parent.join("repo-canonical/REQ-001.md").exists());
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_migrate_subfolder_rename_ambiguous_both_exist() {
        let tmp = TempDir::new().unwrap();
        let parent = tmp.path();
        fs::create_dir_all(parent.join("old")).unwrap();
        fs::create_dir_all(parent.join("new")).unwrap();

        let mut warnings = Vec::new();
        let result = migrate_subfolder_rename(parent, "old", "new", &mut warnings).unwrap();
        assert!(!result);
        assert!(parent.join("old").exists());
        assert!(parent.join("new").exists());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("ambiguous"));
    }

    // ── F-033 Stage 1f Case C: migrate_flat_to_nested ────────────────────────

    #[test]
    fn test_case_c_single_parent_migrates_to_subfolder() {
        let tmp = TempDir::new().unwrap();
        let ms_flat = tmp.path().join("server-requirements");
        fs::create_dir_all(&ms_flat).unwrap();
        fs::write(ms_flat.join("REQ-001.md"), "payload").unwrap();
        fs::write(ms_flat.join("REQ-002.md"), "payload2").unwrap();

        // Parent side — both REQ present with identical content.
        let parent_side = tmp.path().join("parent-side");
        fs::create_dir_all(&parent_side).unwrap();
        fs::write(parent_side.join("REQ-001.md"), "payload").unwrap();
        fs::write(parent_side.join("REQ-002.md"), "payload2").unwrap();

        let lookup = |name: &str| -> Vec<(String, std::path::PathBuf)> {
            vec![("parent-A".to_string(), parent_side.join(name))]
        };

        let mut warnings = Vec::new();
        let migrated = migrate_flat_to_nested(&ms_flat, lookup, &mut warnings).unwrap();
        assert_eq!(migrated, 2);
        assert!(ms_flat.join("parent-A/REQ-001.md").exists());
        assert!(ms_flat.join("parent-A/REQ-002.md").exists());
        assert!(!ms_flat.join("REQ-001.md").exists());
        assert!(!ms_flat.join("REQ-002.md").exists());
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_case_c_multi_parent_same_content_duplicates() {
        let tmp = TempDir::new().unwrap();
        let ms_flat = tmp.path().join("server-requirements");
        fs::create_dir_all(&ms_flat).unwrap();
        fs::write(ms_flat.join("REQ-007.md"), "shared").unwrap();

        let pa = tmp.path().join("pa");
        let pb = tmp.path().join("pb");
        fs::create_dir_all(&pa).unwrap();
        fs::create_dir_all(&pb).unwrap();
        fs::write(pa.join("REQ-007.md"), "shared").unwrap();
        fs::write(pb.join("REQ-007.md"), "shared").unwrap();

        let lookup = |name: &str| -> Vec<(String, std::path::PathBuf)> {
            vec![
                ("parentA".to_string(), pa.join(name)),
                ("parentB".to_string(), pb.join(name)),
            ]
        };

        let mut warnings = Vec::new();
        let migrated = migrate_flat_to_nested(&ms_flat, lookup, &mut warnings).unwrap();
        assert_eq!(migrated, 1);
        assert!(ms_flat.join("parentA/REQ-007.md").exists());
        assert!(ms_flat.join("parentB/REQ-007.md").exists());
        assert!(!ms_flat.join("REQ-007.md").exists());
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_case_c_multi_parent_different_content_unsafe() {
        let tmp = TempDir::new().unwrap();
        let ms_flat = tmp.path().join("server-requirements");
        fs::create_dir_all(&ms_flat).unwrap();
        fs::write(ms_flat.join("REQ-010.md"), "payloadA").unwrap();

        let pa = tmp.path().join("pa");
        let pb = tmp.path().join("pb");
        fs::create_dir_all(&pa).unwrap();
        fs::create_dir_all(&pb).unwrap();
        fs::write(pa.join("REQ-010.md"), "payloadA").unwrap();
        fs::write(pb.join("REQ-010.md"), "payloadB-different").unwrap();

        let lookup = |name: &str| -> Vec<(String, std::path::PathBuf)> {
            vec![
                ("parentA".to_string(), pa.join(name)),
                ("parentB".to_string(), pb.join(name)),
            ]
        };

        let mut warnings = Vec::new();
        let migrated = migrate_flat_to_nested(&ms_flat, lookup, &mut warnings).unwrap();
        assert_eq!(migrated, 0);
        assert!(ms_flat.join("REQ-010.md").exists(), "kept in flat root");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("diverged") || warnings[0].contains("parentB"));
    }

    #[test]
    fn test_case_c_orphan_file() {
        let tmp = TempDir::new().unwrap();
        let ms_flat = tmp.path().join("server-requirements");
        fs::create_dir_all(&ms_flat).unwrap();
        fs::write(ms_flat.join("REQ-999.md"), "stale").unwrap();

        let pa = tmp.path().join("pa");
        fs::create_dir_all(&pa).unwrap();
        // parent has no REQ-999

        let lookup = |name: &str| -> Vec<(String, std::path::PathBuf)> {
            vec![("parentA".to_string(), pa.join(name))]
        };

        let mut warnings = Vec::new();
        let migrated = migrate_flat_to_nested(&ms_flat, lookup, &mut warnings).unwrap();
        assert_eq!(migrated, 0);
        assert!(ms_flat.join("REQ-999.md").exists());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("no matching parent copy"));
    }

    #[test]
    fn test_case_c_idempotent() {
        let tmp = TempDir::new().unwrap();
        let ms_flat = tmp.path().join("server-requirements");
        fs::create_dir_all(&ms_flat).unwrap();
        fs::write(ms_flat.join("REQ-001.md"), "x").unwrap();
        let pa = tmp.path().join("pa");
        fs::create_dir_all(&pa).unwrap();
        fs::write(pa.join("REQ-001.md"), "x").unwrap();

        let mk_lookup = || {
            let pa_cloned = pa.clone();
            move |name: &str| -> Vec<(String, std::path::PathBuf)> {
                vec![("parentA".to_string(), pa_cloned.join(name))]
            }
        };

        let mut w1 = Vec::new();
        let m1 = migrate_flat_to_nested(&ms_flat, mk_lookup(), &mut w1).unwrap();
        assert_eq!(m1, 1);

        // Second pass: flat root is empty of REQ-*.md → no-op.
        let mut w2 = Vec::new();
        let m2 = migrate_flat_to_nested(&ms_flat, mk_lookup(), &mut w2).unwrap();
        assert_eq!(m2, 0);
        assert!(w2.is_empty());
    }

    #[test]
    fn test_case_c_response_file_migrates_too() {
        let tmp = TempDir::new().unwrap();
        let ms_flat = tmp.path().join("server-requirements");
        fs::create_dir_all(&ms_flat).unwrap();
        fs::write(ms_flat.join("REQ-001.md"), "req").unwrap();
        fs::write(ms_flat.join("REQ-001.response.md"), "resp").unwrap();

        let pa = tmp.path().join("pa");
        fs::create_dir_all(&pa).unwrap();
        fs::write(pa.join("REQ-001.md"), "req").unwrap();
        fs::write(pa.join("REQ-001.response.md"), "resp").unwrap();

        let lookup = |name: &str| -> Vec<(String, std::path::PathBuf)> {
            vec![("parentA".to_string(), pa.join(name))]
        };

        let mut warnings = Vec::new();
        let migrated = migrate_flat_to_nested(&ms_flat, lookup, &mut warnings).unwrap();
        assert_eq!(migrated, 2);
        assert!(ms_flat.join("parentA/REQ-001.md").exists());
        assert!(ms_flat.join("parentA/REQ-001.response.md").exists());
    }

    // ── B-000021: confirm_pair bilateral REQ-pair delete ─────────────────────

    fn mk_repo(id: i64, github_name: &str, role: &str, local_path: &Path) -> Repository {
        Repository {
            id,
            project_id: None,
            github_name: Some(github_name.to_string()),
            github_url: None,
            role: Some(role.to_string()),
            description: None,
            language: None,
            last_pushed_at: None,
            added_at: String::new(),
            updated_at: String::new(),
            local_path: Some(local_path.to_string_lossy().to_string()),
            github_id: None,
            deploy_target: None,
        }
    }

    fn seed_pair(dir: &Path, filename: &str) {
        fs::create_dir_all(dir).unwrap();
        let stem = filename.trim_end_matches(".md");
        fs::write(dir.join(filename), "req body").unwrap();
        fs::write(dir.join(format!("{stem}.response.md")), "receipt body").unwrap();
    }

    #[test]
    fn test_confirm_pair_client_to_server_deletes_both_sides() {
        let tmp = TempDir::new().unwrap();
        let client_base = tmp.path().join("client");
        let server_base = tmp.path().join("server");
        let client = mk_repo(1, "owner/web-client", "client", &client_base);
        let server = mk_repo(2, "owner/backend", "server", &server_base);

        let client_dir = client_base.join("docs/backend-requirements");
        let server_dir = server_base.join("docs/client-requirements/web-client");
        seed_pair(&client_dir, "REQ-001_login.md");
        seed_pair(&server_dir, "REQ-001_login.md");

        confirm_pair(&client, &server, "REQ-001_login.md").unwrap();

        assert!(!client_dir.join("REQ-001_login.md").exists());
        assert!(!client_dir.join("REQ-001_login.response.md").exists());
        assert!(!server_dir.join("REQ-001_login.md").exists());
        assert!(!server_dir.join("REQ-001_login.response.md").exists());
    }

    #[test]
    fn test_confirm_pair_server_to_microservice_deletes_both_sides() {
        let tmp = TempDir::new().unwrap();
        let server_base = tmp.path().join("server");
        let ms_base = tmp.path().join("ms");
        let server = mk_repo(1, "owner/backend", "server", &server_base);
        let ms_server = mk_repo(2, "owner/avatar-ms", "server", &ms_base);

        let server_dir = server_base.join("docs/microservice-requirements/avatar-ms");
        let ms_dir = ms_base.join("docs/server-requirements/backend");
        seed_pair(&server_dir, "REQ-007_blobs.md");
        seed_pair(&ms_dir, "REQ-007_blobs.md");

        confirm_pair(&server, &ms_server, "REQ-007_blobs.md").unwrap();

        assert!(!server_dir.join("REQ-007_blobs.md").exists());
        assert!(!server_dir.join("REQ-007_blobs.response.md").exists());
        assert!(!ms_dir.join("REQ-007_blobs.md").exists());
        assert!(!ms_dir.join("REQ-007_blobs.response.md").exists());
    }

    #[test]
    fn test_confirm_pair_only_deletes_target_specific_paths() {
        // Sibling MS with same NNN should NOT be touched (replicates C1
        // disambiguation invariant from v0.27.1).
        let tmp = TempDir::new().unwrap();
        let server_base = tmp.path().join("server");
        let ms_a_base = tmp.path().join("ms-a");
        let ms_b_base = tmp.path().join("ms-b");
        let server = mk_repo(1, "owner/backend", "server", &server_base);
        let ms_a = mk_repo(2, "owner/avatar-ms", "server", &ms_a_base);
        let _ms_b = mk_repo(3, "owner/notify-ms", "server", &ms_b_base);

        let server_dir_a = server_base.join("docs/microservice-requirements/avatar-ms");
        let server_dir_b = server_base.join("docs/microservice-requirements/notify-ms");
        let ms_a_dir = ms_a_base.join("docs/server-requirements/backend");
        let ms_b_dir = ms_b_base.join("docs/server-requirements/backend");
        seed_pair(&server_dir_a, "REQ-005_x.md");
        seed_pair(&server_dir_b, "REQ-005_x.md");
        seed_pair(&ms_a_dir, "REQ-005_x.md");
        seed_pair(&ms_b_dir, "REQ-005_x.md");

        confirm_pair(&server, &ms_a, "REQ-005_x.md").unwrap();

        // MS-A pair gone
        assert!(!server_dir_a.join("REQ-005_x.md").exists());
        assert!(!ms_a_dir.join("REQ-005_x.md").exists());
        // MS-B pair untouched (sibling NNN collision protected)
        assert!(server_dir_b.join("REQ-005_x.md").exists());
        assert!(server_dir_b.join("REQ-005_x.response.md").exists());
        assert!(ms_b_dir.join("REQ-005_x.md").exists());
        assert!(ms_b_dir.join("REQ-005_x.response.md").exists());
    }

    #[test]
    fn test_confirm_pair_unknown_role_errors() {
        let tmp = TempDir::new().unwrap();
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        let source = mk_repo(1, "owner/tool", "tool", &a);
        let target = mk_repo(2, "owner/other", "server", &b);

        let err = confirm_pair(&source, &target, "REQ-001.md").unwrap_err();
        assert!(err.contains("Unexpected source repo role"), "got: {}", err);
    }

    #[test]
    fn test_confirm_pair_missing_local_path_noop() {
        let tmp = TempDir::new().unwrap();
        let a_path = tmp.path().join("a");
        let mut source = mk_repo(1, "owner/client", "client", &a_path);
        source.local_path = None;
        let target = mk_repo(2, "owner/server", "server", tmp.path());

        // Does not error — silent no-op when either side has no local_path,
        // mirroring previous in-place behavior (paths can't be derived).
        confirm_pair(&source, &target, "REQ-001.md").unwrap();
    }
}
