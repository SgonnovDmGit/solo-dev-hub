// F-000044 / T-000225: end-to-end scenarios of contributor mode.
//
// Two installs (an owner and a helper) on two clones of one bare remote, two
// in-memory databases, driven in one process through the entry points a user's
// actions reach and nothing else:
//
// * "sync"            = `run_project_sync_in` (phase A, skeleton writes,
//                       phase B), exactly what the sync button / timer runs;
// * "open the repo"   = `migrate_bugs_for_repo` + `sync_tasks_for_repo`, what
//                       opening the repo in the UI does (a repo the user has
//                       never opened has no database mirror to import into);
// * "+ Add bug"       = the body of `create_bug` (reconcile, band allocator,
//                       insert, event, regenerate);
// * "the local AI edits a file" / "the user edits todo.md" = a write in the
//                       working tree;
// * the buttons       = `enable_shared_mode`, `approve_join_request`,
//                       `merge_shared_upstream`, `disable_shared_mode`.
//
// Each test is one item of the Verification section of the spec
// (`docs/superpowers/specs/2026-10-19-v1.20.0-contributor-mode.md`).
//
// The remote carries an `update` hook that records (and refuses) any update of
// `master` / `main`, any ref deletion and any non-fast-forward update, so
// "nothing ever writes master / forces" is checked by the remote itself, and
// again by comparing ref snapshots.
//
// The merge driver is the app executable in production; here the executable is
// the test harness. `arm_driver` points the driver of a clone at the harness
// running `merge_driver_shim` (a test that, when called by git with the
// `sdh-shim` marker, performs `merge_driver::run_from_args` and exits with its
// code), through an `include` at the end of `.git/config`: the real `git merge`
// of the real driver code runs, only the executable differs. The product's
// registration of the driver is left untouched (it still writes its own value).

use super::project_sync::run_project_sync_in;
use super::shared::test_support::*;
use super::shared::*;
use super::shared_structure as ss;
use crate::db::AppDb;
use crate::models::{ActivityEvent, SyncResult, TimelineFilter};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

const ZERO: &str = "0000000000000000000000000000000000000000";

// ---------------------------------------------------------------------------
// The merge-driver shim
// ---------------------------------------------------------------------------

/// Not a test of its own: git runs the harness with this test's name and the
/// `sdh-shim` marker as the merge driver of the e2e clones. Run normally (no
/// marker among the arguments) it does nothing.
#[test]
fn merge_driver_shim() {
    let args: Vec<String> = std::env::args().collect();
    let Some(pos) = args.iter().position(|a| a == "sdh-shim") else {
        return;
    };
    let mut call = vec!["app".to_string(), "--merge-driver".to_string()];
    call.extend(args[pos + 1..].iter().cloned());
    let code = crate::merge_driver::run_from_args(&call).unwrap_or(1);
    std::process::exit(code);
}

/// Make the clone merge with the shim. Idempotent. Needs the `[merge "sdh"]`
/// section the product registered to exist already (the include comes after it
/// and so wins); does nothing before that.
fn arm_driver(dir: &Path) {
    let cfg = dir.join(".git").join("config");
    let text = fs::read_to_string(&cfg).unwrap();
    if text.contains("sdh-test-driver.config") || !text.contains("[merge \"sdh\"]") {
        return;
    }
    let exe = std::env::current_exe()
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/");
    let driver = format!(
        "[merge \"sdh\"]\n\tdriver = \"{}\" --exact --nocapture sync::shared_e2e_tests::merge_driver_shim -- sdh-shim %O %A %B %P\n",
        exe
    );
    fs::write(dir.join(".git").join("sdh-test-driver.config"), driver).unwrap();
    let mut new = text;
    if !new.ends_with('\n') {
        new.push('\n');
    }
    new.push_str("[include]\n\tpath = sdh-test-driver.config\n");
    fs::write(cfg, new).unwrap();
}

// ---------------------------------------------------------------------------
// The world
// ---------------------------------------------------------------------------

struct World {
    fx: TwoInstalls,
    owner_home: TempDir,
    helper_home: TempDir,
    /// A project of the helper that has nothing to do with the shared repo:
    /// the helper's first sync (before the join) runs on it.
    scratch: i64,
    master0: String,
    hook_log: PathBuf,
    /// Refs of the remote at the last check.
    refs: RefCell<BTreeMap<String, String>>,
}

