// A find box over a sidebar tree: what is typed narrows the tree to the matches and the
// branches above them, opened in a set of their own so the tree's own open branches stay
// as they were for when the box is cleared.

import { SvelteSet } from "svelte/reactivity";
import { searchTree, type Node } from "./tree";

export type TreeFind = {
  query: string;
  /** Something is typed. */
  readonly searching: boolean;
  /** The tree to show: all of it, or the matches. */
  readonly nodes: Node[];
  /** While searching, the branches opened to show the matches. */
  readonly open: SvelteSet<string>;
};

/** A find box over the tree `nodes` returns. Call while a component is set up. */
export function treeFind(nodes: () => Node[]): TreeFind {
  let query = $state("");
  const view = $derived(searchTree(nodes(), query));
  const open = new SvelteSet<string>();
  $effect(() => {
    open.clear();
    for (const key of view.open) open.add(key);
  });
  return {
    get query() {
      return query;
    },
    set query(v: string) {
      query = v;
    },
    get searching() {
      return !!query.trim();
    },
    get nodes() {
      return view.nodes;
    },
    open,
  };
}
