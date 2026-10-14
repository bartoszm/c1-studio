<script lang="ts">
  import { untrack } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import * as api from "./lib/api";
  import type { Filter } from "./lib/api";
  import { parseGeo } from "./lib/format";

  // The photos of the view the map cannot show: with no position at all, or with only a
  // place's name. Drag them onto the map, type where they were, or let their neighbours
  // in time say.
  let { filter, version, counts, ondragrows, onsetgeo, onneighbours, onshow, onpick, onerror }: {
    filter: Filter;
    version: number;
    /** From the map: photos with neither a position nor a place, and with a place only. */
    counts: { none: number; place: number };
    /** Photos picked here are being dragged (null: no longer). */
    ondragrows: (rows: number[] | null) => void;
    onsetgeo: (rows: number[], lat: number, lon: number) => void;
    onneighbours: () => void;
    /** Show the tab's photos in the grid. */
    onshow: (placed: boolean) => void;
    onpick: (row: number) => void;
    onerror: (text: string) => void;
  } = $props();

  const PAGE = 150;
  let open = $state(false);
  let tab = $state<"none" | "place">("none");
  let rows = $state<number[]>([]);
  let total = $state(0);
  let loading = $state(false);
  const picked = new SvelteSet<number>();
  let anchor: number | null = null;
  let typing = $state(false);
  let typed = $state("");
  let typedError = $state<string | null>(null);
  let asked = 0;

  let key = $derived(JSON.stringify({ ...filter, within: undefined }));

  async function load(more = false) {
    const mine = ++asked;
    loading = true;
    try {
      const shown = { ...$state.snapshot(filter), within: undefined, gps: false, placed: tab === "place" };
      const r = await api.query(shown, { sort: "Date", limit: PAGE, offset: more ? rows.length : 0 });
      if (mine !== asked) return;
      total = r.total;
      rows = more ? [...rows, ...r.rows] : r.rows;
      if (!more) {
        // What was picked and is still here stays picked.
        const here = new Set(rows);
        for (const row of [...picked]) if (!here.has(row)) picked.delete(row);
      }
    } catch (e) {
      if (mine === asked) onerror(String(e));
    } finally {
      if (mine === asked) loading = false;
    }
  }

  $effect(() => {
    void key;
    void version;
    void tab;
    if (open) untrack(() => load());
  });

  function choose(e: MouseEvent, i: number) {
    const row = rows[i];
    if (e.shiftKey && anchor !== null) {
      const [lo, hi] = anchor < i ? [anchor, i] : [i, anchor];
      if (!e.ctrlKey && !e.metaKey) picked.clear();
      for (const r of rows.slice(lo, hi + 1)) picked.add(r);
      return;
    }
    if (e.ctrlKey || e.metaKey) {
      if (picked.has(row)) picked.delete(row);
      else picked.add(row);
    } else {
      picked.clear();
      picked.add(row);
    }
    anchor = i;
  }

  function dragStart(e: DragEvent, row: number) {
    const moving = picked.has(row) ? [...picked] : [row];
    e.dataTransfer?.setData("text/plain", "photos");
    if (e.dataTransfer) e.dataTransfer.effectAllowed = "copy";
    ondragrows(moving);
  }

  function place(e: Event) {
    e.preventDefault();
    const at = parseGeo(typed);
    if (at === null) return;
    if (typeof at === "string") {
      typedError = at;
      return;
    }
    onsetgeo([...picked], at[0], at[1]);
    typing = false;
    typed = "";
    typedError = null;
  }

  function scrolled(e: Event) {
    const el = e.currentTarget as HTMLElement;
    if (!loading && rows.length < total && el.scrollLeft + el.clientWidth > el.scrollWidth - 400) load(true);
  }
</script>

