// F-000044 / T-000224 (Р16, owner's decision В4): the rules of a shared repo
// follow the owner.
//
// In a shared repo the owner's install renders and commits `docs/sdh_skills/`;
// a contributor's install never renders there. What it does instead, after the
// Phase A import, is read the owner's committed copies and take their text as
// its OWN canon -- `templates._global.skill.<name>.md.tmpl`, custom -- so that
// the contributor's other repos and `~/.claude/skills/` carry the same rules.
//
// A rule taken this way is *followed*: it is recorded in the setting
// `rules_from_main` (a JSON map rule -> `Followed`) and the Rules screen shows
// it read-only. The record is valid only while the repo it came through is
// still a contributor repo of this install; `followed_rules` drops the rest
// whenever it reads (role cleared, repo deleted). The text itself stays as the
// install's custom canon when the lock goes.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::db::{rule_event, AppDb};
use crate::git_ops::{spawn_cmd, GitBinary};
use crate::git_sync;
use crate::models::Repository;
use crate::sync::shared::{blob_text, ls_tree, SKILLS_DIR};
use crate::sync::skills::{list_skill_templates, rule_copy_label, rules_guard};

/// Setting key: JSON map rule name -> [`Followed`].
pub const RULES_FROM_MAIN_KEY: &str = "rules_from_main";

/// Prefix of the error a save / reset of a followed rule is refused with
/// (`rule_follows_main: <login>`). The frontend disables the buttons first;
/// this is the backstop.
pub const ERR_FOLLOWS_MAIN: &str = "rule_follows_main";

/// Where one followed rule's text came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Followed {
    /// Login of the owner (main) whose copy it is.
    pub login: String,
    /// The shared repo of this install the copy was read in.
    pub repo_id: i64,
    /// Unix seconds of the last commit that touched that copy; decides which of
    /// two repos wins when they carry different texts of one rule.
    pub committed_at: i64,
}

/// What one adoption pass did.
#[derive(Debug, Default)]
pub struct Adopted {
    /// Rules whose canon changed.
    pub changed: Vec<String>,
    /// Steps that failed (reported, never fatal).
    pub errors: Vec<String>,
}