impl World {
    fn new() -> World {
        let fx = two_installs();
        crate::template_seeder::seed_bundled_templates(&fx.main_db).unwrap();
        crate::template_seeder::seed_bundled_templates(&fx.helper_db).unwrap();
        let scratch = fx
            .helper_db
            .create_project("Scratch", None, "standard")
            .unwrap()
            .id;
        // The owner's repo carries a user-section `.gitignore` that hides the
        // task files (the global rules say it should).
        write(&fx.main_dir, ".gitignore", "docs/todo.md\ndocs/done.md\n");
        g(&fx.main_dir, &["add", ".gitignore"]);
        g(&fx.main_dir, &["commit", "-q", "-m", "gitignore"]);
        g(&fx.main_dir, &["push", "-q", "origin", "dev"]);
        g(&fx.helper_dir, &["fetch", "-q", "origin"]);

        let hook_log = fx._tmp.path().join("violations.log");
        let hook = fx.origin.join("hooks").join("update");
        let script = format!(
            "#!/bin/sh\n\
             LOG=\"{log}\"\n\
             ref=\"$1\"; old=\"$2\"; new=\"$3\"\n\
             case \"$ref\" in\n\
               refs/heads/master|refs/heads/main) echo \"$ref $old $new: master/main written\" >> \"$LOG\"; exit 1;;\n\
             esac\n\
             if [ \"$new\" = \"{zero}\" ]; then echo \"$ref deleted\" >> \"$LOG\"; exit 1; fi\n\
             if [ \"$old\" != \"{zero}\" ] && ! git merge-base --is-ancestor \"$old\" \"$new\"; then\n\
               echo \"$ref $old $new: not a fast-forward\" >> \"$LOG\"; exit 1\n\
             fi\n\
             exit 0\n",
            log = hook_log.to_string_lossy().replace('\\', "/"),
            zero = ZERO
        );
        fs::write(&hook, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let master0 = fx.remote_rev("master").unwrap();
        let w = World {
            fx,
            owner_home: TempDir::new().unwrap(),
            helper_home: TempDir::new().unwrap(),
            scratch,
            master0,
            hook_log,
            refs: RefCell::new(BTreeMap::new()),
        };
        *w.refs.borrow_mut() = w.origin_refs();
        w
    }

    // ----- the user's actions ------------------------------------------------------

    /// The owner presses sync on the project.
    fn sync_owner(&self) -> SyncResult {
        let r = run_project_sync_in(
            &self.fx.main_db,
            self.fx.project_id,
            Some(self.owner_home.path().to_path_buf()),
        )
        .unwrap();
        arm_driver(&self.fx.main_dir);
        self.log("owner", &r);
        self.check_origin();
        r
    }

    /// The helper presses sync on the project the repo is in (before the join
    /// has put the repo in one: on an unrelated project of theirs).
    fn sync_helper(&self) -> SyncResult {
        let project = self
            .fx
            .helper_db
            .get_repository(self.fx.helper_repo)
            .unwrap()
            .project_id
            .unwrap_or(self.scratch);
        let r = run_project_sync_in(
            &self.fx.helper_db,
            project,
            Some(self.helper_home.path().to_path_buf()),
        )
        .unwrap();
        arm_driver(&self.fx.helper_dir);
        self.log("helper", &r);
        self.check_origin();
        r
    }

    /// The user opens the repo in the UI: the bug mirror is migrated and the
    /// task mirror is built from the files.
    fn open(db: &AppDb, repo: i64) {
        crate::sync::bugs::migrate_bugs_for_repo(db, repo).unwrap();
        crate::sync::tasks::sync_tasks_for_repo(db, repo).unwrap();
    }

    fn open_owner(&self) {
        Self::open(&self.fx.main_db, self.fx.main_repo);
    }

    fn open_helper(&self) {
        Self::open(&self.fx.helper_db, self.fx.helper_repo);
    }

    /// The "+ Add bug" button: the body of `create_bug`.
    fn add_bug(db: &AppDb, repo: i64, description: &str) -> String {
        let _ = crate::sync::reconcile_bugs_for_repo(db, repo);
        let nid = ss::allocate_bug_numeric_id(db, repo).unwrap();
        let now = crate::db::utc_now_rfc3339();
        let bug = db
            .insert_bug(
                repo,
                nid,
                &now,
                description,
                "major",
                "logic",
                "created",
                0,
                None,
                None,
            )
            .unwrap();
        db.insert_bug_event(bug.id, "created", None, Some("created"), &bug.created_at)
            .unwrap();
        crate::sync::regenerate_bugs_md(db, repo).unwrap();
        bug.display_id
    }

    /// The owner switches shared mode on with the file tracking the dialog
    /// asks for: enable (removing the blocking lines), a done.md that exists,
    /// one sync. Leaves the owner on `dev` with everything pushed.
    fn owner_shares(&self) {
        enable_shared_mode(&self.fx.main_db, &gb(), self.fx.main_repo, true).unwrap();
        write(&self.fx.main_dir, "docs/done.md", &done_md(&["T-000900"]));
        self.sync_owner();
    }

    /// The helper joined and the owner approved, through real syncs of both.
    fn join(&self) {
        self.owner_shares();
        self.sync_helper(); // probes the remote: pending, the request goes to dev
        self.sync_owner(); // the request arrives
        approve_join_request(&self.fx.main_db, Some(&gb()), self.fx.main_repo, "helper").unwrap();
        self.sync_owner(); // approval committed and pushed, helper-dev plumbed
        self.sync_helper(); // contributor now, publishes its own branch
        self.sync_owner(); // the structure is plumbed into the helper's branch
        self.sync_helper(); // and fast-forwarded onto
    }

    /// Joined, both installs have the repo open and have seen each other's
    /// files (so every later change is an import, not a baseline).
    fn participants(&self) {
        self.join();
        self.open_owner();
        self.open_helper();
        // Opening the repo regenerated the bug file in both work trees while the
        // other side's commits were on their way: the repo may ask for the Merge
        // button, as a user would press it.
        self.settle();
    }

    // ----- observation -----------------------------------------------------------------

    fn origin_refs(&self) -> BTreeMap<String, String> {
        g(
            &self.fx.origin,
            &["for-each-ref", "--format=%(refname) %(objectname)"],
        )
        .lines()
        .filter_map(|l| l.split_once(' '))
        .map(|(r, o)| (r.to_string(), o.to_string()))
        .collect()
    }

    /// The remote's own record is empty, master is where it was, no `main`
    /// branch appeared and every ref moved forward only.
    fn check_origin(&self) {
        let log = fs::read_to_string(&self.hook_log).unwrap_or_default();
        assert!(
            log.trim().is_empty(),
            "the remote refused updates:\n{}",
            log
        );
        let now = self.origin_refs();
        assert_eq!(
            now.get("refs/heads/master").map(String::as_str),
            Some(self.master0.as_str()),
            "master moved"
        );
        assert!(
            !now.contains_key("refs/heads/main"),
            "a main branch appeared"
        );
        for (r, old) in self.refs.borrow().iter() {
            let new = now.get(r).unwrap_or_else(|| panic!("{} was deleted", r));
            let ok = Command::new("git")
                .args(["merge-base", "--is-ancestor", old, new])
                .current_dir(&self.fx.origin)
                .status()
                .unwrap()
                .success();
            assert!(ok, "{} did not fast-forward: {} -> {}", r, old, new);
        }
        *self.refs.borrow_mut() = now;
    }

    /// Shown only when a test fails (captured output): what each sync said.
    fn log(&self, who: &str, r: &SyncResult) {
        let issues: Vec<String> = r
            .errors
            .iter()
            .map(|i| {
                format!(
                    "{:?}[{}]{}",
                    i.code,
                    i.target.as_deref().unwrap_or(""),
                    i.detail
                        .as_deref()
                        .map(|d| format!(": {}", d.replace('\n', " / ").replace('\t', "")))
                        .unwrap_or_default()
                )
            })
            .collect();
        eprintln!(
            "sync {:6} committed={:?} owner_status={:?} helper_status={:?} issues={:?}",
            who,
            r.committed,
            self.fx.main_status(),
            self.fx.helper_status(),
            issues
        );
        for (name, dir, status) in [
            ("owner", &self.fx.main_dir, self.fx.main_status()),
            ("helper", &self.fx.helper_dir, self.fx.helper_status()),
        ] {
            if status.is_some() {
                eprintln!(
                    "  {} tree: [{}] graph: [{}]",
                    name,
                    g(dir, &["status", "--porcelain"]).replace('\n', "; "),
                    g(dir, &["log", "--graph", "--oneline", "--all", "-12"]).replace('\n', " | ")
                );
            }
        }
    }

    fn owner_file(&self, rel: &str) -> String {
        fs::read_to_string(self.fx.main_dir.join(rel)).unwrap_or_default()
    }

    fn helper_file(&self, rel: &str) -> String {
        fs::read_to_string(self.fx.helper_dir.join(rel)).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn done_md(ids: &[&str]) -> String {
    let mut s = String::from("## 2026-10-20\n\n");
    for id in ids {
        s.push_str(&format!("- {} | task {} | v1.0.0\n", id, id));
    }
    s
}

fn task_line(id: &str, description: &str, status: &str) -> String {
    format!(
        "- [ ] {} | {} | 1 | low | {} | 2026-10-01\n",
        id, description, status
    )
}

fn append(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    let mut cur = fs::read_to_string(&p).unwrap_or_default();
    if !cur.is_empty() && !cur.ends_with('\n') {
        cur.push('\n');
    }
    cur.push_str(text);
    fs::write(p, cur).unwrap();
}

/// Rewrite the one line of `rel` whose id is `id`.
fn edit_line(dir: &Path, rel: &str, id: &str, f: impl Fn(&str) -> String) {
    let p = dir.join(rel);
    let text = fs::read_to_string(&p).unwrap();
    let mut found = false;
    let out: Vec<String> = text
        .lines()
        .map(|l| {
            if l.contains(id) && !found {
                found = true;
                f(l)
            } else {
                l.to_string()
            }
        })
        .collect();
    assert!(found, "{} has no line for {}:\n{}", rel, id, text);
    fs::write(p, out.join("\n") + "\n").unwrap();
}

/// The `|`-separated field `n` (0 = id) of the todo line of `id`.
fn todo_field(text: &str, id: &str, n: usize) -> Option<String> {
    let line = text.lines().find(|l| l.contains(id))?;
    let line = line
        .trim_start_matches("- [ ] ")
        .trim_start_matches("- [x] ");
    line.split('|').nth(n).map(|s| s.trim().to_string())
}

fn status_in(text: &str, id: &str) -> Option<String> {
    todo_field(text, id, 4)
}

fn no_markers(text: &str) {
    for m in ["<<<<<<<", "=======", ">>>>>>>"] {
        assert!(!text.contains(m), "conflict marker {} in:\n{}", m, text);
    }
}

fn events(db: &AppDb) -> Vec<ActivityEvent> {
    db.read_timeline_filtered(
        &TimelineFilter {
            start_date: "1970-01-01".into(),
            end_date: "2100-01-01".into(),
            event_kinds: None,
            project_ids: None,
            repo_ids: None,
            search: None,
            author: None,
            author_is_me: false,
        },
        0,
        1000,
    )
    .unwrap()
}

fn task_db_status(db: &AppDb, repo: i64, id: &str) -> Option<String> {
    db.list_tasks_by_repo(repo, "todo")
        .unwrap()
        .into_iter()
        .find(|t| t.task_id == id)
        .and_then(|t| t.status)
}

fn task_db_description(db: &AppDb, repo: i64, id: &str) -> Option<String> {
    db.list_tasks_by_repo(repo, "todo")
        .unwrap()
        .into_iter()
        .find(|t| t.task_id == id)
        .map(|t| t.description)
}

fn shared_role(db: &AppDb, repo: i64) -> Option<String> {
    db.get_shared_state(repo).unwrap().shared_role
}

fn tracked(dir: &Path, rev: &str) -> Vec<String> {
    g(dir, &["ls-tree", "-r", "--name-only", rev])
        .lines()
        .map(String::from)
        .collect()
}

// ---------------------------------------------------------------------------
// Verification 1
// ---------------------------------------------------------------------------

/// Verification 1: the owner switches shared mode on. The preflight reports the
/// `.gitignore` line that hides the task files, enable refuses until the line
/// is removed, and after one sync origin/dev carries `.sdh/project.json`, the
/// `merge=sdh` attributes and the tracked task files; the driver is registered
/// in `.git/config`.
#[test]
fn v1_owner_enables_shared_mode_and_the_sync_publishes_the_structure() {
    let w = World::new();
    let fx = &w.fx;

    let pre = shared_mode_preflight(&fx.main_db, &gb(), fx.main_repo).unwrap();
    assert!(!pre.missing_project && !pre.missing_dev_branch);
    let lines: Vec<&str> = pre.blocking_lines.iter().map(|l| l.text.as_str()).collect();
    assert!(lines.contains(&"docs/todo.md"), "{:?}", pre.blocking_lines);
    // Without the removal the dialog's answer is a refusal and nothing flipped.
    assert!(enable_shared_mode(&fx.main_db, &gb(), fx.main_repo, false).is_err());
    assert!(
        !fx.main_db
            .get_shared_state(fx.main_repo)
            .unwrap()
            .shared_mode
    );
    assert!(w.owner_file(".gitignore").contains("docs/todo.md"));

    enable_shared_mode(&fx.main_db, &gb(), fx.main_repo, true).unwrap();
    let ignore = w.owner_file(".gitignore");
    assert!(
        !ignore.lines().any(|l| l.trim() == "docs/todo.md"),
        "the blocking line is gone: {}",
        ignore
    );
    write(&fx.main_dir, "docs/done.md", &done_md(&["T-000900"]));
    let res = w.sync_owner();
    assert!(
        res.committed.contains(&"api".to_string()),
        "{:?} {:?}",
        res.committed,
        res.errors
    );

    let structure = fx.remote_file("dev", ".sdh/project.json").unwrap();
    assert!(structure.contains("\"Owner\""), "{}", structure);
    let attrs = fx.remote_file("dev", ".gitattributes").unwrap();
    for rule in [
        "docs/todo.md merge=sdh",
        "docs/done.md merge=sdh",
        "docs/bug-reports.md merge=sdh",
    ] {
        assert!(attrs.lines().any(|l| l == rule), "{}\n{}", rule, attrs);
    }
    let files = tracked(&fx.main_dir, "origin/dev");
    for f in ["docs/todo.md", "docs/done.md", "docs/bug-reports.md"] {
        assert!(
            files.iter().any(|x| x == f),
            "{} not tracked: {:?}",
            f,
            files
        );
    }
    // Not ignored any more, and the driver is registered.
    assert!(!Command::new("git")
        .args(["check-ignore", "-q", "docs/todo.md"])
        .current_dir(&fx.main_dir)
        .status()
        .unwrap()
        .success());
    assert!(g(&fx.main_dir, &["config", "--local", "merge.sdh.driver"]).contains("--merge-driver"));
    // Local and remote dev agree; a second sync has nothing left to do.
    assert_eq!(
        g(&fx.main_dir, &["rev-parse", "dev"]),
        fx.remote_rev("dev").unwrap()
    );
    let again = w.sync_owner();
    assert!(again.committed.is_empty(), "{:?}", again.committed);
    assert_eq!(w.fx.main_status(), None);
    w.check_origin();
}

// ---------------------------------------------------------------------------
// Verification 2
// ---------------------------------------------------------------------------

/// Verification 2: the helper clones, picks their own branch and syncs; the
/// owner's next sync shows the request; the owner approves; after both sync the
/// helper is a contributor with the band 500000, and the helper's project was
/// created locally with the role of the repo read from the structure.
#[test]
fn v2_helper_joins_the_owner_approves_and_the_helper_becomes_a_contributor() {
    let w = World::new();
    let fx = &w.fx;
    w.owner_shares();

    fx.helper_db
        .set_autocommit_branch(fx.helper_repo, Some("helper-dev"))
        .unwrap();
    let before = fx.helper_db.get_repository(fx.helper_repo).unwrap();
    assert!(
        before.project_id.is_none(),
        "the repo starts outside any project"
    );
    w.sync_helper();
    assert_eq!(fx.helper_status().as_deref(), Some("pending_approval"));
    assert_eq!(
        shared_role(&fx.helper_db, fx.helper_repo).as_deref(),
        Some("pending")
    );
    // The helper's project was created from the structure, with the role in it.
    let repo = fx.helper_db.get_repository(fx.helper_repo).unwrap();
    assert_eq!(repo.role.as_deref(), Some("server"));
    let project = fx.helper_db.get_project(repo.project_id.unwrap()).unwrap();
    assert_eq!(project.name, "Acme");
    assert!(fx
        .remote_file("dev", ".sdh/requests/helper.json")
        .unwrap()
        .contains("helper-dev"));

    // The owner's sync brings the request in front of them.
    w.sync_owner();
    let reqs = list_join_requests(&fx.main_db, Some(&gb()), fx.main_repo).unwrap();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].login, "helper");
    assert_eq!(reqs[0].branch, "helper-dev");

    let approved = approve_join_request(&fx.main_db, Some(&gb()), fx.main_repo, "helper").unwrap();
    assert_eq!(approved.participants.len(), 1);
    w.sync_owner(); // approval committed and pushed
    w.sync_helper(); // contributor now, publishes its own branch
    w.sync_owner(); // the structure is plumbed into the helper's branch
    w.sync_helper(); // and the helper's checkout fast-forwards onto it

    assert_eq!(
        shared_role(&fx.helper_db, fx.helper_repo).as_deref(),
        Some("contributor")
    );
    assert_eq!(fx.helper_status(), None);
    let s = get_repo_structure(&fx.helper_db, Some(&gb()), fx.helper_repo)
        .unwrap()
        .unwrap();
    assert_eq!(s.participants.len(), 1);
    assert_eq!(s.participants[0].login, "helper");
    assert_eq!(s.participants[0].band_start, 500_000);
    // The owner's side: the request is settled, the participant is recorded.
    assert!(list_join_requests(&fx.main_db, Some(&gb()), fx.main_repo)
        .unwrap()
        .is_empty());
    assert_eq!(fx.remote_file("dev", ".sdh/requests/helper.json"), None);
    // The helper's checkout got the shared files; the project stays the same one.
    assert!(w.helper_file(".sdh/project.json").contains("\"Owner\""));
    assert!(fx.helper_dir.join("docs/todo.md").exists());
    let repo = fx.helper_db.get_repository(fx.helper_repo).unwrap();
    assert_eq!(repo.project_id, Some(project.id));
    assert_eq!(repo.role.as_deref(), Some("server"));
    w.check_origin();
}

// ---------------------------------------------------------------------------
// Scenario helpers
// ---------------------------------------------------------------------------

impl World {
    /// The two users keep pressing sync, and "Merge" whenever a repo asks for
    /// it (the `needs_manual_merge` status), until a whole round changes
    /// nothing. Returns who had to press the button.
    fn settle(&self) -> Vec<&'static str> {
        let mut pressed = Vec::new();
        for _ in 0..6 {
            let mut moved = false;
            moved |= !self.sync_helper().committed.is_empty();
            if self.fx.helper_status().as_deref() == Some("needs_manual_merge") {
                let r =
                    merge_shared_upstream(&self.fx.helper_db, &gb(), self.fx.helper_repo).unwrap();
                assert_eq!(r.outcome, "merged", "helper: {:?}", r);
                pressed.push("helper");
                moved = true;
            }
            moved |= !self.sync_owner().committed.is_empty();
            if self.fx.main_status().as_deref() == Some("needs_manual_merge") {
                let r = merge_shared_upstream(&self.fx.main_db, &gb(), self.fx.main_repo).unwrap();
                assert_eq!(r.outcome, "merged", "owner: {:?}", r);
                pressed.push("owner");
                moved = true;
            }
            if !moved && self.fx.helper_status().is_none() && self.fx.main_status().is_none() {
                return pressed;
            }
        }
        panic!(
            "the two installs did not settle: helper {:?}, owner {:?}",
            self.fx.helper_status(),
            self.fx.main_status()
        );
    }

