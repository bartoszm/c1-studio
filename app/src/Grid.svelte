<script lang="ts">
  import type { Snippet } from "svelte";
  import * as api from "./lib/api";

  // Only the cells on screen (plus a few rows either side) exist in the page; the
  // rest of the result is a tall empty box that keeps the scrollbar honest.
  let { total, rowAt, need, onopen, children }: {
    total: number;
    /** The photo at a position, once its page has loaded. */
    rowAt: (i: number) => number | undefined;
    /** Positions [from, to) are about to show. */
    need: (from: number, to: number) => void;
    onopen: (i: number) => void;
    children?: Snippet;
  } = $props();

  const MIN = 170;
  const GAP = 6;
  const PAD = 10;
  const OVERSCAN = 3;

  let el: HTMLElement;
  let width = $state(0);
  let height = $state(0);
  let top = $state(0);

  let cols = $derived(Math.max(1, Math.floor((width - 2 * PAD + GAP) / (MIN + GAP))));
  let cellW = $derived(Math.max(1, (width - 2 * PAD - GAP * (cols - 1)) / cols));
  let cellH = $derived(Math.round((cellW * 2) / 3));
  let lineH = $derived(cellH + GAP);
  let lines = $derived(Math.ceil(total / cols));
  let first = $derived(Math.max(0, Math.floor((top - PAD) / lineH) - OVERSCAN));
  let last = $derived(Math.min(lines, Math.ceil((top + height) / lineH) + OVERSCAN));
  let shown = $derived(
    Array.from({ length: Math.max(0, Math.min(total, last * cols) - first * cols) }, (_, k) => first * cols + k),
  );

  $effect(() => {
    if (width > 0 && total > 0) need(first * cols, Math.min(total, last * cols));
  });

  /** Back to the top, for a new result. */
  export function reset() {
    if (el) el.scrollTop = 0;
    top = 0;
  }

  /** Scrolls just enough to bring position `i` into view. */
  export function reveal(i: number) {
    if (!el) return;
    const y = PAD + Math.floor(i / cols) * lineH;
    if (y < el.scrollTop) el.scrollTop = y - PAD;
    else if (y + cellH > el.scrollTop + height) el.scrollTop = y + cellH + PAD - height;
  }
</script>

<main bind:this={el} bind:clientWidth={width} bind:clientHeight={height} onscroll={() => (top = el.scrollTop)}>
  {@render children?.()}
  <div class="space" style:height="{total ? lines * lineH - GAP + 2 * PAD : 0}px">
    {#each shown as i (i)}
      {@const row = rowAt(i)}
      <button
        class="cell"
        style:width="{cellW}px"
        style:height="{cellH}px"
        style:transform="translate({PAD + (i % cols) * (cellW + GAP)}px, {PAD + Math.floor(i / cols) * lineH}px)"
        onclick={() => onopen(i)}
      >
        {#if row !== undefined}<img src={api.thumbUrl(row)} alt="" decoding="async" />{/if}
      </button>
    {/each}
  </div>
</main>
