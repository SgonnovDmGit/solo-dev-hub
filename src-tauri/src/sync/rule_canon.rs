// T-000213: which forked rules have a canon that moved after the fork.
//
// A rule the owner edited is a fork (stored text differs from the text bundled
// with the build, the canon). The seeder never overwrites a fork, so a build
// that ships a changed canon leaves the fork on its old text and nothing says
// the canon moved. This module keeps, per forked rule, the sha256 of the canon
// as it stood when the owner last accepted it, in the setting
// `rule_fork_canon` (JSON object `{ "<rule>": "<hex>" }`). The canon is constant
// inside one process and changes only with a new build, so a record that no
// longer matches the current canon means the canon moved since.
//
// - `reconcile`: a fork without a record gets the CURRENT canon (the baseline:
//   forks from before this feature show no mark until the canon next moves);
//   a record whose rule is no longer a fork, or no longer exists, is dropped.
// - `acknowledge`: the owner (or an adopted hand-back) has accepted the current
//   canon — a fork gets the current canon recorded, a non-fork loses its record.

use crate::db::AppDb;
use crate::sync::skills::{rule_name_of, GLOBAL_SECTION_FILE, GLOBAL_SECTION_RULE};
use crate::template_seeder;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::Mutex;

/// The settings key holding the records.
pub const RULE_FORK_CANON_KEY: &str = "rule_fork_canon";

/// Serialises the read-modify-write of the setting; the database lock only
/// covers each single statement.
static STORE: Mutex<()> = Mutex::new(());

/// Lower-case hex sha256 of a canon text.
pub fn canon_hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

/// The `_global` template file that stores `rule`.
fn file_name_of(rule: &str) -> String {
    if rule == GLOBAL_SECTION_RULE {
        GLOBAL_SECTION_FILE.to_string()
    } else {
        format!("skill.{}.md.tmpl", rule)
    }
}

/// A rule is a fork when the bundle has its text and the stored one differs.
/// Text against the bundle, not `is_custom` (same predicate as `RuleInfo.changed`).
fn is_fork(canon: Option<&str>, content: &str) -> bool {
    canon.is_some_and(|canon| canon != content)
}

/// The stored records. A missing or unreadable value is an empty map: the next
/// reconcile re-baselines, which only costs a mark.
pub fn read_records(db: &AppDb) -> Result<BTreeMap<String, String>, String> {
    let raw = db
        .get_setting(RULE_FORK_CANON_KEY)
        .map_err(|e| e.to_string())?;
    Ok(raw
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default())
}

fn write_records(db: &AppDb, records: &BTreeMap<String, String>) -> Result<(), String> {
    let json = serde_json::to_string(records).map_err(|e| e.to_string())?;
    db.set_setting(RULE_FORK_CANON_KEY, &json)
        .map_err(|e| e.to_string())
}

/// `RuleInfo.canon_moved`: the rule is a fork, has a record, and the record is
/// not the current canon's hash.
pub fn is_moved(
    records: &BTreeMap<String, String>,
    rule: &str,
    canon: Option<&str>,
    content: &str,
) -> bool {
    is_fork(canon, content)
        && canon.is_some_and(|canon| {
            records
                .get(rule)
                .is_some_and(|recorded| *recorded != canon_hash(canon))
        })
}

/// Bring the records in line with the stored rules (see the module header).
/// Run at startup after seeding and at the end of every rules refresh pass.
pub fn reconcile(db: &AppDb) -> Result<(), String> {
    let _guard = STORE.lock().unwrap_or_else(|e| e.into_inner());
    let old = read_records(db)?;
    let files = db
        .list_template_files("_global")
        .map_err(|e| e.to_string())?;
    let mut records = BTreeMap::new();
    for file in files {
        let Some(rule) = rule_name_of(&file.file_name) else {
            continue;
        };
        let canon = template_seeder::bundled_file_content("_global", &file.file_name);
        if let Some(canon) = canon.as_deref().filter(|c| *c != file.content) {
            let hash = old.get(rule).cloned().unwrap_or_else(|| canon_hash(canon));
            records.insert(rule.to_string(), hash);
        }
    }
    if records != old {
        write_records(db, &records)?;
    }
    Ok(())
}