    /// Both users have the same three open tasks, T-000001 (the skeleton's) to
    /// T-000003, in their `todo.md`, mirrored in both databases.
    fn shared_tasks(&self) {
        self.participants();
        append(
            &self.fx.main_dir,
            "docs/todo.md",
            &(task_line("T-000002", "second task", "open")
                + &task_line("T-000003", "third task", "open")),
        );
        self.open_owner();
        self.sync_owner();
        self.sync_helper();
        self.open_helper();
        for text in [
            self.owner_file("docs/todo.md"),
            self.helper_file("docs/todo.md"),
        ] {
            assert!(
                text.contains("T-000002") && text.contains("T-000003"),
                "{}",
                text
            );
        }
    }

    /// The helper's work of item 3: a task by hand and a bug by the button,
    /// synced; the owner synced once to receive it.
    fn helper_work(&self) -> String {
        append(
            &self.fx.helper_dir,
            "docs/todo.md",
            &task_line("T-500000", "helper task", "open"),
        );
        self.open_helper();
        let bug = World::add_bug(&self.fx.helper_db, self.fx.helper_repo, "helper bug");
        self.sync_helper();
        self.sync_owner();
        bug
    }
}

// ---------------------------------------------------------------------------
// Verification 3
// ---------------------------------------------------------------------------

