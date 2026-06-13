use serde::{Deserialize, Serialize};

/// A secret bundle's metadata + the names of its items (NEVER the values).
/// JSON snake_case (project convention — this is a Tauri tool contract).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SecretBundle {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub created_at: String,
    pub updated_at: String,
    pub secret_names: Vec<String>,
}

/// One decrypted secret name+value. Returned only by `get_bundle_decrypted`,
/// consumed transiently by the frontend for the GitHub push.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SecretBundleItemValue {
    pub id: i64,
    pub secret_name: String,
    pub value: String,
}
