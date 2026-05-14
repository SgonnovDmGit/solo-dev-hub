use serde::{Deserialize, Serialize};

/// F-021: a single open/in-progress task parsed from `docs/todo.md`.
/// Format: `- [ ] <id> | <description> | <effort> | <priority> | <status>`
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TodoTask {
    pub id: String,
    pub description: String,
    pub effort: String,
    pub priority: String,
    pub status: String,
    pub created_at: String,  // YYYY-MM-DD; "" if 5-field legacy
}

#[derive(Debug, Serialize)]
pub struct ReadTodoResult {
    pub tasks: Vec<TodoTask>,
    pub warnings: Vec<String>,
}

/// F-021: a completed task parsed from `docs/done.md`.
/// Format (v0.13.9+):
///   `## <YYYY-MM-DD>`       ← date section header
///   `- <id> | <description> | <version>`  ← 3 pipe-separated fields, id may be empty
/// No `[x]` checkbox — the file itself is the "done" list.
/// If `id` is empty, the parser assigns `D-NNN` sequentially (in-memory only, file untouched).
/// `date` is inherited from the nearest preceding `## YYYY-MM-DD` header.
/// `version` is a free-form version tag (e.g. `v0.13.9`, `0.13.9`, or empty).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DoneTask {
    pub id: String,
    pub description: String,
    pub date: String,
    pub version: String,
}

#[derive(Debug, Serialize)]
pub struct ReadDoneResult {
    pub tasks: Vec<DoneTask>,
    pub warnings: Vec<String>,
}

/// v0.20.0: DB row for `tasks` table (mini-SoT for parsed todo.md/done.md).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Task {
    pub id: i64,
    pub repository_id: i64,
    pub task_id: String,
    pub prefix: String,
    pub description: String,
    pub effort: Option<f64>,
    pub priority: Option<String>,
    pub status: Option<String>,
    pub version: Option<String>,
    pub source: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TaskEvent {
    pub id: i64,
    pub task_id: i64,
    pub event_type: String,
    pub ts: String,
    pub from_status: Option<String>,
    pub to_status: Option<String>,
}
