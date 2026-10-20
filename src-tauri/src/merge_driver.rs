// F-000044 / T-000218: git merge driver for the shared MD files.
//
// Each shared clone registers `merge.sdh.driver` in its `.git/config` pointing
// at this very executable with `--merge-driver %O %A %B %P`. Git then calls us
// for `docs/todo.md`, `docs/done.md`, `docs/bug-reports.md` (marked
// `merge=sdh` in `.gitattributes`); we merge the three versions by entity id
// (`sync::merge_by_id`) and write the result over `%A`.
//
// The entry point (`run_from_args`) runs FIRST in `main`, before any database,
// keyring, window or Tauri runtime is touched.

#![allow(dead_code)]

use crate::git_ops::GitBinary;
use crate::git_sync::{config_get, config_set};
use crate::sync::merge_by_id::{kind_for_path, merge_shared_bytes};
use std::path::Path;

const FLAG: &str = "--merge-driver";

/// Handle a `--merge-driver O A B P` invocation. `None` when `args` is not a
/// driver call (the app then starts normally); otherwise the process exit code.
/// A malformed call (flag without four paths) is a driver call that failed: 1.
pub fn run_from_args(args: &[String]) -> Option<i32> {
    let pos = args.iter().position(|a| a == FLAG)?;
    let rest = &args[pos + 1..];
    if rest.len() < 4 {
        return Some(1);
    }
    let (o, a, b, p) = (&rest[0], &rest[1], &rest[2], &rest[3]);
    Some(match run(o, a, b, p) {
        Ok(()) => 0,
        Err(_) => 1,
    })
}

fn run(o: &str, a: &str, b: &str, p: &str) -> Result<(), String> {
    let kind = kind_for_path(p).ok_or_else(|| format!("not a shared file: {}", p))?;
    let base = std::fs::read(o).map_err(|e| format!("read {}: {}", o, e))?;
    let ours = std::fs::read(a).map_err(|e| format!("read {}: {}", a, e))?;
    let theirs = std::fs::read(b).map_err(|e| format!("read {}: {}", b, e))?;
    let merged = merge_shared_bytes(kind, &base, &ours, &theirs).map_err(|e| e.to_string())?;
    // Write next to A and rename, so a failure never leaves A half-written.
    let tmp = format!("{}.sdh-merge.tmp", a);
    std::fs::write(&tmp, merged.as_bytes()).map_err(|e| format!("write {}: {}", tmp, e))?;
    if let Err(e) = std::fs::rename(&tmp, a) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("rename onto {}: {}", a, e));
    }
    Ok(())
}

/// The executable the driver command must point at. On a Linux AppImage
/// `current_exe` lives in a temporary mount that disappears on exit, so the
/// `APPIMAGE` path wins when set and non-empty. Backslashes become slashes (git
/// runs the command through its own sh).
pub(crate) fn resolve_exe(appimage: Option<&str>, current_exe: &Path) -> String {
    let chosen = match appimage {
        Some(a) if !a.trim().is_empty() => a.to_string(),
        _ => current_exe.to_string_lossy().into_owned(),
    };
    chosen.replace('\\', "/")
}

/// The `merge.sdh.driver` value for an executable path.
pub(crate) fn driver_command(exe: &str) -> String {
    format!("\"{}\" {} %O %A %B %P", exe, FLAG)
}

const DRIVER_NAME: &str = "Solo Dev Hub merge by id";

/// Register the `sdh` merge driver in the repo's local git config. Writes only
/// what differs from the current values; returns whether anything was written.
pub fn register_merge_driver(git: &GitBinary, repo: &Path) -> Result<bool, String> {
    let exe_path = std::env::current_exe().map_err(|e| format!("current_exe: {}", e))?;
    let appimage = std::env::var("APPIMAGE").ok();
    let command = driver_command(&resolve_exe(appimage.as_deref(), &exe_path));
    register_with(git, repo, &command)
}

