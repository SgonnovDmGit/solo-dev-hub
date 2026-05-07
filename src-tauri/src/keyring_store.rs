use keyring::Entry;

const SERVICE_NAME: &str = "github-repo-manager";
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
