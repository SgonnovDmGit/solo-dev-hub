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
use std::collections::BTreeMap;
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

/// A login that may be used as a request file name: a valid login that is not
/// a Windows reserved device name (`con`, `nul`, `com1`, ...).
pub fn is_valid_request_login(login: &str) -> bool {
    if !is_valid_login(login) {
        return false;
    }
    let l = login.to_ascii_lowercase();
    let reserved = matches!(l.as_str(), "con" | "prn" | "aux" | "nul")
        || ((l.starts_with("com") || l.starts_with("lpt"))
            && l.len() == 4
            && matches!(l.as_bytes()[3], b'1'..=b'9'));
    !reserved
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
// Validation of untrusted input
// ---------------------------------------------------------------------------

/// Largest structure file / blob that is read.
pub const MAX_STRUCTURE_BYTES: u64 = 256 * 1024;
/// Largest request file that is read.
const MAX_REQUEST_BYTES: u64 = 16 * 1024;
/// At most this many request files are looked at.
pub const MAX_REQUEST_FILES: usize = 50;
/// At most this many request-named files are opened to find those 50.
const MAX_REQUEST_INSPECTED: usize = 500;

/// The repositories.role CHECK set (db/migrations.rs).
const REPO_ROLES: &[&str] = &[
    "server",
    "client",
    "test_client",
    "admin_client",
    "microservice",
    "landing",
    "tool",
    "other",
];
/// The project types `update_project_type` accepts.
const PROJECT_TYPES: &[&str] = &["standard", "microservice", "codex"];

/// A branch name safe to hand to git and to use as a ref: non-empty, trimmed,
/// no leading `-` / `/` / `.` per component, no whitespace, control or
/// ref-special characters, no `..`, `//`, `@{`, not ending in `.lock`, `/` or
/// `.`, not `HEAD`, and never `master` / `main`.
pub fn is_valid_branch_name(name: &str) -> bool {
    if name.is_empty() || name != name.trim() || name.len() > 200 {
        return false;
    }
    if name.starts_with('-') || name.starts_with('/') || name.ends_with('/') || name.ends_with('.')
    {
        return false;
    }
    if name.ends_with(".lock") || name == "@" || name.eq_ignore_ascii_case("HEAD") {
        return false;
    }
    if name.contains("..") || name.contains("//") || name.contains("@{") {
        return false;
    }
    if name
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c))
    {
        return false;
    }
    if name
        .split('/')
        .any(|part| part.starts_with('.') || part.ends_with(".lock"))
    {
        return false;
    }
    !git_sync::is_protected_branch(name)
}

