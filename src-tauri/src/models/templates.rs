use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TemplateFile {
    pub language_key: String,
    pub file_name: String,
    pub content: String,
    pub is_custom: bool,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TemplateLanguage {
    pub language_key: String,
    pub display_name: String,
    pub file_count: i64,
}
