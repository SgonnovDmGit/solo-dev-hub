// Filesystem primitives: path safety, file copy/move, REQ scan helpers.

use std::fs;
use std::path::{Path, PathBuf};

/// Ensure repo root folder exists on disk. Returns error if missing — never creates.
/// Used as a guard at entry points before any create_dir_all calls, so that a stale
/// local_path in the DB does not silently recreate a deleted/moved folder.
pub fn ensure_root_exists(root: &Path) -> Result<(), String> {
    if !root.exists() {
        return Err(format!(
            "Repo root folder not found: {}. Update local_path in settings or restore the folder.",
            root.display()
        ));
    }
    if !root.is_dir() {
        return Err(format!(
            "Repo root path is not a directory: {}",
            root.display()
        ));
    }
    Ok(())
}

/// Reject paths that would escape the repo root: absolute paths, paths
/// containing `..`, or paths with drive letters / root components. Used to
/// guard `write_deploy_files` against malicious `file_targets` in meta.json.
/// Accepts forward and backward separators uniformly via `Path::components()`.
pub fn is_safe_subpath(rel: &str) -> bool {
    use std::path::Component;
    let p = Path::new(rel);
    if p.is_absolute() {
        return false;
    }
    p.components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// Remove `.git` folder inside `repo_root` if it exists. No-op if missing.
/// Used by B-003 delete-repo flow; leaves other files in the folder untouched.
pub fn remove_git_dir(repo_root: &Path) -> Result<(), String> {
    let git_dir = repo_root.join(".git");
    if git_dir.exists() {
        fs::remove_dir_all(&git_dir)
            .map_err(|e| format!("Failed to remove {}: {}", git_dir.display(), e))?;
    }
    Ok(())
}

/// Scan directory for REQ-*.md files (not .response.md, not .impl.md).
/// The `.impl.md` acknowledgement is a close *signal*, not a requirement — it
/// is consumed by `auto_close_impl_pairs` (which runs first in the sync pass).
/// Excluding it here is a safety net: if that teardown ever fails partway (e.g.
/// a Windows file lock on delete), a stray `.impl.md` must not be mistaken for
/// a requirement and copied to the recipient as a phantom pair.
pub fn scan_requirements(dir: &Path) -> Vec<String> {
    if !dir.exists() {
        return vec![];
    }
    fs::read_dir(dir)
        .ok()
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|name| {
                    name.starts_with("REQ-")
                        && name.ends_with(".md")
                        && !name.ends_with(".response.md")
                        && !name.ends_with(".impl.md")
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Scan directory for *.response.md files
pub fn scan_responses(dir: &Path) -> Vec<String> {
    if !dir.exists() {
        return vec![];
    }
    fs::read_dir(dir)
        .ok()
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|name| name.ends_with(".response.md"))
                .collect()
        })
        .unwrap_or_default()
}

/// F-000039: scan directory for REQ-*.impl.md acknowledgement files.
pub fn scan_impl_files(dir: &Path) -> Vec<String> {
    if !dir.exists() {
        return vec![];
    }
    fs::read_dir(dir)
        .ok()
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|name| name.starts_with("REQ-") && name.ends_with(".impl.md"))
                .collect()
        })
        .unwrap_or_default()
}