<div class="tray" class:open>
  <div class="bar">
    <button class="fold" aria-expanded={open} onclick={() => (open = !open)}>
      {open ? "▾" : "▸"} Not on the map
      <span class="muted">{counts.none.toLocaleString()} without a location · {counts.place.toLocaleString()} with only a place</span>
    </button>
    {#if open}
      <span class="tabs" role="tablist">
        <button role="tab" aria-selected={tab === "none"} class:on={tab === "none"} onclick={() => (tab = "none")}>No location</button>
        <button role="tab" aria-selected={tab === "place"} class:on={tab === "place"} onclick={() => (tab = "place")}>Only a place</button>
      </span>
      <span class="grow"></span>
      {#if picked.size}
        <span class="muted">{picked.size.toLocaleString()} picked · drag onto the map</span>
        <button title="Type where they were taken: 50.0614, 19.9366 or 44°58′N 14°45′E" onclick={() => (typing = !typing)}>Coordinates…</button>
      {/if}
      <button title="Positions (or places) from the photos taken just before and after, with one" onclick={onneighbours}>From neighbours…</button>
      <button title="Show these photos in the grid" onclick={() => onshow(tab === "place")}>Show in grid</button>
    {/if}
  </div>
  {#if open}
    {#if typing && picked.size}
      <form class="typed" onsubmit={place}>
        <input
          placeholder="44°58′N 14°45′E"
          spellcheck="false"
          bind:value={typed}
          oninput={() => (typedError = null)}
          onkeydown={(e) => { if (e.key === "Escape") { typing = false; e.stopPropagation(); } }}
        />
        <button class="primary" disabled={!typed.trim()}>Place {picked.size.toLocaleString()}</button>
        {#if typedError}<span class="error">{typedError}</span>{/if}
      </form>
    {/if}
    <div class="strip" onscroll={scrolled}>
      {#each rows as row, i (row)}
        <button
          class="thumb"
          class:picked={picked.has(row)}
          draggable="true"
          title="Click to pick (Ctrl, Shift add) · drag onto the map · double-click for details"
          onclick={(e) => choose(e, i)}
          ondblclick={() => onpick(row)}
          ondragstart={(e) => dragStart(e, row)}
          ondragend={() => ondragrows(null)}
        ><img src={api.thumbUrl(row)} alt="" draggable="false" loading="lazy" /></button>
      {:else}
        <p class="muted empty">{loading ? "…" : tab === "none" ? "Every photo here has a position or a place." : "No photos here have only a place."}</p>
      {/each}
    </div>
  {/if}
</div>

<style>
  .tray {
    position: absolute; left: 8px; right: 8px; bottom: 32px; z-index: 4;
    background: rgba(20, 20, 20, 0.92); border: 1px solid var(--line, #444); border-radius: 4px; font-size: 12px;
  }
  .tray:not(.open) { right: auto; }
  .bar { display: flex; align-items: center; gap: 6px; padding: 3px 6px; flex-wrap: wrap; }
  .bar button { padding: 2px 8px; border-radius: 3px; border: 1px solid var(--line, #444); }
  .bar button:hover { background: var(--hover, #333); }
  .bar .fold { border: 0; padding: 2px 4px; }
  .muted { color: var(--muted, #8a8a8a); }
  .grow { flex: 1; }
  .tabs { display: inline-flex; gap: 2px; }
  .tabs .on { background: var(--queued, #3b82f6); border-color: var(--queued, #3b82f6); color: #fff; }
  .strip { display: flex; gap: 4px; overflow-x: auto; padding: 4px 6px 6px; }
  .thumb { flex: none; width: 64px; height: 64px; padding: 0; border: 2px solid transparent; border-radius: 3px; background: #333; cursor: grab; }
  .thumb img { display: block; width: 100%; height: 100%; object-fit: cover; border-radius: 1px; }
  .thumb.picked { border-color: #fbbf24; }
  .empty { margin: 8px; }
  .typed { display: flex; align-items: center; gap: 6px; padding: 0 6px 4px; }
  .typed input { width: 220px; padding: 2px 6px; font: inherit; color: inherit; background: var(--bg, #111); border: 1px solid var(--line, #444); border-radius: 3px; }
  .typed .primary { padding: 2px 8px; border: 0; border-radius: 3px; background: var(--queued, #3b82f6); color: #fff; font-weight: 600; }
  .typed .primary:disabled { opacity: 0.4; }
  .error { color: #f87171; }
</style>