fn register_with(git: &GitBinary, repo: &Path, command: &str) -> Result<bool, String> {
    let mut wrote = false;
    for (key, value) in [
        ("merge.sdh.name", DRIVER_NAME),
        ("merge.sdh.driver", command),
    ] {
        if config_get(git, repo, key)?.as_deref() != Some(value) {
            config_set(git, repo, key, value)?;
            wrote = true;
        }
    }
    Ok(wrote)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use tempfile::TempDir;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    const BASE: &str = "## v1\n\n- [ ] T-000001 | one | 1 | low | open | 2026-10-01\n- [ ] T-000002 | two | 1 | low | open | 2026-10-02\n";

    fn setup(dir: &TempDir, base: &[u8], ours: &[u8], theirs: &[u8]) -> (String, String, String) {
        let p = |n: &str| dir.path().join(n).to_string_lossy().into_owned();
        fs::write(p("O"), base).unwrap();
        fs::write(p("A"), ours).unwrap();
        fs::write(p("B"), theirs).unwrap();
        (p("O"), p("A"), p("B"))
    }

    #[test]
    fn not_a_driver_call_is_none() {
        assert_eq!(run_from_args(&args(&["app.exe"])), None);
        assert_eq!(run_from_args(&args(&["app.exe", "--other", "x"])), None);
        assert_eq!(run_from_args(&[]), None);
    }

    #[test]
    fn flag_without_paths_is_failure() {
        assert_eq!(
            run_from_args(&args(&["app.exe", "--merge-driver", "a", "b"])),
            Some(1)
        );
    }

    #[test]
    fn success_writes_result_over_a() {
        let dir = TempDir::new().unwrap();
        let ours = BASE.replace("one", "one ours");
        let theirs = BASE.replace("two", "два theirs");
        let (o, a, b) = setup(&dir, BASE.as_bytes(), ours.as_bytes(), theirs.as_bytes());
        let code = run_from_args(&args(&[
            "app.exe",
            "--merge-driver",
            &o,
            &a,
            &b,
            "docs/todo.md",
        ]));
        assert_eq!(code, Some(0));
        let res = fs::read_to_string(&a).unwrap();
        assert!(res.contains("one ours") && res.contains("два theirs"));
        // no temp file left behind
        assert!(!PathBuf::from(format!("{}.sdh-merge.tmp", a)).exists());
    }

    #[test]
    fn unknown_kind_fails_and_leaves_a_untouched() {
        let dir = TempDir::new().unwrap();
        let ours = BASE.replace("one", "one ours");
        let (o, a, b) = setup(&dir, BASE.as_bytes(), ours.as_bytes(), BASE.as_bytes());
        let code = run_from_args(&args(&[
            "app.exe",
            "--merge-driver",
            &o,
            &a,
            &b,
            "README.md",
        ]));
        assert_eq!(code, Some(1));
        assert_eq!(fs::read_to_string(&a).unwrap(), ours);
    }

    #[test]
    fn non_utf8_fails_and_leaves_a_untouched() {
        let dir = TempDir::new().unwrap();
        let bad: &[u8] = &[b'-', b' ', 0xff, 0xfe, b'\n'];
        let (o, a, b) = setup(
            &dir,
            BASE.as_bytes(),
            bad,
            BASE.replace("two", "2").as_bytes(),
        );
        let code = run_from_args(&args(&[
            "app.exe",
            "--merge-driver",
            &o,
            &a,
            &b,
            "docs/todo.md",
        ]));
        assert_eq!(code, Some(1));
        assert_eq!(fs::read(&a).unwrap(), bad);
    }

    #[test]
    fn missing_input_file_fails() {
        let dir = TempDir::new().unwrap();
        let (o, a, b) = setup(&dir, b"", b"x", b"y");
        fs::remove_file(&o).unwrap();
        let code = run_from_args(&args(&[
            "app.exe",
            "--merge-driver",
            &o,
            &a,
            &b,
            "docs/todo.md",
        ]));
        assert_eq!(code, Some(1));
        assert_eq!(fs::read_to_string(&a).unwrap(), "x");
    }

    #[test]
    fn exe_resolution() {
        let cur = Path::new("C:\\Program Files\\Solo Dev Hub\\app.exe");
        assert_eq!(
            resolve_exe(None, cur),
            "C:/Program Files/Solo Dev Hub/app.exe"
        );
        assert_eq!(
            resolve_exe(Some("/home/u/SoloDevHub.AppImage"), cur),
            "/home/u/SoloDevHub.AppImage"
        );
        // empty / blank APPIMAGE is ignored
        assert_eq!(
            resolve_exe(Some(""), cur),
            "C:/Program Files/Solo Dev Hub/app.exe"
        );
        assert_eq!(
            resolve_exe(Some("  "), cur),
            "C:/Program Files/Solo Dev Hub/app.exe"
        );
        assert_eq!(
            resolve_exe(Some("C:\\x\\y.AppImage"), cur),
            "C:/x/y.AppImage"
        );
    }

    #[test]
    fn driver_command_is_quoted() {
        assert_eq!(
            driver_command("C:/Program Files/App/app.exe"),
            "\"C:/Program Files/App/app.exe\" --merge-driver %O %A %B %P"
        );
    }

    fn git(dir: &Path, a: &[&str]) {
        let out = Command::new("git")
            .args(a)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {:?}: {}",
            a,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    fn registration_writes_once_then_is_idempotent() {
        let dir = TempDir::new().unwrap();
        git(dir.path(), &["init", "-q"]);
        let gb = GitBinary(PathBuf::from("git"));
        let cmd = driver_command("C:/Program Files/App/app.exe");
        assert_eq!(register_with(&gb, dir.path(), &cmd), Ok(true));
        assert_eq!(register_with(&gb, dir.path(), &cmd), Ok(false));
        assert_eq!(
            config_get(&gb, dir.path(), "merge.sdh.driver")
                .unwrap()
                .as_deref(),
            Some(cmd.as_str())
        );
        assert_eq!(
            config_get(&gb, dir.path(), "merge.sdh.name")
                .unwrap()
                .as_deref(),
            Some(DRIVER_NAME)
        );
        // a changed exe path rewrites only the driver
        let cmd2 = driver_command("D:/new/app.exe");
        assert_eq!(register_with(&gb, dir.path(), &cmd2), Ok(true));
        assert_eq!(register_with(&gb, dir.path(), &cmd2), Ok(false));
    }
}
