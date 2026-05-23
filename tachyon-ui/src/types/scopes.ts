export type ScopeCategory =
  | "secrets"
  | "kv"
  | "graph"
  | "sql"
  | "http_client"
  | "blob"
  | "messaging"
  | "crypto"
  | "routing"
  | "compute";

export const SCOPE_CATEGORIES: ScopeCategory[] = [
  "secrets",
  "kv",
  "graph",
  "sql",
  "http_client",
  "blob",
  "messaging",
  "crypto",
  "routing",
  "compute",
];

export type ScopesMap = Partial<Record<ScopeCategory, string[]>>;