/// Remove a file
pub fn remove_file_if_exists(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Copy file, overwriting only if content changed
pub fn copy_file_if_changed(source: &Path, target: &Path) -> Result<bool, String> {
    if !source.exists() {
        return Ok(false);
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if target.exists() {
        let src = fs::read(source).map_err(|e| e.to_string())?;
        let dst = fs::read(target).map_err(|e| e.to_string())?;
        if src == dst {
            return Ok(false);
        }
    }
    fs::copy(source, target).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Outcome of a directory mirror pass.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MirrorOutcome {
    /// Files written because content was new or changed.
    pub copied: usize,
    /// Files and directories removed from `dst` because they vanished from `src`.
    pub pruned: usize,
}

/// Mirror `src` tree onto `dst` read-only: copy changed files (via
/// `copy_file_if_changed`, preserving relative subpaths) and prune any file or
/// directory under `dst` that has no counterpart under `src`. `dst` is created
/// if missing. Returns counts of files copied and entries pruned.
///
/// Errors if `src` is missing or is not a directory (the caller is responsible
/// for checking source existence — a mirror of "nothing" would wipe `dst`,
/// which must be an explicit caller decision, not a silent side effect).
pub fn mirror_dir(src: &Path, dst: &Path) -> Result<MirrorOutcome, String> {
    if !src.exists() {
        return Err(format!("Mirror source not found: {}", src.display()));
    }
    if !src.is_dir() {
        return Err(format!(
            "Mirror source is not a directory: {}",
            src.display()
        ));
    }
    // Ensure the destination root exists even when src holds no files.
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;

    let mut outcome = MirrorOutcome::default();
    copy_tree(src, dst, &mut outcome.copied)?;
    prune_tree(src, dst, &mut outcome.pruned)?;
    Ok(outcome)
}

/// Recursively copy every file under `src` into the matching path under `dst`,
/// skipping unchanged content. `copied` is incremented for each real write.
fn copy_tree(src: &Path, dst: &Path, copied: &mut usize) -> Result<(), String> {
    let entries = fs::read_dir(src).map_err(|e| e.to_string())?;
    for entry in entries.filter_map(|e| e.ok()) {
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_tree(&src_path, &dst_path, copied)?;
        } else if src_path.is_file() && copy_file_if_changed(&src_path, &dst_path)? {
            *copied += 1;
        }
    }
    Ok(())
}

/// Recursively remove any entry under `dst` whose counterpart under `src` is
/// gone. A vanished directory is removed whole (counted once) and not descended
/// into; a vanished file is removed and counted once.
fn prune_tree(src: &Path, dst: &Path, pruned: &mut usize) -> Result<(), String> {
    let entries = fs::read_dir(dst).map_err(|e| e.to_string())?;
    for entry in entries.filter_map(|e| e.ok()) {
        let dst_path = entry.path();
        let src_path = src.join(entry.file_name());
        if dst_path.is_dir() {
            if src_path.is_dir() {
                // Counterpart survives — descend to prune within it.
                prune_tree(&src_path, &dst_path, pruned)?;
            } else {
                // Whole subtree gone from src — remove as a unit, count once.
                fs::remove_dir_all(&dst_path).map_err(|e| e.to_string())?;
                *pruned += 1;
            }
        } else if !src_path.exists() {
            fs::remove_file(&dst_path).map_err(|e| e.to_string())?;
            *pruned += 1;
        }
    }
    Ok(())
}

/// T-000139: copy every `*.md` from `<sender_docs>/my_api/` into `recipient_inbox`
/// (flat). Returns the count of files actually written (content changed). A
/// missing `my_api` folder → Ok(0). Non-`.md` files and subdirectories are
/// ignored. Reuses `copy_file_if_changed` (which creates the inbox dir + skips
/// unchanged files).
pub fn copy_my_api_dir(sender_docs: &Path, recipient_inbox: &Path) -> Result<usize, String> {
    let my_api = sender_docs.join("my_api");
    if !my_api.is_dir() {
        return Ok(0);
    }
    let mut copied = 0usize;
    let entries = fs::read_dir(&my_api).map_err(|e| e.to_string())?;
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_md = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.eq_ignore_ascii_case("md"))
            .unwrap_or(false);
        if !is_md {
            continue;
        }
        let name = entry.file_name();
        let target = recipient_inbox.join(&name);
        if copy_file_if_changed(&path, &target)? {
            copied += 1;
        }
    }
    Ok(copied)
}

/// T-000173: count regular FILES (not directories) under `root`, recursively.
/// Returns 0 if `root` does not exist or is not a directory. Same std::fs
/// walking idiom as `copy_tree` — no external walkdir crate.
pub fn count_files_recursive(root: &Path) -> usize {
    if !root.is_dir() {
        return 0;
    }
    let mut count = 0usize;
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() {
                count += count_files_recursive(&path);
            } else if path.is_file() {
                count += 1;
            }
        }
    }
    count
}

/// True iff `dst` is an exact mirror of `src`: identical set of relative file
/// paths and identical bytes for each file. Returns false if `src` is missing or
/// empty (nothing to deliver) or `dst` is missing. Lets a codex broadcast edge
/// report synced vs pending without performing the copy — same semantics
/// `mirror_dir` would produce (a mirror is in sync when it would copy/prune 0).
pub fn dirs_in_sync(src: &Path, dst: &Path) -> bool {
    if !src.is_dir() || !dst.is_dir() {
        return false;
    }
    let mut src_files: Vec<PathBuf> = Vec::new();
    collect_rel_files(src, Path::new(""), &mut src_files);
    if src_files.is_empty() {
        return false; // no svod content → nothing is "synced"
    }
    let mut dst_files: Vec<PathBuf> = Vec::new();
    collect_rel_files(dst, Path::new(""), &mut dst_files);
    src_files.sort();
    dst_files.sort();
    if src_files != dst_files {
        return false;
    }
    src_files.iter().all(|rel| {
        matches!(
            (fs::read(src.join(rel)), fs::read(dst.join(rel))),
            (Ok(a), Ok(b)) if a == b
        )
    })
}

