<script lang="ts">
  import type { SvelteSet } from "svelte/reactivity";
  import Icon from "./Icon.svelte";
  import type { Node } from "./lib/tree";
  import Tree from "./Tree.svelte";

  // One level of a tree; expanded children render a nested Tree, so closed branches
  // cost nothing however large they are.
  let { nodes, selected, mark, expanded, onpick, oncontext, drag, candrop, ondropnode, depth = 0 }: {
    nodes: Node[];
    selected: string | undefined;
    /** Whether a node is picked in (✓) or out (excluded): several can be at once. */
    mark?: (node: Node) => "in" | "out" | null;
    expanded: SvelteSet<string>;
    onpick: (node: Node) => void;
    /** Right-click on a node (its menu). */
    oncontext?: (node: Node, e: MouseEvent) => void;
    /** Whether a node can be dragged; called again when the drag starts. */
    drag?: (node: Node, starting: boolean) => boolean;
    /** Whether what is being dragged can be dropped on a node. */
    candrop?: (node: Node) => boolean;
    ondropnode?: (node: Node) => void;
    depth?: number;
  } = $props();

  /** The node a drag is over, shown as a drop target. */
  let over = $state<string | null>(null);

  function flip(node: Node) {
    if (expanded.has(node.key)) expanded.delete(node.key);
    else expanded.add(node.key);
  }
</script>

{#each nodes as node (node.key)}
  {@const open = expanded.has(node.key)}
  {@const m = mark?.(node) ?? null}
  <div
    class="node"
    class:on={selected === node.key || m === "in"}
    class:out={m === "out"}
    class:empty={node.count === 0}
    class:queued={node.queued}
    style:padding-left="{6 + depth * 14}px"
    title={node.disabled ?? (node.queued ? "Changed here; not yet written to Capture One" : undefined)}
    role="treeitem"
    aria-selected={selected === node.key || m === "in"}
    tabindex="-1"
    oncontextmenu={oncontext && ((e) => { e.preventDefault(); oncontext(node, e); })}
    class:drop={over === node.key}
    draggable={!!drag?.(node, false)}
    ondragstart={(e) => {
      e.stopPropagation();
      e.dataTransfer?.setData("text/plain", node.name);
      if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
      drag?.(node, true);
    }}
    ondragover={(e) => {
      if (!candrop?.(node)) return;
      e.preventDefault();
      e.stopPropagation();
      over = node.key;
    }}
    ondragleave={() => { if (over === node.key) over = null; }}
    ondrop={(e) => {
      over = null;
      if (!candrop?.(node)) return;
      e.preventDefault();
      e.stopPropagation();
      ondropnode?.(node);
    }}
  >
    {#if node.children.length}
      <button class="chev" class:open aria-label={open ? "Collapse" : "Expand"} onclick={() => flip(node)}>›</button>
    {:else}
      <span class="chev"></span>
    {/if}
    <button
      class="label"
      disabled={!!node.disabled}
      onclick={() => (node.container ? flip(node) : onpick(node))}
    >
      <Icon name={node.icon} />
      <span class="name">{node.name}</span>
      {#if node.count !== undefined}<span class="n">{node.count.toLocaleString()}</span>{/if}
    </button>
  </div>
  {#if open && node.children.length}
    <Tree nodes={node.children} {selected} {mark} {expanded} {onpick} {oncontext} {drag} {candrop} {ondropnode} depth={depth + 1} />
  {/if}
{/each}
