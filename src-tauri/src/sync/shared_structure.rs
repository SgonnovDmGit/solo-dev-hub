// F-000044 / T-000219: model and pure logic of the shared-project structure
// file used by contributor mode.
//
// The owner's install ("main") publishes `.sdh/project.json` in every shared
// repo; a helper's install ("contributor") only reads it. A helper that is not
// approved yet writes a join request `.sdh/requests/<login>.json`. This module
// owns the file formats, role derivation, number-band allocation and the
// contributor-side discovery / local application. It does no committing or
// pushing (T-000220) and no UI (T-000223).
//
// Logins are compared ASCII case-insensitively everywhere.

#![allow(dead_code)]

use crate::db::AppDb;
use crate::git_ops::{spawn_cmd, GitBinary};
use crate::git_sync;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const STRUCTURE_FORMAT: u32 = 1;
pub const STRUCTURE_REL_PATH: &str = ".sdh/project.json";
pub const REQUESTS_REL_DIR: &str = ".sdh/requests";

/// First participant band starts here; each participant gets `BAND_SIZE` ids.
pub const FIRST_PARTICIPANT_BAND: u32 = 500_000;
pub const BAND_SIZE: u32 = 100_000;
pub const MAX_PARTICIPANTS: u32 = 5;
/// Highest id main may use when nobody has a band yet.
const MAIN_BAND_CEILING: u32 = FIRST_PARTICIPANT_BAND - 1;

