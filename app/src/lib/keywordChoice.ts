// Which of the catalog's keywords a vision model is offered to choose from (Settings > AI):
// the same rules as `c1ai::LeftOut`, and the keyword tree they are picked in.

export type LeftOut = {
  /** Pipe paths, each left out with all below it. */
  paths: string[];
  /** Keywords named only with numbers ("01", "2023") are left out too. */
  numbers: boolean;
};

/** A name made of digits and separators only: "01", "2023", "2023-05", "3.". */
export function numberName(name: string): boolean {
  const n = name.trim();
  return /\d/.test(n) && /^[\d \-_./#]+$/.test(n);
}

const leaf = (path: string) => path.slice(path.lastIndexOf("|") + 1);

/** The path, or one above it, that leaves `path` out by name; null when none does. */
export function leftOutBy(path: string, left: LeftOut): string | null {
  return left.paths.find((p) => path === p || path.startsWith(p + "|")) ?? null;
}

/** The keyword at `path` is not offered. */
export function covers(path: string, left: LeftOut): boolean {
  return (left.numbers && numberName(leaf(path))) || leftOutBy(path, left) !== null;
}

export type KeywordNode = {
  path: string;
  name: string;
  /** Photos carrying this keyword itself. */
  uses: number;
  children: KeywordNode[];
};

/** The tree of `keywords` (pipe paths and their uses), levels in between made where
 *  missing, siblings by name. */
export function keywordTree(keywords: [string, number][]): KeywordNode[] {
  const byPath = new Map<string, KeywordNode>();
  const roots: KeywordNode[] = [];
  const node = (path: string): KeywordNode => {
    let n = byPath.get(path);
    if (n) return n;
    n = { path, name: leaf(path), uses: 0, children: [] };
    byPath.set(path, n);
    const cut = path.lastIndexOf("|");
    (cut < 0 ? roots : node(path.slice(0, cut)).children).push(n);
    return n;
  };
  for (const [path, uses] of keywords) node(path).uses += uses;
  const sort = (nodes: KeywordNode[]) => {
    nodes.sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true }));
    for (const n of nodes) sort(n.children);
  };
  sort(roots);
  return roots;
}

/** Of `keywords`, the distinct names a model is offered and the distinct names in use:
 *  only keywords on photos count, as only they are offered. */
export function offeredCount(keywords: [string, number][], left: LeftOut): { offered: number; all: number } {
  const all = new Set<string>();
  const offered = new Set<string>();
  for (const [path, uses] of keywords) {
    if (uses < 1) continue;
    const name = leaf(path).trim().toLowerCase();
    all.add(name);
    if (!covers(path, left)) offered.add(name);
  }
  return { offered: offered.size, all: all.size };
}

/** `left` with `path` offered again or left out. Leaving one out drops the paths below it
 *  from the list, which it now covers. */
export function toggle(left: LeftOut, path: string, offer: boolean): LeftOut {
  const paths = offer
    ? left.paths.filter((p) => p !== path)
    : [...left.paths.filter((p) => p !== path && !p.startsWith(path + "|")), path];
  return { ...left, paths };
}

/** Whether `node` or anything below it matches `find` (lower case). */
export function matches(node: KeywordNode, find: string): boolean {
  return node.name.toLowerCase().includes(find) || node.children.some((c) => matches(c, find));
}
