/**
 * Validates a glob pattern for WIT import scopes.
 * Rejects: empty strings, unbalanced braces, bare whitespace.
 * Does not fully replicate globset semantics — the server is authoritative.
 */
export function isValidGlob(pattern: string): boolean {
  if (!pattern || pattern.trim().length === 0) return false;
  if (pattern !== pattern.trim()) return false;

  let depth = 0;
  for (const ch of pattern) {
    if (ch === "{") depth++;
    else if (ch === "}") depth--;
    if (depth < 0) return false;
  }
  if (depth !== 0) return false;

  if (/\s/.test(pattern)) return false;

  return true;
}

/**
 * Validates a routing-category tuple pattern (`source -> dest`).
 * Must contain exactly one ` -> ` separator with non-empty parts on each side.
 * Does not apply the no-whitespace rule from isValidGlob because the separator
 * itself contains spaces.
 */
function hasBalancedBraces(s: string): boolean {
  let depth = 0;
  for (const ch of s) {
    if (ch === "{") depth++;
    else if (ch === "}") depth--;
    if (depth < 0) return false;
  }
  return depth === 0;
}

export function isValidRoutingTuple(pattern: string): boolean {
  if (!pattern || pattern.trim().length === 0) return false;
  if (!hasBalancedBraces(pattern)) return false;

  const sep = " -> ";
  const idx = pattern.indexOf(sep);
  if (idx === -1) return false;

  const src = pattern.slice(0, idx).trim();
  const dst = pattern.slice(idx + sep.length).trim();
  return src.length > 0 && dst.length > 0;
}
