<script lang="ts">
  import { onMount } from "svelte";
  import * as api from "./lib/api";
  import { source } from "./lib/source.svelte";
  import { covers, keywordTree, leftOutBy, matches, numberName, offeredCount, toggle, type KeywordNode, type LeftOut } from "./lib/keywordChoice";

  // The keywords a model is offered to choose from (Settings > AI): the tree with a box
  // each, unticked for those left out (with all below them), and keywords named only with
  // numbers left out with one box.
  let { left, busy, onchange }: { left: LeftOut; busy: boolean; onchange: (left: LeftOut) => void } = $props();

  let keywords = $state<[string, number][]>([]);
  let error = $state("");
  onMount(() => {
    api.keywords().then((k) => (keywords = k)).catch((e) => (error = String(e)));
  });
  let tree = $derived(keywordTree(keywords));
  let counts = $derived(offeredCount(keywords, left));

  let open = $state(new Set<string>());
  let find = $state("");
  let needle = $derived(find.trim().toLowerCase());
  const expanded = (n: KeywordNode) => (needle ? n.children.some((c) => matches(c, needle)) : open.has(n.path));
  function flip(path: string) {
    const next = new Set(open);
    if (!next.delete(path)) next.add(path);
    open = next;
  }
  /** Why a keyword is not offered though its own box may be ticked: a level above it, or
   *  its name. */
  function because(n: KeywordNode): string | null {
    const by = leftOutBy(n.path, left);
    if (by && by !== n.path) return `left out with ${by.replaceAll("|", " › ")}`;
    if (left.numbers && numberName(n.name)) return "a number";
    return null;
  }
</script>

{#snippet branch(nodes: KeywordNode[], depth: number)}
  {#each nodes as n (n.path)}
    {#if !needle || matches(n, needle)}
      {@const why = because(n)}
      <li style:padding-left="{depth * 16}px">
        {#if n.children.length}
          <button class="twisty" aria-label={expanded(n) ? "Fold" : "Unfold"} aria-expanded={expanded(n)} onclick={() => flip(n.path)}>{expanded(n) ? "▾" : "▸"}</button>
        {:else}
          <span class="twisty"></span>
        {/if}
        <label class:out={covers(n.path, left)}>
          <input
            type="checkbox"
            checked={!covers(n.path, left)}
            disabled={busy || why !== null}
            title={why ? `Not offered: ${why}` : ""}
            onchange={(e) => onchange(toggle(left, n.path, e.currentTarget.checked))}
          />
          {n.name}
          {#if n.uses}<span class="muted">{n.uses.toLocaleString()}</span>{/if}
          {#if why}<span class="muted why">· {why}</span>{/if}
        </label>
      </li>
      {#if n.children.length && expanded(n)}{@render branch(n.children, depth + 1)}{/if}
    {/if}
  {/each}
{/snippet}

<p class="muted small">
  {counts.offered.toLocaleString()} of {counts.all.toLocaleString()} keyword names on photos are offered{counts.offered > 400
    ? ": the 400 most used of them go with each call"
    : ""}. Untick a keyword to leave it out, with all below it. The model is not offered it, and an answer naming it is not
  matched to it.
</p>
<label class="numbers">
  <input type="checkbox" checked={left.numbers} disabled={busy} onchange={(e) => onchange({ ...left, numbers: e.currentTarget.checked })} />
  Leave out keywords named only with numbers (01, 02, 2023)
</label>
<div class="tools">
  <input type="search" placeholder="Find a keyword" bind:value={find} aria-label="Find a keyword" />
  {#if left.paths.length}
    <button class="small-btn" disabled={busy} onclick={() => onchange({ ...left, paths: [] })}>Offer all {left.paths.length} again</button>
  {/if}
</div>
{#if error}
  <p class="warn">{error}</p>
{:else if !keywords.length}
  <p class="muted small">The {source.noun} has no keywords yet.</p>
{:else}
  <ul class="tree" aria-label="Keywords offered to the model">{@render branch(tree, 0)}</ul>
{/if}

<style>
  .small { font-size: 12px; }
  .muted { color: var(--muted); }
  .warn { color: #e8b03a; }
  p { margin: 6px 0; }
  .numbers { display: flex; align-items: center; gap: 6px; font-size: 12px; cursor: pointer; }
  .tools { display: flex; gap: 8px; align-items: center; margin: 6px 0; }
  .tools input { padding: 2px 6px; background: var(--panel); color: inherit; border: 1px solid var(--line, #444); border-radius: 3px; font: inherit; width: 200px; }
  .small-btn { padding: 2px 10px; border: 1px solid var(--line); border-radius: 4px; }
  .small-btn:hover:not(:disabled) { background: var(--hover); }
  .tree { list-style: none; margin: 0; padding: 4px 0; max-height: 260px; overflow-y: auto; border: 1px solid var(--line); border-radius: 3px; font-size: 12px; }
  .tree li { display: flex; align-items: center; gap: 2px; padding-right: 6px; }
  .tree label { display: inline-flex; align-items: center; gap: 5px; cursor: pointer; }
  .tree label.out { color: var(--muted); }
  .twisty { width: 16px; padding: 0; text-align: center; color: var(--muted); flex: none; }
  .why { font-style: italic; }
</style>
