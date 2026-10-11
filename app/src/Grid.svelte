<script lang="ts">
  import type { Snippet } from "svelte";
  import * as api from "./lib/api";

  // Only the cells on screen (plus a few rows either side) exist in the page; the
  // rest of the result is a tall empty box that keeps the scrollbar honest.
  let { total, rowAt, need, selected, picked, queued, onselect, onopen, children }: {
    total: number;
    /** The photo at a position, once its page has loaded. */
    rowAt: (i: number) => number | undefined;
    /** Positions [from, to) are about to show. */
    need: (from: number, to: number) => void;
    /** The focused position: the one the details panel and viewer show. */
    selected: number | null;
    /** Is the photo at a position in the selection (several can be)? */
    picked: (i: number) => boolean;
    /** Does it have edits waiting to be written to Capture One? */
    queued: (i: number) => boolean;
    /** A click, with its modifier keys for adding to the selection. */
    onselect: (i: number, how?: { add: boolean; range: boolean }) => void;
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

  /** Moves the selection by arrow key; returns false for keys it does not handle. */
  export function move(key: string): boolean {
    const step: Record<string, number> = {
      ArrowLeft: -1, ArrowRight: 1, ArrowUp: -cols, ArrowDown: cols,
      PageUp: -cols * Math.max(1, Math.floor(height / lineH)), PageDown: cols * Math.max(1, Math.floor(height / lineH)),
    };
    let to: number;
    if (key === "Home") to = 0;
    else if (key === "End") to = total - 1;
    else if (key in step) to = (selected ?? -step[key]) + step[key];
    else return false;
    to = Math.max(0, Math.min(total - 1, to));
    if (total) {
      onselect(to);
      reveal(to);
    }
    return true;
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
        class:sel={selected === i}
        class:picked={picked(i)}
        style:width="{cellW}px"
        style:height="{cellH}px"
        style:transform="translate({PAD + (i % cols) * (cellW + GAP)}px, {PAD + Math.floor(i / cols) * lineH}px)"
        onclick={(e) => onselect(i, { add: e.ctrlKey || e.metaKey, range: e.shiftKey })}
        ondblclick={() => onopen(i)}
      >
        {#if row !== undefined}<img src={api.thumbUrl(row)} alt="" decoding="async" />{/if}
        {#if queued(i)}<span class="queued" title="Edited here, not yet written to Capture One"></span>{/if}
      </button>
    {/each}
  </div>
</main>
