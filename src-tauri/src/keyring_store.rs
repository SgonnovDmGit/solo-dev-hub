use keyring::Entry;

const SERVICE_NAME: &str = "solo-dev-hub";
const LEGACY_SERVICE_NAME: &str = "github-repo-manager";
const PAT_KEY: &str = "github-pat";

pub fn store_pat(token: &str) -> Result<(), String> {
    let entry = Entry::new(SERVICE_NAME, PAT_KEY).map_err(|e| e.to_string())?;
    entry.set_password(token).map_err(|e| e.to_string())
}

pub fn get_pat() -> Result<Option<String>, String> {
    let entry = Entry::new(SERVICE_NAME, PAT_KEY).map_err(|e| e.to_string())?;
    match entry.get_password() {
        Ok(password) => Ok(Some(password)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub fn delete_pat() -> Result<(), String> {
    let entry = Entry::new(SERVICE_NAME, PAT_KEY).map_err(|e| e.to_string())?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

// T-000063: one-time PAT migration from the legacy keyring service to the new one.
// Called early at app startup. Idempotent: if new service already has a PAT we
// leave both alone (a previous migration already ran). Best-effort: any failure
// just leaves the legacy entry orphaned, harmless.
pub fn migrate_legacy_pat() {
    let new_entry = match Entry::new(SERVICE_NAME, PAT_KEY) {
        Ok(e) => e,
        Err(_) => return,
    };
    if new_entry.get_password().is_ok() {
        return;
    }
    let legacy_entry = match Entry::new(LEGACY_SERVICE_NAME, PAT_KEY) {
        Ok(e) => e,
        Err(_) => return,
    };
    let token = match legacy_entry.get_password() {
        Ok(t) => t,
        Err(_) => return,
    };
    if new_entry.set_password(&token).is_err() {
        return;
    }
    let _ = legacy_entry.delete_credential();
}
