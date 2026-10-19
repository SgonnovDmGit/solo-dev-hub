// F-000044 / T-000218: the real app executable, registered as a git merge
// driver, merges docs/todo.md by entity id in a real `git merge`.
// Run with `cargo test --test merge_driver_git` (builds the bin).

use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {:?} failed: {}{}",
        args,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

const BASE: &str = "# Todo\n\n## v1.20.0 — режим помощника\n\n- [ ] T-000001 | первая задача | 2 | high | open | 2026-10-01\n- [ ] T-000002 | вторая задача | 1 | low | open | 2026-10-02\n- [ ] T-000003 | третья задача | 3 | medium | open | 2026-10-03\n";

#[test]
fn git_merge_uses_the_registered_driver() {
    let exe = env!("CARGO_BIN_EXE_solo-dev-hub").replace(std::path::MAIN_SEPARATOR, "/");
    let tmp = TempDir::new().unwrap();
    // a directory with a space exercises the quoted command
    let repo = tmp.path().join("my repo");
    fs::create_dir_all(repo.join("docs")).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "t@example.com"]);
    git(&repo, &["config", "user.name", "T"]);
    git(&repo, &["config", "core.autocrlf", "false"]);
    git(&repo, &["config", "merge.sdh.name", "Solo Dev Hub merge by id"]);
    git(
        &repo,
        &[
            "config",
            "merge.sdh.driver",
            &format!("\"{}\" --merge-driver %O %A %B %P", exe),
        ],
    );
    fs::write(repo.join(".gitattributes"), "docs/todo.md merge=sdh\n").unwrap();
    fs::write(repo.join("docs/todo.md"), BASE).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "base"]);

    git(&repo, &["checkout", "-q", "-b", "other"]);
    fs::write(
        repo.join("docs/todo.md"),
        BASE.replace("третья задача | 3 | medium | open", "третья задача | 3 | medium | review"),
    )
    .unwrap();
    git(&repo, &["commit", "-q", "-am", "theirs"]);

    git(&repo, &["checkout", "-q", "main"]);
    // adjacent line edit: a textual merge would conflict only if hunks touch
    fs::write(
        repo.join("docs/todo.md"),
        BASE.replace("вторая задача | 1", "вторая задача, правка | 1"),
    )
    .unwrap();
    git(&repo, &["commit", "-q", "-am", "ours"]);

    git(&repo, &["merge", "-q", "--no-edit", "other"]);
    let merged = fs::read_to_string(repo.join("docs/todo.md")).unwrap();
    assert!(merged.contains("вторая задача, правка | 1"), "{}", merged);
    assert!(merged.contains("третья задача | 3 | medium | review"), "{}", merged);
    assert!(!merged.contains("<<<<<<<"));
}
