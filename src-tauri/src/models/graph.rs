use serde::{Deserialize, Serialize};

// ── F-013 Project graph DTOs ──────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum GraphNodeKind {
    Repo,
    Project,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum GraphEdgeKind {
    InProject,
    CrossProjectMs,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GraphNode {
    pub id: String,                  // "repo:42" or "project:7"
    pub label: String,               // display_name (repo) or project.name
    pub kind: GraphNodeKind,
    pub role: Option<String>,        // 'server' | 'client' | 'landing' | 'tool' | 'microservice' | None
    pub repo_id: Option<i64>,
    pub project_id: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    pub kind: GraphEdgeKind,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProjectGraph {
    pub center: Option<GraphNode>,
    pub ring: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}