/// Verification 3: the helper creates the task `T-500000` and a bug with the
/// button (`B-500000`); after the helper's sync and the owner's, the owner has
/// both, with the helper as the author in the timeline, and the owner's next
/// bug from the button is the owner's next number, not `B-500001`.
#[test]
fn v3_helpers_task_and_bug_reach_the_owner_with_their_author_and_the_bands_hold() {
    let w = World::new();
    let fx = &w.fx;
    w.participants();
    let own = World::add_bug(&fx.main_db, fx.main_repo, "owner bug");
    // The template's example row sits in an HTML comment and is not a bug.
    assert_eq!(own, "B-000001");
    let own_n: i64 = own[2..].parse().unwrap();
    w.sync_owner();
    w.sync_helper();

    let bug = w.helper_work();
    assert_eq!(bug, "B-500000", "the helper's band");

    // The owner's database has the task and the bug.
    assert_eq!(
        task_db_description(&fx.main_db, fx.main_repo, "T-500000").as_deref(),
        Some("helper task")
    );
    let got = fx
        .main_db
        .get_bug_by_display_id(fx.main_repo, "B-500000")
        .unwrap()
        .expect("the helper's bug is in the owner's database");
    assert_eq!(got.description, "helper bug");
    assert_eq!(got.numeric_id, 500_000);
    // ... and in the files the owner's tabs are rendered from.
    assert!(w.owner_file("docs/todo.md").contains("T-500000"));
    assert!(w.owner_file("docs/bug-reports.md").contains("B-500000"));
    // Opening the tab does not duplicate or lose the task.
    w.open_owner();
    let rows = fx.main_db.list_tasks_by_repo(fx.main_repo, "todo").unwrap();
    assert_eq!(rows.iter().filter(|t| t.task_id == "T-500000").count(), 1);

    // The timeline names the helper as the author of both.
    let evs = events(&fx.main_db);
    let task_ev: Vec<_> = evs
        .iter()
        .filter(|e| e.kind == "task_event" && e.task_display_id.as_deref() == Some("T-500000"))
        .collect();
    let bug_ev: Vec<_> = evs
        .iter()
        .filter(|e| e.kind == "bug_event" && e.bug_display_id.as_deref() == Some("B-500000"))
        .collect();
    assert!(!task_ev.is_empty() && !bug_ev.is_empty(), "{:?}", evs);
    for e in task_ev.iter().chain(bug_ev.iter()) {
        assert_eq!(e.author.as_deref(), Some("helper"), "{:?}", e);
    }
    // The owner's own bug is theirs (no author).
    assert!(evs
        .iter()
        .filter(|e| e.bug_display_id.as_deref() == Some(own.as_str()))
        .all(|e| e.author.is_none()));

    // The owner's next bug stays below the helper's band.
    assert_eq!(
        ss::allocate_bug_numeric_id(&fx.main_db, fx.main_repo).unwrap(),
        own_n + 1
    );
    assert_eq!(
        World::add_bug(&fx.main_db, fx.main_repo, "owner bug 2"),
        format!("B-{:06}", own_n + 1)
    );
    w.check_origin();
}

