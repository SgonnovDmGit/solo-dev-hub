// Filesystem primitives: path safety, file copy/move, REQ scan helpers.

use std::fs;
use std::path::Path;

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

/// Scan directory for REQ-*.md files (not .response.md)
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
}
