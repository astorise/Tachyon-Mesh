/**
 * Tiny DOM construction helper that uses `textContent` for all string content,
 * so user-controlled values can never inject HTML. This is the safe replacement
 * for `escapeHtml(value)` in template literal `innerHTML` patterns.
 *
 * Example:
 *   const row = el("tr", {},
 *     el("td", { class: "py-1" }, user.name),       // textContent — safe
 *     el("td", { class: "py-1" }, user.role),
 *   );
 *   container.replaceChildren(row);
 */
export type ElChild = Node | string | null | undefined | false;

export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Record<string, string | boolean | null | undefined> = {},
  ...children: ElChild[]
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (value === false || value === null || value === undefined) continue;
    if (key === "class" || key === "className") {
      node.className = String(value);
    } else if (key === "style") {
      node.setAttribute("style", String(value));
    } else if (key.startsWith("data-") || key.startsWith("aria-") || key === "role") {
      node.setAttribute(key, String(value));
    } else if (key in node) {
      // For type, value, id, etc. that are direct properties.
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      (node as any)[key] = value;
    } else {
      node.setAttribute(key, String(value));
    }
  }
  for (const child of children) {
    if (child === null || child === undefined || child === false) continue;
    if (typeof child === "string") {
      node.appendChild(document.createTextNode(child));
    } else {
      node.appendChild(child);
    }
  }
  return node;
}

/**
 * Convenience: build a DocumentFragment from a list of children.
 */
export function frag(...children: ElChild[]): DocumentFragment {
  const f = document.createDocumentFragment();
  for (const child of children) {
    if (child === null || child === undefined || child === false) continue;
    if (typeof child === "string") {
      f.appendChild(document.createTextNode(child));
    } else {
      f.appendChild(child);
    }
  }
  return f;
}
