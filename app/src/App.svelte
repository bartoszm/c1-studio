<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { SvelteMap, SvelteSet } from "svelte/reactivity";
  import * as api from "./lib/api";
  import type { Dim, Filter, QueryResult } from "./lib/api";
  import { COLORS } from "./lib/format";
  import { albumTree, pathTree, type Node } from "./lib/tree";
  import Grid from "./Grid.svelte";
  import Info from "./Info.svelte";
  import Tree from "./Tree.svelte";
  import Viewer from "./Viewer.svelte";
  import WriteDialog from "./WriteDialog.svelte";

  const PAGE = 500;
  const DIMS: Dim[] = ["rating", "color", "year", "camera", "country", "keyword_tree", "collection", "folder"];
  /** Groups listed as plain values; the rest are trees. */
  const FLAT: Dim[] = ["rating", "color", "year", "camera", "country"];

  let status = $state<api.Status | null>(null);
  let error = $state<string | null>(null);
  let filter = $state<Filter>({});
  let search = $state("");
  let result = $state<QueryResult | null>(null);
  let collections = $state<api.Collection[]>([]);
  /** Keyword paths for completion, most used first. */
  let keywordList = $state<string[]>([]);
  let smart = $state<Map<string, number> | null>(null);
  /** Position of the photo picked in the grid; the viewer and the panel show it. */
  let selected = $state<number | null>(null);
  let viewing = $state(false);
  let syncNote = $state("syncing…");
  let grid: Grid;

  /** Rows of the photos picked for editing; a click picks one, Ctrl and Shift add. */
  const picked = new SvelteSet<number>();
  /** Where a Shift-click range starts. */
  let anchor: number | null = null;
  /** Row → fields edited here and not yet written to Capture One. */
  let queued = $state.raw(new Map<number, string[]>());
  let queuedCount = $state(0);
  /** Bumped after every edit, so the details panel reads the photo again. */
  let version = $state(0);
  let writing = $state(false);
  /** Write the queue to Capture One by itself; off unless turned on. */
  let autosaveOn = $state(false);
  let autosaveState = $state<api.AutosaveState>({ state: "off" });
  let editError = $state<string | null>(null);
  // Each entry undoes one edit (one key press or click, any number of photos).
  let undoStack: [number, api.Op][][] = [];
  let redoStack: [number, api.Op][][] = [];

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
  let showInfo = $state(!remembered("folded").includes("info"));
  $effect(() => {
    try {
      localStorage.setItem("expanded", JSON.stringify([...expanded]));
      localStorage.setItem("folded", JSON.stringify([...folded, ...(showInfo ? [] : ["info"])]));
    } catch {
      // Storage unavailable: the layout just isn't remembered.
    }
  });

  // Each query gets a number; answers to older ones are dropped.
  let generation = 0;
  // The result's photos, fetched a page at a time as the grid reaches them. A new result
  // swaps in a new map, so the variable itself must be reactive too: cells already on
  // screen would otherwise keep reading the old one and show the previous result's photos.
  let pages = $state.raw(new SvelteMap<number, number[]>());
  let loading = new Map<number, Promise<void>>();

  /** Runs the filter. `keep` re-runs it in place after an edit: same scroll, same picks. */
  async function run(keep = false) {
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
      if (keep) {
        if (selected !== null && selected >= r.total) selected = r.total ? r.total - 1 : null;
      } else {
        selected = null;
        picked.clear();
        anchor = null;
        viewing = false;
        grid?.reset();
      }
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

  /** Reads everything again; `keep` holds the scroll and selection (after a sync). */
  async function refresh(keep = false) {
    try {
      status = await api.status();
      collections = await api.collections();
      keywordList = (await api.keywords()).sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).map(([k]) => k);
      await refreshQueued();
      await run(keep);
    } catch (e) {
      error = String(e);
    }
  }

  onMount(() => {
    refresh();
    api.autosave().then(([on, state]) => { autosaveOn = on; autosaveState = state; }).catch(() => {});
    const off = [
      listen("library-changed", () => { syncNote = ""; refresh(true); }),
      listen<string>("sync-failed", (e) => { syncNote = `sync failed: ${e.payload}`; }),
      listen<api.AutosaveState>("autosave", (e) => { autosaveState = e.payload; }),
      listen("queue-written", () => { edited(); }),
    ];
    // The startup sync reports only changes; an unchanged catalog says nothing.
    const quiet = setTimeout(() => { if (syncNote === "syncing…") syncNote = ""; }, 3000);
    return () => { clearTimeout(quiet); off.forEach((p) => p.then((f) => f())); };
  });

  async function refreshQueued() {
    const list = await api.pending();
    const by = new Map<number, string[]>();
    for (const p of list) {
      if (p.row === null) continue;
      const field = p.key.slice(p.key.lastIndexOf("\u001f") + 1);
      by.set(p.row, [...(by.get(p.row) ?? []), field]);
    }
    queued = by;
    queuedCount = list.length;
  }

  /** The photos an edit applies to: the picked ones, else the focused one. */
  function targets(): number[] {
    if (picked.size) return [...picked];
    const row = selected === null ? undefined : rowAt(selected);
    return row === undefined ? [] : [row];
  }

  async function edited() {
    version++;
    await refreshQueued();
    await run(true);
  }

  async function applyEdit(edit: api.Edit) {
    const rows = targets();
    if (!rows.length) return;
    try {
      const undo = await api.edit(rows, edit);
      if (undo.length) {
        undoStack.push(undo);
        redoStack = [];
      }
      editError = null;
    } catch (e) {
      editError = String(e);
    }
    await edited();
  }

  async function undoRedo(from: [number, api.Op][][], to: [number, api.Op][][]) {
    const ops = from.pop();
    if (!ops) return;
    try {
      to.push(await api.editOps(ops));
    } catch (e) {
      editError = String(e);
    }
    await edited();
  }

  async function pick(i: number, how?: { add: boolean; range: boolean }) {
    const row = rowAt(i);
    if (how?.range && anchor !== null) {
      const [lo, hi] = anchor < i ? [anchor, i] : [i, anchor];
      await need(lo, hi + 1);
      if (!how.add) picked.clear();
      for (let k = lo; k <= hi; k++) {
        const r = rowAt(k);
        if (r !== undefined) picked.add(r);
      }
    } else if (how?.add && row !== undefined) {
      if (picked.size === 0 && selected !== null && rowAt(selected) !== undefined) picked.add(rowAt(selected)!);
      if (picked.has(row)) picked.delete(row);
      else picked.add(row);
      anchor = i;
    } else {
      picked.clear();
      if (row !== undefined) picked.add(row);
      anchor = i;
    }
    selected = i;
  }

  async function pickAll() {
    if (!result) return;
    const all = await api.query($state.snapshot(filter), { sort: "DateDesc" });
    picked.clear();
    for (const r of all.rows) picked.add(r);
  }

  // Capture One's keys: 0-5 rate, - + * tag red, green, yellow.
  const COLOR_KEYS: Record<string, api.Color> = { "-": "Red", "+": "Green", "*": "Yellow" };

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
    if (selected === null || !result) return;
    const to = selected + d;
    if (to < 0 || to >= result.total) return;
    await need(to, to + 2);
    selected = to;
  }

  function close() {
    if (selected !== null) grid?.reveal(selected);
    viewing = false;
  }

  async function toggleAutosave(on: boolean) {
    autosaveOn = on;
    try {
      await api.setAutosave(on);
    } catch (e) {
      editError = String(e);
    }
  }

  /** The footer's word on autosave, and whether clicking it should open the list. */
  function autosaveText(s: api.AutosaveState): [string, boolean] {
    switch (s.state) {
      case "pending": return ["saving to Capture One when you pause…", false];
      case "waiting":
        return [/captureone|imgcore/i.test(s.by) ? "autosave waits for Capture One to close" : "autosave waits: the catalog is open in another program", false];
      case "attention": {
        const parts = [s.conflicts && `${s.conflicts} conflict${s.conflicts > 1 ? "s" : ""}`, s.blocked && `${s.blocked} that cannot be written`];
        return [`autosave paused: ${parts.filter(Boolean).join(", ")}`, true];
      }
      case "saved": {
        const time = new Date(s.at).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
        return [`saved ${s.written} edit${s.written === 1 ? "" : "s"} to Capture One at ${time}`, false];
      }
      case "failed": return [`autosave failed: ${s.error}`, true];
      default: return ["", false];
    }
  }

  function onkey(e: KeyboardEvent) {
    if (writing || (e.target instanceof HTMLInputElement && e.target.type !== "checkbox")) return;
    const ctrl = e.ctrlKey || e.metaKey;
    const key = e.key.toLowerCase();
    if (ctrl && !e.altKey) {
      if (key === "z" && !e.shiftKey) undoRedo(undoStack, redoStack);
      else if (key === "y" || (key === "z" && e.shiftKey)) undoRedo(redoStack, undoStack);
      else if (key === "a" && !viewing) pickAll();
      else if (key === "k" && showInfo) document.getElementById("keyword-input")?.focus();
      else return;
      e.preventDefault();
      return;
    }
    if (e.altKey) return;
    if (/^[0-5]$/.test(e.key)) applyEdit({ Rating: +e.key || null });
    else if (e.key in COLOR_KEYS) applyEdit({ Color: COLOR_KEYS[e.key] });
    else if (e.key === "i") showInfo = !showInfo;
    else if (viewing) return; // the viewer has its own keys
    else if (e.key === "Escape" && picked.size > 1) {
      picked.clear();
      const row = selected === null ? undefined : rowAt(selected);
      if (row !== undefined) picked.add(row);
    } else if (e.key === "Enter" && selected !== null) viewing = true;
    else if (grid?.move(e.key)) e.preventDefault();
  }

  // Links in the info panel: filter by what was clicked.
  function only(f: Filter) {
    filter = { ...filter, ...f };
    run();
  }
  function showKeyword(path: string) {
    // Open the branches above it so the pick is visible in the tree.
    const parts = path.split("|");
    for (let i = 1; i < parts.length; i++) expanded.add("k:" + parts.slice(0, i).join("|"));
    only({ keywords_under: [path] });
  }
  function showFolder(path: string) {
    const parts = path.split("/");
    for (let i = 1; i < parts.length; i++) expanded.add("f:" + parts.slice(0, i).join("/"));
    only({ folder: path });
  }
  function showAlbum(id: string) {
    for (let c = byId.get(id); c?.parent; c = byId.get(c.parent)) expanded.add("a:" + c.parent);
    only({ collections: [id] });
  }

  let byId = $derived(new Map(collections.map((c) => [c.id, c])));

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

