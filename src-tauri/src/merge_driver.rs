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
use crate::sync::edit_clocks::ClockLedger;
use crate::sync::merge_by_id::{kind_for_path, merge_shared_bytes};
use crate::sync::shared::merge_shared_with_history;
use std::path::{Path, PathBuf};

const FLAG: &str = "--merge-driver";
/// T-000260: inner merges of the recursive strategy must stay flat — git
/// re-exports the same `GITHEAD_*` env for the virtual-base passes, where
/// the "remote" is a synthetic commit the provenance path cannot read.
/// `merge.sdh.recursive = sdhflat` names this driver for those passes.
const FLAG_FLAT: &str = "--merge-driver-flat";
/// T-000265: union merge driver for the edit-clock ledgers — CRDT per-key
/// max, commutative by construction, never positional.
const FLAG_CLOCKS: &str = "--merge-driver-clocks";

/// Handle a `--merge-driver[-flat|-clocks] O A B P` invocation. `None` when
/// `args` is not a driver call (the app then starts normally); otherwise the
/// process exit code. A malformed call (flag without four paths) is a
/// driver call that failed: 1.
pub fn run_from_args(args: &[String]) -> Option<i32> {
    if let Some(pos) = args.iter().position(|a| a == FLAG_CLOCKS) {
        let rest = &args[pos + 1..];
        if rest.len() < 4 {
            return Some(1);
        }
        return Some(match run_clocks(&rest[1], &rest[2]) {
            Ok(()) => 0,
            Err(_) => 1,
        });
    }
    let (pos, flat) = match args.iter().position(|a| a == FLAG_FLAT) {
        Some(p) => (p, true),
        None => (args.iter().position(|a| a == FLAG)?, false),
    };
    let rest = &args[pos + 1..];
    if rest.len() < 4 {
        return Some(1);
    }
    let (o, a, b, p) = (&rest[0], &rest[1], &rest[2], &rest[3]);
    Some(match run(o, a, b, p, flat) {
        Ok(()) => 0,
        Err(_) => 1,
    })
}

/// Union-merge an edit-clock ledger: the CRDT makes the base irrelevant —
/// ours ∪ theirs by per-key maximum is the unique correct result whatever
/// the ancestor held. Both sides must parse; a corrupt ledger fails the
/// merge into a manual conflict rather than silently dropping entries.
fn run_clocks(a: &str, b: &str) -> Result<(), String> {
    let ours = std::fs::read(a).map_err(|e| format!("read {}: {}", a, e))?;
    let theirs = std::fs::read(b).map_err(|e| format!("read {}: {}", b, e))?;
    let lo = ClockLedger::parse(&ours, a)?;
    let lt = ClockLedger::parse(&theirs, b)?;
    let merged = lo.merge(&lt).serialize()?;
    write_over(a, merged.as_bytes())
}

/// Write next to `target` and rename into place, so a failure never leaves
/// the merge result half-written.
fn write_over(target: &str, bytes: &[u8]) -> Result<(), String> {
    let tmp = format!("{}.sdh-merge.tmp", target);
    std::fs::write(&tmp, bytes).map_err(|e| format!("write {}: {}", tmp, e))?;
    if let Err(e) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("rename onto {}: {}", target, e));
    }
    Ok(())
}

/// The single remote commit git is merging into HEAD, or `None` — git
/// exposes it as `GITHEAD_<oid>=<name>` environment variables while the
/// merge strategy runs (`MERGE_HEAD` is not written yet at driver time).
/// Exactly one valid oid that differs from HEAD counts; zero (rebase,
/// cherry-pick, a manual invocation) or several (octopus) mean "no
/// provenance" and the caller falls back to the flat merge.
fn remote_merge_oid(git: &GitBinary, root: &Path) -> Option<String> {
    let head = crate::git_sync::rev_parse(git, root, "HEAD")
        .ok()
        .flatten()?;
    pick_remote_oid(&head, std::env::vars())
}

