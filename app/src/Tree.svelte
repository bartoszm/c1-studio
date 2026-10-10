<script lang="ts">
  import type { SvelteSet } from "svelte/reactivity";
  import Icon from "./Icon.svelte";
  import type { Node } from "./lib/tree";
  import Tree from "./Tree.svelte";

  // One level of a tree; expanded children render a nested Tree, so closed branches
  // cost nothing however large they are.
  let { nodes, selected, expanded, onpick, depth = 0 }: {
    nodes: Node[];
    selected: string | undefined;
    expanded: SvelteSet<string>;
    onpick: (node: Node) => void;
    depth?: number;
  } = $props();

  function flip(node: Node) {
    if (expanded.has(node.key)) expanded.delete(node.key);
    else expanded.add(node.key);
  }
</script>

{#each nodes as node (node.key)}
  {@const open = expanded.has(node.key)}
  <div
    class="node"
    class:on={selected === node.key}
    class:empty={node.count === 0}
    style:padding-left="{6 + depth * 14}px"
    title={node.disabled}
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
    <Tree nodes={node.children} {selected} {expanded} {onpick} depth={depth + 1} />
  {/if}
{/each}
