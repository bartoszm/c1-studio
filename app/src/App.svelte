<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { SvelteMap, SvelteSet } from "svelte/reactivity";
  import * as api from "./lib/api";
  import type { Dim, Filter, QueryResult } from "./lib/api";
  import { albumTree, pathTree, type Node } from "./lib/tree";
  import Grid from "./Grid.svelte";
  import Tree from "./Tree.svelte";
  import Viewer from "./Viewer.svelte";

  const PAGE = 500;
  const DIMS: Dim[] = ["rating", "color", "year", "camera", "country", "keyword_tree", "collection", "folder"];
  /** Groups listed as plain values; the rest are trees. */
  const FLAT: Dim[] = ["rating", "color", "year", "camera", "country"];
  const COLORS: Record<string, string> = {
    red: "#d9534f", orange: "#e8913a", yellow: "#e8d23a", green: "#5cb85c",
    blue: "#4a90d9", pink: "#e87ab8", purple: "#9b6bd6", none: "transparent",
  };

  let status = $state<api.Status | null>(null);
  let error = $state<string | null>(null);
  let filter = $state<Filter>({});
  let search = $state("");
  let result = $state<QueryResult | null>(null);
  let collections = $state<api.Collection[]>([]);
  let smart = $state<Map<string, number> | null>(null);
  let viewing = $state<number | null>(null);
  let syncNote = $state("syncing…");
  let grid: Grid;

  // Sidebar layout survives restarts: which branches are open, which sections folded.
  function remembered(key: string): string[] {
    try {
      return JSON.parse(localStorage.getItem(key) ?? "[]");
    } catch {
      return [];
    }
  }
  const expanded = new SvelteSet<string>(remembered("expanded"));
  const folded = new SvelteSet<string>(remembered("folded"));
  $effect(() => {
    try {
      localStorage.setItem("expanded", JSON.stringify([...expanded]));
      localStorage.setItem("folded", JSON.stringify([...folded]));
    } catch {
      // Storage unavailable: the layout just isn't remembered.
    }
  });

  // Each query gets a number; answers to older ones are dropped.
  let generation = 0;
  // The result's photos, fetched a page at a time as the grid reaches them.
  let pages = new SvelteMap<number, number[]>();
  let loading = new Map<number, Promise<void>>();

  async function run() {
    const mine = ++generation;
    const asked = $state.snapshot(filter);
    // Smart albums are counted apart: one query each, so they never hold up the grid.
    api.smartCounts(asked).then((c) => { if (mine === generation) smart = new Map(c); }).catch(() => {});
    try {
      const r = await api.query(asked, {
        sort: "DateDesc", limit: PAGE, offset: 0, facets: DIMS, facets_ignore_own: true,
      });
      if (mine !== generation) return;
      result = r;
      pages = new SvelteMap([[0, r.rows]]);
      loading = new Map();
      error = null;
      grid?.reset();
    } catch (e) {
      if (mine === generation) error = String(e);
    }
  }

  const rowAt = (i: number) => pages.get(Math.floor(i / PAGE))?.[i % PAGE];

  function load(page: number): Promise<void> {
    const pending = loading.get(page);
    if (pending) return pending;
    const mine = generation;
    const into = pages;
    const p = api.query($state.snapshot(filter), { sort: "DateDesc", limit: PAGE, offset: page * PAGE })
      .then((r) => { if (mine === generation) into.set(page, r.rows); })
      .catch((e) => { if (mine === generation) error = String(e); });
    loading.set(page, p);
    return p;
  }

  /** Loads whatever of positions [from, to) is missing. */
  function need(from: number, to: number): Promise<unknown> {
    const wanted = [];
    for (let page = Math.floor(from / PAGE); page * PAGE < to; page++) {
      if (!pages.has(page)) wanted.push(load(page));
    }
    return Promise.all(wanted);
  }

  async function refresh() {
    try {
      status = await api.status();
      collections = await api.collections();
      await run();
    } catch (e) {
      error = String(e);
    }
  }

  onMount(() => {
    refresh();
    const off = [
      listen("library-changed", () => { syncNote = ""; refresh(); }),
      listen<string>("sync-failed", (e) => { syncNote = `sync failed: ${e.payload}`; }),
    ];
    // The startup sync reports only changes; an unchanged catalog says nothing.
    const quiet = setTimeout(() => { if (syncNote === "syncing…") syncNote = ""; }, 3000);
    return () => { clearTimeout(quiet); off.forEach((p) => p.then((f) => f())); };
  });

  // Typing searches after a short pause.
  let typing: ReturnType<typeof setTimeout>;
  function onSearch() {
    clearTimeout(typing);
    typing = setTimeout(() => {
      filter.text = search.trim() || undefined;
      run();
    }, 150);
  }

  // Flat sidebar values ↔ filter fields.
  function filterFor(dim: Dim, value: string): Filter {
    switch (dim) {
      case "rating": return { rating: value === "none" ? "None" : { Values: { min: +value, max: +value } } };
      case "color": return { color: value === "none" ? "None" : { Values: [(value[0].toUpperCase() + value.slice(1)) as api.Color] } };
      case "year": return { year: +value };
      case "camera": return { camera: value };
      case "country": return { country: value };
      default: return {};
    }
  }

  function isActive(dim: Dim, value: string) {
    const want = filterFor(dim, value);
    const key = Object.keys(want)[0] as keyof Filter;
    return JSON.stringify(filter[key]) === JSON.stringify(want[key]);
  }

  function toggle(dim: Dim, value: string) {
    const want = filterFor(dim, value);
    const key = Object.keys(want)[0] as keyof Filter;
    filter = isActive(dim, value) ? { ...filter, [key]: undefined } : { ...filter, ...want };
    run();
  }

  // Trees: picking a node filters by it, picking it again lets go.
  function pickAlbum(n: Node) {
    filter = { ...filter, collections: filter.collections?.[0] === n.value ? undefined : [n.value] };
    run();
  }
  function pickFolder(n: Node) {
    filter = { ...filter, folder: filter.folder === n.value ? undefined : n.value };
    run();
  }
  function pickKeyword(n: Node) {
    filter = { ...filter, keywords_under: filter.keywords_under?.[0] === n.value ? undefined : [n.value] };
    run();
  }

  function clearAll() {
    filter = {};
    search = "";
    run();
  }

  /** The values a flat group shows. */
  function values(dim: Dim): [string, number][] {
    const all = result?.facets[dim] ?? [];
    if (dim === "year" || dim === "rating") return [...all].sort((a, b) => b[0].localeCompare(a[0]));
    return all.slice(0, 10);
  }

  function label(dim: Dim, value: string) {
    if (dim === "rating") return value === "none" ? "unrated" : "★".repeat(+value);
    return value;
  }

  const TITLES: Record<string, string> = {
    rating: "Rating", color: "Colour", year: "Year", camera: "Camera", country: "Country",
    albums: "Albums", folders: "Folders", keywords: "Keywords",
  };

  let albums = $derived(albumTree(collections, new Map(result?.facets.collection ?? []), smart));
  let folders = $derived(pathTree(result?.facets.folder ?? [], "/", "f:", "folder"));
  let keywords = $derived(pathTree(result?.facets.keyword_tree ?? [], "|", "k:", "keyword"));

  let active = $derived(Object.values(filter).some((v) => v !== undefined));

  async function step(d: number) {
    if (viewing === null || !result) return;
    const to = viewing + d;
    if (to < 0 || to >= result.total) return;
    await need(to, to + 2);
    viewing = to;
  }

  function close() {
    if (viewing !== null) grid?.reveal(viewing);
    viewing = null;
  }

  function fold(key: string) {
    if (folded.has(key)) folded.delete(key);
    else folded.add(key);
  }