fn is_valid_repo_full_name(github: &str) -> bool {
    let Some((owner, name)) = github.split_once('/') else {
        return false;
    };
    is_valid_login(owner)
        && !name.is_empty()
        && name.len() <= 100
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

fn is_valid_free_text(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 200 && !s.chars().any(|c| c.is_control())
}

fn is_valid_band_start(b: u32) -> bool {
    b >= FIRST_PARTICIPANT_BAND
        && b % BAND_SIZE == 0
        && (b - FIRST_PARTICIPANT_BAND) / BAND_SIZE < MAX_PARTICIPANTS
}

/// Trust check for a structure read from a working tree or a ref. The
/// structure is accepted only if its `main.login` is the owner of the repo it
/// was found in, and every field is well-formed and in range.
pub fn validate_structure(s: &ProjectStructure, github_name: &str) -> Result<(), String> {
    if s.format != STRUCTURE_FORMAT {
        return Err(format!("unsupported format {}", s.format));
    }
    if !is_valid_login(&s.main.login) {
        return Err("main.login is not a valid GitHub login".to_string());
    }
    if !repo_owner_is(github_name, &s.main.login) {
        return Err(format!(
            "main is not the repo owner (main.login '{}', repo '{}')",
            s.main.login, github_name
        ));
    }
    if !is_valid_branch_name(&s.main.dev_branch) {
        return Err("main.dev_branch is not a usable branch name".to_string());
    }
    if !is_valid_free_text(&s.project.name) {
        return Err("project.name is empty or malformed".to_string());
    }
    if !PROJECT_TYPES.contains(&s.project.project_type.as_str()) {
        return Err(format!("unknown project type '{}'", s.project.project_type));
    }
    if s.repos.len() > 200 || s.microservices.len() > 200 {
        return Err("too many repos or microservices".to_string());
    }
    for r in &s.repos {
        if !is_valid_repo_full_name(&r.github) {
            return Err(format!("repo '{}' is not owner/name", r.github));
        }
        if !REPO_ROLES.contains(&r.role.as_str()) {
            return Err(format!("unknown repo role '{}'", r.role));
        }
    }
    if s.microservices.iter().any(|m| !is_valid_free_text(m)) {
        return Err("a microservice name is empty or malformed".to_string());
    }
    if s.participants.len() > MAX_PARTICIPANTS as usize {
        return Err("too many participants".to_string());
    }
    for (i, p) in s.participants.iter().enumerate() {
        if !is_valid_login(&p.login) {
            return Err(format!("participant login '{}' is invalid", p.login));
        }
        if logins_equal(&p.login, &s.main.login) {
            return Err("main is listed as a participant".to_string());
        }
        if !is_valid_branch_name(&p.branch) {
            return Err(format!("participant '{}' has an unusable branch", p.login));
        }
        if !is_valid_band_start(p.band_start) {
            return Err(format!(
                "participant '{}' has an invalid band {}",
                p.login, p.band_start
            ));
        }
        for q in &s.participants[..i] {
            if logins_equal(&q.login, &p.login) {
                return Err(format!("participant '{}' is listed twice", p.login));
            }
            if q.band_start == p.band_start {
                return Err(format!("band {} is used twice", p.band_start));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

fn structure_path(repo_root: &Path) -> PathBuf {
    repo_root.join(".sdh").join("project.json")
}

/// Refuse a path that is a symlink (an existing path only).
fn ensure_not_symlink(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => Err(format!(
            "{} is a symlink, refusing to use it",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {}", path.display(), e)),
    }
}

fn ensure_sdh_dir_safe(repo_root: &Path) -> Result<(), String> {
    ensure_not_symlink(&repo_root.join(".sdh"))
}

fn parse_structure(bytes: &[u8]) -> Result<ProjectStructure, String> {
    serde_json::from_slice(bytes)
        .map_err(|e| format!("{}: invalid JSON: {}", STRUCTURE_REL_PATH, e))
}

/// Unvalidated read of the working-tree file: `None` when absent.
fn read_structure_raw(repo_root: &Path) -> Result<Option<ProjectStructure>, String> {
    ensure_sdh_dir_safe(repo_root)?;
    let path = structure_path(repo_root);
    ensure_not_symlink(&path)?;
    let meta = match fs::metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {}", path.display(), e)),
    };
    if meta.len() > MAX_STRUCTURE_BYTES {
        return Err(format!(
            "{} is too large ({} bytes)",
            STRUCTURE_REL_PATH,
            meta.len()
        ));
    }
    let bytes = fs::read(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
    parse_structure(&bytes).map(Some)
}

/// The working-tree structure of the repo `github_name`, validated: `None` when
/// the file does not exist, `Err` when it exists but is unusable or untrusted.
pub fn read_structure_file(
    repo_root: &Path,
    github_name: &str,
) -> Result<Option<ProjectStructure>, String> {
    match read_structure_raw(repo_root)? {
        Some(s) => {
            validate_structure(&s, github_name)?;
            Ok(Some(s))
        }
        None => Ok(None),
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

/// Remove temp files left by a crashed write: `.<name>.json.<pid>.tmp`, the
/// exact pattern `write_if_changed` uses. Regular files only, nothing else.
fn remove_stale_temps(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Some(core) = name.strip_prefix('.').and_then(|n| n.strip_suffix(".tmp")) else {
            continue;
        };
        let Some((stem, pid)) = core.rsplit_once('.') else {
            continue;
        };
        if !stem.ends_with(".json") || pid.is_empty() || !pid.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        if matches!(e.file_type(), Ok(t) if t.is_file()) {
            let _ = fs::remove_file(e.path());
        }
    }
}

/// Clear crashed-write leftovers in `.sdh` and `.sdh/requests` (neither may be a
/// symlink; callers have checked).
fn clean_sdh_temps(repo_root: &Path) {
    remove_stale_temps(&repo_root.join(".sdh"));
    remove_stale_temps(&requests_dir(repo_root));
}

/// Write `content` unless the file already holds it (line endings compared
/// after CRLF -> LF, so an autocrlf checkout does not churn). The write goes
/// through a temp file and a rename, so a reader never sees a partial file.
fn write_if_changed(path: &Path, content: &str) -> Result<bool, String> {
    ensure_not_symlink(path)?;
    if let Ok(existing) = fs::read(path) {
        let norm = String::from_utf8_lossy(&existing).replace("\r\n", "\n");
        if norm == content {
            return Ok(false);
        }
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {}", dir.display(), e))?;
        remove_stale_temps(dir);
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{}.{}.tmp", name, std::process::id()));
    fs::write(&tmp, content).map_err(|e| format!("{}: {}", tmp.display(), e))?;
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("{}: {}", path.display(), e));
    }
    Ok(true)
}

/// `Ok(true)` when the file was written, `Ok(false)` when it was already identical.
pub fn write_structure_if_changed(repo_root: &Path, s: &ProjectStructure) -> Result<bool, String> {
    ensure_sdh_dir_safe(repo_root)?;
    ensure_not_symlink(&requests_dir(repo_root))?;
    clean_sdh_temps(repo_root);
    write_if_changed(&structure_path(repo_root), &serialize_structure(s))
}

/// Main side: build the structure of the project a shared repo belongs to.
/// The repo must be owned by `main_login`. Participants are carried over from
/// `existing` unchanged. The result is validated before it is returned.
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
    let github_name = repo
        .github_name
        .clone()
        .ok_or_else(|| "shared mode needs a GitHub repository".to_string())?;
    if !repo_owner_is(&github_name, main_login) {
        return Err(format!(
            "shared mode can only be switched on by the repo owner ({} is not {})",
            main_login, github_name
        ));
    }
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
            if !is_valid_repo_full_name(&github) || role.trim().is_empty() {
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

    let built = ProjectStructure {
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
    };
    validate_structure(&built, &github_name)?;
    Ok(built)
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
/// Arithmetic saturates, so a malformed structure cannot overflow.
pub fn band_for(structure: &ProjectStructure, login: &str) -> Option<(u32, u32)> {
    if logins_equal(&structure.main.login, login) {
        let top = structure
            .participants
            .iter()
            .map(|p| p.band_start)
            .min()
            .map(|b| b.saturating_sub(1))
            .unwrap_or(MAIN_BAND_CEILING);
        return Some((1, top));
    }
    structure
        .participants
        .iter()
        .find(|p| logins_equal(&p.login, login))
        .map(|p| (p.band_start, p.band_start.saturating_add(BAND_SIZE - 1)))
}

// ---------------------------------------------------------------------------
// Bug number bands (Р12)
// ---------------------------------------------------------------------------

/// Which numbers this install may give to a new bug of a repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BandDecision {
    /// Not a shared repo: the plain per-repo counter.
    Unbanded,
    /// Inclusive id range of this install.
    Band(i64, i64),
    /// Creating a bug is not allowed now; the text says why.
    Refused(String),
}

fn band_of(structure: &ProjectStructure, login: &str) -> BandDecision {
    match band_for(structure, login) {
        Some((lo, hi)) => BandDecision::Band(lo as i64, hi as i64),
        None => BandDecision::Refused("this install has no number range in the project".into()),
    }
}

/// Pure decision. `structure` is the outcome of `read_structure_file`;
/// `shared_mode` / `shared_role` are the cached columns of the repo.
pub fn decide_band(
    structure: Result<Option<ProjectStructure>, String>,
    shared_mode: bool,
    shared_role: Option<&str>,
    my_login: Option<&str>,
) -> BandDecision {
    let structure = match structure {
        Ok(s) => s,
        Err(reason) => {
            return BandDecision::Refused(format!(
                "the shared project structure is not trusted: {}",
                reason
            ))
        }
    };
    let Some(s) = structure else {
        // No structure file in the working tree.
        if !shared_mode && shared_role.is_none() {
            return BandDecision::Unbanded;
        }
        return match shared_role {
            // The owner switched sharing on but the file is not written yet.
            None | Some("main") => BandDecision::Band(1, MAIN_BAND_CEILING as i64),
            Some("pending") => BandDecision::Refused(PENDING_REASON.into()),
            Some(_) => BandDecision::Refused(
                "the shared project structure is not available yet - sync the project first".into(),
            ),
        };
    };
    match derive_role(Some(&s), my_login) {
        None => BandDecision::Refused(UNKNOWN_LOGIN_REASON.into()),
        Some(Role::Pending) => BandDecision::Refused(PENDING_REASON.into()),
        Some(Role::Main | Role::Contributor) => band_of(&s, my_login.unwrap_or_default().trim()),
    }
}

const PENDING_REASON: &str = "waiting for the owner to approve your join request";
const UNKNOWN_LOGIN_REASON: &str = "GitHub login of this install is unknown - save the token again";

/// The decision for a repo of the local DB: reads the shared state, the
/// working-tree structure file and this install's login.
pub fn bug_band_for_repo(db: &AppDb, repo_id: i64) -> BandDecision {
    let state = match db.get_shared_state(repo_id) {
        Ok(s) => s,
        Err(e) => return BandDecision::Refused(e.to_string()),
    };
    let repo = match db.get_repository(repo_id) {
        Ok(r) => r,
        Err(e) => return BandDecision::Refused(e.to_string()),
    };
    let structure = match (repo.local_path.as_deref(), repo.github_name.as_deref()) {
        (Some(path), Some(name)) if !path.trim().is_empty() => {
            read_structure_file(Path::new(path), name)
        }
        _ => Ok(None),
    };
    decide_band(
        structure,
        state.shared_mode,
        state.shared_role.as_deref(),
        db.github_login().as_deref(),
    )
}

/// Allocate the number of a new bug of `repo_id`: the plain counter for a repo
/// that is not shared, the next free id of this install's band otherwise.
pub fn allocate_bug_numeric_id(db: &AppDb, repo_id: i64) -> Result<i64, String> {
    match bug_band_for_repo(db, repo_id) {
        BandDecision::Unbanded => db.next_numeric_id(repo_id).map_err(|e| e.to_string()),
        BandDecision::Band(lo, hi) => db
            .next_numeric_id_in_band(repo_id, lo, hi)
            .map_err(|e| e.to_string())?
            .map_err(|full| full.to_string()),
        BandDecision::Refused(reason) => Err(reason),
    }
}

// ---------------------------------------------------------------------------
// Join requests
// ---------------------------------------------------------------------------

fn requests_dir(repo_root: &Path) -> PathBuf {
    repo_root.join(".sdh").join("requests")
}

fn request_path(repo_root: &Path, login: &str) -> Result<PathBuf, String> {
    if !is_valid_request_login(login) {
        return Err(format!("invalid GitHub login '{}'", login));
    }
    Ok(requests_dir(repo_root).join(format!("{}.json", login)))
}

pub fn serialize_request(r: &JoinRequest) -> String {
    let mut out = serde_json::to_string_pretty(r).expect("request serializes");
    out.push('\n');
    out
}

/// Pending requests of the working tree of the repo `github_name`, sorted by
/// login. Looks at no more than `MAX_REQUEST_FILES` files (by file name).
/// Skips unparsable, oversized or symlinked files, names that are not usable
/// logins, a name that does not match the login inside, an unusable branch,
/// later duplicates of a login (case-insensitive), and requests of the main
/// owner or of an existing participant (when the structure file is valid).
pub fn list_requests(repo_root: &Path, github_name: &str) -> Vec<JoinRequest> {
    if ensure_sdh_dir_safe(repo_root).is_err() {
        return Vec::new();
    }
    let dir = requests_dir(repo_root);
    if ensure_not_symlink(&dir).is_err() {
        return Vec::new();
    }
    let structure = read_structure_file(repo_root, github_name).ok().flatten();
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    // Names that cannot be a request are dropped first, so junk files cannot
    // push real requests out of the window; the number of files actually
    // opened is bounded too.
    let mut names: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.strip_suffix(".json").is_some_and(is_valid_request_login))
        .collect();
    names.sort();
    names.truncate(MAX_REQUEST_INSPECTED);

    let mut out: Vec<JoinRequest> = Vec::new();
    for name in names {
        if out.len() >= MAX_REQUEST_FILES {
            break;
        }
        let Some(stem) = name.strip_suffix(".json") else {
            continue;
        };
        let path = dir.join(&name);
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.file_type().is_file() || meta.len() > MAX_REQUEST_BYTES {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else { continue };
        let Ok(mut req) = serde_json::from_slice::<JoinRequest>(&bytes) else {
            continue;
        };
        req.branch = req.branch.trim().to_string();
        if !logins_equal(&req.login, stem) || !is_valid_branch_name(&req.branch) {
            continue;
        }
        if out.iter().any(|r| logins_equal(&r.login, &req.login)) {
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

/// `Ok(true)` when written, `Ok(false)` when already identical. The branch is
/// trimmed and must be a usable branch name.
pub fn write_request_if_changed(repo_root: &Path, req: &JoinRequest) -> Result<bool, String> {
    let branch = req.branch.trim();
    if !is_valid_branch_name(branch) {
        return Err(format!("'{}' is not a usable branch name", branch));
    }
    let path = request_path(repo_root, &req.login)?;
    ensure_sdh_dir_safe(repo_root)?;
    ensure_not_symlink(&requests_dir(repo_root))?;
    clean_sdh_temps(repo_root);
    let clean = JoinRequest {
        login: req.login.clone(),
        branch: branch.to_string(),
    };
    write_if_changed(&path, &serialize_request(&clean))
}

/// Main side: the structure with `req` added as a participant in the lowest
/// free band. Removing the request file is the caller's job.
pub fn approve_request(
    structure: &ProjectStructure,
    req: &JoinRequest,
) -> Result<ProjectStructure, String> {
    if !is_valid_request_login(&req.login) {
        return Err(format!("invalid GitHub login '{}'", req.login));
    }
    let branch = req.branch.trim();
    if !is_valid_branch_name(branch) {
        return Err(format!("'{}' is not a usable branch name", branch));
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
        branch: branch.to_string(),
        band_start: FIRST_PARTICIPANT_BAND + k * BAND_SIZE,
    });
    next.participants.sort_by_key(|p| p.band_start);
    Ok(next)
}

// ---------------------------------------------------------------------------
// Contributor-side discovery
// ---------------------------------------------------------------------------

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

/// Result of `discover_structure`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoverOutcome {
    /// A trusted structure and where it came from: `SOURCE_WORKTREE` or e.g.
    /// `origin/dev`.
    Found(ProjectStructure, String),
    /// No structure file anywhere.
    NotFound,
    /// A structure was found but failed validation; the text says why (e.g.
    /// "main is not the repo owner ..."). Nothing from it may be used.
    Untrusted(String),
}

/// What this install remembers about the shared files of a repo, stored as JSON
/// in `repositories.shared_import`. `pinned_dev_branch` is the trust-on-first-use
/// pin of the owner's dev branch; `blobs` is reserved for the last-imported blob
/// shas (T-000221) and stays empty for now. Parsing is tolerant.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedImport {
    #[serde(default)]
    pub pinned_dev_branch: Option<String>,
    #[serde(default)]
    pub blobs: BTreeMap<String, String>,
}

/// Tolerant parse: empty or invalid JSON (or a different shape) -> default.
pub fn parse_shared_import(json: Option<&str>) -> SharedImport {
    json.and_then(|j| serde_json::from_str::<SharedImport>(j).ok())
        .unwrap_or_default()
}

pub fn read_shared_import(db: &AppDb, repo_id: i64) -> SharedImport {
    parse_shared_import(db.get_shared_import(repo_id).ok().flatten().as_deref())
}

pub fn write_shared_import(db: &AppDb, repo_id: i64, v: &SharedImport) -> Result<(), String> {
    let json = serde_json::to_string(v).map_err(|e| e.to_string())?;
    db.set_shared_import(repo_id, Some(&json))
        .map_err(|e| e.to_string())
}

/// Pin the owner's dev branch once a structure has been accepted (trust on
/// first use): later discovery looks only at `origin/<pin>`.
pub fn pin_after_accept(db: &AppDb, repo_id: i64, s: &ProjectStructure) -> Result<(), String> {
    let mut v = read_shared_import(db, repo_id);
    v.pinned_dev_branch = Some(s.main.dev_branch.clone());
    write_shared_import(db, repo_id, &v)
}

/// Find the trusted structure of the repo `github_name` (`owner/name`).
///
/// Meant for a contributor's repos only (the caller checks the owner is not
/// this install's login) and only after a fetch: a not-yet-approved helper has
/// no file in the working tree, so the owner's published copy is looked up in
/// the remote-tracking refs.
///
/// Trust anchor: a structure is accepted only if `validate_structure` passes,
/// i.e. its `main.login` is the repo's owner.
///
/// * `pinned` is `Some(dev_branch)` once a structure was accepted before (see
///   `pin_after_accept`). Then ONLY `origin/<pinned>` is consulted (and the
///   worktree file, if it names the same dev branch). A structure there whose
///   `dev_branch` differs from the pin is `Untrusted("dev branch changed ...")`.
/// * Unpinned, every remote ref that sits on `origin/<its own main.dev_branch>`
///   and validates is a candidate. One candidate is the answer; two or more
///   are `Untrusted("ambiguous structure: ...")` - none is ever picked.
///
/// A worktree file that fails validation does not stop the search of the refs.
pub fn discover_structure(
    git: &GitBinary,
    repo_root: &Path,
    github_name: &str,
    pinned: Option<&str>,
) -> Result<DiscoverOutcome, String> {
    let pinned = pinned.map(str::trim).filter(|p| !p.is_empty());
    let mut untrusted: Option<String> = None;

    match read_structure_raw(repo_root) {
        Ok(Some(s)) => match validate_structure(&s, github_name) {
            Ok(()) => match pinned {
                Some(pin) if s.main.dev_branch != pin => {
                    untrusted = Some(format!(
                        "{}: dev branch changed from {} to {}",
                        STRUCTURE_REL_PATH, pin, s.main.dev_branch
                    ));
                }
                _ => return Ok(DiscoverOutcome::Found(s, SOURCE_WORKTREE.to_string())),
            },
            Err(reason) => untrusted = Some(format!("{}: {}", STRUCTURE_REL_PATH, reason)),
        },
        Ok(None) => {}
        Err(reason) => untrusted = Some(reason),
    }

    let read_ref = |full: &str| -> Option<ProjectStructure> {
        match git_sync::show_file_capped(
            git,
            repo_root,
            full,
            STRUCTURE_REL_PATH,
            MAX_STRUCTURE_BYTES,
        ) {
            Ok(Some(b)) => parse_structure(&b).ok(),
            _ => None,
        }
    };

    if let Some(pin) = pinned {
        let full = format!("refs/remotes/origin/{}", pin);
        if let Some(s) = read_ref(&full) {
            if s.main.dev_branch != pin {
                untrusted = Some(format!(
                    "origin/{}: dev branch changed from {} to {}",
                    pin, pin, s.main.dev_branch
                ));
            } else {
                match validate_structure(&s, github_name) {
                    Ok(()) => return Ok(DiscoverOutcome::Found(s, format!("origin/{}", pin))),
                    Err(reason) => untrusted = Some(format!("origin/{}: {}", pin, reason)),
                }
            }
        }
        return Ok(match untrusted {
            Some(reason) => DiscoverOutcome::Untrusted(reason),
            None => DiscoverOutcome::NotFound,
        });
    }

    let mut candidates: Vec<(ProjectStructure, String)> = Vec::new();
    for full in list_origin_refs(git, repo_root)? {
        let Some(branch) = full.strip_prefix("refs/remotes/origin/") else {
            continue;
        };
        let Some(s) = read_ref(&full) else { continue };
        if s.main.dev_branch != branch {
            continue;
        }
        match validate_structure(&s, github_name) {
            Ok(()) => candidates.push((s, format!("origin/{}", branch))),
            Err(reason) => {
                untrusted.get_or_insert(format!("origin/{}: {}", branch, reason));
            }
        }
    }
    if candidates.len() > 1 {
        let names: Vec<&str> = candidates.iter().map(|(_, r)| r.as_str()).collect();
        return Ok(DiscoverOutcome::Untrusted(format!(
            "ambiguous structure: {}",
            names.join(" vs ")
        )));
    }
    if let Some((s, r)) = candidates.pop() {
        return Ok(DiscoverOutcome::Found(s, r));
    }
    Ok(match untrusted {
        Some(reason) => DiscoverOutcome::Untrusted(reason),
        None => DiscoverOutcome::NotFound,
    })
}

// ---------------------------------------------------------------------------
// Contributor-side local application
// ---------------------------------------------------------------------------

/// Make the local DB reflect the structure: find or create the project by name,
/// assign this repo and every other local repo named in the structure and owned
/// by `main.login`, connect microservice projects that exist locally.
///
/// The whole structure is validated against the called repo before anything is
/// written. Never deletes anything; a second call with the same input changes
/// nothing. A repo owned by this install's own GitHub login is never moved
/// (and the called repo must not be one). A microservice that cannot be
/// connected (a cycle, a wrong type) is skipped.
pub fn apply_structure_locally(
    db: &AppDb,
    repo_id: i64,
    s: &ProjectStructure,
) -> Result<(), String> {
    let called = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let called_name = called
        .github_name
        .clone()
        .ok_or_else(|| "the repository has no GitHub name".to_string())?;
    validate_structure(s, &called_name)?;
    // Fail closed: without a known login this install cannot tell its own
    // repos from the owner's, so it must not move anything.
    let my_login = db
        .github_login()
        .ok_or_else(|| "the GitHub login of this install is unknown".to_string())?;
    let mine = |github: &str| repo_owner_is(github, &my_login);
    if mine(&called_name) {
        return Err("this install owns the repository: nothing to apply as a contributor".into());
    }

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
        let Some(github) = repo.github_name.as_deref() else {
            continue;
        };
        if !repo_owner_is(github, &s.main.login) || mine(github) {
            continue;
        }
        let listed_role = s
            .repos
            .iter()
            .find(|r| r.github.eq_ignore_ascii_case(github))
            .map(|r| r.role.clone());
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
                // A cycle or a stale type must not abort the rest.
                let _ = db.connect_microservice(project.id, ms.id);
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

    fn make_helper_db() -> AppDb {
        let db = make_db();
        db.set_setting("github_login", "helper").unwrap();
        db
    }

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
        let back = read_structure_file(tmp.path(), "Owner/web")
            .unwrap()
            .unwrap();
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
        assert_eq!(read_structure_file(tmp.path(), "Owner/web").unwrap(), None);
        fs::create_dir_all(tmp.path().join(".sdh")).unwrap();
        fs::write(tmp.path().join(".sdh/project.json"), "not json").unwrap();
        assert!(read_structure_file(tmp.path(), "Owner/web").is_err());
        let mut s = sample();
        s.format = 2;
        fs::write(
            tmp.path().join(".sdh/project.json"),
            serde_json::to_string(&s).unwrap(),
        )
        .unwrap();
        assert!(read_structure_file(tmp.path(), "Owner/web")
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
        assert!(list_requests(root, "Owner/web").is_empty());

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

        let logins: Vec<_> = list_requests(root, "Owner/web")
            .into_iter()
            .map(|r| r.login)
            .collect();
        assert_eq!(logins, vec!["amy", "zed"]);

        // main and existing participants are filtered out
        let mut s = sample();
        s.participants.push(participant("AMY", 500_000));
        write_structure_if_changed(root, &s).unwrap();
        write_request_if_changed(root, &req("owner")).unwrap();
        let logins: Vec<_> = list_requests(root, "Owner/web")
            .into_iter()
            .map(|r| r.login)
            .collect();
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

    /// Commit `st` as `.sdh/project.json` onto branch `name`, forked from the
    /// init commit, at commit time `date`, and push it.
    fn publish_struct(fx: &Fx, name: &str, st: &ProjectStructure, date: &str) {
        g(&fx.a, &["checkout", "-q", "-B", name, "HEAD"]);
        write_structure_if_changed(&fx.a, st).unwrap();
        g(&fx.a, &["add", ".sdh"]);
        g_env(&fx.a, &["commit", "-q", "-m", "structure"], Some(date));
        g(&fx.a, &["push", "-q", "origin", name]);
        // back to a structure-free base for the next branch
        let base = g(&fx.a, &["rev-list", "--max-parents=0", "HEAD"]);
        g(&fx.a, &["checkout", "-q", "--detach", &base]);
    }

    fn publish(fx: &Fx, name: &str, dev_branch: &str, date: &str) {
        let mut st = sample();
        st.main.dev_branch = dev_branch.to_string();
        publish_struct(fx, name, &st, date);
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

    const REPO: &str = "Owner/web";

    fn found(o: DiscoverOutcome) -> (ProjectStructure, String) {
        match o {
            DiscoverOutcome::Found(s, src) => (s, src),
            other => panic!("expected Found, got {:?}", other),
        }
    }

    #[test]
    fn discover_prefers_working_tree() {
        let fx = fx();
        write_structure_if_changed(&fx.a, &sample()).unwrap();
        let (s, src) = found(discover_structure(&gb(), &fx.a, REPO, None).unwrap());
        assert_eq!(src, SOURCE_WORKTREE);
        assert_eq!(s.project.name, "Acme");
    }

    #[test]
    fn discover_none_without_structure() {
        let fx = fx();
        g(&fx.a, &["push", "-q", "origin", "dev"]);
        let c = contributor(&fx);
        assert_eq!(
            discover_structure(&gb(), &c, REPO, None).unwrap(),
            DiscoverOutcome::NotFound
        );
    }

    #[test]
    fn discover_reads_remote_ref_matching_dev_branch() {
        let fx = fx();
        publish(&fx, "dev", "dev", "2026-01-01T00:00:00Z");
        let c = contributor(&fx);
        let (s, src) = found(discover_structure(&gb(), &c, REPO, None).unwrap());
        assert_eq!(src, "origin/dev");
        assert_eq!(s.main.dev_branch, "dev");
    }

    #[test]
    fn discover_ignores_ref_that_is_not_the_published_dev_branch() {
        let fx = fx();
        // someone's branch carrying a copy that names another dev branch
        publish(&fx, "helper-branch", "dev", "2026-01-01T00:00:00Z");
        let c = contributor(&fx);
        assert_eq!(
            discover_structure(&gb(), &c, REPO, None).unwrap(),
            DiscoverOutcome::NotFound
        );
    }

    #[test]
    fn discover_compares_dev_branch_and_ref_name_exactly() {
        let fx = fx();
        publish(&fx, "dev", "DEV", "2026-01-01T00:00:00Z");
        let c = contributor(&fx);
        assert_eq!(
            discover_structure(&gb(), &c, REPO, None).unwrap(),
            DiscoverOutcome::NotFound
        );
    }

    #[test]
    fn discover_rejects_a_forged_self_consistent_structure() {
        let fx = fx();
        // a helper pushes origin/helper claiming to be main of the repo
        let mut forged = sample();
        forged.main = StructMain {
            login: "helper".into(),
            dev_branch: "helper".into(),
        };
        publish_struct(&fx, "helper", &forged, "2026-06-01T00:00:00Z");
        let c = contributor(&fx);
        match discover_structure(&gb(), &c, REPO, None).unwrap() {
            DiscoverOutcome::Untrusted(reason) => {
                assert!(reason.contains("main is not the repo owner"), "{}", reason)
            }
            other => panic!("expected Untrusted, got {:?}", other),
        }

        // the legitimate copy is still found, however old
        publish(&fx, "dev", "dev", "2020-01-01T00:00:00Z");
        g(&c, &["fetch", "-q", "origin"]);
        let (s, src) = found(discover_structure(&gb(), &c, REPO, None).unwrap());
        assert_eq!(src, "origin/dev");
        assert_eq!(s.main.login, "Owner");
    }

    #[test]
    fn discover_untrusted_worktree_file_still_checks_refs() {
        let fx = fx();
        let c = contributor(&fx);
        let mut forged = sample();
        forged.main.login = "helper".into();
        write_structure_if_changed(&c, &forged).unwrap();
        assert!(matches!(
            discover_structure(&gb(), &c, REPO, None).unwrap(),
            DiscoverOutcome::Untrusted(_)
        ));
        publish(&fx, "dev", "dev", "2026-01-01T00:00:00Z");
        g(&c, &["fetch", "-q", "origin"]);
        let (_, src) = found(discover_structure(&gb(), &c, REPO, None).unwrap());
        assert_eq!(src, "origin/dev");
    }

    #[test]
    fn discover_refuses_an_oversized_blob() {
        let fx = fx();
        let mut big = sample();
        big.microservices = vec!["x".repeat(200); 200];
        big.project.name = "y".repeat(200);
        // pad past the cap with many long repo entries
        big.repos = (0..3000)
            .map(|i| StructRepo {
                github: format!("Owner/{}-{}", "r".repeat(90), i),
                role: "tool".into(),
            })
            .collect();
        assert!(serialize_structure(&big).len() as u64 > MAX_STRUCTURE_BYTES);
        publish_struct(&fx, "dev", &big, "2026-01-01T00:00:00Z");
        let c = contributor(&fx);
        assert_eq!(
            discover_structure(&gb(), &c, REPO, None).unwrap(),
            DiscoverOutcome::NotFound
        );
    }

    // ----- apply locally -----------------------------------------------------

    #[test]
    fn apply_creates_project_assigns_roles_and_is_idempotent() {
        let db = make_helper_db();
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
        let db = make_helper_db();
        let web = local_repo(&db, "Owner/web");
        let existing = db.create_project("ACME", None, "standard").unwrap();
        apply_structure_locally(&db, web, &sample()).unwrap();
        assert_eq!(db.list_projects().unwrap().len(), 1);
        assert_eq!(
            db.get_repository(web).unwrap().project_id,
            Some(existing.id)
        );
    }

    // ----- review follow-up (C1, C2, I1, I2, M*) --------------------------------

    #[test]
    fn validate_accepts_the_sample_and_rejects_foreign_main() {
        assert!(validate_structure(&sample(), "Owner/web").is_ok());
        assert!(validate_structure(&sample(), "owner/web").is_ok());
        let err = validate_structure(&sample(), "Other/web").unwrap_err();
        assert!(err.contains("main is not the repo owner"), "{}", err);
        assert!(validate_structure(&sample(), "web").is_err());
    }

    #[test]
    fn validate_rejects_malformed_fields() {
        let check = |f: &dyn Fn(&mut ProjectStructure)| {
            let mut s = sample();
            f(&mut s);
            validate_structure(&s, "Owner/web")
        };
        assert!(check(&|s| s.format = 2).is_err());
        assert!(check(&|s| s.main.login = "a/b".into()).is_err());
        assert!(check(&|s| s.main.dev_branch = "-x".into()).is_err());
        assert!(check(&|s| s.main.dev_branch = "main".into()).is_err());
        assert!(check(&|s| s.project.project_type = "weird".into()).is_err());
        assert!(check(&|s| s.project.name = "  ".into()).is_err());
        assert!(check(&|s| s.repos[0].role = "boss".into()).is_err());
        assert!(check(&|s| s.repos[0].github = "noslash".into()).is_err());
        assert!(check(&|s| s.repos[0].github = "a/../b".into()).is_err());
        assert!(check(&|s| s.microservices.push("".into())).is_err());
        // participants
        for band in [0u32, 1, 499_999, 550_000, 1_000_000, u32::MAX] {
            assert!(
                check(&|s| s.participants.push(participant("h", band))).is_err(),
                "band {}",
                band
            );
        }
        assert!(check(&|s| s.participants.push(participant("h", 500_000))).is_ok());
        assert!(check(&|s| {
            let mut p = participant("h", 500_000);
            p.branch = "bad branch".into();
            s.participants.push(p);
        })
        .is_err());
        assert!(check(&|s| s.participants.push(participant("owner", 500_000))).is_err());
        assert!(check(&|s| {
            s.participants.push(participant("h", 500_000));
            s.participants.push(participant("H", 600_000));
        })
        .is_err());
        assert!(check(&|s| {
            s.participants.push(participant("h", 500_000));
            s.participants.push(participant("i", 500_000));
        })
        .is_err());
        assert!(check(&|s| {
            for i in 0..6 {
                s.participants
                    .push(participant(&format!("u{}", i), 500_000 + i * 100_000));
            }
        })
        .is_err());
    }

    #[test]
    fn read_structure_file_validates_ownership() {
        let tmp = TempDir::new().unwrap();
        write_structure_if_changed(tmp.path(), &sample()).unwrap();
        assert!(read_structure_file(tmp.path(), "Owner/web")
            .unwrap()
            .is_some());
        assert!(read_structure_file(tmp.path(), "Other/web")
            .unwrap_err()
            .contains("main is not the repo owner"));
    }

    #[test]
    fn branch_name_validation() {
        for ok in ["dev", "feature/x", "helper-1", "Dev_2", "a.b"] {
            assert!(is_valid_branch_name(ok), "{}", ok);
        }
        for bad in [
            "",
            " dev",
            "dev ",
            "-x",
            "a b",
            "a..b",
            "x.lock",
            "x/",
            "/x",
            "a//b",
            "HEAD",
            "head",
            "master",
            "Main",
            "a\tb",
            "a~b",
            "a^b",
            "a:b",
            "a?b",
            "a*b",
            "a[b",
            "a@{b",
            "x.",
            "a/.hidden",
            "@",
            "a\nb",
        ] {
            assert!(!is_valid_branch_name(bad), "{:?}", bad);
        }
    }

    #[test]
    fn band_for_saturates_on_malformed_input() {
        let mut s = sample();
        s.participants = vec![participant("a", 0), participant("b", u32::MAX)];
        assert_eq!(band_for(&s, "owner"), Some((1, 0)));
        assert_eq!(band_for(&s, "a"), Some((0, 99_999)));
        assert_eq!(band_for(&s, "b"), Some((u32::MAX, u32::MAX)));
    }

    #[test]
    fn reserved_device_names_are_not_request_logins() {
        for bad in [
            "con", "CON", "prn", "Aux", "nul", "com1", "COM9", "lpt1", "LPT9",
        ] {
            assert!(is_valid_login(bad));
            assert!(!is_valid_request_login(bad), "{}", bad);
        }
        for ok in ["com0", "com10", "console", "lpt", "conn"] {
            assert!(is_valid_request_login(ok), "{}", ok);
        }
        let tmp = TempDir::new().unwrap();
        assert!(write_request_if_changed(tmp.path(), &req("nul")).is_err());
        assert!(approve_request(&sample(), &req("aux")).is_err());
    }

    fn req_branch(login: &str, branch: &str) -> JoinRequest {
        JoinRequest {
            login: login.into(),
            branch: branch.into(),
        }
    }

    #[test]
    fn request_branch_is_trimmed_and_validated() {
        let tmp = TempDir::new().unwrap();
        assert!(write_request_if_changed(tmp.path(), &req_branch("amy", "  amy-dev ")).unwrap());
        assert_eq!(list_requests(tmp.path(), REPO)[0].branch, "amy-dev");
        for bad in ["main", "-x", "a b", "HEAD", ""] {
            let r = req_branch("amy", bad);
            assert!(write_request_if_changed(tmp.path(), &r).is_err(), "{}", bad);
            assert!(approve_request(&sample(), &r).is_err(), "{}", bad);
        }
        let ok = approve_request(&sample(), &req_branch("xavier", " x-dev ")).unwrap();
        assert_eq!(ok.participants[0].branch, "x-dev");
    }

    #[test]
    fn list_requests_caps_files_and_skips_bad_branches_and_big_files() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        for i in 10..70 {
            write_request_if_changed(root, &req(&format!("u{}", i))).unwrap();
        }
        assert_eq!(list_requests(root, REPO).len(), MAX_REQUEST_FILES);
        let dir = root.join(".sdh/requests");
        // u10 becomes unusable (protected branch), u11 oversized
        fs::write(dir.join("u10.json"), r#"{"login":"u10","branch":"main"}"#).unwrap();
        fs::write(
            dir.join("u11.json"),
            format!(r#"{{"login":"u11","branch":"{}"}}"#, "a".repeat(20_000)),
        )
        .unwrap();
        let logins: Vec<_> = list_requests(root, REPO)
            .into_iter()
            .map(|r| r.login)
            .collect();
        assert!(!logins.contains(&"u10".to_string()));
        assert!(!logins.contains(&"u11".to_string()));
        assert!(logins.contains(&"u12".to_string()));
    }

    #[test]
    fn list_requests_dedupes_by_lowercased_login() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join(".sdh/requests");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("Amy.json"), r#"{"login":"Amy","branch":"a1"}"#).unwrap();
        fs::write(dir.join("amy.json"), r#"{"login":"amy","branch":"a2"}"#).unwrap();
        // on a case-insensitive filesystem the second write replaced the first
        assert_eq!(list_requests(tmp.path(), REPO).len(), 1);
    }

    #[test]
    fn write_if_changed_ignores_crlf_and_leaves_no_temp_file() {
        let tmp = TempDir::new().unwrap();
        let s = sample();
        write_structure_if_changed(tmp.path(), &s).unwrap();
        let p = tmp.path().join(".sdh/project.json");
        let lf = fs::read_to_string(&p).unwrap();
        fs::write(&p, lf.replace('\n', "\r\n")).unwrap();
        assert!(!write_structure_if_changed(tmp.path(), &s).unwrap());
        // a real change rewrites the file and leaves only project.json behind
        let mut changed = s;
        changed.microservices.push("zzz".into());
        assert!(write_structure_if_changed(tmp.path(), &changed).unwrap());
        let names: Vec<_> = fs::read_dir(tmp.path().join(".sdh"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["project.json"]);
    }

    #[test]
    fn oversized_structure_file_is_refused() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".sdh")).unwrap();
        fs::write(
            tmp.path().join(".sdh/project.json"),
            vec![b' '; MAX_STRUCTURE_BYTES as usize + 1],
        )
        .unwrap();
        assert!(read_structure_file(tmp.path(), REPO)
            .unwrap_err()
            .contains("too large"));
    }

    fn try_symlink_dir(target: &Path, link: &Path) -> bool {
        #[cfg(windows)]
        let r = std::os::windows::fs::symlink_dir(target, link);
        #[cfg(unix)]
        let r = std::os::unix::fs::symlink(target, link);
        r.is_ok()
    }

    #[test]
    fn symlinked_sdh_dirs_are_refused() {
        let tmp = TempDir::new().unwrap();
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        let root = tmp.path().join("repo");
        fs::create_dir_all(&root).unwrap();
        if !try_symlink_dir(&outside, &root.join(".sdh")) {
            return; // no symlink privilege on this machine
        }
        assert!(write_structure_if_changed(&root, &sample()).is_err());
        assert!(read_structure_file(&root, REPO).is_err());
        assert!(write_request_if_changed(&root, &req("amy")).is_err());
        assert!(list_requests(&root, REPO).is_empty());
        assert!(fs::read_dir(&outside).unwrap().next().is_none());

        let root2 = tmp.path().join("repo2");
        fs::create_dir_all(root2.join(".sdh")).unwrap();
        if !try_symlink_dir(&outside, &root2.join(".sdh/requests")) {
            return;
        }
        assert!(write_request_if_changed(&root2, &req("amy")).is_err());
        assert!(list_requests(&root2, REPO).is_empty());
    }

    #[test]
    fn build_structure_requires_the_owner_to_be_main() {
        let db = make_db();
        let (api_id, _) = seeded(&db);
        db.set_autocommit_branch(api_id, Some("dev")).unwrap();
        let err = build_structure(&db, api_id, "someone-else", None).unwrap_err();
        assert!(err.contains("repo owner"), "{}", err);
        assert!(build_structure(&db, api_id, "owner", None).is_ok());
    }

    #[test]
    fn apply_rejects_an_invalid_structure_before_writing_anything() {
        let db = make_helper_db();
        let web = local_repo(&db, "Owner/web");
        let mut bad = sample();
        bad.repos[0].role = "boss".into();
        assert!(apply_structure_locally(&db, web, &bad).is_err());
        let mut foreign = sample();
        foreign.main.login = "intruder".into();
        assert!(apply_structure_locally(&db, web, &foreign).is_err());
        assert!(db.list_projects().unwrap().is_empty());
        assert_eq!(db.get_repository(web).unwrap().project_id, None);
    }

    #[test]
    fn apply_touches_only_repos_of_the_main_owner() {
        let db = make_helper_db();
        let web = local_repo(&db, "Owner/web");
        let foreign = local_repo(&db, "Stranger/api");
        let mut s = sample();
        s.repos.push(StructRepo {
            github: "Stranger/api".into(),
            role: "server".into(),
        });
        apply_structure_locally(&db, web, &s).unwrap();
        assert!(db.get_repository(web).unwrap().project_id.is_some());
        assert_eq!(db.get_repository(foreign).unwrap().project_id, None);
    }

    #[test]
    fn apply_never_moves_repos_of_this_installs_own_login() {
        let db = make_helper_db();
        db.set_setting("github_login", "Owner").unwrap();
        let web = local_repo(&db, "Owner/web");
        assert!(apply_structure_locally(&db, web, &sample())
            .unwrap_err()
            .contains("owns the repository"));
        assert_eq!(db.get_repository(web).unwrap().project_id, None);
        assert!(db.list_projects().unwrap().is_empty());
    }

    #[test]
    fn apply_skips_a_microservice_that_cannot_be_connected() {
        let db = make_helper_db();
        let web = local_repo(&db, "Owner/web");
        // Acme is itself a microservice that Mailer depends on; connecting
        // Mailer to Acme would close a cycle and must be skipped quietly.
        let acme = db.create_project("Acme", None, "microservice").unwrap();
        let mailer = db.create_project("Mailer", None, "microservice").unwrap();
        let other = db.create_project("Other", None, "microservice").unwrap();
        db.connect_microservice(mailer.id, acme.id).unwrap();
        let mut s = sample();
        s.project.project_type = "microservice".into();
        s.microservices = vec!["Mailer".into(), "Other".into()];
        apply_structure_locally(&db, web, &s).unwrap();
        assert_eq!(
            db.list_project_microservices(acme.id).unwrap(),
            vec![other.id]
        );
    }

    // ----- second review follow-up (N1-N4) -------------------------------------

    fn publish_dev_and_aaa(fx: &Fx) {
        publish(fx, "dev", "dev", "2026-01-01T00:00:00Z");
        // a helper's branch carrying the real owner's login, naming itself
        publish(fx, "aaa", "aaa", "2026-06-01T00:00:00Z");
    }

    #[test]
    fn discover_unpinned_with_two_valid_refs_is_ambiguous() {
        let fx = fx();
        publish_dev_and_aaa(&fx);
        let c = contributor(&fx);
        match discover_structure(&gb(), &c, REPO, None).unwrap() {
            DiscoverOutcome::Untrusted(reason) => {
                assert!(reason.starts_with("ambiguous structure: "), "{}", reason);
                assert!(reason.contains("origin/aaa") && reason.contains("origin/dev"));
            }
            other => panic!("expected Untrusted, got {:?}", other),
        }
    }

    #[test]
    fn discover_pinned_ignores_other_refs() {
        let fx = fx();
        publish_dev_and_aaa(&fx);
        let c = contributor(&fx);
        let (s, src) = found(discover_structure(&gb(), &c, REPO, Some("dev")).unwrap());
        assert_eq!(src, "origin/dev");
        assert_eq!(s.main.dev_branch, "dev");
        // an absent pinned ref is simply not found
        assert_eq!(
            discover_structure(&gb(), &c, REPO, Some("gone")).unwrap(),
            DiscoverOutcome::NotFound
        );
    }

    #[test]
    fn discover_pinned_detects_a_changed_dev_branch() {
        let fx = fx();
        // origin/dev now claims another dev branch
        publish(&fx, "dev", "other", "2026-01-01T00:00:00Z");
        let c = contributor(&fx);
        match discover_structure(&gb(), &c, REPO, Some("dev")).unwrap() {
            DiscoverOutcome::Untrusted(reason) => {
                assert!(
                    reason.contains("dev branch changed from dev to other"),
                    "{}",
                    reason
                )
            }
            other => panic!("expected Untrusted, got {:?}", other),
        }
    }

    #[test]
    fn discover_pinned_worktree_file_must_name_the_pinned_branch() {
        let fx = fx();
        let mut st = sample();
        st.main.dev_branch = "other".into();
        write_structure_if_changed(&fx.a, &st).unwrap();
        assert!(matches!(
            discover_structure(&gb(), &fx.a, REPO, Some("dev")).unwrap(),
            DiscoverOutcome::Untrusted(_)
        ));
        assert!(matches!(
            discover_structure(&gb(), &fx.a, REPO, Some("other")).unwrap(),
            DiscoverOutcome::Found(_, _)
        ));
    }

    #[test]
    fn shared_import_parses_tolerantly_and_round_trips() {
        assert_eq!(parse_shared_import(None), SharedImport::default());
        assert_eq!(parse_shared_import(Some("")), SharedImport::default());
        assert_eq!(
            parse_shared_import(Some("not json")),
            SharedImport::default()
        );
        assert_eq!(parse_shared_import(Some("[1,2]")), SharedImport::default());
        assert_eq!(parse_shared_import(Some("{}")), SharedImport::default());
        let v = parse_shared_import(Some(
            r#"{"pinned_dev_branch":"dev","blobs":{"docs/todo.md":"abc"},"extra":1}"#,
        ));
        assert_eq!(v.pinned_dev_branch.as_deref(), Some("dev"));
        assert_eq!(v.blobs.get("docs/todo.md").map(String::as_str), Some("abc"));
        assert_eq!(
            parse_shared_import(Some(r#"{"blobs":{}}"#)).pinned_dev_branch,
            None
        );

        let db = make_helper_db();
        let id = local_repo(&db, "Owner/web");
        assert_eq!(read_shared_import(&db, id), SharedImport::default());
        pin_after_accept(&db, id, &sample()).unwrap();
        assert_eq!(
            read_shared_import(&db, id).pinned_dev_branch.as_deref(),
            Some("dev")
        );
        // pinning keeps what else is stored
        let mut v = read_shared_import(&db, id);
        v.blobs.insert("a".into(), "b".into());
        write_shared_import(&db, id, &v).unwrap();
        let mut moved = sample();
        moved.main.dev_branch = "next".into();
        pin_after_accept(&db, id, &moved).unwrap();
        let back = read_shared_import(&db, id);
        assert_eq!(back.pinned_dev_branch.as_deref(), Some("next"));
        assert_eq!(back.blobs.len(), 1);
    }

    #[test]
    fn apply_fails_closed_without_a_known_login() {
        let db = make_db();
        let web = local_repo(&db, "Owner/web");
        assert!(apply_structure_locally(&db, web, &sample())
            .unwrap_err()
            .contains("login"));
        assert!(db.list_projects().unwrap().is_empty());
        assert_eq!(db.get_repository(web).unwrap().project_id, None);
    }

    #[test]
    fn junk_files_do_not_push_real_requests_out_of_the_window() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let dir = root.join(".sdh/requests");
        fs::create_dir_all(&dir).unwrap();
        // 60 junk files sorting before the real one, with valid names
        for i in 10..70 {
            fs::write(dir.join(format!("aaa{}.json", i)), "garbage").unwrap();
        }
        // and 60 files whose NAMES are unusable
        for i in 0..60 {
            fs::write(dir.join(format!("bad name {}.json", i)), "x").unwrap();
        }
        write_request_if_changed(root, &req("zed")).unwrap();
        let logins: Vec<_> = list_requests(root, REPO)
            .into_iter()
            .map(|r| r.login)
            .collect();
        assert_eq!(logins, vec!["zed"]);
    }

    #[test]
    fn stale_temp_files_are_removed_on_write_but_nothing_else() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let sdh = root.join(".sdh");
        let reqs = sdh.join("requests");
        fs::create_dir_all(&reqs).unwrap();
        fs::write(sdh.join(".project.json.4242.tmp"), "x").unwrap();
        fs::write(reqs.join(".amy.json.7.tmp"), "x").unwrap();
        // not our pattern: kept
        fs::write(sdh.join(".keep.tmp"), "x").unwrap();
        fs::write(sdh.join(".project.json.abc.tmp"), "x").unwrap();
        fs::write(sdh.join("notes.tmp"), "x").unwrap();
        fs::write(reqs.join(".gitkeep"), "x").unwrap();

        write_structure_if_changed(root, &sample()).unwrap();
        assert!(!sdh.join(".project.json.4242.tmp").exists());
        assert!(!reqs.join(".amy.json.7.tmp").exists());

        fs::write(reqs.join(".bob.json.9.tmp"), "x").unwrap();
        write_request_if_changed(root, &req("amy")).unwrap();
        assert!(!reqs.join(".bob.json.9.tmp").exists());

        for kept in [
            sdh.join(".keep.tmp"),
            sdh.join(".project.json.abc.tmp"),
            sdh.join("notes.tmp"),
            reqs.join(".gitkeep"),
        ] {
            assert!(kept.exists(), "{}", kept.display());
        }
    }

    // ----- bug number bands (R12) -------------------------------------------

    fn ok(s: ProjectStructure) -> Result<Option<ProjectStructure>, String> {
        Ok(Some(s))
    }

    #[test]
    fn decide_band_not_shared_is_unbanded() {
        assert_eq!(
            decide_band(Ok(None), false, None, Some("owner")),
            BandDecision::Unbanded
        );
        assert_eq!(
            decide_band(Ok(None), false, None, None),
            BandDecision::Unbanded
        );
    }

    #[test]
    fn decide_band_shared_mode_without_file_is_main_band() {
        assert_eq!(
            decide_band(Ok(None), true, None, Some("owner")),
            BandDecision::Band(1, 499_999)
        );
        assert_eq!(
            decide_band(Ok(None), true, Some("main"), Some("owner")),
            BandDecision::Band(1, 499_999)
        );
    }

    #[test]
    fn decide_band_cached_role_without_file() {
        assert_eq!(
            decide_band(Ok(None), false, Some("pending"), Some("helper")),
            BandDecision::Refused(PENDING_REASON.into())
        );
        assert!(matches!(
            decide_band(Ok(None), false, Some("contributor"), Some("helper")),
            BandDecision::Refused(r) if r.contains("sync the project")
        ));
    }

    #[test]
    fn decide_band_with_structure_by_role() {
        let mut s = sample();
        s.participants = vec![participant("Helper", 500_000), participant("two", 600_000)];
        assert_eq!(
            decide_band(ok(s.clone()), true, None, Some("OWNER")),
            BandDecision::Band(1, 499_999)
        );
        assert_eq!(
            decide_band(ok(s.clone()), false, None, Some("helper")),
            BandDecision::Band(500_000, 599_999)
        );
        assert_eq!(
            decide_band(ok(s.clone()), false, None, Some(" two ")),
            BandDecision::Band(600_000, 699_999)
        );
        assert_eq!(
            decide_band(ok(s.clone()), false, None, Some("stranger")),
            BandDecision::Refused(PENDING_REASON.into())
        );
        assert_eq!(
            decide_band(ok(s.clone()), false, None, None),
            BandDecision::Refused(UNKNOWN_LOGIN_REASON.into())
        );
        assert_eq!(
            decide_band(ok(s), false, None, Some("  ")),
            BandDecision::Refused(UNKNOWN_LOGIN_REASON.into())
        );
    }

    #[test]
    fn decide_band_untrusted_structure_is_refused_with_the_reason() {
        let d = decide_band(
            Err("main is not the repo owner".into()),
            true,
            None,
            Some("owner"),
        );
        assert!(matches!(d, BandDecision::Refused(r) if r.contains("main is not the repo owner")));
    }

    /// Repo `github_name` in the DB with a working tree at `root`.
    fn repo_at(db: &AppDb, github_name: &str, root: &Path) -> i64 {
        let id = local_repo(db, github_name);
        db.set_repo_local_path(id, Some(root.to_str().unwrap()))
            .unwrap();
        id
    }

    fn seed_numeric(db: &AppDb, repo_id: i64, nid: i64) {
        db.insert_bug(
            repo_id,
            nid,
            "2026-03-29T00:00:00Z",
            "seed",
            "minor",
            "other",
            "created",
            0,
            None,
            None,
        )
        .unwrap();
    }

    #[test]
    fn bug_band_for_repo_branches_with_real_files() {
        let tmp = TempDir::new().unwrap();
        let db = make_db();
        db.set_setting("github_login", "Owner").unwrap();
        let rid = repo_at(&db, "Owner/web", tmp.path());

        // not shared, no file
        assert_eq!(bug_band_for_repo(&db, rid), BandDecision::Unbanded);
        // shared mode on, file not written yet
        db.set_shared_mode(rid, true).unwrap();
        assert_eq!(bug_band_for_repo(&db, rid), BandDecision::Band(1, 499_999));

        // file written, main
        let mut s = sample();
        s.participants.push(participant("helper", 500_000));
        write_structure_if_changed(tmp.path(), &s).unwrap();
        assert_eq!(bug_band_for_repo(&db, rid), BandDecision::Band(1, 499_999));

        // the same install as a contributor
        db.set_setting("github_login", "helper").unwrap();
        assert_eq!(
            bug_band_for_repo(&db, rid),
            BandDecision::Band(500_000, 599_999)
        );
        // a login that is not in the structure
        db.set_setting("github_login", "stranger").unwrap();
        assert_eq!(
            bug_band_for_repo(&db, rid),
            BandDecision::Refused(PENDING_REASON.into())
        );
        // login unknown
        db.set_setting("github_login", "").unwrap();
        assert_eq!(
            bug_band_for_repo(&db, rid),
            BandDecision::Refused(UNKNOWN_LOGIN_REASON.into())
        );
    }

    #[test]
    fn bug_band_for_repo_file_wins_over_cleared_shared_mode() {
        let tmp = TempDir::new().unwrap();
        let db = make_db();
        db.set_setting("github_login", "owner").unwrap();
        let rid = repo_at(&db, "Owner/web", tmp.path());
        write_structure_if_changed(tmp.path(), &sample()).unwrap();
        // shared_mode off, but the file is there: still banded
        assert_eq!(bug_band_for_repo(&db, rid), BandDecision::Band(1, 499_999));
    }

    #[test]
    fn bug_band_for_repo_refuses_a_foreign_structure() {
        let tmp = TempDir::new().unwrap();
        let db = make_db();
        db.set_setting("github_login", "owner").unwrap();
        // the repo is owned by someone else than main.login of the file
        let rid = repo_at(&db, "Mallory/web", tmp.path());
        write_structure_if_changed(tmp.path(), &sample()).unwrap();
        assert!(matches!(
            bug_band_for_repo(&db, rid),
            BandDecision::Refused(r) if r.contains("main is not the repo owner")
        ));
        // garbage file
        fs::write(tmp.path().join(".sdh/project.json"), "nope").unwrap();
        assert!(matches!(
            bug_band_for_repo(&db, rid),
            BandDecision::Refused(r) if r.contains("not trusted")
        ));
    }

    #[test]
    fn bug_band_for_repo_without_local_path_uses_the_cached_state_only() {
        let db = make_db();
        let rid = local_repo(&db, "Owner/web");
        assert_eq!(bug_band_for_repo(&db, rid), BandDecision::Unbanded);
        db.set_shared_mode(rid, true).unwrap();
        assert_eq!(bug_band_for_repo(&db, rid), BandDecision::Band(1, 499_999));
    }

    #[test]
    fn allocate_bug_numeric_id_unshared_uses_plain_counter() {
        let db = make_db();
        let rid = local_repo(&db, "Owner/web");
        assert_eq!(allocate_bug_numeric_id(&db, rid), Ok(1));
        seed_numeric(&db, rid, 500_001);
        // no sharing: the old behaviour, untouched
        assert_eq!(allocate_bug_numeric_id(&db, rid), Ok(500_002));
    }

    #[test]
    fn allocate_bug_numeric_id_main_stays_below_imported_helper_bugs() {
        let tmp = TempDir::new().unwrap();
        let db = make_db();
        db.set_setting("github_login", "Owner").unwrap();
        let rid = repo_at(&db, "Owner/web", tmp.path());
        let mut s = sample();
        s.participants.push(participant("helper", 500_000));
        write_structure_if_changed(tmp.path(), &s).unwrap();
        seed_numeric(&db, rid, 35);
        seed_numeric(&db, rid, 500_001);
        assert_eq!(allocate_bug_numeric_id(&db, rid), Ok(36));
    }

    #[test]
    fn allocate_bug_numeric_id_helper_counts_in_its_own_band() {
        let tmp = TempDir::new().unwrap();
        let db = make_helper_db();
        let rid = repo_at(&db, "Owner/web", tmp.path());
        let mut s = sample();
        s.participants.push(participant("helper", 500_000));
        write_structure_if_changed(tmp.path(), &s).unwrap();
        // the owner's bugs were imported
        seed_numeric(&db, rid, 35);
        assert_eq!(allocate_bug_numeric_id(&db, rid), Ok(500_000));
        seed_numeric(&db, rid, 500_000);
        assert_eq!(allocate_bug_numeric_id(&db, rid), Ok(500_001));
    }

    #[test]
    fn allocate_bug_numeric_id_refusals_and_full_band() {
        let tmp = TempDir::new().unwrap();
        let db = make_helper_db();
        let rid = repo_at(&db, "Owner/web", tmp.path());
        write_structure_if_changed(tmp.path(), &sample()).unwrap();
        // helper is not approved yet
        assert_eq!(
            allocate_bug_numeric_id(&db, rid),
            Err(PENDING_REASON.to_string())
        );

        let mut s = sample();
        s.participants.push(participant("helper", 500_000));
        write_structure_if_changed(tmp.path(), &s).unwrap();
        seed_numeric(&db, rid, 599_999);
        let err = allocate_bug_numeric_id(&db, rid).unwrap_err();
        assert!(err.contains("B-500000..B-599999"), "{}", err);
    }
}
