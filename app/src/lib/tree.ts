// Sidebar trees: albums from the catalog's collections, folders and keywords from the
// counts the library returns (each folder and keyword is counted with everything under it).

import type { Collection } from "./api";

export type Icon = "album" | "smart" | "group" | "project" | "folder" | "keyword" | "date";

export type Node = {
  /** Unique across all trees: `a:<id>`, `f:<path>`, `k:<path>`. */
  key: string;
  /** What picking it filters by: a collection id, folder path or keyword path. */
  value: string;
  name: string;
  icon: Icon;
  count?: number;
  /** Only expands; picking it filters nothing (album groups). */
  container?: boolean;
  /** Why it cannot be picked. */
  disabled?: string;
  children: Node[];
};

// Natural order, ignoring case and the stray spaces some folder names start with.
const byName = (a: Node, b: Node) =>
  a.name.trim().localeCompare(b.name.trim(), undefined, { numeric: true, sensitivity: "base" });

function sortDeep(nodes: Node[], order: (a: Node, b: Node) => number = byName): Node[] {
  nodes.sort(order);
  for (const n of nodes) sortDeep(n.children, order);
  return nodes;
}

/** Facet values that are paths (`a|b|c`, `H:/fotos/2026`), every prefix a value of its own. */
export function pathTree(values: [string, number][], sep: string, prefix: string, icon: Icon): Node[] {
  const nodes = new Map<string, Node>();
  for (const [value, count] of values) {
    const at = value.lastIndexOf(sep);
    nodes.set(value, { key: prefix + value, value, name: at > 0 ? value.slice(at + 1) : value, icon, count, children: [] });
  }
  const roots: Node[] = [];
  for (const node of nodes.values()) {
    const at = node.value.lastIndexOf(sep);
    const parent = at > 0 ? nodes.get(node.value.slice(0, at)) : undefined;
    (parent ? parent.children : roots).push(node);
  }
  return sortDeep(roots);
}

const ICONS: Partial<Record<Collection["kind"], Icon>> = {
  Album: "album",
  Smart: "smart",
  Group: "group",
  Project: "project",
};

/**
 * Albums, smart albums, projects and their groups, as Capture One nests them. Albums and
 * projects count their members; smart albums count what their rule selects.
 */
export function albumTree(
  collections: Collection[],
  counts: Map<string, number>,
  smart: Map<string, number> | null,
): Node[] {
  const nodes = new Map<string, Node>();
  for (const c of collections) {
    const icon = ICONS[c.kind];
    if (!icon) continue;
    const unsupported = c.rule?.conditions.find((x) => "Unsupported" in x);
    nodes.set(c.id, {
      key: "a:" + c.id,
      value: c.id,
      name: c.name || "(unnamed)",
      icon,
      count: c.kind === "Group" ? undefined : c.kind === "Smart" ? smart?.get(c.id) : (counts.get(c.id) ?? 0),
      container: c.kind === "Group",
      disabled: unsupported && "Unsupported" in unsupported ? `Rule not supported: ${unsupported.Unsupported}` : undefined,
      children: [],
    });
  }
  const roots: Node[] = [];
  for (const c of collections) {
    const node = nodes.get(c.id);
    if (!node) continue;
    const parent = c.parent ? nodes.get(c.parent) : undefined;
    (parent ? parent.children : roots).push(node);
  }
  // Groups first, as folders come before files.
  return sortDeep(roots, (a, b) => Number(!!b.container) - Number(!!a.container) || byName(a, b));
}

/**
 * The nodes whose name contains `query` (ignoring case), with the branches above them.
 * A match keeps everything under it. `open` lists the branches to show expanded: those
 * leading to a match.
 */
export function searchTree(nodes: Node[], query: string): { nodes: Node[]; open: string[] } {
  const q = query.trim().toLowerCase();
  const open: string[] = [];
  function walk(list: Node[]): Node[] {
    const out: Node[] = [];
    for (const node of list) {
      const below = walk(node.children);
      if (node.name.toLowerCase().includes(q)) {
        if (below.length) open.push(node.key);
        out.push(node);
      } else if (below.length) {
        open.push(node.key);
        out.push({ ...node, children: below });
      }
    }
    return out;
  }
  return { nodes: q ? walk(nodes) : nodes, open };
}
