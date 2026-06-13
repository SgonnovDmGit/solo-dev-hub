// v1.3.0: secret bundles. Mirrors Rust SecretBundle / SecretBundleItemValue
// (snake_case — Tauri contract convention).

export interface SecretBundle {
  id: number;
  name: string;
  description: string;
  created_at: string;
  updated_at: string;
  secret_names: string[];
}

export interface SecretBundleItemValue {
  secret_name: string;
  value: string;
}
