<script lang="ts" module>
  /** How tiles are sized: about this wide (the count per row follows the window), or a
   *  fixed count per row (the width follows). */
  export type Tiles = { by: "size"; px: number } | { by: "count"; n: number };
  export type Shape = "3:2" | "1:1";
  export const DEFAULT_TILES: Tiles = { by: "size", px: 170 };
  export const MIN_TILE = 60;
  export const MAX_COUNT = 30;
  /** What the grid laid out: tiles per row, their width, the width they share. */
  export type Layout = { cols: number; width: number; room: number };

  /** A small copy of the photo as the drag image, above and right of the cursor, so the
   *  cursor's tip (where a map drop lands) stays in sight. */
  function ghost(transfer: DataTransfer, img: HTMLImageElement) {
    // A thumbnail still loading, or one that failed, keeps the browser's own drag image.
    if (!img.complete || !img.naturalWidth) return;
    const size = 64;
    const scale = size / Math.max(img.naturalWidth || 1, img.naturalHeight || 1);
    const canvas = document.createElement("canvas");
    canvas.width = Math.max(1, Math.round((img.naturalWidth || size) * scale));
    canvas.height = Math.max(1, Math.round((img.naturalHeight || size) * scale));
    try {
      canvas.getContext("2d")?.drawImage(img, 0, 0, canvas.width, canvas.height);
    } catch {
      return;
    }
    // Only drawn while in the document; gone once the browser has its picture.
    canvas.style.cssText = "position:fixed;left:-1000px;top:0;border:2px solid #fff;border-radius:3px";
    document.body.append(canvas);
    transfer.setDragImage(canvas, -14, canvas.height + 14);
    setTimeout(() => canvas.remove());
  }
</script>