/// The current canon of `rule` has been accepted: a fork records it, a rule
/// that is not a fork drops its record.
pub fn acknowledge(db: &AppDb, rule: &str) -> Result<(), String> {
    let _guard = STORE.lock().unwrap_or_else(|e| e.into_inner());
    let file_name = file_name_of(rule);
    let row = db
        .get_template_file("_global", &file_name)
        .map_err(|e| e.to_string())?;
    let canon = template_seeder::bundled_file_content("_global", &file_name);
    let mut records = read_records(db)?;
    let before = records.clone();
    match (row, canon) {
        (Some(row), Some(canon)) if canon != row.content => {
            records.insert(rule.to_string(), canon_hash(&canon));
        }
        _ => {
            records.remove(rule);
        }
    }
    if records != before {
        write_records(db, &records)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULE: &str = "sdh-retro";
    const FILE: &str = "skill.sdh-retro.md.tmpl";

    fn seeded() -> AppDb {
        let db = AppDb::new(std::path::PathBuf::from(":memory:")).unwrap();
        template_seeder::seed_bundled_templates(&db).unwrap();
        db
    }

    fn canon() -> String {
        template_seeder::bundled_file_content("_global", FILE).unwrap()
    }

    fn fork(db: &AppDb) {
        db.upsert_template_file("_global", FILE, "OWNER EDIT", true)
            .unwrap();
    }

    fn moved(db: &AppDb) -> bool {
        let row = db.get_template_file("_global", FILE).unwrap().unwrap();
        is_moved(
            &read_records(db).unwrap(),
            RULE,
            Some(canon().as_str()),
            &row.content,
        )
    }

    #[test]
    fn test_a_fresh_install_has_no_records_and_no_flag() {
        let db = seeded();
        reconcile(&db).unwrap();
        assert!(read_records(&db).unwrap().is_empty());
        assert!(!moved(&db));
    }

    #[test]
    fn test_a_new_fork_records_the_current_canon() {
        let db = seeded();
        fork(&db);
        reconcile(&db).unwrap();
        let records = read_records(&db).unwrap();
        assert_eq!(records.get(RULE), Some(&canon_hash(&canon())));
        assert_eq!(records.len(), 1);
        assert!(!moved(&db));
    }

    /// K2: a fork from before the feature has no record; the first reconcile
    /// takes the current canon as its baseline, so it shows no mark.
    #[test]
    fn test_an_old_fork_without_a_record_is_baselined_not_flagged() {
        let db = seeded();
        fork(&db);
        assert!(read_records(&db).unwrap().is_empty());
        assert!(!moved(&db), "no record, no flag");
        reconcile(&db).unwrap();
        assert!(!moved(&db));
    }

    #[test]
    fn test_a_moved_canon_is_flagged_and_reconcile_keeps_the_old_record() {
        let db = seeded();
        fork(&db);
        reconcile(&db).unwrap();
        let mut records = read_records(&db).unwrap();
        records.insert(RULE.into(), canon_hash("an older canon"));
        write_records(&db, &records).unwrap();
        assert!(moved(&db));

        reconcile(&db).unwrap();
        assert!(moved(&db), "reconcile never re-accepts a canon");
        assert_eq!(
            read_records(&db).unwrap().get(RULE),
            Some(&canon_hash("an older canon"))
        );
    }

    #[test]
    fn test_saving_after_the_canon_moved_acknowledges_it() {
        let db = seeded();
        fork(&db);
        let mut records = BTreeMap::new();
        records.insert(RULE.to_string(), canon_hash("an older canon"));
        write_records(&db, &records).unwrap();
        assert!(moved(&db));

        acknowledge(&db, RULE).unwrap();
        assert!(!moved(&db));
        assert_eq!(
            read_records(&db).unwrap().get(RULE),
            Some(&canon_hash(&canon()))
        );
    }

    #[test]
    fn test_a_healed_fork_or_a_reset_drops_the_record() {
        let db = seeded();
        fork(&db);
        reconcile(&db).unwrap();
        assert_eq!(read_records(&db).unwrap().len(), 1);

        // Healed: the text became equal to the canon.
        db.upsert_template_file("_global", FILE, &canon(), true)
            .unwrap();
        reconcile(&db).unwrap();
        assert!(read_records(&db).unwrap().is_empty());

        // Reset: acknowledge on a non-fork drops it as well.
        fork(&db);
        reconcile(&db).unwrap();
        db.upsert_template_file("_global", FILE, &canon(), false)
            .unwrap();
        acknowledge(&db, RULE).unwrap();
        assert!(read_records(&db).unwrap().is_empty());
    }

    #[test]
    fn test_a_record_of_a_rule_that_no_longer_exists_is_dropped() {
        let db = seeded();
        let mut records = BTreeMap::new();
        records.insert("sdh-gone".to_string(), canon_hash("x"));
        write_records(&db, &records).unwrap();
        reconcile(&db).unwrap();
        assert!(read_records(&db).unwrap().is_empty());
    }

    #[test]
    fn test_a_rule_that_is_not_a_fork_is_never_flagged() {
        let db = seeded();
        let mut records = BTreeMap::new();
        records.insert(RULE.to_string(), canon_hash("an older canon"));
        write_records(&db, &records).unwrap();
        assert!(!moved(&db), "text equals the canon");
        // No bundled canon at all: nothing to compare with.
        assert!(!is_moved(&records, RULE, None, "anything"));
    }

    #[test]
    fn test_the_global_section_is_handled_like_a_skill() {
        let db = seeded();
        db.upsert_template_file("_global", GLOBAL_SECTION_FILE, "MY SECTION", true)
            .unwrap();
        reconcile(&db).unwrap();
        assert!(read_records(&db).unwrap().contains_key(GLOBAL_SECTION_RULE));
        let mut records = read_records(&db).unwrap();
        records.insert(GLOBAL_SECTION_RULE.into(), canon_hash("older"));
        write_records(&db, &records).unwrap();
        acknowledge(&db, GLOBAL_SECTION_RULE).unwrap();
        let canon = template_seeder::bundled_file_content("_global", GLOBAL_SECTION_FILE).unwrap();
        assert_eq!(
            read_records(&db).unwrap().get(GLOBAL_SECTION_RULE),
            Some(&canon_hash(&canon))
        );
    }

    #[test]
    fn test_an_unreadable_record_store_counts_as_empty() {
        let db = seeded();
        db.set_setting(RULE_FORK_CANON_KEY, "not json").unwrap();
        assert!(read_records(&db).unwrap().is_empty());
        fork(&db);
        reconcile(&db).unwrap();
        assert_eq!(read_records(&db).unwrap().len(), 1);
    }
}