fn read_raw(db: &AppDb) -> BTreeMap<String, Followed> {
    db.get_setting(RULES_FROM_MAIN_KEY)
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

fn write_raw(db: &AppDb, map: &BTreeMap<String, Followed>) -> Result<(), String> {
    if map.is_empty() {
        return db
            .delete_setting(RULES_FROM_MAIN_KEY)
            .map_err(|e| e.to_string());
    }
    let json = serde_json::to_string(map).map_err(|e| e.to_string())?;
    db.set_setting(RULES_FROM_MAIN_KEY, &json)
        .map_err(|e| e.to_string())
}

/// The rules this install takes from an owner, rule name -> source. An entry is
/// valid only while its repo is a contributor repo of this install: the rest
/// (the repo stopped being shared, or was deleted) are removed here, which is
/// what unlocks those rules. A failed state read keeps the entry.
pub fn followed_rules(db: &AppDb) -> BTreeMap<String, Followed> {
    let mut map = read_raw(db);
    let before = map.len();
    map.retain(|_, f| match db.get_shared_state(f.repo_id) {
        Ok(state) => state.shared_role.as_deref() == Some("contributor"),
        Err(_) => true,
    });
    if map.len() != before {
        // Best effort: the next read prunes again.
        let _ = write_raw(db, &map);
    }
    map
}

/// The owner's login when `rule` follows an owner, `None` when the install
/// owns the text.
pub fn follows_main(db: &AppDb, rule: &str) -> Option<String> {
    followed_rules(db).remove(rule).map(|f| f.login)
}

/// Refuse a save / reset of a followed rule.
pub fn refuse_if_followed(db: &AppDb, rule: &str) -> Result<(), String> {
    match follows_main(db, rule) {
        Some(login) => Err(format!("{}: {}", ERR_FOLLOWS_MAIN, login)),
        None => Ok(()),
    }
}

/// Unix seconds of the newest commit of `rev` that touched `path`.
fn last_commit_time(git: &GitBinary, root: &Path, rev: &str, path: &str) -> i64 {
    spawn_cmd(&git.0)
        .args(["log", "-1", "--format=%ct", rev, "--", path])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(0)
}

/// Take the owner's published rule copies of `repo` (a contributor repo whose
/// owner is `main_login`, dev branch `dev_branch`) as this install's canon.
///
/// The copies are read from `origin/<dev_branch>` -- the owner's published
/// branch, the same trust boundary as the structure file -- never from the
/// checked-out branch, which the contributor writes himself. A missing ref
/// (nothing fetched yet) skips the pass.
///
/// For each `docs/sdh_skills/<name>.md` tracked there whose name is a skill this
/// install knows, the rule is *followed*: remembered in `rules_from_main` and so
/// locked in the app, whether or not the text differs. When it differs from this
/// install's canon, the text is also stored as the canon (custom) and a `taken`
/// event with the repo and the note `canon of main <login>` is recorded. When
/// another repo already feeds the same rule, the copy whose last commit is
/// newest wins; a tie keeps the one already followed.
///
/// Called by Phase A after the import and the fast-forward. The caller renders
/// everywhere when `changed` is not empty.
pub fn adopt_main_canon(
    db: &AppDb,
    git: &GitBinary,
    repo: &Repository,
    root: &Path,
    main_login: &str,
    dev_branch: &str,
) -> Result<Adopted, String> {
    let mut out = Adopted::default();
    let rev = format!("refs/remotes/origin/{}", dev_branch);
    if git_sync::rev_parse(git, root, &rev)?.is_none() {
        return Ok(out);
    }
    let tree = ls_tree(git, root, &rev, &[SKILLS_DIR.to_string()])?;
    if tree.is_empty() {
        return Ok(out);
    }
    // One hold for the read of the canon, the writes and the record: a rule
    // save or an uptake cannot slip in between.
    let _guard = rules_guard();
    let known: BTreeMap<String, String> = list_skill_templates(db)?.into_iter().collect();
    let mut followed = followed_rules(db);
    let label = rule_copy_label(repo);
    let prefix = format!("{}/", SKILLS_DIR);
    let mut dirty = false;
    for (path, sha) in &tree {
        let Some(name) = path
            .strip_prefix(&prefix)
            .and_then(|f| f.strip_suffix(".md"))
            .filter(|n| !n.contains('/'))
        else {
            continue;
        };
        let Some(current) = known.get(name) else {
            continue;
        };
        let at = last_commit_time(git, root, &rev, path);
        if let Some(f) = followed.get(name) {
            if f.repo_id != repo.id && f.committed_at >= at {
                continue;
            }
        }
        let text = match blob_text(git, root, sha) {
            Ok(t) => t,
            Err(e) => {
                out.errors.push(format!("{}: {}", path, e));
                continue;
            }
        };
        let differs = text != *current;
        if differs {
            let file_name = format!("skill.{}.md.tmpl", name);
            if let Err(e) = db.upsert_template_file("_global", &file_name, &text, true) {
                out.errors.push(format!("{}: {}", file_name, e));
                continue;
            }
            out.changed.push(name.to_string());
            let note = format!("canon of main {}", main_login);
            if let Err(e) =
                db.insert_rule_event(name, rule_event::TAKEN, Some(&label), Some(note.as_str()))
            {
                out.errors.push(format!("rule_events: {}", e));
            }
        }
        let entry = Followed {
            login: main_login.to_string(),
            repo_id: repo.id,
            committed_at: at,
        };
        if followed.get(name) != Some(&entry) {
            followed.insert(name.to_string(), entry);
            dirty = true;
        }
    }
    if dirty {
        if let Err(e) = write_raw(db, &followed) {
            out.errors.push(format!("{}: {}", RULES_FROM_MAIN_KEY, e));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::shared::test_support::{configure, g, gb, write};
    use std::path::Path;
    use std::process::Command;
    use tempfile::TempDir;

    fn db() -> AppDb {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("t.db");
        std::mem::forget(tmp);
        let db = AppDb::new(path).unwrap();
        db.upsert_template_file("_global", "skill.sdh-alpha.md.tmpl", "ALPHA BODY", false)
            .unwrap();
        db.upsert_template_file("_global", "skill.sdh-beta.md.tmpl", "BETA BODY", false)
            .unwrap();
        db
    }

    /// A clone on branch `dev` holding `files` under `docs/sdh_skills/`,
    /// committed on `date` and published: `origin/dev` points at the commit.
    fn owner_clone(files: &[(&str, &str)], date: &str) -> TempDir {
        let tmp = TempDir::new().unwrap();
        g(tmp.path(), &["init", "-q"]);
        g(tmp.path(), &["symbolic-ref", "HEAD", "refs/heads/dev"]);
        configure(tmp.path());
        for (name, text) in files {
            write(tmp.path(), &format!("docs/sdh_skills/{}", name), text);
        }
        g(tmp.path(), &["add", "."]);
        let out = Command::new("git")
            .args(["commit", "-q", "-m", "rules"])
            .env("GIT_COMMITTER_DATE", date)
            .env("GIT_AUTHOR_DATE", date)
            .current_dir(tmp.path())
            .output()
            .unwrap();
        assert!(out.status.success(), "{:?}", out);
        publish(tmp.path());
        tmp
    }

    /// What a fetch of the owner's pushed branch does to the clone.
    fn publish(dir: &Path) {
        g(dir, &["update-ref", "refs/remotes/origin/dev", "HEAD"]);
    }

    fn contributor_repo(db: &AppDb, dir: &TempDir, name: &str) -> Repository {
        let repo = db
            .insert_local_repository(&dir.path().to_string_lossy(), name, None, None)
            .unwrap();
        db.set_shared_role(repo.id, Some("contributor")).unwrap();
        db.get_repository(repo.id).unwrap()
    }

    fn canon(db: &AppDb, rule: &str) -> (String, bool) {
        let f = db
            .get_template_file("_global", &format!("skill.{}.md.tmpl", rule))
            .unwrap()
            .unwrap();
        (f.content, f.is_custom)
    }

    fn adopt(db: &AppDb, repo: &Repository, dir: &TempDir) -> Adopted {
        adopt_main_canon(db, &gb(), repo, dir.path(), "owner", "dev").unwrap()
    }

    #[test]
    fn a_differing_copy_becomes_the_canon_an_equal_one_only_locks_an_unknown_one_nothing() {
        let db = db();
        let dir = owner_clone(
            &[
                ("sdh-alpha.md", "ALPHA OF THE OWNER"),
                ("sdh-beta.md", "BETA BODY"),
                ("sdh-gamma.md", "A RULE THIS INSTALL DOES NOT HAVE"),
                ("sdh-alpha.md.approved", "marker"),
                ("notes.txt", "x"),
            ],
            "2026-01-01T00:00:00",
        );
        let repo = contributor_repo(&db, &dir, "owner/app");

        let done = adopt(&db, &repo, &dir);

        assert_eq!(done.changed, vec!["sdh-alpha"]);
        assert!(done.errors.is_empty(), "{:?}", done.errors);
        assert_eq!(canon(&db, "sdh-alpha"), ("ALPHA OF THE OWNER".into(), true));
        assert_eq!(canon(&db, "sdh-beta"), ("BETA BODY".into(), false));
        assert!(db
            .get_template_file("_global", "skill.sdh-gamma.md.tmpl")
            .unwrap()
            .is_none());
        let origin = db.latest_origin_event("sdh-alpha").unwrap().unwrap();
        assert_eq!(origin.kind, rule_event::TAKEN);
        assert_eq!(
            origin.repo.as_deref(),
            Some(rule_copy_label(&repo).as_str())
        );
        assert_eq!(origin.note.as_deref(), Some("canon of main owner"));
        assert!(db.latest_origin_event("sdh-beta").unwrap().is_none());
        assert_eq!(follows_main(&db, "sdh-alpha").as_deref(), Some("owner"));
        // An equal text is not stored again, but the rule is the owner's all the same.
        assert_eq!(follows_main(&db, "sdh-beta").as_deref(), Some("owner"));
        assert_eq!(follows_main(&db, "sdh-gamma"), None);

        // Nothing changed since: nothing is taken, nothing recorded twice.
        assert!(adopt(&db, &repo, &dir).changed.is_empty());
        assert_eq!(db.list_rule_events("sdh-alpha").unwrap().len(), 1);
    }

    #[test]
    fn a_new_commit_of_the_owner_moves_the_canon_again() {
        let db = db();
        let dir = owner_clone(&[("sdh-alpha.md", "V1")], "2026-01-01T00:00:00");
        let repo = contributor_repo(&db, &dir, "owner/app");
        adopt(&db, &repo, &dir);
        write(dir.path(), "docs/sdh_skills/sdh-alpha.md", "V2");
        g(dir.path(), &["commit", "-qam", "again"]);
        publish(dir.path());

        assert_eq!(adopt(&db, &repo, &dir).changed, vec!["sdh-alpha"]);
        assert_eq!(canon(&db, "sdh-alpha").0, "V2");
        assert_eq!(db.list_rule_events("sdh-alpha").unwrap().len(), 2);
    }

    #[test]
    fn with_two_shared_repos_the_newest_copy_wins_and_stays() {
        let db = db();
        let old = owner_clone(
            &[("sdh-alpha.md", "FROM THE OLD ONE")],
            "2026-01-01T00:00:00",
        );
        let new = owner_clone(
            &[("sdh-alpha.md", "FROM THE NEW ONE")],
            "2026-06-01T00:00:00",
        );
        let old_repo = contributor_repo(&db, &old, "o1/app");
        let new_repo = contributor_repo(&db, &new, "o2/app");

        adopt(&db, &old_repo, &old);
        assert_eq!(canon(&db, "sdh-alpha").0, "FROM THE OLD ONE");
        adopt(&db, &new_repo, &new);
        assert_eq!(canon(&db, "sdh-alpha").0, "FROM THE NEW ONE");
        // The older repo does not take it back on its next pass.
        assert!(adopt(&db, &old_repo, &old).changed.is_empty());
        assert_eq!(canon(&db, "sdh-alpha").0, "FROM THE NEW ONE");
        assert_eq!(followed_rules(&db)["sdh-alpha"].repo_id, new_repo.id);
    }

    #[test]
    fn a_rule_is_unlocked_when_its_repo_stops_being_shared_and_keeps_its_text() {
        let db = db();
        let dir = owner_clone(
            &[("sdh-alpha.md", "ALPHA OF THE OWNER")],
            "2026-01-01T00:00:00",
        );
        let repo = contributor_repo(&db, &dir, "owner/app");
        adopt(&db, &repo, &dir);
        assert!(refuse_if_followed(&db, "sdh-alpha")
            .unwrap_err()
            .starts_with(ERR_FOLLOWS_MAIN));

        db.set_shared_role(repo.id, None).unwrap();

        assert_eq!(follows_main(&db, "sdh-alpha"), None);
        assert!(followed_rules(&db).is_empty());
        assert!(db.get_setting(RULES_FROM_MAIN_KEY).unwrap().is_none());
        assert!(refuse_if_followed(&db, "sdh-alpha").is_ok());
        assert_eq!(canon(&db, "sdh-alpha"), ("ALPHA OF THE OWNER".into(), true));
    }

    #[test]
    fn a_rule_stays_locked_through_the_other_repo_when_one_of_two_stops_sharing() {
        let db = db();
        let a = owner_clone(&[("sdh-alpha.md", "ALPHA A")], "2026-01-01T00:00:00");
        let b = owner_clone(&[("sdh-alpha.md", "ALPHA B")], "2026-06-01T00:00:00");
        let ra = contributor_repo(&db, &a, "o1/app");
        let rb = contributor_repo(&db, &b, "o2/app");
        adopt(&db, &ra, &a);
        adopt(&db, &rb, &b);
        // The repo that fed the rule leaves: the lock goes with it, and the
        // other repo takes the rule over at its next pass.
        db.set_shared_role(rb.id, None).unwrap();
        assert_eq!(follows_main(&db, "sdh-alpha"), None);
        assert_eq!(adopt(&db, &ra, &a).changed, vec!["sdh-alpha"]);
        assert_eq!(canon(&db, "sdh-alpha").0, "ALPHA A");
        assert_eq!(followed_rules(&db)["sdh-alpha"].repo_id, ra.id);
    }

    #[test]
    fn a_clone_without_rule_copies_or_commits_adopts_nothing() {
        let db = db();
        let dir = TempDir::new().unwrap();
        g(dir.path(), &["init", "-q"]);
        let repo = contributor_repo(&db, &dir, "owner/app");
        assert!(adopt(&db, &repo, &dir).changed.is_empty());
        assert!(followed_rules(&db).is_empty());
    }

    #[test]
    fn only_the_owners_published_branch_counts_not_the_checked_out_one() {
        let db = db();
        let dir = owner_clone(&[("sdh-alpha.md", "PUBLISHED")], "2026-01-01T00:00:00");
        let repo = contributor_repo(&db, &dir, "owner/app");
        // The contributor's own branch carries another text and another rule.
        write(dir.path(), "docs/sdh_skills/sdh-alpha.md", "SNEAKY LOCAL");
        write(dir.path(), "docs/sdh_skills/sdh-beta.md", "SNEAKY BETA");
        g(dir.path(), &["add", "."]);
        g(dir.path(), &["commit", "-qm", "local"]);

        let done = adopt(&db, &repo, &dir);

        assert_eq!(done.changed, vec!["sdh-alpha"]);
        assert_eq!(canon(&db, "sdh-alpha").0, "PUBLISHED");
        assert_eq!(canon(&db, "sdh-beta"), ("BETA BODY".into(), false));
        assert_eq!(follows_main(&db, "sdh-beta"), None);
    }

    #[test]
    fn without_the_owners_ref_nothing_is_adopted() {
        let db = db();
        let dir = owner_clone(&[("sdh-alpha.md", "LOCAL ONLY")], "2026-01-01T00:00:00");
        g(dir.path(), &["update-ref", "-d", "refs/remotes/origin/dev"]);
        let repo = contributor_repo(&db, &dir, "owner/app");

        assert!(adopt(&db, &repo, &dir).changed.is_empty());
        assert_eq!(canon(&db, "sdh-alpha").0, "ALPHA BODY");
        assert!(followed_rules(&db).is_empty());
    }

    #[test]
    fn a_rule_the_owner_shares_unchanged_is_locked_and_relocked_after_a_role_flap() {
        let db = db();
        let dir = owner_clone(&[("sdh-alpha.md", "ALPHA BODY")], "2026-01-01T00:00:00");
        let repo = contributor_repo(&db, &dir, "owner/app");
        let done = adopt(&db, &repo, &dir);
        assert!(done.changed.is_empty());
        assert_eq!(follows_main(&db, "sdh-alpha").as_deref(), Some("owner"));
        assert!(db.list_rule_events("sdh-alpha").unwrap().is_empty());

        // Sharing flaps off and on: the lock goes with the role ...
        db.set_shared_role(repo.id, None).unwrap();
        assert_eq!(follows_main(&db, "sdh-alpha"), None);
        db.set_shared_role(repo.id, Some("contributor")).unwrap();
        assert_eq!(follows_main(&db, "sdh-alpha"), None);
        // ... and the next pass puts it back, with nothing else to record.
        assert!(adopt(&db, &repo, &dir).changed.is_empty());
        assert_eq!(follows_main(&db, "sdh-alpha").as_deref(), Some("owner"));
        assert!(db.list_rule_events("sdh-alpha").unwrap().is_empty());
    }
}