<script lang="ts">
  import { untrack, type Snippet } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import * as api from "./lib/api";
  import { source } from "./lib/source.svelte";

  // Only the cells on screen (plus a few rows either side) exist in the page; the
  // rest of the result is a tall empty box that keeps the scrollbar honest.
  let {
    total, rowAt, need, selected, picked, queued, hidden = () => false, offline, onselect, onopen, ondragcell, children,
    tiles, shape, ontiles, layout = $bindable(),
  }: {
    total: number;
    /** The photo at a position, once its page has loaded. */
    rowAt: (i: number) => number | undefined;
    /** Positions [from, to) are about to show. */
    need: (from: number, to: number) => void;
    /** The focused position: the one the details panel and viewer show. */
    selected: number | null;
    /** Is the photo at a position in the selection (several can be)? */
    picked: (i: number) => boolean;
    /** Does it have edits waiting to be written to the source? */
    queued: (i: number) => boolean;
    /** Is it hidden (shown only while hidden photos are)? */
    hidden?: (i: number) => boolean;
    /** Is its original out of reach now (a drive that is not connected)? */
    offline?: (i: number) => boolean;
    /** A click, with its modifier keys for adding to the selection. */
    onselect: (i: number, how?: { add: boolean; range: boolean }) => void;
    onopen: (i: number) => void;
    /** A photo is being dragged (onto an album). */
    ondragcell?: (i: number) => void;
    children?: Snippet;
    tiles: Tiles;
    shape: Shape;
    /** Ctrl+wheel asks for bigger or smaller tiles. */
    ontiles: (tiles: Tiles) => void;
    layout?: Layout;
  } = $props();

  const GAP = 6;
  const PAD = 10;
  const OVERSCAN = 3;

  let el: HTMLElement;
  let width = $state(0);
  let height = $state(0);
  let top = $state(0);

  let room = $derived(Math.max(1, width - 2 * PAD));
  let cols = $derived(
    Math.max(1, Math.min(
      tiles.by === "count" ? tiles.n : Math.round((room + GAP) / (tiles.px + GAP)),
      Math.floor((room + GAP) / (MIN_TILE + GAP)) || 1,
    )),
  );
  let cellW = $derived(Math.max(1, (room - GAP * (cols - 1)) / cols));
  let cellH = $derived(Math.round(shape === "1:1" ? cellW : (cellW * 2) / 3));
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

  $effect(() => {
    layout = { cols, width: Math.round(cellW), room };
  });

  /** The tile width that gives `n` per row. */
  function widthFor(n: number) {
    return Math.max(MIN_TILE, Math.floor((room - GAP * (n - 1)) / n));
  }

  /** One step bigger (fewer per row) or smaller (more per row), keeping the mode. */
  export function step(bigger: boolean) {
    const n = Math.max(1, Math.min(MAX_COUNT, cols + (bigger ? -1 : 1)));
    if (n === cols && tiles.by === "count") return;
    ontiles(tiles.by === "count" ? { by: "count", n } : { by: "size", px: widthFor(n) });
  }

  // When tiles change size (or the window does), the photo in focus stays where it was
  // on screen, or else the row at the top stays at the top. That row is remembered
  // until the next scroll, so steps one after another don't drift.
  let before: { cols: number; lineH: number } | null = null;
  let anchor: { i: number; past: number } | null = null;
  let placed = -1;
  function scrolled() {
    top = el.scrollTop;
    if (Math.abs(el.scrollTop - placed) > 1) anchor = null;
  }
  $effect(() => {
    const now = { cols, lineH };
    untrack(() => {
      if (el && before && (before.cols !== now.cols || before.lineH !== now.lineH)) {
        const old = el.scrollTop;
        const lineOf = (i: number, l: { cols: number; lineH: number }) => PAD + Math.floor(i / l.cols) * l.lineH;
        const focusY = selected === null ? -1 : lineOf(selected, before) - old;
        if (selected !== null && focusY > -before.lineH && focusY < height) el.scrollTop = lineOf(selected, now) - focusY;
        else {
          if (!anchor) {
            // The row nearest the top edge, and how far it is past it.
            const at = Math.max(0, old - PAD) / before.lineH;
            const i = Math.round(at) * before.cols;
            anchor = { i, past: old - lineOf(i, before) };
          }
          el.scrollTop = lineOf(anchor.i, now) + anchor.past;
        }
        placed = el.scrollTop;
        top = el.scrollTop;
      }
      before = now;
    });
  });

  // Thumbnails are 450 px wide: tiles bigger than that take the preview once scrolling
  // stops, laid over the thumbnail when it arrives.
  const sharp = new SvelteSet<number>();
  let wantSharp = $derived(cellW * devicePixelRatio > 560);
  $effect(() => {
    if (!wantSharp) {
      untrack(() => sharp.clear());
      return;
    }
    // Only what is on screen: each preview takes a moment to render.
    const from = Math.floor((top - PAD) / lineH) * cols;
    const to = Math.ceil((top + height - PAD) / lineH) * cols;
    const rows = shown.filter((i) => i >= from && i < to).map(rowAt).filter((r) => r !== undefined);
    const wait = setTimeout(() => {
      if (sharp.size > 200) sharp.clear();
      for (const r of rows) sharp.add(r);
    }, 300);
    return () => clearTimeout(wait);
  });

  // Ctrl+wheel scales the tiles. Not a passive listener: the page must not zoom.
  $effect(() => {
    const wheel = (e: WheelEvent) => {
      if (!(e.ctrlKey || e.metaKey) || !e.deltaY) return;
      e.preventDefault();
      step(e.deltaY < 0);
    };
    el.addEventListener("wheel", wheel, { passive: false });
    return () => el.removeEventListener("wheel", wheel);
  });

  /** The positions on screen (with the rows kept above and below), [from, to). */
  export function range(): [number, number] {
    return [first * cols, Math.min(total, last * cols)];
  }

  /** Back to the top, for a new result. */
  export function reset() {
    if (el) el.scrollTop = 0;
    top = 0;
  }

  /** Moves the selection by arrow key; `range` (Shift) stretches the selection from where
   *  it started instead. Returns false for keys it does not handle. */
  export function move(key: string, range = false): boolean {
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
      onselect(to, range ? { add: false, range: true } : undefined);
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

  /** Scrolls position `i` to the middle, for a photo kept picked in a new result. */
  export function center(i: number) {
    if (!el) return;
    const y = PAD + Math.floor(i / cols) * lineH;
    el.scrollTop = Math.max(0, y - (height - cellH) / 2);
    top = el.scrollTop;
  }
</script>

<main bind:this={el} bind:clientWidth={width} bind:clientHeight={height} onscroll={scrolled}>
  {@render children?.()}
  <div class="space" style:height="{total ? lines * lineH - GAP + 2 * PAD : 0}px">
    {#each shown as i (i)}
      {@const row = rowAt(i)}
      <button
        class="cell"
        class:sel={selected === i}
        class:picked={picked(i)}
        class:offline={offline?.(i)}
        title={offline?.(i) ? "Offline: the original is not reachable now" : undefined}
        style:width="{cellW}px"
        style:height="{cellH}px"
        style:transform="translate({PAD + (i % cols) * (cellW + GAP)}px, {PAD + Math.floor(i / cols) * lineH}px)"
        onclick={(e) => onselect(i, { add: e.ctrlKey || e.metaKey, range: e.shiftKey })}
        ondblclick={() => onopen(i)}
        draggable={row !== undefined && !!ondragcell}
        ondragstart={(e) => {
          e.dataTransfer?.setData("text/plain", "photos");
          if (e.dataTransfer) e.dataTransfer.effectAllowed = "copy";
          ondragcell?.(i);
          const img = e.currentTarget.querySelector("img");
          if (e.dataTransfer && img) ghost(e.dataTransfer, img);
        }}
      >
        {#if row !== undefined}
          <img src={api.thumbUrl(row)} alt="" decoding="async" />
          {#if wantSharp && sharp.has(row)}
            <img class="sharp" src={api.previewUrl(row)} alt="" decoding="async" onload={(e) => e.currentTarget.classList.add("ready")} />
          {/if}
        {/if}
        {#if queued(i)}<span class="queued" title="Edited here, not yet written to {source.word}"></span>{/if}
        {#if hidden(i)}<span class="hidden-mark" title="Hidden: left out of every view unless Hidden is on">hidden</span>{/if}
      </button>
    {/each}
  </div>
</main>