// ---------------------------------------------------------------------------
// Verification 4
// ---------------------------------------------------------------------------

/// Verification 4: the owner and the helper change different tasks of the same
/// `todo.md` between two syncs. After the syncs (and the Merge button wherever
/// a repo asks for it) both work trees and both branches on the remote hold
/// both edits, and no conflict marker is anywhere.
#[test]
fn v4_different_tasks_of_one_todo_are_both_kept_everywhere() {
    let w = World::new();
    let fx = &w.fx;
    w.shared_tasks();

    edit_line(&fx.main_dir, "docs/todo.md", "T-000002", |l| {
        l.replace("second task", "second task, edited by the owner")
    });
    edit_line(&fx.helper_dir, "docs/todo.md", "T-000003", |l| {
        l.replace("third task", "third task, edited by the helper")
    });
    let pressed = w.settle();
    eprintln!("v4: Merge pressed by {:?}", pressed);

    let places = [
        ("owner work tree", w.owner_file("docs/todo.md")),
        ("helper work tree", w.helper_file("docs/todo.md")),
        ("origin/dev", fx.remote_file("dev", "docs/todo.md").unwrap()),
        (
            "origin/helper-dev",
            fx.remote_file("helper-dev", "docs/todo.md").unwrap(),
        ),
    ];
    for (name, text) in &places {
        no_markers(text);
        assert_eq!(
            todo_field(text, "T-000002", 1).as_deref(),
            Some("second task, edited by the owner"),
            "{}:\n{}",
            name,
            text
        );
        assert_eq!(
            todo_field(text, "T-000003", 1).as_deref(),
            Some("third task, edited by the helper"),
            "{}:\n{}",
            name,
            text
        );
    }
    // The files of the two checkouts are identical, and so are the databases.
    assert_eq!(places[0].1, places[1].1);
    w.open_owner();
    w.open_helper();
    for (db, repo) in [(&fx.main_db, fx.main_repo), (&fx.helper_db, fx.helper_repo)] {
        assert_eq!(
            task_db_description(db, repo, "T-000002").as_deref(),
            Some("second task, edited by the owner")
        );
        assert_eq!(
            task_db_description(db, repo, "T-000003").as_deref(),
            Some("third task, edited by the helper")
        );
    }
    w.check_origin();
}

// ---------------------------------------------------------------------------
// Verification 5
// ---------------------------------------------------------------------------

/// Verification 5: both change the status of the same task between syncs (the
/// owner to `in-progress`, the helper to `review`). Both sides end with the same
/// result: the status that is further along, `review`; and the other way round.
#[test]
fn v5_the_same_task_changed_on_both_sides_ends_with_the_further_status() {
    let w = World::new();
    let fx = &w.fx;
    w.shared_tasks();

    edit_line(&fx.main_dir, "docs/todo.md", "T-000002", |l| {
        l.replace("| open |", "| in-progress |")
    });
    edit_line(&fx.helper_dir, "docs/todo.md", "T-000002", |l| {
        l.replace("| open |", "| review |")
    });
    let pressed = w.settle();
    eprintln!("v5: Merge pressed by {:?}", pressed);

    for (name, text) in [
        ("owner work tree", w.owner_file("docs/todo.md")),
        ("helper work tree", w.helper_file("docs/todo.md")),
        ("origin/dev", fx.remote_file("dev", "docs/todo.md").unwrap()),
        (
            "origin/helper-dev",
            fx.remote_file("helper-dev", "docs/todo.md").unwrap(),
        ),
    ] {
        no_markers(&text);
        assert_eq!(
            status_in(&text, "T-000002").as_deref(),
            Some("review"),
            "{}:\n{}",
            name,
            text
        );
    }
    w.open_owner();
    w.open_helper();
    assert_eq!(
        task_db_status(&fx.main_db, fx.main_repo, "T-000002").as_deref(),
        Some("review")
    );
    assert_eq!(
        task_db_status(&fx.helper_db, fx.helper_repo, "T-000002").as_deref(),
        Some("review")
    );

    // And the other way round: this time the owner is the one further along.
    edit_line(&fx.main_dir, "docs/todo.md", "T-000003", |l| {
        l.replace("| open |", "| review |")
    });
    edit_line(&fx.helper_dir, "docs/todo.md", "T-000003", |l| {
        l.replace("| open |", "| in-progress |")
    });
    w.settle();
    assert_eq!(
        status_in(&w.owner_file("docs/todo.md"), "T-000003").as_deref(),
        Some("review")
    );
    assert_eq!(
        status_in(&w.helper_file("docs/todo.md"), "T-000003").as_deref(),
        Some("review")
    );
    w.check_origin();
}

// ---------------------------------------------------------------------------
// Verification 6
// ---------------------------------------------------------------------------

/// Verification 6: the owner's local AI edits a bug description in the work
/// tree (uncommitted): the next sync restores it and nothing of the edit is ever
/// committed. The helper edits the description of `T-500000` in their `todo.md`
/// and syncs: the owner's next sync takes the new description in, in the
/// database and in the file.
#[test]
fn v6_a_local_ai_edit_of_a_bug_is_undone_and_a_committed_task_edit_is_taken() {
    let w = World::new();
    let fx = &w.fx;
    w.participants();

    // (a) the local AI rewrites the bug text.
    let id = World::add_bug(&fx.main_db, fx.main_repo, "original text");
    w.sync_owner();
    let path = fx.main_dir.join("docs/bug-reports.md");
    let edited = fs::read_to_string(&path)
        .unwrap()
        .replace("original text", "local AI changed it");
    assert!(edited.contains("local AI changed it"));
    fs::write(&path, edited).unwrap();
    w.sync_owner();
    let got = fx
        .main_db
        .get_bug_by_display_id(fx.main_repo, &id)
        .unwrap()
        .unwrap();
    assert_eq!(got.description, "original text");
    let file = w.owner_file("docs/bug-reports.md");
    assert!(file.contains("original text") && !file.contains("local AI changed it"));
    // The edit never became history, locally or on the remote.
    for dir in [&fx.main_dir, &fx.origin] {
        assert_eq!(
            g(
                dir,
                &["log", "--all", "-S", "local AI changed it", "--oneline"]
            ),
            ""
        );
    }
    w.sync_helper();
    assert!(w
        .helper_file("docs/bug-reports.md")
        .contains("original text"));

    // (b) the helper rewords their own task.
    w.helper_work();
    w.sync_helper();
    assert_eq!(
        task_db_description(&fx.main_db, fx.main_repo, "T-500000").as_deref(),
        Some("helper task")
    );
    edit_line(&fx.helper_dir, "docs/todo.md", "T-500000", |l| {
        l.replace("helper task", "reworded by the helper")
    });
    w.sync_helper();
    w.sync_owner();
    assert_eq!(
        todo_field(&w.owner_file("docs/todo.md"), "T-500000", 1).as_deref(),
        Some("reworded by the helper")
    );
    // The tasks table (what the TasksTab shows) takes the new text too, and the
    // disk run of the tab does not bring the old one back.
    assert_eq!(
        task_db_description(&fx.main_db, fx.main_repo, "T-500000").as_deref(),
        Some("reworded by the helper")
    );
    w.open_owner();
    assert_eq!(
        task_db_description(&fx.main_db, fx.main_repo, "T-500000").as_deref(),
        Some("reworded by the helper")
    );
    w.check_origin();
}

