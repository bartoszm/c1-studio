// Sidebar trees: albums from the catalog's collections, folders and keywords from the
// counts the library returns (each folder and keyword is counted with everything under it).

import type { Collection } from "./api";

export type Icon = "album" | "smart" | "group" | "project" | "folder" | "keyword" | "date" | "place";

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
  /** Changed here and not yet written to the source. */
  queued?: boolean;
  children: Node[];
};

// Natural order, ignoring case and the stray spaces some folder names start with.
const byName = (a: Node, b: Node) =>
  a.name.trim().localeCompare(b.name.trim(), undefined, { numeric: true, sensitivity: "base" });

/** Most counted first, then by name. */
export const byCount = (a: Node, b: Node) => (b.count ?? 0) - (a.count ?? 0) || byName(a, b);

function sortDeep(nodes: Node[], order: (a: Node, b: Node) => number = byName): Node[] {
  nodes.sort(order);
  for (const n of nodes) sortDeep(n.children, order);
  return nodes;
}

/** Facet values that are paths (`a|b|c`, `H:/fotos/2026`), every prefix a value of its own. */
export function pathTree(
  values: [string, number][],
  sep: string,
  prefix: string,
  icon: Icon,
  order: (a: Node, b: Node) => number = byName,
): Node[] {
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
  return sortDeep(roots, order);
}

/** How many photos use each keyword or one under it, from the uses of each keyword
 *  itself. A photo with two keywords under one parent counts twice for the parent, so
 *  this can be high; it is for telling rarely used keywords apart. */
export function usesWithin(uses: [string, number][], sep = "|"): Map<string, number> {
  const out = new Map<string, number>();
  for (const [path, n] of uses) {
    let at = path.length;
    while (at > 0) {
      const p = path.slice(0, at);
      out.set(p, (out.get(p) ?? 0) + n);
      at = path.lastIndexOf(sep, at - 1);
    }
  }
  return out;
}

/** The tree without the nodes used fewer than `min` times (by `uses`), unless `keep` says
 *  otherwise or something under them stays. A node `uses` does not know (a keyword added
 *  since it was counted) stays. `hidden`: how many nodes were left out. */
export function hideRare(
  nodes: Node[],
  uses: Map<string, number>,
  min: number,
  keep: (node: Node) => boolean = () => false,
): { nodes: Node[]; hidden: number } {
  let hidden = 0;
  const size = (n: Node): number => 1 + n.children.reduce((sum, c) => sum + size(c), 0);
  function walk(list: Node[]): Node[] {
    const out: Node[] = [];
    for (const node of list) {
      const before = hidden;
      const below = walk(node.children);
      const n = uses.get(node.value);
      if (n === undefined || n >= min || keep(node) || below.length) {
        const same = below.length === node.children.length && below.every((c, i) => c === node.children[i]);
        out.push(same ? node : { ...node, children: below });
      } else {
        hidden = before + size(node);
      }
    }
    return out;
  }
  const out = min > 1 ? walk(nodes) : nodes;
  return { nodes: out, hidden };
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
  queued: Set<string> = new Set(),
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
      queued: queued.has(c.id),
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

/** What a level of the place tree is called when the photo has no value for it. */
const NO_PLACE = ["(no country)", "(no state)", "(no city)", "(no location)"];

/** Country › State › City › Location, from the `place_tree` facet. A missing level
 *  (`—` in the index) keeps its place, named for what is missing. A city some of whose
 *  photos have a location and some not gets a "(no location)" entry for the rest. */
export function placeTree(values: [string, number][]): Node[] {
  const name = (nodes: Node[], depth: number) => {
    for (const n of nodes) {
      if (n.name === "—") n.name = NO_PLACE[depth] ?? "—";
      name(n.children, depth + 1);
      const rest = (n.count ?? 0) - n.children.reduce((s, c) => s + (c.count ?? 0), 0);
      if (depth === 2 && n.children.length && rest > 0) {
        const value = `${n.value}|—`;
        n.children.unshift({ key: "p:" + value, value, name: NO_PLACE[3], icon: "place", count: rest, children: [] });
      }
    }
  };
  const roots = pathTree(values, "|", "p:", "place");
  name(roots, 0);
  return roots;
}

/** The city a place tree's "(no location)" entry is under (`Japan|Kanto|Tokyo|—` →
 *  `Japan|Kanto|Tokyo`); null for any other entry. The index has no such place: it is
 *  the city's photos with no location. */
export function cityWithoutLocation(value: string): string | null {
  const parts = value.split("|");
  return parts.length === 4 && parts[3] === "—" ? parts.slice(0, 3).join("|") : null;
}

/** `Poland|—|Kraków` → `Poland › (no state) › Kraków`. */
export function placeText(path: string): string {
  return path.split("|").map((part, i) => (part === "—" ? NO_PLACE[i] ?? part : part)).join(" › ");
}
