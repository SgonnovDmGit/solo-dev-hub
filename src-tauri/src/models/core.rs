use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub created_at: String,
    pub project_type: String, // "standard" | "microservice"
    pub auto_sync_enabled: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Repository {
    pub id: i64,
    pub project_id: Option<i64>,
    pub github_name: Option<String>,
    pub github_url: Option<String>,
    pub role: Option<String>,
    pub description: Option<String>,
    pub language: Option<String>,
    pub last_pushed_at: Option<String>,
    pub added_at: String,
    pub updated_at: String,
    pub local_path: Option<String>,
    pub github_id: Option<i64>,
    pub deploy_target: Option<String>,
}

impl Repository {
    /// Display-friendly name. For GitHub repos returns the last segment of
    /// `github_name` (mirrors frontend `getDisplayName`). Falls back to
    /// `description`, then `<local>`.
    pub fn display_name(&self) -> String {
        if let Some(ref gh) = self.github_name {
            gh.rsplit('/').next().unwrap_or("").to_string()
        } else if let Some(ref desc) = self.description {
            desc.clone()
        } else {
            "<local>".to_string()
        }
    }

    /// F-033: canonical folder name used in cross-repo sync directory paths.
    /// For GitHub repos → last segment after '/' (e.g. `owner/foo-bar` → `foo-bar`).
    /// For local-only repos → `description` if set, else `local-<id>`.
    /// This is the single source-of-truth for naming sync subfolders like
    /// `client-requirements/<name>/` or `server-requirements/<parent-name>/`.
    pub fn canonical_folder_name(&self) -> String {
        if let Some(ref gh) = self.github_name {
            gh.rsplit('/').next().unwrap_or("").to_string()
        } else if let Some(ref desc) = self.description {
            desc.clone()
        } else {
            format!("local-{}", self.id)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_repo(id: i64, github_name: Option<&str>, description: Option<&str>) -> Repository {
        Repository {
            id,
            project_id: None,
            github_name: github_name.map(String::from),
            github_url: None,
            role: None,
            description: description.map(String::from),
            language: None,
            last_pushed_at: None,
            added_at: String::new(),
            updated_at: String::new(),
            local_path: None,
            github_id: None,
            deploy_target: None,
        }
    }

    #[test]
    fn display_name_strips_owner_prefix() {
        let r = mk_repo(1, Some("SgonnovDM/swanqu"), None);
        assert_eq!(r.display_name(), "swanqu");
    }

    #[test]
    fn display_name_handles_no_slash() {
        let r = mk_repo(1, Some("solo"), None);
        assert_eq!(r.display_name(), "solo");
    }

    #[test]
    fn display_name_falls_back_to_description() {
        let r = mk_repo(1, None, Some("local-only-tool"));
        assert_eq!(r.display_name(), "local-only-tool");
    }

    #[test]
    fn display_name_final_fallback() {
        let r = mk_repo(1, None, None);
        assert_eq!(r.display_name(), "<local>");
    }
}