</script>

{#snippet head(key: string)}
  <button class="head" onclick={() => fold(key)}>
    <span class="chev" class:open={!folded.has(key)}>›</span>{TITLES[key]}
  </button>
{/snippet}

<div class="app">
  <header>
    <span class="brand">C1 Studio</span>
    <input class="search" placeholder="Search keywords, captions, places, filenames" bind:value={search} oninput={onSearch} />
    {#if active}<button class="clear" onclick={clearAll}>Clear filters</button>{/if}
    <span class="count">
      {#if result}{result.total.toLocaleString()} photos · {(result.micros / 1000).toFixed(1)} ms{/if}
    </span>
  </header>

  <aside>
    <section>
      {@render head("albums")}
      {#if !folded.has("albums")}
        <Tree nodes={albums} selected={filter.collections && "a:" + filter.collections[0]} {expanded} onpick={pickAlbum} />
      {/if}
    </section>
    <section>
      {@render head("folders")}
      {#if !folded.has("folders")}
        <Tree nodes={folders} selected={filter.folder && "f:" + filter.folder} {expanded} onpick={pickFolder} />
      {/if}
    </section>
    <section>
      {@render head("keywords")}
      {#if !folded.has("keywords")}
        <Tree nodes={keywords} selected={filter.keywords_under && "k:" + filter.keywords_under[0]} {expanded} onpick={pickKeyword} />
      {/if}
    </section>
    {#each FLAT as dim}
      {@const list = values(dim)}
      {#if list.length}
        <section>
          {@render head(dim)}
          {#if !folded.has(dim)}
            {#each list as [value, n]}
              <button class="facet" class:on={isActive(dim, value)} onclick={() => toggle(dim, value)}>
                {#if dim === "color"}<i class="dot" style:background={COLORS[value]}></i>{/if}
                <span class="name" class:stars={dim === "rating" && value !== "none"}>{label(dim, value)}</span>
                <span class="n">{n.toLocaleString()}</span>
              </button>
            {/each}
          {/if}
        </section>
      {/if}
    {/each}
  </aside>

  <Grid bind:this={grid} total={result?.total ?? 0} {rowAt} {need} onopen={(i) => (viewing = i)}>
    {#if error}
      <p class="error">{error}</p>
    {:else if result && result.total === 0}
      <p class="empty">Nothing matches.</p>
    {/if}
  </Grid>

  <footer>
    {#if status}
      <span>{status.source}</span>
      <span>{status.photos.toLocaleString()} in library</span>
      <span>Capture One: {status.writes.startsWith("Locked") ? "open" : "closed"}</span>
    {/if}
    {#if syncNote}<span class="sync">{syncNote}</span>{/if}
  </footer>
</div>

{#if viewing !== null && rowAt(viewing) !== undefined}
  <Viewer row={rowAt(viewing)!} next={rowAt(viewing + 1)} onclose={close} onstep={step} />
{/if}
