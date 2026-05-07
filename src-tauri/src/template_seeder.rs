use crate::db::AppDb;
use include_dir::{include_dir, Dir};

/// Bundled templates embedded at compile time from `src-tauri/templates/`.
/// Each subdirectory is a language (e.g. `flutter_web`) with its own files.
pub static TEMPLATES_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/templates");

/// On app startup, seed & auto-migrate the `templates` table from the bundle.
///
/// Invariant: all `is_custom=0` rows in the DB are byte-equal to the current bundle.
/// - Not in DB → INSERT with `is_custom=0`
/// - `is_custom=1` (user-edited) → left alone
/// - `is_custom=0` and content differs from bundle → UPDATE to bundle (picks up app upgrades)
///
/// Returns the count of rows touched (inserted or updated).
pub fn seed_bundled_templates(db: &AppDb) -> Result<usize, String> {
    let mut touched = 0usize;
    for lang_dir in TEMPLATES_DIR.dirs() {
        let lang_key = lang_dir
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| "Invalid language folder name".to_string())?;

        for file in lang_dir.files() {
            let file_name = file
                .path()
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| "Invalid file name".to_string())?;
            let bundle_content = file
                .contents_utf8()
                .ok_or_else(|| format!("Template file {} is not valid UTF-8", file_name))?;

            let existing = db
                .get_template_file(lang_key, file_name)
                .map_err(|e| e.to_string())?;

            match existing {
                None => {
                    db.upsert_template_file(lang_key, file_name, bundle_content, false)
                        .map_err(|e| e.to_string())?;
                    touched += 1;
                }
                Some(row) if !row.is_custom && row.content != bundle_content => {
                    db.upsert_template_file(lang_key, file_name, bundle_content, false)
                        .map_err(|e| e.to_string())?;
                    touched += 1;
                }
                _ => {}
            }
        }
    }
    Ok(touched)
}

/// Return content of a bundled file, or None if not in the bundle (used for Reset-to-default).
pub fn bundled_file_content(language_key: &str, file_name: &str) -> Option<String> {
    let lang_dir = TEMPLATES_DIR.get_dir(language_key)?;
    let file = lang_dir.get_file(format!("{}/{}", language_key, file_name))?;
    file.contents_utf8().map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::AppDb;
    use tempfile::TempDir;

    fn make_db() -> AppDb {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.db");
        std::mem::forget(tmp);
        AppDb::new(path).unwrap()
    }

    #[test]
    fn test_seed_inserts_flutter_web() {
        let db = make_db();
        assert!(!db
            .list_template_languages()
            .unwrap()
            .contains(&"flutter_web".to_string()));

        let n = seed_bundled_templates(&db).unwrap();
        assert!(n >= 3, "at least 3 files seeded, got {}", n);
        assert!(db
            .list_template_languages()
            .unwrap()
            .contains(&"flutter_web".to_string()));

        let files = db.list_template_files("flutter_web").unwrap();
        let names: Vec<&str> = files.iter().map(|f| f.file_name.as_str()).collect();
        assert!(names.contains(&"deploy.yml.tmpl"));
        assert!(names.contains(&"dockerfile.tmpl"));
        assert!(names.contains(&"meta.json"));

        for f in &files {
            assert!(!f.is_custom);
        }
    }

    #[test]
    fn test_seed_preserves_custom_files() {
        let db = make_db();
        seed_bundled_templates(&db).unwrap();

        // User edits a file → is_custom=1
        db.upsert_template_file("flutter_web", "dockerfile.tmpl", "CUSTOM", true)
            .unwrap();

        let n2 = seed_bundled_templates(&db).unwrap();
        assert_eq!(
            n2, 0,
            "nothing should be touched on re-seed when everything is in sync"
        );

        let f = db
            .get_template_file("flutter_web", "dockerfile.tmpl")
            .unwrap()
            .unwrap();
        assert_eq!(f.content, "CUSTOM");
        assert!(f.is_custom);
    }

    #[test]
    fn test_seed_updates_non_custom_on_bundle_change() {
        let db = make_db();
        seed_bundled_templates(&db).unwrap();

        // Simulate "previous app version" that seeded outdated content (is_custom=0).
        db.upsert_template_file("flutter_web", "meta.json", "{\"old\":true}", false)
            .unwrap();
        let pre = db
            .get_template_file("flutter_web", "meta.json")
            .unwrap()
            .unwrap();
        assert_eq!(pre.content, "{\"old\":true}");
        assert!(!pre.is_custom);

        // Re-seed: non-custom + content differs → must be updated back to bundle.
        let n = seed_bundled_templates(&db).unwrap();
        assert!(n >= 1);
        let post = db
            .get_template_file("flutter_web", "meta.json")
            .unwrap()
            .unwrap();
        assert!(
            post.content.contains("display_name"),
            "meta.json must be bundled version"
        );
        assert!(!post.is_custom);
    }

    #[test]
    fn test_bundled_file_content_flutter_web() {
        let content = bundled_file_content("flutter_web", "meta.json").unwrap();
        assert!(content.contains("display_name"));
        assert!(
            content.contains("file_targets"),
            "meta.json v2 must include file_targets"
        );
    }

    #[test]
    fn test_bundled_file_content_missing() {
        assert!(bundled_file_content("nonexistent_lang", "foo.txt").is_none());
    }
}