/// The env-scan half of `remote_merge_oid`, pure for tests: among
/// `GITHEAD_<oid>` keys exactly one valid, non-HEAD oid may stand.
fn pick_remote_oid(head: &str, vars: impl IntoIterator<Item = (String, String)>) -> Option<String> {
    let mut oids: Vec<String> = vars
        .into_iter()
        .filter_map(|(k, _)| k.strip_prefix("GITHEAD_").map(str::to_owned))
        .filter(|s| s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit()))
        .filter(|s| !s.eq_ignore_ascii_case(head))
        .collect();
    oids.dedup();
    (oids.len() == 1).then(|| oids.remove(0))
}

fn run(o: &str, a: &str, b: &str, p: &str, flat: bool) -> Result<(), String> {
    let kind = kind_for_path(p).ok_or_else(|| format!("not a shared file: {}", p))?;
    let base = std::fs::read(o).map_err(|e| format!("read {}: {}", o, e))?;
    let ours = std::fs::read(a).map_err(|e| format!("read {}: {}", a, e))?;
    let theirs = std::fs::read(b).map_err(|e| format!("read {}: {}", b, e))?;
    // T-000260: with exactly one real remote commit visible, merge with the
    // file histories and the ancestor timeline behind the merge-base —
    // versioned headers and stale-remote bytes then resolve by provenance,
    // not by the flat last-writer rule. Every gap in that chain (no
    // GITHEAD_*, no merge-base, missing history) degrades to the flat
    // merge, never to a refusal.
    let merged = if flat {
        None
    } else {
        let git = GitBinary(PathBuf::from("git"));
        std::env::current_dir()
            .ok()
            .and_then(|root| {
                remote_merge_oid(&git, &root).map(|oid| {
                    merge_shared_with_history(&git, &root, p, &base, &ours, &theirs, &oid)
                })
            })
            .and_then(|r| r.ok())
    };
    let merged = match merged {
        Some(m) => m,
        None => merge_shared_bytes(kind, &base, &ours, &theirs).map_err(|e| e.to_string())?,
    };
    write_over(a, merged.as_bytes())
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

/// The `merge.sdhflat.driver` value — the same executable with the flat
/// flag (T-000260).
fn flat_driver_command(exe: &str) -> String {
    format!("\"{}\" {} %O %A %B %P", exe, FLAG_FLAT)
}

/// The `merge.sdhclocks.driver` value — the same executable with the clocks
/// flag (T-000265).
fn clocks_driver_command(exe: &str) -> String {
    format!("\"{}\" {} %O %A %B %P", exe, FLAG_CLOCKS)
}

const DRIVER_NAME: &str = "Solo Dev Hub merge by id";
const FLAT_DRIVER_NAME: &str = "Solo Dev Hub merge by id (flat)";
const CLOCKS_DRIVER_NAME: &str = "Solo Dev Hub edit-clock union merge";

/// Register the `sdh` merge driver in the repo's local git config — and the
/// `sdhflat` variant `merge.sdh.recursive` names for the internal merges
/// of the recursive strategy (T-000260: they inherit the top-level
/// `GITHEAD_*` env, whose entries do not describe the virtual bases they
/// merge). Writes only what differs from the current values; returns
/// whether anything was written.
pub fn register_merge_driver(git: &GitBinary, repo: &Path) -> Result<bool, String> {
    let exe_path = std::env::current_exe().map_err(|e| format!("current_exe: {}", e))?;
    let appimage = std::env::var("APPIMAGE").ok();
    let exe = resolve_exe(appimage.as_deref(), &exe_path);
    let command = driver_command(&exe);
    let flat = flat_driver_command(&exe);
    let clocks = clocks_driver_command(&exe);
    register_with(git, repo, &command, &flat, &clocks)
}

fn register_with(
    git: &GitBinary,
    repo: &Path,
    command: &str,
    flat_command: &str,
    clocks_command: &str,
) -> Result<bool, String> {
    let mut wrote = false;
    for (key, value) in [
        ("merge.sdh.name", DRIVER_NAME),
        ("merge.sdh.driver", command),
        ("merge.sdh.recursive", "sdhflat"),
        ("merge.sdhflat.name", FLAT_DRIVER_NAME),
        ("merge.sdhflat.driver", flat_command),
        ("merge.sdhclocks.name", CLOCKS_DRIVER_NAME),
        ("merge.sdhclocks.driver", clocks_command),
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
        let exe = "C:/Program Files/App/app.exe";
        let cmd = driver_command(exe);
        let flat = flat_driver_command(exe);
        let clocks = clocks_driver_command(exe);
        assert_eq!(
            register_with(&gb, dir.path(), &cmd, &flat, &clocks),
            Ok(true)
        );
        assert_eq!(
            register_with(&gb, dir.path(), &cmd, &flat, &clocks),
            Ok(false)
        );
        for (key, want) in [
            ("merge.sdh.driver", cmd.as_str()),
            ("merge.sdh.name", DRIVER_NAME),
            ("merge.sdh.recursive", "sdhflat"),
            ("merge.sdhflat.driver", flat.as_str()),
            ("merge.sdhflat.name", FLAT_DRIVER_NAME),
            ("merge.sdhclocks.driver", clocks.as_str()),
            ("merge.sdhclocks.name", CLOCKS_DRIVER_NAME),
        ] {
            assert_eq!(
                config_get(&gb, dir.path(), key).unwrap().as_deref(),
                Some(want),
                "{}",
                key
            );
        }
        // a changed exe path rewrites only the three driver commands
        let cmd2 = driver_command("D:/new/app.exe");
        let flat2 = flat_driver_command("D:/new/app.exe");
        let clocks2 = clocks_driver_command("D:/new/app.exe");
        assert_eq!(
            register_with(&gb, dir.path(), &cmd2, &flat2, &clocks2),
            Ok(true)
        );
        assert_eq!(
            register_with(&gb, dir.path(), &cmd2, &flat2, &clocks2),
            Ok(false)
        );
    }

    #[test]
    fn flat_flag_merges_without_history_or_env() {
        let dir = TempDir::new().unwrap();
        let ours = BASE.replace("one", "one ours");
        let theirs = BASE.replace("two", "два theirs");
        let (o, a, b) = setup(&dir, BASE.as_bytes(), ours.as_bytes(), theirs.as_bytes());
        let code = run_from_args(&args(&[
            "app.exe",
            "--merge-driver-flat",
            &o,
            &a,
            &b,
            "docs/todo.md",
        ]));
        assert_eq!(code, Some(0));
        let res = fs::read_to_string(&a).unwrap();
        assert!(res.contains("one ours") && res.contains("два theirs"));
    }

    #[test]
    fn remote_oid_detection_accepts_exactly_one_valid_non_head_oid() {
        let head = "a".repeat(40);
        let remote = "b".repeat(40);
        let other = "c".repeat(40);
        let env = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<Vec<_>>()
        };
        // One valid remote oid → provenance.
        assert_eq!(
            pick_remote_oid(&head, env(&[(&format!("GITHEAD_{}", remote), "dev")])),
            Some(remote.clone())
        );
        // Absent (rebase, cherry-pick, a manual call) → flat.
        assert_eq!(pick_remote_oid(&head, env(&[])), None);
        assert_eq!(pick_remote_oid(&head, env(&[("GITHEAD_", "x")])), None);
        // Several (octopus) → flat.
        assert_eq!(
            pick_remote_oid(
                &head,
                env(&[
                    (&format!("GITHEAD_{}", remote), "dev"),
                    (&format!("GITHEAD_{}", other), "next"),
                ])
            ),
            None
        );
        // HEAD's own oid does not count; and a non-hex suffix is not an oid.
        assert_eq!(
            pick_remote_oid(
                &head,
                env(&[
                    (&format!("GITHEAD_{}", head), "ours"),
                    ("GITHEAD_short", "s"),
                    ("GITHEAD_zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz", "x"),
                ])
            ),
            None
        );
    }

    #[test]
    fn a_driver_call_without_githead_env_falls_back_to_flat() {
        // No GITHEAD_* in this process's env (the test harness exports
        // none): the ordinary flag must still merge, just without
        // provenance.
        let dir = TempDir::new().unwrap();
        let ours = BASE.replace("one", "one ours");
        let (o, a, b) = setup(&dir, BASE.as_bytes(), ours.as_bytes(), BASE.as_bytes());
        let code = run_from_args(&args(&[
            "app.exe",
            "--merge-driver",
            &o,
            &a,
            &b,
            "docs/todo.md",
        ]));
        assert_eq!(code, Some(0));
        assert!(fs::read_to_string(&a).unwrap().contains("one ours"));
    }
}
