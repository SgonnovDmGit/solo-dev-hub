use base64::{engine::general_purpose::STANDARD, Engine as _};
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
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(e) => return Err(e.to_string()),
    }
    // H7 review-fix: also wipe the legacy entry. Otherwise
    // `migrate_legacy_pat()` would resurrect the deleted token on the next
    // cold start (it copies legacy → new whenever new is missing). The user
    // would see the token reappear after restart with no obvious cause.
    if let Ok(legacy) = Entry::new(LEGACY_SERVICE_NAME, PAT_KEY) {
        let _ = legacy.delete_credential();
    }
    Ok(())
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

const BUNDLE_KEY: &str = "secret-bundle-key";

/// v1.3.0: get the 32-byte data key used to encrypt secret-bundle values,
/// generating + persisting it on first use. Stored base64 in the OS keyring
/// under the same service as the PAT — trust model identical to the PAT.
pub fn get_or_create_bundle_key() -> Result<[u8; 32], String> {
    let entry = Entry::new(SERVICE_NAME, BUNDLE_KEY).map_err(|e| e.to_string())?;
    match entry.get_password() {
        Ok(b64) => {
            let bytes = STANDARD.decode(b64).map_err(|e| e.to_string())?;
            let arr: [u8; 32] = bytes
                .try_into()
                .map_err(|_| "stored bundle key has wrong length".to_string())?;
            Ok(arr)
        }
        Err(keyring::Error::NoEntry) => {
            let key = crate::crypto::bundle_cipher::generate_data_key();
            entry
                .set_password(&STANDARD.encode(key))
                .map_err(|e| e.to_string())?;
            Ok(key)
        }
        Err(e) => Err(e.to_string()),
    }
}
