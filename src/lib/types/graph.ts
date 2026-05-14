// F-013 Project graph mirror types (matches src-tauri/src/models/graph.rs)

export type GraphNodeKind = 'repo' | 'project';
export type GraphEdgeKind = 'in_project' | 'cross_project_ms';

export interface GraphNode {
  id: string;
  label: string;
  kind: GraphNodeKind;
  role: string | null;
  repo_id: number | null;
  project_id: number | null;
}

export interface GraphEdge {
  source: string;
  target: string;
  kind: GraphEdgeKind;
}

export interface ProjectGraph {
  center: GraphNode | null;
  ring: GraphNode[];
  edges: GraphEdge[];
}