// ---------------------------------------------------------------------------
// Verification 7
// ---------------------------------------------------------------------------

/// Verification 7: the owner commits code on `dev` and does not push; the
/// helper pushes a task. The owner's sync reports `needs_manual_merge`, master
/// is untouched and nothing is forced (every remote ref only moved forward: the
/// old `origin/dev` is an ancestor of the new one); the Merge button merges
/// without a conflict in `todo.md` -- both sides appended to the end of the
/// file, which only the registered driver (merging by id) can resolve -- and the
/// next sync pushes the merge.
///
/// The driver in the clones is the app executable in production. A unit test
/// runs inside the test harness, which is not the app; see `arm_driver` for how
/// the very same driver code runs inside a real `git merge` here. The real
/// binary in a real `git merge` is covered by `tests/merge_driver_git.rs`.
#[test]
fn v7_unpushed_code_and_a_helpers_push_need_a_manual_merge_that_merges_by_id() {
    let w = World::new();
    let fx = &w.fx;
    w.participants();

    // The owner's local commit: code and a task, at the end of todo.md.
    append(
        &fx.main_dir,
        "docs/todo.md",
        &task_line("T-000010", "owner task", "open"),
    );
    write(&fx.main_dir, "src.rs", "fn main() {}\n");
    g(&fx.main_dir, &["add", "src.rs", "docs/todo.md"]);
    g(&fx.main_dir, &["commit", "-q", "-m", "code and a task"]);
    let local_commit = g(&fx.main_dir, &["rev-parse", "HEAD"]);

    // The helper pushes a task, also at the end of the file.
    append(
        &fx.helper_dir,
        "docs/todo.md",
        &task_line("T-500000", "helper task", "open"),
    );
    w.sync_helper();
    let old_dev = fx.remote_rev("dev").unwrap();
    assert!(fx
        .remote_file("dev", "docs/todo.md")
        .unwrap()
        .contains("T-500000"));

    let out = w.sync_owner();
    assert_eq!(fx.main_status().as_deref(), Some("needs_manual_merge"));
    assert!(out
        .errors
        .iter()
        .any(|i| i.code == crate::models::SyncIssueCode::SharedNeedsManualMerge));
    // Nothing was forced, pushed over or merged behind the owner's back.
    assert_eq!(fx.remote_rev("dev").unwrap(), old_dev);
    assert_eq!(
        fx.remote_file("dev", "src.rs"),
        None,
        "code must not travel unasked"
    );
    assert_eq!(g(&fx.main_dir, &["rev-parse", "HEAD"]), local_commit);
    assert!(!w.owner_file("docs/todo.md").contains("T-500000"));
    assert_eq!(
        fx.remote_rev("master").unwrap(),
        w.master0,
        "master is untouched"
    );

    // The registered driver is the shim for this clone (see `arm_driver`).
    assert!(g(&fx.main_dir, &["config", "--get", "merge.sdh.driver"]).contains("sdh-shim"));
    let res = merge_shared_upstream(&fx.main_db, &gb(), fx.main_repo).unwrap();
    assert_eq!(res.outcome, "merged", "{:?}", res);
    let todo = w.owner_file("docs/todo.md");
    no_markers(&todo);
    assert!(
        todo.contains("T-000010") && todo.contains("T-500000"),
        "{}",
        todo
    );
    assert_eq!(fx.main_status(), None);
    assert_eq!(
        g(&fx.main_dir, &["rev-list", "--parents", "-n", "1", "HEAD"])
            .split(' ')
            .count(),
        3,
        "a merge commit"
    );

    // The next sync pushes the merge: a fast-forward of the remote's dev.
    w.sync_owner();
    let new_dev = fx.remote_rev("dev").unwrap();
    assert_ne!(new_dev, old_dev);
    g(
        &fx.origin,
        &["merge-base", "--is-ancestor", &old_dev, &new_dev],
    );
    assert!(
        fx.remote_file("dev", "src.rs").is_some(),
        "the owner's commit is pushed now"
    );
    let remote_todo = fx.remote_file("dev", "docs/todo.md").unwrap();
    assert!(remote_todo.contains("T-000010") && remote_todo.contains("T-500000"));
    assert_eq!(fx.main_status(), None);
    // The helper receives the merged file.
    w.settle();
    let helper_todo = w.helper_file("docs/todo.md");
    no_markers(&helper_todo);
    assert!(
        helper_todo.contains("T-000010") && helper_todo.contains("T-500000"),
        "{}",
        helper_todo
    );
    w.check_origin();
}

// ---------------------------------------------------------------------------
// Verification 11
// ---------------------------------------------------------------------------

/// Verification 11: the owner switches the mode off. `.sdh/` is gone from
/// origin/dev and from the owner's tree, the owner's repo is ordinary, and the
/// helper's next syncs raise no status and leave every row of the helper's
/// database intact; the repo is ordinary for the helper (no role) and its branch
/// no longer carries `.sdh/` (BUG-3, below).
#[test]
fn v11_switching_off_removes_sdh_and_the_helpers_repo_is_ordinary_again() {
    let w = World::new();
    let fx = &w.fx;
    w.participants();
    let own = World::add_bug(&fx.main_db, fx.main_repo, "owner bug");
    w.sync_owner();
    w.sync_helper();
    w.helper_work();
    w.sync_helper();
    w.open_helper();
    assert_eq!(
        shared_role(&fx.helper_db, fx.helper_repo).as_deref(),
        Some("contributor")
    );
    let rows = |db: &AppDb, repo: i64| {
        let tasks: Vec<String> = db
            .list_tasks_by_repo(repo, "todo")
            .unwrap()
            .into_iter()
            .map(|t| format!("{}|{}|{:?}", t.task_id, t.description, t.status))
            .collect();
        let bugs: Vec<String> = db
            .list_bugs_for_md(repo)
            .unwrap()
            .into_iter()
            .map(|b| format!("{}|{}|{}", b.display_id, b.description, b.status))
            .collect();
        (tasks, bugs)
    };
    let before = rows(&fx.helper_db, fx.helper_repo);
    assert!(before.0.iter().any(|t| t.starts_with("T-500000")));
    assert!(before.1.iter().any(|b| b.starts_with("B-500000")));
    assert!(before.1.iter().any(|b| b.starts_with(own.as_str())));

    let res = disable_shared_mode(&fx.main_db, &gb(), fx.main_repo).unwrap();
    assert!(res.pushed, "{:?}", res);
    w.check_origin();
    let files = tracked(&fx.origin, "dev");
    assert!(
        !files.iter().any(|f| f.starts_with(".sdh/")),
        ".sdh is gone from origin/dev: {:?}",
        files
    );
    assert!(!fx.main_dir.join(".sdh").exists());
    let st = fx.main_db.get_shared_state(fx.main_repo).unwrap();
    assert!(!st.shared_mode && st.shared_role.is_none());
    // The owner's repo is ordinary and syncs as before.
    w.sync_owner();
    assert_eq!(fx.main_status(), None);

    // The helper's next sync.
    w.sync_helper();
    assert_eq!(shared_role(&fx.helper_db, fx.helper_repo), None);
    assert_eq!(fx.helper_status(), None);
    assert_eq!(
        rows(&fx.helper_db, fx.helper_repo),
        before,
        "the helper's rows survive"
    );
    w.sync_helper();
    w.sync_helper();
    assert_eq!(shared_role(&fx.helper_db, fx.helper_repo), None);
    assert_eq!(fx.helper_status(), None);
    assert_eq!(rows(&fx.helper_db, fx.helper_repo), before);
    assert_no_sdh_anywhere(&w);
    w.check_origin();
}

