<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { SvelteMap } from "svelte/reactivity";
  import * as api from "./lib/api";
  import type { Dim, Filter, QueryResult } from "./lib/api";
  import Grid from "./Grid.svelte";
  import Viewer from "./Viewer.svelte";

  const PAGE = 500;
  const DIMS: Dim[] = ["rating", "color", "year", "camera", "keyword_tree", "collection", "country"];
  const COLORS: Record<string, string> = {
    red: "#d9534f", orange: "#e8913a", yellow: "#e8d23a", green: "#5cb85c",
    blue: "#4a90d9", pink: "#e87ab8", purple: "#9b6bd6", none: "transparent",
  };

  let status = $state<api.Status | null>(null);
  let error = $state<string | null>(null);
  let filter = $state<Filter>({});
  let search = $state("");
  let result = $state<QueryResult | null>(null);
  let names = $state(new Map<string, string>());
  let viewing = $state<number | null>(null);
  let syncNote = $state("syncing…");
  let grid: Grid;

  // Each query gets a number; answers to older ones are dropped.
  let generation = 0;
  // The result's photos, fetched a page at a time as the grid reaches them.
  let pages = new SvelteMap<number, number[]>();
  let loading = new Map<number, Promise<void>>();

  async function run() {
    const mine = ++generation;
    try {
      const r = await api.query($state.snapshot(filter), {
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
      names = new Map((await api.collections()).map((c) => [c.id, c.path]));
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

  // Sidebar values ↔ filter fields.
  function filterFor(dim: Dim, value: string): Filter {
    switch (dim) {
      case "rating": return { rating: value === "none" ? "None" : { Values: { min: +value, max: +value } } };
      case "color": return { color: value === "none" ? "None" : { Values: [(value[0].toUpperCase() + value.slice(1)) as api.Color] } };
      case "year": return { year: +value };
      case "camera": return { camera: value };
      case "country": return { country: value };
      case "keyword_tree": return { keywords_under: [value] };
      case "collection": return { collections: [value] };
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
    if (isActive(dim, value)) {
      // Clicking an active keyword steps back up its tree instead of clearing it.
      const parent = dim === "keyword_tree" && value.includes("|") ? value.slice(0, value.lastIndexOf("|")) : null;
      filter = { ...filter, [key]: parent ? [parent] : undefined };
    } else {
      filter = { ...filter, ...want };
    }
    run();
  }

  function clearAll() {
    filter = {};
    search = "";
    run();
  }

  /** The values a sidebar group shows: keywords one level below the current one. */
  function values(dim: Dim): [string, number][] {
    const all = result?.facets[dim] ?? [];
    if (dim === "year") return [...all].sort((a, b) => b[0].localeCompare(a[0]));
    if (dim === "rating") return [...all].sort((a, b) => b[0].localeCompare(a[0]));
    if (dim === "keyword_tree") {
      const at = filter.keywords_under?.[0];
      const depth = at ? at.split("|").length : 0;
      return all.filter(([v]) => v.split("|").length === depth + 1 && (!at || v.startsWith(at + "|"))).slice(0, 20);
    }
    return all.slice(0, dim === "collection" ? 15 : 10);
  }

  function label(dim: Dim, value: string) {
    if (dim === "rating") return value === "none" ? "unrated" : "★".repeat(+value);
    if (dim === "collection") return names.get(value) ?? `#${value}`;
    if (dim === "keyword_tree") return value.split("|").pop();
    return value;
  }

  const TITLES: Record<string, string> = {
    rating: "Rating", color: "Colour", year: "Year", camera: "Camera",
    keyword_tree: "Keywords", collection: "Albums", country: "Country",
  };

  /** Breadcrumb of the keyword branch being browsed. */
  function keywordTrail(): { name: string; path: string | null }[] {
    const at = filter.keywords_under?.[0];
    if (!at) return [];
    const parts = at.split("|");
    return [{ name: "All keywords", path: null }, ...parts.map((p, i) => ({ name: p, path: parts.slice(0, i + 1).join("|") }))];
  }

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
</script>

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
    {#if keywordTrail().length}
      <div class="trail">
        {#each keywordTrail() as step}
          <button onclick={() => { filter = { ...filter, keywords_under: step.path ? [step.path] : undefined }; run(); }}>{step.name}</button>
        {/each}
      </div>
    {/if}
    {#each DIMS as dim}
      {@const list = values(dim)}
      {#if list.length}
        <section>
          <h3>{TITLES[dim]}</h3>
          {#each list as [value, n]}
            <button class="facet" class:on={isActive(dim, value)} onclick={() => toggle(dim, value)}>
              {#if dim === "color"}<i class="dot" style:background={COLORS[value]}></i>{/if}
              <span class="name" class:stars={dim === "rating" && value !== "none"}>{label(dim, value)}</span>
              <span class="n">{n.toLocaleString()}</span>
            </button>
          {/each}
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