/// Collect relative file paths (dirs recursed, files pushed) under `root`.
fn collect_rel_files(root: &Path, prefix: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            let rel = prefix.join(entry.file_name());
            if path.is_dir() {
                collect_rel_files(&path, &rel, out);
            } else if path.is_file() {
                out.push(rel);
            }
        }
    }
}

/// Move a file atomically: write to target first, then delete source.
/// If write fails, source remains intact — no data loss.
/// Target's parent directory is created if needed.
pub fn migrate_file(source: &Path, target: &Path) -> Result<(), String> {
    if !source.exists() {
        return Err(format!("Source not found: {}", source.display()));
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::copy(source, target).map_err(|e| format!("Copy failed: {}", e))?;
    fs::remove_file(source).map_err(|e| format!("Remove source failed: {}", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_ensure_root_exists_ok() {
        let tmp = TempDir::new().unwrap();
        assert!(ensure_root_exists(tmp.path()).is_ok());
    }

    #[test]
    fn test_is_safe_subpath_accepts_normal() {
        assert!(is_safe_subpath("Dockerfile"));
        assert!(is_safe_subpath(".github/workflows/deploy.yml"));
        assert!(is_safe_subpath("subdir/file.txt"));
        assert!(is_safe_subpath("./Dockerfile"));
    }

    #[test]
    fn test_is_safe_subpath_rejects_parent_dir() {
        assert!(!is_safe_subpath("../escape.yml"));
        assert!(!is_safe_subpath("ok/../../escape.yml"));
        assert!(!is_safe_subpath(".github/../../../etc/passwd"));
    }

    #[test]
    fn test_is_safe_subpath_rejects_absolute() {
        assert!(!is_safe_subpath("/etc/passwd"));
        assert!(!is_safe_subpath("/tmp/x.yml"));
    }

    #[cfg(windows)]
    #[test]
    fn test_is_safe_subpath_rejects_windows_absolute() {
        assert!(!is_safe_subpath("C:\\Windows\\system32\\foo"));
        assert!(!is_safe_subpath("C:/x.yml"));
        assert!(!is_safe_subpath("\\server\\share\\x"));
    }

    #[test]
    fn test_ensure_root_exists_missing() {
        let result = ensure_root_exists(Path::new("/nonexistent/abc123/xyz"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_remove_git_dir_missing_is_noop() {
        let tmp = TempDir::new().unwrap();
        // No .git folder — should succeed without errors.
        assert!(remove_git_dir(tmp.path()).is_ok());
    }

    #[test]
    fn test_remove_git_dir_removes_only_git() {
        let tmp = TempDir::new().unwrap();
        let git = tmp.path().join(".git");
        fs::create_dir(&git).unwrap();
        fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        let keep = tmp.path().join("README.md");
        fs::write(&keep, "hello").unwrap();

        remove_git_dir(tmp.path()).unwrap();

        assert!(!git.exists(), ".git should be removed");
        assert!(keep.exists(), "other files must be kept");
    }

    #[test]
    fn test_ensure_root_exists_is_file() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("file.txt");
        fs::write(&file, "x").unwrap();
        let result = ensure_root_exists(&file);
        assert!(result.is_err());
    }

    #[test]
    fn test_scan_requirements_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let result = scan_requirements(tmp.path());
        assert!(result.is_empty());
    }

    #[test]
    fn test_scan_requirements_nonexistent_dir() {
        let result = scan_requirements(Path::new("/nonexistent/path/abc123"));
        assert!(result.is_empty());
    }

    #[test]
    fn test_scan_requirements_finds_req_files() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("REQ-001.md"), "req content").unwrap();
        fs::write(tmp.path().join("REQ-002.md"), "req content 2").unwrap();
        fs::write(tmp.path().join("REQ-001.response.md"), "response").unwrap();
        fs::write(tmp.path().join("other.md"), "other").unwrap();

        let result = scan_requirements(tmp.path());
        assert_eq!(result.len(), 2);
        assert!(result.contains(&"REQ-001.md".to_string()));
        assert!(result.contains(&"REQ-002.md".to_string()));
    }

    #[test]
    fn test_scan_requirements_excludes_impl_files() {
        // A `.impl.md` close-signal must never be treated as a requirement —
        // otherwise a stray one (left behind if teardown failed) would be
        // re-copied to the recipient as a phantom pair.
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("REQ-001_x.md"), "req").unwrap();
        fs::write(tmp.path().join("REQ-001_x.impl.md"), "").unwrap();

        let result = scan_requirements(tmp.path());
        assert_eq!(result, vec!["REQ-001_x.md".to_string()]);
        assert!(!result.contains(&"REQ-001_x.impl.md".to_string()));
    }

    #[test]
    fn test_scan_responses() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("REQ-001.response.md"), "response").unwrap();
        fs::write(tmp.path().join("REQ-002.response.md"), "response 2").unwrap();
        fs::write(tmp.path().join("REQ-001.md"), "req").unwrap();

        let result = scan_responses(tmp.path());
        assert_eq!(result.len(), 2);
        assert!(result.contains(&"REQ-001.response.md".to_string()));
    }

    #[test]
    fn test_scan_responses_nonexistent_dir() {
        let result = scan_responses(Path::new("/nonexistent/path/abc123"));
        assert!(result.is_empty());
    }

    #[test]
    fn test_scan_impl_files() {
        // F-000039: only REQ-*.impl.md files are picked up.
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("REQ-001_x.impl.md"), "ack").unwrap();
        fs::write(tmp.path().join("REQ-002.md"), "req").unwrap();
        fs::write(tmp.path().join("foo.impl.md"), "not a REQ").unwrap();

        let result = scan_impl_files(tmp.path());
        assert_eq!(result, vec!["REQ-001_x.impl.md".to_string()]);
    }

    #[test]
    fn test_scan_impl_files_nonexistent_dir() {
        let result = scan_impl_files(Path::new("/nonexistent/path/abc123"));
        assert!(result.is_empty());
    }

    #[test]
    fn test_remove_file_if_exists() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("test.md");
        fs::write(&file, "content").unwrap();
        assert!(file.exists());

        remove_file_if_exists(&file).unwrap();
        assert!(!file.exists());
    }

    #[test]
    fn test_remove_file_if_exists_nonexistent() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("nonexistent.md");
        let result = remove_file_if_exists(&file);
        assert!(result.is_ok());
    }

    #[test]
    fn test_migrate_file_moves_content_and_removes_source() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("docs/api.md");
        let dst = tmp.path().join("docs/server-api/api.md");
        fs::create_dir_all(src.parent().unwrap()).unwrap();
        fs::write(&src, "api content").unwrap();

        migrate_file(&src, &dst).unwrap();

        assert!(!src.exists(), "source must be removed after migration");
        assert!(dst.exists(), "target must exist");
        assert_eq!(fs::read_to_string(&dst).unwrap(), "api content");
    }

    #[test]
    fn test_migrate_file_creates_target_parent() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("api.md");
        let dst = tmp.path().join("nested/deep/server-api/api.md");
        fs::write(&src, "x").unwrap();

        migrate_file(&src, &dst).unwrap();
        assert!(dst.exists());
    }

    #[test]
    fn test_migrate_file_source_missing_errors() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("missing.md");
        let dst = tmp.path().join("dst.md");
        let result = migrate_file(&src, &dst);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Source not found"));
    }

    #[test]
    fn test_auto_migration_flow_simulates_first_sync() {
        // Simulates 0.9.0 client-side flow:
        // - Old docs/api.md exists (content A).
        // - Server has docs/api.md (content B).
        // - After sync: docs/api.md gone, docs/server-api/api.md has content B.
        let tmp = TempDir::new().unwrap();
        let old_api = tmp.path().join("docs/api.md");
        let new_api = tmp.path().join("docs/server-api/api.md");
        fs::create_dir_all(old_api.parent().unwrap()).unwrap();
        fs::write(&old_api, "content A").unwrap();

        // Step 1: migrate old → new (happens when old exists and new doesn't)
        assert!(old_api.exists() && !new_api.exists());
        migrate_file(&old_api, &new_api).unwrap();

        // Step 2: copy server api.md on top
        let srv_api = tmp.path().join("server_api.md");
        fs::write(&srv_api, "content B").unwrap();
        let copied = copy_file_if_changed(&srv_api, &new_api).unwrap();
        assert!(copied, "content differs, must overwrite");

        assert!(!old_api.exists());
        assert_eq!(fs::read_to_string(&new_api).unwrap(), "content B");
    }

    #[test]
    fn test_copy_my_api_dir_copies_only_md_flat() {
        // T-000139: copies every *.md flat into the inbox, ignoring non-.md files.
        let tmp = TempDir::new().unwrap();
        let sender_docs = tmp.path().join("sender/docs");
        let my_api = sender_docs.join("my_api");
        fs::create_dir_all(&my_api).unwrap();
        fs::write(my_api.join("api.md"), "api content").unwrap();
        fs::write(my_api.join("admin-api.md"), "admin content").unwrap();
        fs::write(my_api.join("notes.txt"), "not a contract").unwrap();
        let inbox = tmp.path().join("recipient/docs/server-api");

        let copied = copy_my_api_dir(&sender_docs, &inbox).unwrap();

        assert_eq!(copied, 2, "only the two .md files should be copied");
        assert!(inbox.join("api.md").exists());
        assert!(inbox.join("admin-api.md").exists());
        assert!(
            !inbox.join("notes.txt").exists(),
            "non-.md files must be ignored"
        );
    }

    #[test]
    fn test_copy_my_api_dir_missing_folder_is_noop() {
        // T-000139: missing my_api folder → Ok(0), no error.
        let tmp = TempDir::new().unwrap();
        let sender_docs = tmp.path().join("sender/docs");
        fs::create_dir_all(&sender_docs).unwrap();
        let inbox = tmp.path().join("recipient/docs/server-api");

        let result = copy_my_api_dir(&sender_docs, &inbox);
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn test_copy_my_api_dir_idempotent() {
        // T-000139: second call with unchanged content copies nothing.
        let tmp = TempDir::new().unwrap();
        let sender_docs = tmp.path().join("sender/docs");
        let my_api = sender_docs.join("my_api");
        fs::create_dir_all(&my_api).unwrap();
        fs::write(my_api.join("api.md"), "api content").unwrap();
        let inbox = tmp.path().join("recipient/docs/server-api");

        assert_eq!(copy_my_api_dir(&sender_docs, &inbox).unwrap(), 1);
        assert_eq!(
            copy_my_api_dir(&sender_docs, &inbox).unwrap(),
            0,
            "unchanged content must be skipped on the second call"
        );
    }

    // --- T-000164: mirror_dir ---

    /// Build a small nested source tree: a.txt, b.txt, sub/c.txt, sub/deep/d.txt.
    fn build_src_tree(root: &Path) {
        fs::create_dir_all(root.join("sub/deep")).unwrap();
        fs::write(root.join("a.txt"), "a").unwrap();
        fs::write(root.join("b.txt"), "b").unwrap();
        fs::write(root.join("sub/c.txt"), "c").unwrap();
        fs::write(root.join("sub/deep/d.txt"), "d").unwrap();
    }

    #[test]
    fn test_mirror_dir_fresh_copies_all_nested() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst"); // does not exist yet
        build_src_tree(&src);

        let out = mirror_dir(&src, &dst).unwrap();

        assert_eq!(out.copied, 4, "all four files copied");
        assert_eq!(out.pruned, 0);
        assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "a");
        assert_eq!(fs::read_to_string(dst.join("sub/c.txt")).unwrap(), "c");
        assert_eq!(fs::read_to_string(dst.join("sub/deep/d.txt")).unwrap(), "d");
    }

    #[test]
    fn test_mirror_dir_idempotent_second_pass() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        build_src_tree(&src);

        mirror_dir(&src, &dst).unwrap();
        let out = mirror_dir(&src, &dst).unwrap();

        assert_eq!(
            out,
            MirrorOutcome::default(),
            "nothing copied, nothing pruned"
        );
    }

    #[test]
    fn test_mirror_dir_prunes_deleted_file() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        build_src_tree(&src);
        mirror_dir(&src, &dst).unwrap();

        fs::remove_file(src.join("b.txt")).unwrap();
        let out = mirror_dir(&src, &dst).unwrap();

        assert_eq!(out.copied, 0);
        assert_eq!(out.pruned, 1);
        assert!(!dst.join("b.txt").exists(), "removed file pruned");
        assert!(dst.join("a.txt").exists(), "other files remain");
        assert!(dst.join("sub/c.txt").exists());
    }

    #[test]
    fn test_mirror_dir_prunes_deleted_subdirectory() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        build_src_tree(&src);
        mirror_dir(&src, &dst).unwrap();

        // Remove the whole sub/ (holds c.txt and deep/d.txt) from src.
        fs::remove_dir_all(src.join("sub")).unwrap();
        let out = mirror_dir(&src, &dst).unwrap();

        assert_eq!(out.copied, 0);
        assert_eq!(
            out.pruned, 1,
            "the subdir is counted once, not per file inside it"
        );
        assert!(!dst.join("sub").exists(), "subtree pruned whole");
        assert!(dst.join("a.txt").exists());
    }

    #[test]
    fn test_mirror_dir_recopies_changed_content() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        build_src_tree(&src);
        mirror_dir(&src, &dst).unwrap();

        fs::write(src.join("a.txt"), "a-changed").unwrap();
        let out = mirror_dir(&src, &dst).unwrap();

        assert_eq!(out.copied, 1, "only the changed file is re-copied");
        assert_eq!(out.pruned, 0);
        assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "a-changed");
        assert_eq!(fs::read_to_string(dst.join("b.txt")).unwrap(), "b");
    }

    #[test]
    fn test_mirror_dir_missing_src_errors() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("nope");
        let dst = tmp.path().join("dst");
        let result = mirror_dir(&src, &dst);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_mirror_dir_prunes_extraneous_dst_only_file() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("keep.txt"), "keep").unwrap();
        // Pre-seed dst with a file that never existed in src.
        fs::create_dir_all(&dst).unwrap();
        fs::write(dst.join("stray.txt"), "stray").unwrap();

        let out = mirror_dir(&src, &dst).unwrap();

        assert_eq!(out.copied, 1);
        assert_eq!(out.pruned, 1, "the dst-only file is pruned on first mirror");
        assert!(dst.join("keep.txt").exists());
        assert!(!dst.join("stray.txt").exists());
    }

    // --- T-000173: count_files_recursive ---

    #[test]
    fn test_count_files_recursive_missing_or_empty_is_zero() {
        // Missing path → 0.
        assert_eq!(
            count_files_recursive(Path::new("/nonexistent/abc123/xyz")),
            0
        );
        // Empty dir → 0.
        let tmp = TempDir::new().unwrap();
        assert_eq!(count_files_recursive(tmp.path()), 0);
    }

    #[test]
    fn test_count_files_recursive_counts_nested_files_only() {
        let tmp = TempDir::new().unwrap();
        // a.txt, b.txt, sub/c.txt, sub/deep/d.txt = 4 files across 2 nested dirs.
        build_src_tree(tmp.path());
        assert_eq!(count_files_recursive(tmp.path()), 4);
    }

    #[test]
    fn test_dirs_in_sync_matches_only_on_identical_tree() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        // Missing dst → not in sync.
        build_src_tree(&src);
        assert!(!dirs_in_sync(&src, &dst));
        // Empty source (dir exists, no files) → not "synced".
        let empty = tmp.path().join("empty");
        fs::create_dir_all(&empty).unwrap();
        fs::create_dir_all(tmp.path().join("empty2")).unwrap();
        assert!(!dirs_in_sync(&empty, &tmp.path().join("empty2")));
        // Exact mirror → in sync.
        mirror_dir(&src, &dst).unwrap();
        assert!(dirs_in_sync(&src, &dst));
        // Edit one file → out of sync.
        fs::write(dst.join("a.txt"), "changed").unwrap();
        assert!(!dirs_in_sync(&src, &dst));
        // Restore, then add an extra file on dst → out of sync (prune pending).
        mirror_dir(&src, &dst).unwrap();
        assert!(dirs_in_sync(&src, &dst));
        fs::write(dst.join("extra.txt"), "x").unwrap();
        assert!(!dirs_in_sync(&src, &dst));
    }

    #[test]
    fn test_mirror_dir_empty_src_creates_dst() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        fs::create_dir_all(&src).unwrap(); // valid dir, no files

        let out = mirror_dir(&src, &dst).unwrap();

        assert_eq!(out, MirrorOutcome::default());
        assert!(dst.is_dir(), "dst is created even for an empty source");
    }
}