// ---------------------------------------------------------------------------
// The guard
// ---------------------------------------------------------------------------

/// Guard: through a whole life of the feature -- enable, join, work on both
/// sides, a concurrent edit, a manual merge, switching off -- nothing ever
/// writes `refs/heads/master` or `main` on the remote, deletes a ref or moves
/// one except forward. The remote's own `update` hook refuses and records any
/// such update (checked after every sync by `World::check_origin`); this test
/// also compares the end state with the start.
#[test]
fn guard_nothing_ever_writes_master_or_main_or_forces_the_remote() {
    let w = World::new();
    let fx = &w.fx;
    let start = w.origin_refs();
    w.shared_tasks();
    w.helper_work();
    edit_line(&fx.main_dir, "docs/todo.md", "T-000002", |l| {
        l.replace("second task", "owner")
    });
    edit_line(&fx.helper_dir, "docs/todo.md", "T-000003", |l| {
        l.replace("third task", "helper")
    });
    w.settle();
    write(&fx.main_dir, "src.rs", "fn main() {}\n");
    g(&fx.main_dir, &["add", "src.rs"]);
    g(&fx.main_dir, &["commit", "-q", "-m", "code"]);
    append(
        &fx.helper_dir,
        "docs/todo.md",
        &task_line("T-500001", "later", "open"),
    );
    w.sync_helper();
    w.sync_owner();
    if fx.main_status().as_deref() == Some("needs_manual_merge") {
        merge_shared_upstream(&fx.main_db, &gb(), fx.main_repo).unwrap();
    }
    w.settle();
    disable_shared_mode(&fx.main_db, &gb(), fx.main_repo).unwrap();
    w.sync_owner();
    w.sync_helper();
    w.sync_helper();

    let end = w.origin_refs();
    assert_eq!(end.get("refs/heads/master"), start.get("refs/heads/master"));
    assert_eq!(
        end.get("refs/heads/master").map(String::as_str),
        Some(w.master0.as_str())
    );
    assert!(!end.contains_key("refs/heads/main"));
    for (r, old) in &start {
        let new = end.get(r).unwrap_or_else(|| panic!("{} deleted", r));
        g(&fx.origin, &["merge-base", "--is-ancestor", old, new]);
    }
    for r in end.keys() {
        assert!(
            r == "refs/heads/dev" || r == "refs/heads/master" || r == "refs/heads/helper-dev",
            "unexpected ref {}",
            r
        );
    }
    let log = fs::read_to_string(&w.hook_log).unwrap_or_default();
    assert!(log.trim().is_empty(), "{}", log);
}

// ---------------------------------------------------------------------------
// BUG-1 (found by these scenarios, fixed)
// ---------------------------------------------------------------------------

/// BUG-1 (fixed, Р16). A contributor's project sync used to render the rule
/// copies into `docs/sdh_skills/` of the shared clone, where they stayed
/// untracked (only the owner commits them); the owner's committed copies then
/// could not be fast-forwarded in -- `git merge --ff-only` refused ("untracked
/// working tree files would be overwritten"), the sync reported a false
/// `needs_manual_merge` and the helper never got the owner's files again. Now a
/// contributor's / pending helper's install renders nothing there, and copies an
/// earlier run left untracked are removed.
#[test]
fn bug1_a_contributors_rendered_skills_do_not_block_the_fast_forward() {
    let w = World::new();
    w.owner_shares();
    w.sync_helper();
    w.sync_owner();
    approve_join_request(&w.fx.main_db, Some(&gb()), w.fx.main_repo, "helper").unwrap();
    w.sync_owner();
    w.sync_helper(); // contributor: renders no rule copies, pushes its branch
    w.sync_owner(); // plumbs the owner's files (with the skills) into it
    w.sync_helper(); // must fast-forward onto them
    assert_eq!(w.fx.helper_status(), None);
    assert!(w.helper_file(".sdh/project.json").contains("\"Owner\""));
    // The owner's committed copies are what the helper's clone holds, tracked.
    assert!(tracked(&w.fx.helper_dir, "HEAD")
        .iter()
        .any(|f| f.starts_with("docs/sdh_skills/sdh-")));
}