/// Working-tree marker returned by `discover_structure` as the source.
pub const SOURCE_WORKTREE: &str = "worktree";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StructProject {
    pub name: String,
    /// Written as `type` in the file (spec R1).
    #[serde(rename = "type")]
    pub project_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StructMain {
    pub login: String,
    pub dev_branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StructRepo {
    pub github: String,
    pub role: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Participant {
    pub login: String,
    pub branch: String,
    pub band_start: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProjectStructure {
    pub format: u32,
    pub project: StructProject,
    pub main: StructMain,
    pub repos: Vec<StructRepo>,
    /// Names of connected microservice projects.
    pub microservices: Vec<String>,
    pub participants: Vec<Participant>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct JoinRequest {
    pub login: String,
    pub branch: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Main,
    Contributor,
    Pending,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Main => "main",
            Role::Contributor => "contributor",
            Role::Pending => "pending",
        }
    }
}

// ---------------------------------------------------------------------------
// Logins and ownership
// ---------------------------------------------------------------------------

/// GitHub login charset and length: `[A-Za-z0-9-]`, 1-39 chars.
pub fn is_valid_login(login: &str) -> bool {
    !login.is_empty()
        && login.len() <= 39
        && login
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

pub fn logins_equal(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// True when the owner segment of `github_name` (`owner/repo`) is `login`.
pub fn repo_owner_is(github_name: &str, login: &str) -> bool {
    match github_name.split_once('/') {
        Some((owner, _)) => !owner.is_empty() && logins_equal(owner, login),
        None => false,
    }
}

// ---------------------------------------------------------------------------
// Structure file
// ---------------------------------------------------------------------------

fn structure_path(repo_root: &Path) -> PathBuf {
    repo_root.join(".sdh").join("project.json")
}

fn parse_structure(bytes: &[u8]) -> Result<ProjectStructure, String> {
    let s: ProjectStructure = serde_json::from_slice(bytes)
        .map_err(|e| format!("{}: invalid JSON: {}", STRUCTURE_REL_PATH, e))?;
    if s.format != STRUCTURE_FORMAT {
        return Err(format!(
            "{}: unsupported format {} (this build reads {})",
            STRUCTURE_REL_PATH, s.format, STRUCTURE_FORMAT
        ));
    }
    Ok(s)
}

/// `None` when the file does not exist; `Err` when it exists but is unusable.
pub fn read_structure_file(repo_root: &Path) -> Result<Option<ProjectStructure>, String> {
    let path = structure_path(repo_root);
    match fs::read(&path) {
        Ok(bytes) => parse_structure(&bytes).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {}", path.display(), e)),
    }
}

/// Deterministic pretty JSON with a trailing newline. Repos are sorted by
/// github name, microservices by name (deduplicated), participants by band.
pub fn serialize_structure(s: &ProjectStructure) -> String {
    let mut s = s.clone();
    s.repos.sort_by(|a, b| {
        a.github
            .to_ascii_lowercase()
            .cmp(&b.github.to_ascii_lowercase())
            .then_with(|| a.github.cmp(&b.github))
    });
    s.microservices.sort();
    s.microservices.dedup();
    s.participants.sort_by(|a, b| {
        a.band_start
            .cmp(&b.band_start)
            .then_with(|| a.login.cmp(&b.login))
    });
    let mut out = serde_json::to_string_pretty(&s).expect("structure serializes");
    out.push('\n');
    out
}

fn write_if_changed(path: &Path, content: &str) -> Result<bool, String> {
    if let Ok(existing) = fs::read(path) {
        if existing == content.as_bytes() {
            return Ok(false);
        }
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {}", dir.display(), e))?;
    }
    fs::write(path, content).map_err(|e| format!("{}: {}", path.display(), e))?;
    Ok(true)
}

/// `Ok(true)` when the file was written, `Ok(false)` when it was already identical.
pub fn write_structure_if_changed(repo_root: &Path, s: &ProjectStructure) -> Result<bool, String> {
    write_if_changed(&structure_path(repo_root), &serialize_structure(s))
}

/// Main side: build the structure of the project a shared repo belongs to.
/// Participants are carried over from `existing` unchanged.
pub fn build_structure(
    db: &AppDb,
    repo_id: i64,
    main_login: &str,
    existing: Option<&ProjectStructure>,
) -> Result<ProjectStructure, String> {
    if !is_valid_login(main_login) {
        return Err(format!("invalid GitHub login '{}'", main_login));
    }
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let project_id = repo
        .project_id
        .ok_or_else(|| "shared mode needs the repository to belong to a project".to_string())?;
    let project = db.get_project(project_id).map_err(|e| e.to_string())?;
    let dev_branch = db
        .get_autocommit_branch(repo_id)
        .map_err(|e| e.to_string())?
        .map(|b| b.trim().to_string())
        .filter(|b| !b.is_empty())
        .ok_or_else(|| {
            "shared mode needs a dev branch: set the auto-commit branch of this repository"
                .to_string()
        })?;

    let repos = db
        .list_repos_by_project(Some(project_id))
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter_map(|r| {
            let github = r.github_name?;
            let role = r.role?;
            if github.trim().is_empty() || role.trim().is_empty() {
                return None;
            }
            Some(StructRepo { github, role })
        })
        .collect();

    let mut microservices = Vec::new();
    for ms_id in db
        .list_project_microservices(project_id)
        .map_err(|e| e.to_string())?
    {
        microservices.push(db.get_project(ms_id).map_err(|e| e.to_string())?.name);
    }

    Ok(ProjectStructure {
        format: STRUCTURE_FORMAT,
        project: StructProject {
            name: project.name,
            project_type: project.project_type,
        },
        main: StructMain {
            login: main_login.to_string(),
            dev_branch,
        },
        repos,
        microservices,
        participants: existing.map(|e| e.participants.clone()).unwrap_or_default(),
    })
}

// ---------------------------------------------------------------------------
// Roles and bands
// ---------------------------------------------------------------------------

pub fn derive_role(structure: Option<&ProjectStructure>, my_login: Option<&str>) -> Option<Role> {
    let s = structure?;
    let login = my_login.map(str::trim).filter(|l| !l.is_empty())?;
    if logins_equal(&s.main.login, login) {
        Some(Role::Main)
    } else if s.participants.iter().any(|p| logins_equal(&p.login, login)) {
        Some(Role::Contributor)
    } else {
        Some(Role::Pending)
    }
}

/// Inclusive id range of `login`: main owns everything below the lowest
/// participant band, a participant owns `BAND_SIZE` ids from its start.
pub fn band_for(structure: &ProjectStructure, login: &str) -> Option<(u32, u32)> {
    if logins_equal(&structure.main.login, login) {
        let top = structure
            .participants
            .iter()
            .map(|p| p.band_start)
            .min()
            .map(|b| b - 1)
            .unwrap_or(MAIN_BAND_CEILING);
        return Some((1, top));
    }
    structure
        .participants
        .iter()
        .find(|p| logins_equal(&p.login, login))
        .map(|p| (p.band_start, p.band_start + BAND_SIZE - 1))
}

// ---------------------------------------------------------------------------
// Join requests
// ---------------------------------------------------------------------------

fn request_path(repo_root: &Path, login: &str) -> Result<PathBuf, String> {
    if !is_valid_login(login) {
        return Err(format!("invalid GitHub login '{}'", login));
    }
    Ok(repo_root
        .join(".sdh")
        .join("requests")
        .join(format!("{}.json", login)))
}

pub fn serialize_request(r: &JoinRequest) -> String {
    let mut out = serde_json::to_string_pretty(r).expect("request serializes");
    out.push('\n');
    out
}

/// Pending requests of the working tree, sorted by login. Skips unparsable
/// files, files whose name is not a valid login or does not match the login
/// inside, and requests of the main owner or of an existing participant.
pub fn list_requests(repo_root: &Path) -> Vec<JoinRequest> {
    let structure = read_structure_file(repo_root).ok().flatten();
    let dir = repo_root.join(".sdh").join("requests");
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<JoinRequest> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(stem) = name.strip_suffix(".json") else {
            continue;
        };
        if !is_valid_login(stem) || !path.is_file() {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else { continue };
        let Ok(req) = serde_json::from_slice::<JoinRequest>(&bytes) else {
            continue;
        };
        if !logins_equal(&req.login, stem) || req.branch.trim().is_empty() {
            continue;
        }
        if let Some(s) = &structure {
            if logins_equal(&s.main.login, &req.login)
                || s.participants
                    .iter()
                    .any(|p| logins_equal(&p.login, &req.login))
            {
                continue;
            }
        }
        out.push(req);
    }
    out.sort_by_key(|r| r.login.to_ascii_lowercase());
    out
}

/// `Ok(true)` when written, `Ok(false)` when already identical.
pub fn write_request_if_changed(repo_root: &Path, req: &JoinRequest) -> Result<bool, String> {
    if req.branch.trim().is_empty() {
        return Err("a join request needs a branch".to_string());
    }
    let path = request_path(repo_root, &req.login)?;
    write_if_changed(&path, &serialize_request(req))
}

/// Main side: the structure with `req` added as a participant in the lowest
/// free band. Removing the request file is the caller's job.
pub fn approve_request(
    structure: &ProjectStructure,
    req: &JoinRequest,
) -> Result<ProjectStructure, String> {
    if !is_valid_login(&req.login) {
        return Err(format!("invalid GitHub login '{}'", req.login));
    }
    if req.branch.trim().is_empty() {
        return Err("a join request needs a branch".to_string());
    }
    if logins_equal(&structure.main.login, &req.login)
        || structure
            .participants
            .iter()
            .any(|p| logins_equal(&p.login, &req.login))
    {
        return Err(format!("{} is already part of this project", req.login));
    }
    let k = (0..MAX_PARTICIPANTS)
        .find(|k| {
            let start = FIRST_PARTICIPANT_BAND + k * BAND_SIZE;
            !structure.participants.iter().any(|p| p.band_start == start)
        })
        .ok_or_else(|| "all five participant bands are taken".to_string())?;
    let mut next = structure.clone();
    next.participants.push(Participant {
        login: req.login.clone(),
        branch: req.branch.clone(),
        band_start: FIRST_PARTICIPANT_BAND + k * BAND_SIZE,
    });
    next.participants.sort_by_key(|p| p.band_start);
    Ok(next)
}

// ---------------------------------------------------------------------------
// Contributor-side discovery
// ---------------------------------------------------------------------------

fn commit_time(git: &GitBinary, repo_root: &Path, rev: &str) -> Result<i64, String> {
    let out = spawn_cmd(&git.0)
        .args(["log", "-1", "--format=%ct", rev])
        .current_dir(repo_root)
        .output()
        .map_err(|e| format!("git log failed to start: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "git log failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<i64>()
        .map_err(|e| format!("git log: bad commit time: {}", e))
}

fn list_origin_refs(git: &GitBinary, repo_root: &Path) -> Result<Vec<String>, String> {
    let out = spawn_cmd(&git.0)
        .args(["for-each-ref", "--format=%(refname)", "refs/remotes/origin"])
        .current_dir(repo_root)
        .output()
        .map_err(|e| format!("git for-each-ref failed to start: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "git for-each-ref failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let mut refs: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && l != "refs/remotes/origin/HEAD")
        .collect();
    refs.sort();
    Ok(refs)
}

/// Find the structure of a repo owned by someone else.
///
/// Meant for a contributor's repos only (the caller checks
/// `!repo_owner_is(github_name, my_login)`) and only after a fetch: a
/// not-yet-approved helper has no file in the working tree, so the owner's
/// published copy is looked up in the remote-tracking refs. A ref qualifies
/// when its short name equals the `main.dev_branch` written in its own file
/// (that is main's copy, not someone's stale or edited copy); of several, the
/// ref with the newest commit wins. Structures naming `my_login` as main are
/// ignored on refs: the owner reads its own working tree.
///
/// Returns the structure and its source: `SOURCE_WORKTREE` or e.g. `origin/dev`.
pub fn discover_structure(
    git: &GitBinary,
    repo_root: &Path,
    my_login: &str,
) -> Result<Option<(ProjectStructure, String)>, String> {
    if let Some(s) = read_structure_file(repo_root)? {
        return Ok(Some((s, SOURCE_WORKTREE.to_string())));
    }
    let mut best: Option<(i64, ProjectStructure, String)> = None;
    for full in list_origin_refs(git, repo_root)? {
        let Some(branch) = full.strip_prefix("refs/remotes/origin/") else {
            continue;
        };
        let bytes = match git_sync::show_file(git, repo_root, &full, STRUCTURE_REL_PATH) {
            Ok(Some(b)) => b,
            _ => continue,
        };
        let Ok(s) = parse_structure(&bytes) else {
            continue;
        };
        if !logins_equal(&s.main.dev_branch, branch) || logins_equal(&s.main.login, my_login) {
            continue;
        }
        let Ok(ts) = commit_time(git, repo_root, &full) else {
            continue;
        };
        // Strictly newer wins; ties keep the first ref in name order.
        if best.as_ref().map_or(true, |(t, _, _)| ts > *t) {
            best = Some((ts, s, format!("origin/{}", branch)));
        }
    }
    Ok(best.map(|(_, s, r)| (s, r)))
}

// ---------------------------------------------------------------------------
// Contributor-side local application
// ---------------------------------------------------------------------------

/// Make the local DB reflect the structure: find or create the project by name,
/// assign this repo and every other local repo named in the structure with the
/// structure's role, connect microservice projects that exist locally.
/// Never deletes anything; a second call with the same input changes nothing.
pub fn apply_structure_locally(
    db: &AppDb,
    repo_id: i64,
    s: &ProjectStructure,
) -> Result<(), String> {
    let projects = db.list_projects().map_err(|e| e.to_string())?;
    let project = match projects
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(&s.project.name))
    {
        Some(p) => p.clone(),
        None => db
            .create_project(&s.project.name, None, &s.project.project_type)
            .map_err(|e| e.to_string())?,
    };

    for repo in db.list_all_repos().map_err(|e| e.to_string())? {
        let listed_role = repo.github_name.as_deref().and_then(|github| {
            s.repos
                .iter()
                .find(|r| r.github.eq_ignore_ascii_case(github))
                .map(|r| r.role.clone())
        });
        match listed_role {
            Some(role) => assign_if_needed(db, &repo, project.id, Some(role))?,
            // This repo carries the file, so it belongs to the project even if
            // the file does not list it; keep whatever role it has.
            None if repo.id == repo_id => {
                assign_if_needed(db, &repo, project.id, repo.role.clone())?
            }
            None => {}
        }
    }

    for name in &s.microservices {
        let ms = projects
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name) && p.project_type == "microservice");
        if let Some(ms) = ms {
            if ms.id != project.id {
                db.connect_microservice(project.id, ms.id)?;
            }
        }
    }
    Ok(())
}

fn assign_if_needed(
    db: &AppDb,
    repo: &crate::models::Repository,
    project_id: i64,
    role: Option<String>,
) -> Result<(), String> {
    if repo.project_id == Some(project_id) && repo.role == role {
        return Ok(());
    }
    db.assign_repository(repo.id, Some(project_id), role.as_deref())
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    fn make_db() -> AppDb {
        AppDb::new(PathBuf::from(":memory:")).unwrap()
    }

    fn gb() -> GitBinary {
        GitBinary(PathBuf::from("git"))
    }

    fn g_env(dir: &Path, args: &[&str], date: Option<&str>) -> String {
        let mut c = Command::new("git");
        c.args(args).current_dir(dir);
        if let Some(d) = date {
            c.env("GIT_COMMITTER_DATE", d).env("GIT_AUTHOR_DATE", d);
        }
        let out = c.output().expect("git invocation failed");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn g(dir: &Path, args: &[&str]) -> String {
        g_env(dir, args, None)
    }

    fn configure(dir: &Path) {
        g(dir, &["config", "user.email", "t@example.com"]);
        g(dir, &["config", "user.name", "T"]);
        g(dir, &["config", "core.autocrlf", "false"]);
        g(dir, &["config", "commit.gpgsign", "false"]);
    }

    fn sample() -> ProjectStructure {
        ProjectStructure {
            format: 1,
            project: StructProject {
                name: "Acme".into(),
                project_type: "standard".into(),
            },
            main: StructMain {
                login: "Owner".into(),
                dev_branch: "dev".into(),
            },
            repos: vec![
                StructRepo {
                    github: "Owner/web".into(),
                    role: "client".into(),
                },
                StructRepo {
                    github: "Owner/api".into(),
                    role: "server".into(),
                },
            ],
            microservices: vec!["mailer".into()],
            participants: vec![],
        }
    }

    fn req(login: &str) -> JoinRequest {
        JoinRequest {
            login: login.into(),
            branch: format!("{}-dev", login),
        }
    }

    fn participant(login: &str, band_start: u32) -> Participant {
        Participant {
            login: login.into(),
            branch: format!("{}-dev", login),
            band_start,
        }
    }

    // ----- ownership / logins ------------------------------------------

    #[test]
    fn repo_owner_is_compares_owner_segment_case_insensitively() {
        assert!(repo_owner_is("Owner/web", "owner"));
        assert!(repo_owner_is("owner/web", "OWNER"));
        assert!(!repo_owner_is("other/web", "owner"));
        assert!(!repo_owner_is("owner-x/web", "owner"));
        assert!(!repo_owner_is("web", "web"));
        assert!(!repo_owner_is("/web", ""));
    }

    #[test]
    fn login_validation() {
        assert!(is_valid_login("a"));
        assert!(is_valid_login("Octo-Cat-1"));
        assert!(is_valid_login(&"a".repeat(39)));
        assert!(!is_valid_login(&"a".repeat(40)));
        assert!(!is_valid_login(""));
        assert!(!is_valid_login("a/b"));
        assert!(!is_valid_login("..\\x"));
        assert!(!is_valid_login("a b"));
        assert!(!is_valid_login("a_b"));
        assert!(!is_valid_login("\u{43f}"));
    }

    // ----- serialization -------------------------------------------------

    #[test]
    fn serialization_round_trips_and_is_deterministic() {
        let tmp = TempDir::new().unwrap();
        let mut s = sample();
        s.participants = vec![participant("b", 600_000), participant("a", 500_000)];
        let text = serialize_structure(&s);
        assert!(text.ends_with("}\n"));
        assert!(text.contains("\"type\": \"standard\""));
        // sorted: api before web, band 500000 before 600000
        assert!(text.find("Owner/api").unwrap() < text.find("Owner/web").unwrap());
        assert!(text.find("\"a\"").unwrap() < text.find("\"b\"").unwrap());

        // order of input does not change the bytes
        let mut shuffled = s.clone();
        shuffled.repos.reverse();
        shuffled.participants.reverse();
        assert_eq!(text, serialize_structure(&shuffled));

        assert!(write_structure_if_changed(tmp.path(), &s).unwrap());
        let back = read_structure_file(tmp.path()).unwrap().unwrap();
        assert_eq!(serialize_structure(&back), text);
        assert_eq!(back.project.project_type, "standard");
    }

    #[test]
    fn write_if_changed_is_a_noop_for_identical_bytes() {
        let tmp = TempDir::new().unwrap();
        let s = sample();
        assert!(write_structure_if_changed(tmp.path(), &s).unwrap());
        let p = tmp.path().join(".sdh/project.json");
        let before = fs::metadata(&p).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(!write_structure_if_changed(tmp.path(), &s).unwrap());
        assert_eq!(before, fs::metadata(&p).unwrap().modified().unwrap());
        let mut changed = s;
        changed.microservices.push("x".into());
        assert!(write_structure_if_changed(tmp.path(), &changed).unwrap());
    }

    #[test]
    fn read_structure_file_absent_garbage_and_wrong_format() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(read_structure_file(tmp.path()).unwrap(), None);
        fs::create_dir_all(tmp.path().join(".sdh")).unwrap();
        fs::write(tmp.path().join(".sdh/project.json"), "not json").unwrap();
        assert!(read_structure_file(tmp.path()).is_err());
        let mut s = sample();
        s.format = 2;
        fs::write(
            tmp.path().join(".sdh/project.json"),
            serde_json::to_string(&s).unwrap(),
        )
        .unwrap();
        assert!(read_structure_file(tmp.path())
            .unwrap_err()
            .contains("unsupported format"));
    }

    // ----- build_structure -------------------------------------------------

    fn local_repo(db: &AppDb, name: &str) -> i64 {
        db.upsert_repository(name, None, None, None, None, None)
            .unwrap();
        db.get_repository_by_name(name).unwrap().id
    }

    fn seeded(db: &AppDb) -> (i64, i64) {
        let project = db.create_project("Acme", None, "standard").unwrap();
        let api = local_repo(db, "Owner/api");
        let web = local_repo(db, "Owner/web");
        db.assign_repository(api, Some(project.id), Some("server"))
            .unwrap();
        db.assign_repository(web, Some(project.id), Some("client"))
            .unwrap();
        (api, project.id)
    }

    #[test]
    fn build_structure_collects_project_repos_and_microservices() {
        let db = make_db();
        let (api_id, pid) = seeded(&db);
        db.set_autocommit_branch(api_id, Some("dev")).unwrap();
        // a repo without a role is left out
        let misc = local_repo(&db, "Owner/misc");
        db.assign_repository(misc, Some(pid), None).unwrap();
        let ms = db.create_project("mailer", None, "microservice").unwrap();
        db.connect_microservice(pid, ms.id).unwrap();

        let mut existing = sample();
        existing.participants.push(participant("helper", 500_000));
        let s = build_structure(&db, api_id, "Owner", Some(&existing)).unwrap();
        assert_eq!(s.format, 1);
        assert_eq!(s.project.name, "Acme");
        assert_eq!(s.project.project_type, "standard");
        assert_eq!(s.main.login, "Owner");
        assert_eq!(s.main.dev_branch, "dev");
        let mut names: Vec<_> = s.repos.iter().map(|r| r.github.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["Owner/api", "Owner/web"]);
        assert_eq!(s.microservices, vec!["mailer".to_string()]);
        assert_eq!(s.participants, existing.participants);

        let fresh = build_structure(&db, api_id, "Owner", None).unwrap();
        assert!(fresh.participants.is_empty());
    }

    #[test]
    fn build_structure_errors() {
        let db = make_db();
        let (api_id, _) = seeded(&db);
        // no dev branch
        assert!(build_structure(&db, api_id, "Owner", None)
            .unwrap_err()
            .contains("dev branch"));
        db.set_autocommit_branch(api_id, Some("dev")).unwrap();
        // invalid login
        assert!(build_structure(&db, api_id, "a/b", None).is_err());
        // no project
        let solo = local_repo(&db, "Owner/solo");
        db.set_autocommit_branch(solo, Some("dev")).unwrap();
        assert!(build_structure(&db, solo, "Owner", None)
            .unwrap_err()
            .contains("project"));
    }

    // ----- roles and bands -------------------------------------------------

    #[test]
    fn derive_role_all_branches() {
        let mut s = sample();
        s.participants.push(participant("Helper", 500_000));
        assert_eq!(derive_role(None, Some("owner")), None);
        assert_eq!(derive_role(Some(&s), None), None);
        assert_eq!(derive_role(Some(&s), Some("  ")), None);
        assert_eq!(derive_role(Some(&s), Some("OWNER")), Some(Role::Main));
        assert_eq!(
            derive_role(Some(&s), Some("helper")),
            Some(Role::Contributor)
        );
        assert_eq!(derive_role(Some(&s), Some("stranger")), Some(Role::Pending));
        assert_eq!(Role::Main.as_str(), "main");
        assert_eq!(Role::Contributor.as_str(), "contributor");
        assert_eq!(Role::Pending.as_str(), "pending");
    }

    #[test]
    fn band_for_main_and_participants() {
        let mut s = sample();
        assert_eq!(band_for(&s, "owner"), Some((1, 499_999)));
        assert_eq!(band_for(&s, "nobody"), None);
        s.participants = vec![participant("b", 600_000), participant("a", 500_000)];
        assert_eq!(band_for(&s, "OWNER"), Some((1, 499_999)));
        assert_eq!(band_for(&s, "A"), Some((500_000, 599_999)));
        assert_eq!(band_for(&s, "b"), Some((600_000, 699_999)));
        // main's ceiling follows the lowest band
        s.participants.remove(1);
        assert_eq!(band_for(&s, "owner"), Some((1, 599_999)));
    }

    // ----- approve ----------------------------------------------------------

    #[test]
    fn approve_request_assigns_lowest_free_band() {
        let s = sample();
        let s1 = approve_request(&s, &req("one")).unwrap();
        assert_eq!(s1.participants[0].band_start, 500_000);
        let s2 = approve_request(&s1, &req("two")).unwrap();
        assert_eq!(s2.participants[1].band_start, 600_000);
        assert_eq!(s2.participants[1].branch, "two-dev");
        // a freed band is reused
        let mut gap = s2.clone();
        gap.participants.remove(0);
        let s3 = approve_request(&gap, &req("three")).unwrap();
        assert!(s3
            .participants
            .iter()
            .any(|p| p.login == "three" && p.band_start == 500_000));
        // the original is untouched
        assert!(s.participants.is_empty());
    }

    #[test]
    fn approve_request_sixth_and_duplicate_are_errors() {
        let mut s = sample();
        for i in 0..5 {
            s = approve_request(&s, &req(&format!("user{}", i))).unwrap();
        }
        assert_eq!(s.participants.len(), 5);
        assert_eq!(s.participants[4].band_start, 900_000);
        assert_eq!(
            approve_request(&s, &req("sixth")).unwrap_err(),
            "all five participant bands are taken"
        );
        let dup = approve_request(&s, &req("USER1")).unwrap_err();
        assert!(dup.contains("already"));
        assert!(approve_request(&s, &req("Owner"))
            .unwrap_err()
            .contains("already"));
        assert!(approve_request(&sample(), &req("bad/login")).is_err());
    }

    // ----- requests ----------------------------------------------------------

    #[test]
    fn requests_write_list_and_filter() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        assert!(list_requests(root).is_empty());

        assert!(write_request_if_changed(root, &req("zed")).unwrap());
        assert!(!write_request_if_changed(root, &req("zed")).unwrap());
        assert!(write_request_if_changed(root, &req("amy")).unwrap());
        assert!(write_request_if_changed(root, &req("../evil")).is_err());
        assert!(write_request_if_changed(
            root,
            &JoinRequest {
                login: "bob".into(),
                branch: " ".into()
            }
        )
        .is_err());

        let dir = root.join(".sdh/requests");
        fs::write(dir.join("broken.json"), "{").unwrap();
        fs::write(
            dir.join("bad name.json"),
            serde_json::to_string(&req("bad")).unwrap(),
        )
        .unwrap();
        fs::write(dir.join("notes.txt"), "x").unwrap();
        // file name and content disagree
        fs::write(
            dir.join("liar.json"),
            serde_json::to_string(&req("someone")).unwrap(),
        )
        .unwrap();

        let logins: Vec<_> = list_requests(root).into_iter().map(|r| r.login).collect();
        assert_eq!(logins, vec!["amy", "zed"]);

        // main and existing participants are filtered out
        let mut s = sample();
        s.participants.push(participant("AMY", 500_000));
        write_structure_if_changed(root, &s).unwrap();
        write_request_if_changed(root, &req("owner")).unwrap();
        let logins: Vec<_> = list_requests(root).into_iter().map(|r| r.login).collect();
        assert_eq!(logins, vec!["zed"]);
    }

    // ----- discover ------------------------------------------------------------

    /// Bare remote plus an owner clone `a` that can publish branches.
    struct Fx {
        tmp: TempDir,
        remote: PathBuf,
        a: PathBuf,
    }

    fn fx() -> Fx {
        let tmp = TempDir::new().unwrap();
        let remote = tmp.path().join("remote.git");
        fs::create_dir(&remote).unwrap();
        g(&remote, &["init", "--bare", "-q"]);
        let a = tmp.path().join("a");
        fs::create_dir(&a).unwrap();
        g(&a, &["init", "-q"]);
        g(&a, &["symbolic-ref", "HEAD", "refs/heads/dev"]);
        configure(&a);
        g(&a, &["remote", "add", "origin", remote.to_str().unwrap()]);
        fs::write(a.join("README.md"), "hi\n").unwrap();
        g(&a, &["add", "."]);
        g(&a, &["commit", "-q", "-m", "init"]);
        Fx { tmp, remote, a }
    }

    /// Commit `.sdh/project.json` (naming `dev_branch`) onto branch `name`,
    /// forked from the init commit, at commit time `date`, and push it.
    fn publish(fx: &Fx, name: &str, dev_branch: &str, date: &str) {
        g(&fx.a, &["checkout", "-q", "-B", name, "HEAD"]);
        let mut s = sample();
        s.main.dev_branch = dev_branch.to_string();
        write_structure_if_changed(&fx.a, &s).unwrap();
        g(&fx.a, &["add", ".sdh"]);
        g_env(&fx.a, &["commit", "-q", "-m", "structure"], Some(date));
        g(&fx.a, &["push", "-q", "origin", name]);
        // back to a structure-free base for the next branch
        let base = g(&fx.a, &["rev-list", "--max-parents=0", "HEAD"]);
        g(&fx.a, &["checkout", "-q", "--detach", &base]);
    }

    /// A contributor clone that fetched everything but checked nothing out.
    fn contributor(fx: &Fx) -> PathBuf {
        let c = fx.tmp.path().join("c");
        fs::create_dir(&c).unwrap();
        g(&c, &["init", "-q"]);
        configure(&c);
        g(
            &c,
            &["remote", "add", "origin", fx.remote.to_str().unwrap()],
        );
        g(&c, &["fetch", "-q", "origin"]);
        c
    }

    #[test]
    fn discover_prefers_working_tree() {
        let fx = fx();
        write_structure_if_changed(&fx.a, &sample()).unwrap();
        let (s, src) = discover_structure(&gb(), &fx.a, "helper").unwrap().unwrap();
        assert_eq!(src, SOURCE_WORKTREE);
        assert_eq!(s.project.name, "Acme");
    }

    #[test]
    fn discover_none_without_structure() {
        let fx = fx();
        g(&fx.a, &["push", "-q", "origin", "dev"]);
        let c = contributor(&fx);
        assert!(discover_structure(&gb(), &c, "helper").unwrap().is_none());
    }

    #[test]
    fn discover_reads_remote_ref_matching_dev_branch() {
        let fx = fx();
        publish(&fx, "dev", "dev", "2026-01-01T00:00:00Z");
        let c = contributor(&fx);
        let (s, src) = discover_structure(&gb(), &c, "helper").unwrap().unwrap();
        assert_eq!(src, "origin/dev");
        assert_eq!(s.main.dev_branch, "dev");
    }

    #[test]
    fn discover_ignores_ref_that_is_not_the_published_dev_branch() {
        let fx = fx();
        // someone's branch carrying a copy that names another dev branch
        publish(&fx, "helper-branch", "dev", "2026-01-01T00:00:00Z");
        let c = contributor(&fx);
        assert!(discover_structure(&gb(), &c, "helper").unwrap().is_none());
    }

    #[test]
    fn discover_chooses_newest_of_several_matches() {
        let fx = fx();
        publish(&fx, "dev", "DEV", "2026-01-01T00:00:00Z");
        publish(&fx, "next", "next", "2026-03-01T00:00:00Z");
        publish(&fx, "old", "old", "2025-01-01T00:00:00Z");
        let c = contributor(&fx);
        let (s, src) = discover_structure(&gb(), &c, "helper").unwrap().unwrap();
        assert_eq!(src, "origin/next");
        assert_eq!(s.main.dev_branch, "next");
    }

    #[test]
    fn discover_skips_refs_of_the_install_own_main() {
        let fx = fx();
        publish(&fx, "dev", "dev", "2026-01-01T00:00:00Z");
        let c = contributor(&fx);
        assert!(discover_structure(&gb(), &c, "OWNER").unwrap().is_none());
    }

    // ----- apply locally -----------------------------------------------------

    #[test]
    fn apply_creates_project_assigns_roles_and_is_idempotent() {
        let db = make_db();
        let web = local_repo(&db, "owner/web"); // case differs from the file
        let api = local_repo(&db, "Owner/api");
        let other = local_repo(&db, "Owner/unrelated");
        let ms = db.create_project("Mailer", None, "microservice").unwrap();

        let mut s = sample();
        s.microservices = vec!["mailer".into(), "does-not-exist".into()];
        apply_structure_locally(&db, web, &s).unwrap();

        let projects = db.list_projects().unwrap();
        let acme = projects.iter().find(|p| p.name == "Acme").unwrap();
        assert_eq!(acme.project_type, "standard");
        let w = db.get_repository(web).unwrap();
        let a = db.get_repository(api).unwrap();
        assert_eq!(
            (w.project_id, w.role.as_deref()),
            (Some(acme.id), Some("client"))
        );
        assert_eq!(
            (a.project_id, a.role.as_deref()),
            (Some(acme.id), Some("server"))
        );
        assert_eq!(db.get_repository(other).unwrap().project_id, None);
        assert_eq!(db.list_project_microservices(acme.id).unwrap(), vec![ms.id]);

        // second call changes nothing
        let snapshot = |db: &AppDb| {
            (
                db.list_projects().unwrap().len(),
                db.list_all_repos()
                    .unwrap()
                    .into_iter()
                    .map(|r| (r.id, r.project_id, r.role, r.updated_at))
                    .collect::<Vec<_>>(),
                db.list_project_microservices(acme.id).unwrap(),
            )
        };
        let before = snapshot(&db);
        apply_structure_locally(&db, web, &s).unwrap();
        assert_eq!(before, snapshot(&db));
    }

    #[test]
    fn apply_reuses_existing_project_by_name_case_insensitively() {
        let db = make_db();
        let web = local_repo(&db, "Owner/web");
        let existing = db.create_project("ACME", None, "standard").unwrap();
        apply_structure_locally(&db, web, &sample()).unwrap();
        assert_eq!(db.list_projects().unwrap().len(), 1);
        assert_eq!(
            db.get_repository(web).unwrap().project_id,
            Some(existing.id)
        );
    }
}