{#snippet details()}
  {@const row = selected === null ? undefined : rowAt(selected)}
  <Info
    {row}
    {version}
    picked={Math.max(picked.size, row === undefined ? 0 : 1)}
    queued={(row !== undefined && queued.get(row)) || []}
    onrate={(n) => applyEdit({ Rating: n })}
    oncolor={(c) => applyEdit({ Color: c })}
    keywords={keywordList}
    onaddkeyword={(path) => {
      if (!keywordList.includes(path)) keywordList = [...keywordList, path];
      applyEdit({ AddKeywords: [path] });
    }}
    onremovekeyword={(path) => applyEdit({ RemoveKeywords: [path] })}
    collections={byId}
    onkeyword={showKeyword}
    onalbum={showAlbum}
    onfolder={showFolder}
    oncamera={(camera) => only({ camera })}
  />
{/snippet}

<svelte:window onkeydown={onkey} />

<div class="app" class:with-info={showInfo}>
  <header>
    <span class="brand">C1 Studio</span>
    <input class="search" placeholder="Search keywords, captions, places, filenames" bind:value={search} oninput={onSearch} />
    {#if active}<button class="clear" onclick={clearAll}>Clear filters</button>{/if}
    <button class="toggle" class:on={showInfo} title="Details (i)" onclick={() => (showInfo = !showInfo)}>Details</button>
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

  <Grid
    bind:this={grid}
    total={result?.total ?? 0}
    {rowAt}
    {need}
    {selected}
    picked={(i) => picked.size > 1 && picked.has(rowAt(i) ?? -1)}
    queued={(i) => queued.has(rowAt(i) ?? -1)}
    onselect={pick}
    onopen={(i) => { pick(i); viewing = true; }}
  >
    {#if error}
      <p class="error">{error}</p>
    {:else if result && result.total === 0}
      <p class="empty">Nothing matches.</p>
    {/if}
  </Grid>

  {#if showInfo && !viewing}{@render details()}{/if}

  <footer>
    {#if status}
      <span>{status.source}</span>
      <span>{status.photos.toLocaleString()} in library</span>
      <span>Capture One: {status.writes.startsWith("Locked") ? "open" : "closed"}</span>
    {/if}
    {#if editError}<span class="edit-error">{editError}</span>{/if}
    {#if syncNote}<span class="sync">{syncNote}</span>{/if}
    <span class="autosave" class:alone={!syncNote}>
      {#if autosaveOn}
        {@const [note, attention] = autosaveText(autosaveState)}
        {#if attention}
          <button class="attention" onclick={() => (writing = true)}>{note}</button>
        {:else if note}
          <span>{note}</span>
        {/if}
      {/if}
      <label title="Write queued edits to Capture One by itself, a few seconds after you pause, whenever it is closed">
        <input type="checkbox" checked={autosaveOn} onchange={(e) => toggleAutosave(e.currentTarget.checked)} />
        Autosave
      </label>
    </span>
    {#if queuedCount}
      <button class="queue" onclick={() => (writing = true)}>
        {queuedCount.toLocaleString()} edit{queuedCount === 1 ? "" : "s"} waiting · Write to Capture One…
      </button>
    {/if}
  </footer>
</div>

{#if writing}
  <WriteDialog onclose={() => (writing = false)} onchanged={edited} />
{/if}

{#if viewing && selected !== null && rowAt(selected) !== undefined}
  <Viewer row={rowAt(selected)!} next={rowAt(selected + 1)} onclose={close} onstep={step}>
    {#if showInfo}{@render details()}{/if}
  </Viewer>
{/if}