/// T-000224 (Р16, the owner's decision В4). Rules follow the owner at a
/// contributor: copies an earlier run rendered untracked into the clone are
/// removed (so the fast-forward goes through), the owner's committed text of a
/// rule that differs from the helper's canon becomes the helper's canon -- custom,
/// with a `taken` event naming the owner -- and is rendered to `~/.claude`, the
/// rule is locked in the app while the repo is shared, and switching sharing off
/// unlocks it, the text staying.
#[test]
fn the_owners_rule_canon_reaches_a_contributor_and_is_released_on_switch_off() {
    const FILE: &str = "skill.sdh-debugging.md.tmpl";
    const OWNER_TEXT: &str = "# The owner's own debugging rule
";
    let w = World::new();
    // Before the join an earlier version of the app rendered the rules here.
    write(
        &w.fx.helper_dir,
        "docs/sdh_skills/sdh-debugging.md",
        "RENDERED BEFORE THE JOIN
",
    );
    write(
        &w.fx.helper_dir,
        "docs/sdh_skills/sdh-retro.md",
        "RENDERED BEFORE THE JOIN
",
    );
    w.fx.main_db
        .upsert_template_file("_global", FILE, OWNER_TEXT, true)
        .unwrap();

    w.join();

    assert_eq!(w.fx.helper_status(), None);
    let db = &w.fx.helper_db;
    assert!(tracked(&w.fx.helper_dir, "HEAD")
        .iter()
        .any(|f| f == "docs/sdh_skills/sdh-retro.md"));
    assert_eq!(
        w.helper_file("docs/sdh_skills/sdh-debugging.md"),
        OWNER_TEXT
    );
    let canon = db.get_template_file("_global", FILE).unwrap().unwrap();
    assert_eq!(canon.content, OWNER_TEXT);
    assert!(canon.is_custom);
    // Only the rule whose text differs was taken, with the owner's name.
    let origin = db.latest_origin_event("sdh-debugging").unwrap().unwrap();
    assert_eq!(origin.kind, "taken");
    assert_eq!(origin.note.as_deref(), Some("canon of main Owner"));
    assert!(db.latest_origin_event("sdh-retro").unwrap().is_none());
    assert_eq!(
        crate::sync::shared_canon::follows_main(db, "sdh-debugging").as_deref(),
        Some("Owner")
    );
    // An unchanged rule is the owner's as well, only nothing was taken for it.
    assert_eq!(
        crate::sync::shared_canon::follows_main(db, "sdh-retro").as_deref(),
        Some("Owner")
    );
    // Delivered to the helper's other targets as well.
    let global = w
        .helper_home
        .path()
        .join(".claude/skills/sdh-debugging/SKILL.md");
    assert_eq!(fs::read_to_string(global).unwrap(), OWNER_TEXT);
    // The CLAUDE.md section tells the helper whose repo it is and his band.
    let claude = w.helper_file("CLAUDE.md");
    assert!(claude.contains("## Shared repo"), "{}", claude);
    assert!(claude.contains("Owner: `Owner`"), "{}", claude);
    assert!(claude.contains("`500000`–`599999`"), "{}", claude);
    // Nothing of this is left to commit by hand.
    assert!(g(
        &w.fx.helper_dir,
        &["status", "--porcelain", "docs/sdh_skills"]
    )
    .is_empty());
    w.check_origin();

    // The owner switches sharing off: the helper's repo is ordinary again, the
    // rule is editable, and its text is the helper's own canon now.
    let res = disable_shared_mode(&w.fx.main_db, &gb(), w.fx.main_repo).unwrap();
    assert!(res.pushed, "{:?}", res);
    w.sync_owner();
    w.sync_helper();
    w.sync_helper();
    assert_eq!(shared_role(&w.fx.helper_db, w.fx.helper_repo), None);
    assert_eq!(
        crate::sync::shared_canon::follows_main(db, "sdh-debugging"),
        None
    );
    let kept = db.get_template_file("_global", FILE).unwrap().unwrap();
    assert_eq!((kept.content.as_str(), kept.is_custom), (OWNER_TEXT, true));
    w.check_origin();
}

// ---------------------------------------------------------------------------
// BUG-2 and BUG-3 (found by these scenarios, fixed)
// ---------------------------------------------------------------------------

/// BUG-2, minimal repro (fixed). Not a shared-mode defect: the tasks table mirrored status, source and version of a
/// task but never its description, effort or priority after the row is
/// inserted. The TasksTab reads that table, so a description reworded in
/// `todo.md` -- by a local AI in an ordinary repo, or by a helper's commit in a
/// shared one (items 4 and 6 of the spec) -- shows the OLD text there while the
/// file has the new one.
#[test]
fn bug2_a_reworded_task_description_reaches_the_tasks_table() {
    let fx = two_installs();
    write(&fx.main_dir, "docs/todo.md", &todo(&["T-000002"]));
    crate::sync::tasks::sync_tasks_for_repo(&fx.main_db, fx.main_repo).unwrap();
    assert_eq!(
        task_db_description(&fx.main_db, fx.main_repo, "T-000002").as_deref(),
        Some("task T-000002")
    );
    write(
        &fx.main_dir,
        "docs/todo.md",
        &todo(&["T-000002"]).replace("task T-000002", "reworded"),
    );
    crate::sync::tasks::sync_tasks_for_repo(&fx.main_db, fx.main_repo).unwrap();
    assert_eq!(
        task_db_description(&fx.main_db, fx.main_repo, "T-000002").as_deref(),
        Some("reworded")
    );
}

/// `.sdh/` is nowhere on the helper's side any more: not in the work tree, not
/// in the local or the remote copy of its branch, not on origin's dev.
fn assert_no_sdh_anywhere(w: &World) {
    // The helper's local branch from its clone; the remote's branches straight
    // from the bare origin, not from the clone's remote-tracking refs.
    for (what, rev, dir) in [
        ("local helper-dev", "helper-dev", &w.fx.helper_dir),
        ("origin helper-dev", "helper-dev", &w.fx.origin),
        ("origin dev", "dev", &w.fx.origin),
    ] {
        let files = tracked(dir, rev);
        assert!(
            !files.iter().any(|f| f.starts_with(".sdh/")),
            "{} still tracks .sdh: {:?}",
            what,
            files
        );
    }
    assert!(!w.fx.helper_dir.join(".sdh").join("project.json").exists());
}

/// BUG-3 (fixed). The owner switches the mode off. `.sdh/` used to be deleted on
/// `dev` only; the helper's own branch kept `.sdh/project.json`, discovery read
/// the work-tree file first and found it valid, so the helper stayed a
/// contributor for good. Now the owner removes it from every participant branch
/// too, the contributor reads the switch-off from origin's dev, and the branch
/// no longer carries the file after a round.
#[test]
fn bug3_switching_off_makes_the_helpers_repo_ordinary_again() {
    let w = World::new();
    w.participants();
    assert_eq!(
        shared_role(&w.fx.helper_db, w.fx.helper_repo).as_deref(),
        Some("contributor")
    );
    let res = disable_shared_mode(&w.fx.main_db, &gb(), w.fx.main_repo).unwrap();
    assert!(res.pushed, "{:?}", res);
    // The owner's deletion reached the participant branch on the remote.
    assert!(!tracked(&w.fx.origin, "helper-dev")
        .iter()
        .any(|f| f.starts_with(".sdh/")));
    w.sync_owner();
    w.sync_helper();
    w.sync_helper();
    assert_eq!(shared_role(&w.fx.helper_db, w.fx.helper_repo), None);
    assert_eq!(w.fx.helper_status(), None);
    assert_no_sdh_anywhere(&w);
    w.check_origin();
}

/// BUG-3, contributor side alone: the owner's switch-off could not reach the
/// helper's branch (here: the owner's structure no longer listed the
/// participant when sharing was switched off). The helper's sync reads the
/// switch-off from origin's dev, clears the role and deletes the stale file from
/// its own branch -- committed on the current branch and pushed -- so a later
/// pull request from it cannot switch sharing back on in dev.
#[test]
fn bug3_the_contributor_drops_a_stale_structure_the_owner_could_not_remove() {
    let w = World::new();
    w.participants();
    // The owner's switch-off does not know the participant's branch.
    let path = w.fx.main_dir.join(".sdh").join("project.json");
    let mut v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    v["participants"] = serde_json::json!([]);
    fs::write(&path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
    let res = disable_shared_mode(&w.fx.main_db, &gb(), w.fx.main_repo).unwrap();
    assert!(res.pushed, "{:?}", res);
    assert!(
        tracked(&w.fx.origin, "helper-dev")
            .iter()
            .any(|f| f == ".sdh/project.json"),
        "the owner left the participant branch alone"
    );

    w.sync_helper();
    assert_eq!(shared_role(&w.fx.helper_db, w.fx.helper_repo), None);
    assert_eq!(w.fx.helper_status(), None);
    assert_no_sdh_anywhere(&w);
    w.sync_helper();
    assert_eq!(shared_role(&w.fx.helper_db, w.fx.helper_repo), None);
    w.check_origin();
}
