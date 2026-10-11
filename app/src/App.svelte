<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { SvelteMap, SvelteSet } from "svelte/reactivity";
  import * as api from "./lib/api";
  import type { Dim, Filter, QueryResult } from "./lib/api";
  import { COLORS, FIELD_NAMES, stars } from "./lib/format";
  import { dateTree, weekText } from "./lib/dates";
  import { albumTree, pathTree, placeText, placeTree, searchTree, type Node } from "./lib/tree";
  import Grid, { DEFAULT_TILES, MAX_COUNT, MIN_TILE, type Layout, type Shape, type Tiles } from "./Grid.svelte";
  import AlbumDialog, { rows as ruleRows } from "./AlbumDialog.svelte";
  import BackupsDialog from "./BackupsDialog.svelte";
  import CopyDialog from "./CopyDialog.svelte";
  import CountryCodesDialog from "./CountryCodesDialog.svelte";
  import PlaceDialog from "./PlaceDialog.svelte";
  import Info from "./Info.svelte";
  import SourceMenu from "./SourceMenu.svelte";
  import KeywordDialog from "./KeywordDialog.svelte";
  import Tree from "./Tree.svelte";
  import Viewer from "./Viewer.svelte";
  import WriteDialog from "./WriteDialog.svelte";

  const PAGE = 500;
  const DIMS: Dim[] = ["rating", "color", "year", "month", "week", "camera", "country", "keyword_tree", "collection", "folder", "place_tree", "flag"];
  /** Groups listed as plain values; the rest are trees. */
  const FLAT: Dim[] = ["rating", "color", "camera"];

  let status = $state<api.Status | null>(null);
  /** The source menu in the header; open by itself when nothing could be opened. */
  let choosingSource = $state(false);
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
  /** Why reading the source failed, in full: shown in the grid when it is left empty. */
  let syncFailed = $state<string | null>(null);
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
  let showBackups = $state(false);
  /** Write the queue to Capture One by itself; off unless turned on. */
  let autosaveOn = $state(false);
  let autosaveState = $state<api.AutosaveState>({ state: "off" });
  let editError = $state<string | null>(null);
  /** A word on what an edit did, shown a few seconds in the footer. */
  let editNote = $state<string | null>(null);
  let noteTimer: ReturnType<typeof setTimeout>;
  function note(text: string) {
    editNote = text;
    clearTimeout(noteTimer);
    noteTimer = setTimeout(() => (editNote = null), 4000);
  }
  /** What is being dragged: photos (onto an album) or an album (onto a group). */
  let dragging = $state<{ rows: number[] } | { album: string } | null>(null);
  // Each entry undoes one edit (one key press or click, any number of photos).
  type Done =
    | { ops: [number, api.Op][] }
    | { remap: number; request: api.RemapRequest }
    | { collection: number; change: api.CollectionChange };
  type Undone = { ops: [number, api.Op][] } | { request: api.RemapRequest } | { change: api.CollectionChange };
  let undoStack: Done[] = [];
  let redoStack: Undone[] = [];

  /** Keyword tree changes queued for Capture One. */
  let remapCount = $state(0);
  let keywordSearch = $state("");
  let keywordMenu = $state<{ x: number; y: number; node: Node } | null>(null);
  /** The Location tree's right-click menu, and its rename / join dialog. */
  let placeMenu = $state<{ x: number; y: number; node: Node } | null>(null);
  let placeDialog = $state<{ mode: "rename" | "join"; path: string } | null>(null);
  let keywordDialog = $state<{ mode: "rename" | "join" | "group"; start: string } | null>(null);
  /** Albums with changes queued for Capture One (made, renamed, given a rule). */
  let queuedAlbums = $state(new Set<string>());
  /** The Albums tree's menu: on an album, or (`node` null) from the section's + button. */
  let albumMenu = $state<{ x: number; y: number; node: Node | null } | null>(null);
  let albumDialog = $state<{
    mode: "create" | "rename" | "rule" | "delete" | "move";
    kind?: "Album" | "Smart" | "Group";
    collection?: api.Collection;
  } | null>(null);

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
  // Tile size and shape, remembered too.
  function rememberedTiles(): { tiles: Tiles; shape: Shape } {
    try {
      const t = JSON.parse(localStorage.getItem("tiles") ?? "null");
      const tiles: Tiles =
        t?.by === "count" && t.n >= 1 ? { by: "count", n: Math.min(MAX_COUNT, Math.round(t.n)) }
        : t?.by === "size" && t.px >= MIN_TILE ? { by: "size", px: Math.round(t.px) }
        : DEFAULT_TILES;
      return { tiles, shape: t?.shape === "1:1" ? "1:1" : "3:2" };
    } catch {
      return { tiles: DEFAULT_TILES, shape: "3:2" };
    }
  }
  const start = rememberedTiles();
  let tiles = $state<Tiles>(start.tiles);
  let shape = $state<Shape>(start.shape);
  let layout = $state<Layout>({ cols: 1, width: 170, room: 1000 });
  $effect(() => {
    try {
      localStorage.setItem("tiles", JSON.stringify({ ...tiles, shape }));
    } catch {
      // Not remembered, that's all.
    }
  });
  /** Tiles per row from the box in the header: kept as the window changes. */
  function setCount(value: string) {
    const n = Math.round(+value);
    if (n >= 1) tiles = { by: "count", n: Math.min(MAX_COUNT, n) };
  }

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
        await followPicks(mine, asked);
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

  /**
   * After an edit re-ran the filter in place: photos that left the result leave the
   * selection, and a single selection follows the focus to whatever photo now sits there
   * (removing `animal` under an `animal` filter moves on to the next photo).
   */
  async function followPicks(mine: number, asked: Filter) {
    if (picked.size > 1) {
      const all = new Set((await api.query(asked, { sort: "DateDesc" })).rows);
      if (mine !== generation) return;
      for (const row of [...picked]) if (!all.has(row)) picked.delete(row);
    }
    if (picked.size <= 1) {
      if (selected !== null) await need(selected, selected + 1);
      if (mine !== generation) return;
      picked.clear();
      const row = selected === null ? undefined : rowAt(selected);
      if (row !== undefined) picked.add(row);
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
      // A sync that failed before the window was listening (at start, or on opening
      // another catalog) still says so.
      syncFailed = status.sync_error;
      if (syncFailed) syncNote = "sync failed";
      await refreshCollections();
      await refreshKeywords();
      await refreshQueued();
      await run(keep);
    } catch (e) {
      error = String(e);
      if (!status) choosingSource = true;
    }
  }

  onMount(() => {
    refresh();
    api.autosave().then(([on, state]) => { autosaveOn = on; autosaveState = state; }).catch(() => {});
    const off = [
      listen("library-changed", () => { syncNote = ""; syncFailed = null; refresh(true); }),
      listen<string>("sync-failed", (e) => { syncNote = "sync failed"; syncFailed = e.payload; }),
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
    remapCount = (await api.remaps()).length;
    const albumChanges = await api.collectionChanges();
    queuedAlbums = new Set(albumChanges.map(([, , id]) => id));
    queuedCount = list.length + remapCount + albumChanges.length;
  }

  /** The albums, after a change here or a sync. An album picked in the filter that is
   *  gone (deleted, or written and back under Capture One's own id) is let go. */
  async function refreshCollections() {
    collections = await api.collections();
    const ids = new Set(collections.map((c) => c.id));
    const kept = filter.collections?.filter((id) => ids.has(id));
    if (kept && kept.length !== filter.collections?.length) filter = { ...filter, collections: kept.length ? kept : undefined };
  }

  /** Queues an album change (shown at once), undoable like any edit. `fill` puts the
   *  picked photos into a new album as well. */
  async function applyCollection(change: api.CollectionChange, fill = false) {
    const [seq, queuedChange] = await api.changeCollection(change);
    undoStack.push({ collection: seq, change: queuedChange });
    redoStack = [];
    editError = null;
    await refreshCollections();
    if (fill && "Create" in queuedChange) await applyEdit({ AddToAlbum: queuedChange.Create.id });
    else await edited();
  }

  async function refreshKeywords() {
    keywordList = (await api.keywords()).sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).map(([k]) => k);
  }

  /** Queues a keyword tree change (shown at once), undoable like any edit. */
  async function applyRemap(request: api.RemapRequest) {
    const seq = await api.remapKeywords(request);
    undoStack.push({ remap: seq, request });
    redoStack = [];
    editError = null;
    await refreshKeywords();
    await edited();
  }

  async function quickRemap(request: api.RemapRequest) {
    keywordMenu = null;
    try {
      await applyRemap(request);
    } catch (e) {
      editError = String(e);
    }
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

  // Copy and paste between photos. The clipboard holds the photo as it was when copied,
  // and which fields to take from it; it survives restarts.
  type Clipboard = { from: api.Photo; fields: api.CopyField[] };
  function rememberedClip(): Clipboard | null {
    try {
      const c = JSON.parse(localStorage.getItem("clipboard") ?? "null");
      return c?.from && Array.isArray(c.fields) ? c : null;
    } catch {
      return null;
    }
  }
  let clipboard = $state<Clipboard | null>(rememberedClip());
  let copying = $state<api.Photo | null>(null);
  /** The keywords across the picked photos, with how many carry each; null for one. */
  let selectionKeywords = $state<[string, number][] | null>(null);
  let keywordsAsked = 0;
  $effect(() => {
    const rows = [...picked];
    void version;
    if (rows.length < 2 || !showInfo) {
      selectionKeywords = null;
      return;
    }
    // A moment's wait: Shift+arrow picks change faster than a big selection reads.
    const mine = ++keywordsAsked;
    const wait = setTimeout(() => {
      api.keywordCounts(rows).then((c) => { if (mine === keywordsAsked) selectionKeywords = c; }).catch(() => {});
    }, 80);
    return () => clearTimeout(wait);
  });
  /** The albums the picked photos are in, with how many each holds; null for one. */
  let selectionAlbums = $state<[string, number][] | null>(null);
  let albumsAsked = 0;
  $effect(() => {
    const rows = [...picked];
    void version;
    if (rows.length < 2 || !showInfo) {
      selectionAlbums = null;
      return;
    }
    const mine = ++albumsAsked;
    const wait = setTimeout(() => {
      api.albumCounts(rows).then((c) => { if (mine === albumsAsked) selectionAlbums = c; }).catch(() => {});
    }, 80);
    return () => clearTimeout(wait);
  });
  /** Sync: the focused photo, whose fields go onto the rest of the selection. */
  let syncing = $state<{ from: api.Photo; row: number } | null>(null);
  /** Paste…: the photos the clipboard goes onto, with a choice of fields and how. */
  let pasting = $state<number[] | null>(null);
  /** What was pasted last, and how: Ctrl+V does the same again. */
  function rememberedPaste(): api.CopyField[] | null {
    try {
      const f = JSON.parse(localStorage.getItem("pasteFields") ?? "null");
      return Array.isArray(f) ? f : null;
    } catch {
      return null;
    }
  }
  let pasteFields = $state<api.CopyField[] | null>(rememberedPaste());
  $effect(() => {
    try {
      localStorage.setItem("pasteFields", JSON.stringify(pasteFields));
    } catch {
      // Not remembered, that's all.
    }
  });

  /** The tile's ↑: copies at once, the fields copied last (or all the photo has). */
  async function copyCell(i: number) {
    const row = rowAt(i);
    if (row === undefined) return;
    const from = await api.photo(row);
    if (!from) return;
    const fields = clipboard?.fields ?? [
      "rating", "color", "keywords", "country", "country_code", "state", "city", "location", "geo",
      "headline", "description", "title", "job_id", "instructions", "source", "copyright", "creator",
    ];
    clipboard = { from, fields };
    pasteFields = null;
    note(`Copied from ${from.file.name}: choose what to paste with ↓ or Ctrl+Shift+V`);
  }

  /** The tile's ↓: onto the selection if the tile is in it, else onto that photo. */
  function pasteCell(i: number) {
    const row = rowAt(i);
    if (row === undefined || !clipboard) return;
    pasting = picked.has(row) && picked.size > 1 ? [...picked] : [row];
  }
  $effect(() => {
    try {
      localStorage.setItem("clipboard", JSON.stringify(clipboard));
    } catch {
      // Not remembered across restarts, that's all.
    }
  });
  let clipText = $derived(
    clipboard &&
      `${clipboard.from.file.name}: ${[...new Set(clipboard.fields.map((f) => FIELD_NAMES[f] ?? f))].join(", ")}`,
  );

  async function openCopy() {
    const row = selected === null ? undefined : rowAt(selected);
    if (row === undefined) return;
    copying = await api.photo(row);
  }

  async function openSync() {
    const row = selected === null ? undefined : rowAt(selected);
    if (row === undefined || picked.size < 2) return;
    const from = await api.photo(row);
    if (from) syncing = { from, row };
  }

  /** Puts fields of `from` on the rest of the selection; they stay on the clipboard too. */
  async function sync({ from, row }: { from: api.Photo; row: number }, fields: api.CopyField[], fill: boolean) {
    clipboard = { from, fields };
    pasteFields = null;
    await pasteRows(targets().filter((r) => r !== row), from, fields, fill);
  }

  async function pasteClip(fill = false) {
    if (clipboard) await pasteRows(targets(), clipboard.from, pasteFields ?? clipboard.fields, fill);
  }

  async function pasteRows(rows: number[], from: api.Photo, fields: api.CopyField[], fill: boolean) {
    if (!rows.length) return;
    try {
      const undo = await api.paste(rows, from, fields, fill);
      if (undo.length) {
        undoStack.push({ ops: undo });
        redoStack = [];
        const n = new Set(undo.map(([r]) => r)).size;
        note(`${fill ? "Merged into" : "Pasted onto"} ${n.toLocaleString()} photo${n === 1 ? "" : "s"}`);
      } else {
        note(`Nothing to ${fill ? "merge" : "paste"}: ${rows.length === 1 ? "the photo holds" : "the photos hold"} these values already`);
      }
      editError = null;
    } catch (e) {
      editError = String(e);
    }
    await edited();
  }

  async function applyEdit(edit: api.Edit, rows = targets()) {
    if (!rows.length) return;
    try {
      const undo = await api.edit(rows, edit);
      if (undo.length) {
        undoStack.push({ ops: undo });
        redoStack = [];
      }
      editError = null;
    } catch (e) {
      editError = String(e);
    }
    await edited();
  }

  async function undo() {
    const last = undoStack.pop();
    if (!last) return;
    try {
      if ("ops" in last) redoStack.push({ ops: await api.editOps(last.ops) });
      else if ("collection" in last) {
        await api.undoCollection(last.collection);
        redoStack.push({ change: last.change });
        await refreshCollections();
      } else {
        // The photos it touched are read from Capture One again: a moment on a big catalog.
        syncNote = "undoing the keyword change…";
        await api.undoRemap(last.remap);
        syncNote = "";
        redoStack.push({ request: last.request });
        version++;
        await refresh(true);
        return;
      }
      editError = null;
    } catch (e) {
      editError = String(e);
      syncNote = "";
    }
    await edited();
  }

  async function redo() {
    const last = redoStack.pop();
    if (!last) return;
    try {
      if ("ops" in last) undoStack.push({ ops: await api.editOps(last.ops) });
      else if ("change" in last) {
        // The same id as before, so photo edits redone after it still find the album.
        const [seq, change] = await api.changeCollection(last.change);
        undoStack.push({ collection: seq, change });
        await refreshCollections();
      } else {
        undoStack.push({ remap: await api.remapKeywords(last.request), request: last.request });
        await refreshKeywords();
      }
      editError = null;
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
  function pickPlace(n: Node) {
    filter = { ...filter, place_under: filter.place_under === n.value ? undefined : n.value };
    run();
  }
  /** Photos in view with a country but no ISO code (from the index's flags). */
  let missingCodes = $derived(result?.facets.flag?.find(([v]) => v === "no_country_code")?.[1] ?? 0);
  let fillingCodes = $state(false);
  async function codesFilled(undo: [number, api.Op][]) {
    if (undo.length) {
      undoStack.push({ ops: undo });
      redoStack = [];
    }
    await edited();
  }
  function pickKeyword(n: Node) {
    filter = { ...filter, keywords_under: filter.keywords_under?.[0] === n.value ? undefined : [n.value] };
    run();
  }

  // Dates: the tree of years, months and weeks, or a custom range of days. Picking one
  // lets go of the other, so the filter never asks for dates that contradict.
  let dateNodes = $derived(
    dateTree(result?.facets.year ?? [], result?.facets.month ?? [], result?.facets.week ?? []),
  );
  let dateSelected = $derived(
    filter.week ? "d:" + filter.week : filter.month ? "d:" + filter.month : filter.year ? "d:" + filter.year : undefined,
  );

  /** A year, month or week: it opens to show what is under it. Picking the one picked
   *  goes up a level. */
  function pickDate(node: Node) {
    const v = node.value;
    const at = (key: string) => ({
      year: +key.slice(0, 4),
      month: key.length >= 7 ? key.slice(0, 7) : undefined,
      week: key.length > 7 ? key : undefined,
      captured: undefined,
    });
    if (dateSelected === node.key) {
      const up = v.length > 7 ? v.slice(0, 7) : v.length > 4 ? v.slice(0, 4) : "";
      filter = { ...filter, ...(up ? at(up) : { year: undefined, month: undefined, week: undefined }) };
    } else {
      filter = { ...filter, ...at(v) };
      expanded.add(node.key);
    }
    run();
  }

  // Range ends are local days, `YYYY-MM-DD` as date inputs give them.
  const pad = (n: number) => String(n).padStart(2, "0");
  const dayOf = (ms: number) => {
    const d = new Date(ms);
    return `${String(d.getFullYear()).padStart(4, "0")}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
  };
  /** Midnight starting the day, `after` days on (setFullYear: `new Date` reads years below 100 as 19xx). */
  function dayStart(day: string, after = 0) {
    const [y, m, d] = day.split("-").map(Number);
    const at = new Date(0);
    at.setFullYear(y, m - 1, d + after);
    at.setHours(0, 0, 0, 0);
    return at.getTime();
  }
  let rangeFrom = $derived(filter.captured?.from === undefined ? "" : dayOf(filter.captured.from));
  let rangeTo = $derived(filter.captured?.to === undefined ? "" : dayOf(filter.captured.to));

  /** Days `from` to `to`, both included; "" leaves that end open. */
  function setRange(from: string, to: string) {
    // Typing a year in a date box passes through 0002, 0020…: wait for a whole one.
    if ([from, to].some((d) => d && +d.slice(0, 4) < 1800)) return;
    if (from && to && from > to) [from, to] = [to, from];
    const captured = from || to
      ? { from: from ? dayStart(from) : undefined, to: to ? dayStart(to, 1) - 1 : undefined }
      : undefined;
    filter = captured
      ? { ...filter, captured, year: undefined, month: undefined, week: undefined }
      : { ...filter, captured };
    run();
  }

  /** The custom range being typed, both ends optional; null while closed. */
  let custom = $state<{ from: string; to: string } | null>(null);
  function openCustom() {
    custom = custom ? null : { from: rangeFrom, to: rangeTo };
  }
  function closeOnEscape(e: KeyboardEvent) {
    if (e.key === "Escape") custom = null;
  }
  function applyCustom(e?: Event) {
    e?.preventDefault();
    if (!custom) return;
    setRange(custom.from, custom.to);
    custom = null;
  }

  // Ranges ending today, left open so photos taken later today join.
  const PRESETS: [string, number][] = [["7 days", 7], ["30 days", 30], ["90 days", 90], ["365 days", 365]];
  function presetFrom(days: number) {
    return dayOf(dayStart(dayOf(Date.now()), 1 - days));
  }
  const isPreset = (days: number) => rangeFrom === presetFrom(days) && rangeTo === "";

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
    rating: "Rating", color: "Colour", year: "Year", date: "Date", camera: "Camera", country: "Country",
    albums: "Albums", folders: "Folders", keywords: "Keywords", places: "Location",
  };

  let albums = $derived(albumTree(collections, new Map(result?.facets.collection ?? []), smart, queuedAlbums));
  let folders = $derived(pathTree(result?.facets.folder ?? [], "/", "f:", "folder"));
  let places = $derived(placeTree(result?.facets.place_tree ?? []));
  let keywords = $derived(pathTree(result?.facets.keyword_tree ?? [], "|", "k:", "keyword"));
  // While searching, the tree shows only matches (and the branches above them), opened.
  let keywordView = $derived(searchTree(keywords, keywordSearch));
  const searchOpen = new SvelteSet<string>();
  $effect(() => {
    searchOpen.clear();
    for (const key of keywordView.open) searchOpen.add(key);
  });

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
    if (e.key === "Escape" && (keywordMenu || placeMenu || albumMenu)) {
      keywordMenu = null;
      placeMenu = null;
      albumMenu = null;
      return;
    }
    if (writing || showBackups || pasting || keywordDialog || placeDialog || albumDialog || fillingCodes || copying || syncing || (e.target instanceof HTMLInputElement && e.target.type !== "checkbox")) return;
    const ctrl = e.ctrlKey || e.metaKey;
    const key = e.key.toLowerCase();
    if (ctrl && e.altKey && !e.shiftKey && key === "v") {
      pasteClip(true);
      e.preventDefault();
      return;
    }
    if (ctrl && !e.altKey) {
      if (key === "z" && !e.shiftKey) undo();
      else if (key === "y" || (key === "z" && e.shiftKey)) redo();
      else if (key === "a" && !viewing) pickAll();
      else if (key === "k" && showInfo) document.getElementById("keyword-input")?.focus();
      else if (key === "c" && (e.shiftKey || !window.getSelection()?.toString())) openCopy();
      else if (key === "v" && e.shiftKey) { if (clipboard) pasting = targets(); }
      else if (key === "v") pasteClip();
      else if (key === "s" && e.shiftKey) openSync();
      else if ((key === "=" || key === "+") && !viewing) grid?.step(true);
      else if (key === "-" && !viewing) grid?.step(false);
      else if (key === "0" && !viewing) tiles = DEFAULT_TILES;
      else return;
      e.preventDefault();
      return;
    }
    if (e.altKey) return;
    if (/^[0-5]$/.test(e.key)) applyEdit({ Rating: +e.key || null });
    else if (e.key in COLOR_KEYS) applyEdit({ Color: COLOR_KEYS[e.key] });
    else if (e.key === "i") showInfo = !showInfo;
    else if ((e.key === "Delete" || e.key === "Backspace") && shownAlbum && !viewing) {
      const album = shownAlbum;
      const n = targets().length;
      applyEdit({ RemoveFromAlbum: album.id }).then(() => {
        if (!editError) note(`${n.toLocaleString()} photo${n === 1 ? "" : "s"} taken out of “${album.name}”`);
      });
    }
    else if (viewing) return; // the viewer has its own keys
    else if (e.key === "Escape" && picked.size > 1) {
      picked.clear();
      const row = selected === null ? undefined : rowAt(selected);
      if (row !== undefined) picked.add(row);
    } else if (e.key === "Enter" && selected !== null) viewing = true;
    else if (grid?.move(e.key, e.shiftKey)) e.preventDefault();
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

  // Drag and drop in the Albums tree: photos onto an album put them in it; an album or
  // group onto a group (or the Albums title, for the top level) moves it there.
  function dragCell(i: number) {
    const row = rowAt(i);
    if (row === undefined) return;
    dragging = { rows: picked.has(row) && picked.size > 1 ? [...picked] : [row] };
  }
  const movable = (c: api.Collection | undefined) => !!c && !c.system && c.kind !== "Project";
  function dragAlbum(n: Node, starting: boolean) {
    const ok = movable(byId.get(n.value));
    if (ok && starting) dragging = { album: n.value };
    return ok;
  }
  /** Can the dragged thing go into `target` (undefined: the top level)? */
  function canDrop(target: api.Collection | undefined): boolean {
    const d = dragging;
    if (!d) return false;
    if ("rows" in d) return target?.kind === "Album";
    if (target && ((target.kind !== "Group" && target.kind !== "Project") || target.system)) return false;
    for (let c = target; c; c = c.parent ? byId.get(c.parent) : undefined) if (c.id === d.album) return false;
    return (byId.get(d.album)?.parent ?? undefined) !== target?.id;
  }
  async function drop(target: api.Collection | undefined) {
    const d = dragging;
    dragging = null;
    if (!d) return;
    if ("rows" in d && target) {
      await applyEdit({ AddToAlbum: target.id }, d.rows);
      if (!editError) note(`${d.rows.length.toLocaleString()} photo${d.rows.length === 1 ? "" : "s"} put into “${target.name}”`);
    } else if ("album" in d) {
      try {
        await applyCollection({ Move: { id: d.album, parent: target?.id ?? null } });
      } catch (e) {
        editError = String(e);
      }
    }
  }
  /** The album the grid shows, if it is one photos can be taken out of. */
  let shownAlbum = $derived(
    filter.collections?.length === 1 && byId.get(filter.collections[0])?.kind === "Album" ? byId.get(filter.collections[0]) : undefined,
  );

  // The filter as chips above the grid, one per value; × on a chip drops just that value.
  // `rest` is what the field holds without it (undefined lets go of the field).
  type Chip = { what?: string; text: string; title?: string; dot?: string; stars?: boolean; rest?: unknown };
  const without = <T,>(list: T[], v: T) => {
    const left = list.filter((x) => x !== v);
    return left.length ? left : undefined;
  };
  function ratingText(r: NonNullable<Filter["rating"]>): string {
    if (r === "None") return "unrated";
    if (r === "Any") return "rated";
    const { min, max } = r.Values;
    if (min !== undefined && min === max) return stars(min);
    if (min !== undefined && max !== undefined) return `★ ${min}–${max}`;
    return min !== undefined ? `★ ${min}+` : `★ up to ${max}`;
  }
  /** Every filter field, in the order its chips show. A new field needs a line here. */
  const CHIPS: { [K in keyof Filter]-?: (v: NonNullable<Filter[K]>) => Chip[] } = {
    text: (v) => [{ what: "Search", text: `“${v}”` }],
    rating: (v) => [{ text: ratingText(v), stars: typeof v === "object" }],
    color: (v) =>
      typeof v === "string"
        ? [{ what: "Colour", text: v === "None" ? "none" : "any" }]
        : v.Values.map((c) => {
            const rest = without(v.Values, c);
            return { what: "Colour", text: c.toLowerCase(), dot: COLORS[c.toLowerCase()], rest: rest && { Values: rest } };
          }),
    // A week carries its month and year: one chip, and dropping it goes up a level.
    year: (v) => (filter.month ? [] : [{ text: String(v) }]),
    month: (v) => (filter.week ? [] : [{ text: monthText(v) }]),
    week: (v) => [{ text: weekText(v) }],
    captured: (v) => [{ what: "Dates", text: rangeText(v) }],
    camera: (v) => [{ what: "Camera", text: v }],
    country: (v) => [{ what: "Country", text: v }],
    keywords_under: (v) => v.map((p) => ({ what: "Keyword", text: p.split("|").join(" › "), title: p, rest: without(v, p) })),
    collections: (v) =>
      v.map((id) => ({ what: "Album", text: byId.get(id)?.name ?? id, title: byId.get(id)?.path, rest: without(v, id) })),
    folder: (v) => [{ what: "Folder", text: v }],
    place_under: (v) => [{ what: "Place", text: placeText(v) }],
    no_country_code: (v) => (v ? [{ text: "no ISO country code" }] : []),
  };
  function monthText(key: string) {
    const [y, m] = key.split("-").map(Number);
    return new Date(y, m - 1, 1).toLocaleDateString(undefined, { month: "long", year: "numeric" });
  }
  function rangeText(r: api.TimeRange) {
    const day = (ms: number) => new Date(ms).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
    if (r.from !== undefined && r.to !== undefined) return `${day(r.from)} – ${day(r.to)}`;
    return r.from !== undefined ? `from ${day(r.from)}` : r.to !== undefined ? `until ${day(r.to)}` : "any";
  }
  let chips = $derived(
    (Object.keys(CHIPS) as (keyof Filter)[]).flatMap((key) => {
      const v = filter[key];
      return v === undefined ? [] : CHIPS[key](v as never).map((c) => ({ ...c, key }));
    }),
  );

  function dropChip(key: keyof Filter, rest: unknown) {
    filter = { ...filter, [key]: rest };
    if (key === "text") search = "";
    run();
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

{#snippet dates()}
  <section>
    {@render head("date")}
    {#if !folded.has("date")}
      <Tree nodes={dateNodes} selected={dateSelected} {expanded} onpick={pickDate} />
      <div class="custom">
        {#if filter.captured}
          <button class="range-on" class:open={custom} title="Change the range" onclick={openCustom}>
            {rangeText(filter.captured)}
          </button>
          <button class="drop" title="Any date" onclick={() => { custom = null; setRange("", ""); }}>×</button>
        {:else}
          <button class:open={custom} onclick={openCustom}>Custom range…</button>
        {/if}
      </div>
      {#if custom}
        <form class="range" onsubmit={applyCustom}>
          <label>From <input type="date" bind:value={custom.from} onkeydown={closeOnEscape} /></label>
          <label>To <input type="date" bind:value={custom.to} onkeydown={closeOnEscape} /></label>
          <p class="hint">Either end can stay empty: from a day on, or up to one.</p>
          <div class="presets">
            Last
            {#each PRESETS as [name, days]}
              <button type="button" class:on={isPreset(days)} onclick={() => { custom = null; setRange(presetFrom(days), ""); }}>
                {name}
              </button>
            {/each}
          </div>
          <div class="actions">
            <button type="button" onclick={() => (custom = null)}>Cancel</button>
            <button type="submit" class="primary">Apply</button>
          </div>
        </form>
      {/if}
    {/if}
  </section>
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
    shared={selectionKeywords}
    onaddkeywords={(paths) => applyEdit({ AddKeywords: paths })}
    collections={byId}
    onkeyword={showKeyword}
    onalbum={showAlbum}
    sharedAlbums={selectionAlbums}
    onaddalbum={(id) => applyEdit({ AddToAlbum: id })}
    onremovealbum={(id) => applyEdit({ RemoveFromAlbum: id })}
    onfolder={showFolder}
    oncamera={(camera) => only({ camera })}
    onplace={(fields) => applyEdit({ Place: fields })}
    ongeo={(value) => applyEdit({ Geo: value })}
    oniptc={(field, value) => applyEdit({ Iptc: { [field]: value } })}
    clip={clipText}
    oncopy={openCopy}
    onpaste={pasteClip}
    onpastechoose={() => { if (clipboard) pasting = targets(); }}
    onsync={openSync}
  />
{/snippet}

<svelte:window onkeydown={onkey} />

<div class="app" class:with-info={showInfo}>
  <header>
    <SourceMenu bind:open={choosingSource} onbackups={() => (showBackups = true)} />
    <input class="search" placeholder="Search keywords, captions, places, filenames" bind:value={search} oninput={onSearch} />
    <div class="tiles" title="Tile size: Ctrl+wheel over the grid, Ctrl+plus and Ctrl+minus; Ctrl+0 resets">
      <button aria-label="Smaller tiles" onclick={() => grid?.step(false)}>−</button>
      <input
        type="range"
        aria-label="Tile size"
        min={MIN_TILE}
        max={Math.max(MIN_TILE + 1, layout.room)}
        value={tiles.by === "size" ? tiles.px : layout.width}
        oninput={(e) => (tiles = { by: "size", px: +e.currentTarget.value })}
        onchange={(e) => e.currentTarget.blur()}
      />
      <button aria-label="Bigger tiles" onclick={() => grid?.step(true)}>+</button>
      <label title="Tiles per row. Typed in, the count stays as the window changes; the slider lets it follow the window again.">
        <input
          type="number"
          min="1"
          max={MAX_COUNT}
          value={layout.cols}
          class:fixed={tiles.by === "count"}
          onchange={(e) => setCount(e.currentTarget.value)}
          onkeydown={(e) => { if (e.key === "Enter" || e.key === "Escape") e.currentTarget.blur(); }}
        />
        per row
      </label>
      <button
        class="shape"
        title={shape === "1:1" ? "Square tiles: click for 3:2" : "3:2 tiles: click for square"}
        onclick={() => (shape = shape === "1:1" ? "3:2" : "1:1")}
      >{shape}</button>
    </div>
    <button class="toggle" class:on={showInfo} title="Details (i)" onclick={() => (showInfo = !showInfo)}>Details</button>
    <span class="count">
      {#if result}{result.total.toLocaleString()} photos · {(result.micros / 1000).toFixed(1)} ms{/if}
    </span>
  </header>

  <aside>
    <section
      class="albums-section"
      role="group"
      aria-label="Albums"
      class:drop={dragging && "album" in dragging && canDrop(undefined)}
      ondragover={(e) => { if (canDrop(undefined)) e.preventDefault(); }}
      ondrop={(e) => { if (canDrop(undefined)) { e.preventDefault(); drop(undefined); } }}
    >
      {@render head("albums")}
      {#if !folded.has("albums")}
        <div class="album-actions">
          <button title="A new album, at the top level" onclick={() => (albumDialog = { mode: "create", kind: "Album" })}>+ Album</button>
          <button title="A new smart album: a rule, not photos" onclick={() => (albumDialog = { mode: "create", kind: "Smart" })}>+ Smart album</button>
          <button title="A new group, to hold albums" onclick={() => (albumDialog = { mode: "create", kind: "Group" })}>+ Group</button>
        </div>
        <Tree
          nodes={albums}
          selected={filter.collections && "a:" + filter.collections[0]}
          {expanded}
          onpick={pickAlbum}
          oncontext={(node, e) => (albumMenu = { x: e.clientX, y: e.clientY, node })}
          drag={dragAlbum}
          candrop={(n) => canDrop(byId.get(n.value))}
          ondropnode={(n) => drop(byId.get(n.value))}
        />
        <p class="tree-hint">
          {#if shownAlbum}Delete takes the selected photos out of “{shownAlbum.name}”.{:else}Drag photos onto an album to put them in; drag an album onto a group to move it.{/if}
        </p>
      {/if}
    </section>
    {@render dates()}
    <section>
      {@render head("places")}
      {#if !folded.has("places")}
        <Tree
          nodes={places}
          selected={filter.place_under && "p:" + filter.place_under}
          {expanded}
          onpick={pickPlace}
          oncontext={(node, e) => (placeMenu = { x: e.clientX, y: e.clientY, node })}
        />
        {#if missingCodes}
          <button class="tree-action" onclick={() => (fillingCodes = true)} title="Fill in the ISO codes of these photos' countries">
            {missingCodes.toLocaleString()} without an ISO country code · Fill in…
          </button>
        {/if}
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
        <input
          class="tree-search"
          type="search"
          placeholder="Find a keyword"
          spellcheck="false"
          bind:value={keywordSearch}
          onkeydown={(e) => { if (e.key === "Escape") { keywordSearch = ""; e.currentTarget.blur(); } }}
        />
        <Tree
          nodes={keywordView.nodes}
          selected={filter.keywords_under && "k:" + filter.keywords_under[0]}
          expanded={keywordSearch.trim() ? searchOpen : expanded}
          onpick={pickKeyword}
          oncontext={(node, e) => (keywordMenu = { x: e.clientX, y: e.clientY, node })}
        />
        {#if keywordSearch.trim() && !keywordView.nodes.length}<p class="tree-none">No keyword matches.</p>{/if}
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

  {#if active}
    <div class="filters" role="group" aria-label="Filters">
      {#each chips as chip}
        <span class="chip" title={chip.title}>
          {#if chip.dot}<i class="dot" style:background={chip.dot}></i>{/if}
          {#if chip.what}<span class="what">{chip.what}:</span>{/if}
          <span class="text" class:stars={chip.stars}>{chip.text}</span>
          <button
            class="drop"
            aria-label="Remove {chip.what ?? ''} {chip.text}"
            onclick={() => dropChip(chip.key, chip.rest)}
          >×</button>
        </span>
      {/each}
      <button class="clear" onclick={clearAll}>Clear filters</button>
    </div>
  {/if}

  <Grid
    bind:this={grid}
    total={result?.total ?? 0}
    {rowAt}
    {need}
    {selected}
    picked={(i) => picked.size > 1 && picked.has(rowAt(i) ?? -1)}
    queued={(i) => queued.has(rowAt(i) ?? -1)}
    onselect={pick}
    ondragcell={dragCell}
    oncopycell={copyCell}
    onpastecell={pasteCell}
    canpaste={!!clipboard}
    onopen={(i) => { pick(i); viewing = true; }}
    {tiles}
    {shape}
    ontiles={(t) => (tiles = t)}
    bind:layout
  >
    {#if error}
      <p class="error">{error}</p>
      {#if !status}<p class="empty"><button class="link" onclick={() => (choosingSource = true)}>Open a catalog…</button></p>{/if}
    {:else if syncFailed && status?.photos === 0}
      <p class="error">Nothing could be read from this catalog.<br />{syncFailed}</p>
    {:else if result && result.total === 0}
      <p class="empty">Nothing matches.</p>
    {/if}
  </Grid>

  {#if showInfo && !viewing}{@render details()}{/if}

  <footer>
    {#if status}
      <span>{status.source}</span>
      {#if status.compatibility.read !== "Tested" || status.compatibility.write !== "Tested"}
        {@const c = status.compatibility}
        <button
          class="compat"
          class:untested={c.read !== "Tested"}
          title="{c.format}. {c.note ?? ''}"
          onclick={() => (choosingSource = true)}
        >
          {c.read !== "Tested" ? "untested version" : "read-only"} · {c.format.replace(/^Capture One (\d+\.\d+).*$/, "$1")}
        </button>
      {/if}
      <span>{status.photos.toLocaleString()} in library</span>
      <span>Capture One: {status.writes.startsWith("Locked") ? "open" : "closed"}</span>
    {/if}
    {#if editError}<span class="edit-error">{editError}</span>{:else if editNote}<span class="edit-note">{editNote}</span>{/if}
    {#if syncNote}<span class="sync" class:failed={!!syncFailed} title={syncFailed ?? ""}>{syncNote}</span>{/if}
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

{#if keywordMenu}
  {@const node = keywordMenu.node}
  <div class="menu-shield" role="presentation" onclick={() => (keywordMenu = null)} oncontextmenu={(e) => { e.preventDefault(); keywordMenu = null; }}></div>
  <div class="menu" role="menu" style:left="{keywordMenu.x}px" style:top="{keywordMenu.y}px">
    <div class="menu-title">{node.value}</div>
    <button role="menuitem" onclick={() => { keywordDialog = { mode: "rename", start: node.value }; keywordMenu = null; }}>Rename…</button>
    <button role="menuitem" onclick={() => { keywordDialog = { mode: "join", start: node.value }; keywordMenu = null; }}>Join or replace…</button>
    <button role="menuitem" onclick={() => { keywordDialog = { mode: "group", start: node.value }; keywordMenu = null; }}>Group under…</button>
    {#if node.value.includes("|")}
      <button role="menuitem" onclick={() => quickRemap({ Group: { paths: [node.value], under: null } })}>Move to the top level</button>
    {/if}
    {#if node.children.length}
      <button role="menuitem" onclick={() => quickRemap({ Ungroup: { path: node.value } })}>Ungroup</button>
    {/if}
  </div>
{/if}

{#if albumMenu}
  {@const c = albumMenu.node ? byId.get(albumMenu.node.value) : undefined}
  {@const holds = !c || ((c.kind === "Group" || c.kind === "Project") && !c.system)}
  {@const beside = !!c && !holds && !(c.parent && byId.get(c.parent)?.system)}
  {@const n = targets().length}
  <div class="menu-shield" role="presentation" onclick={() => (albumMenu = null)} oncontextmenu={(e) => { e.preventDefault(); albumMenu = null; }}></div>
  <div class="menu" role="menu" style:left="{albumMenu.x}px" style:top="{albumMenu.y}px">
    {#if c}<div class="menu-title">{c.path}</div>{/if}
    {#if c?.kind === "Album" && n}
      <button role="menuitem" onclick={() => { const id = c.id; albumMenu = null; applyEdit({ AddToAlbum: id }); }}>
        Put the {n === 1 ? "selected photo" : `${n.toLocaleString()} selected photos`} here
      </button>
    {/if}
    {#if c?.kind === "Smart" && c.rule && ruleRows(c.rule)}
      <button role="menuitem" onclick={() => { albumDialog = { mode: "rule", collection: c }; albumMenu = null; }}>Edit rule…</button>
    {/if}
    {#if holds || beside}
      {#each [["Album", "album"], ["Smart", "smart album"], ["Group", "group"]] as const as [kind, noun]}
        <button role="menuitem" onclick={() => { albumDialog = { mode: "create", kind, collection: c }; albumMenu = null; }}>
          New {noun}{c && holds ? " in it" : ""}…
        </button>
      {/each}
    {/if}
    {#if c && !c.system}
      <button role="menuitem" onclick={() => { albumDialog = { mode: "rename", collection: c }; albumMenu = null; }}>Rename…</button>
      {#if movable(c)}
        <button role="menuitem" onclick={() => { albumDialog = { mode: "move", collection: c }; albumMenu = null; }}>Move to…</button>
        {#if c.parent}
          <button role="menuitem" onclick={() => { const id = c.id; albumMenu = null; applyCollection({ Move: { id, parent: null } }).catch((e) => (editError = String(e))); }}>Move to the top level</button>
        {/if}
      {/if}
      {#if c.kind !== "Project"}
        <button role="menuitem" class="danger" onclick={() => { albumDialog = { mode: "delete", collection: c }; albumMenu = null; }}>Delete…</button>
      {/if}
    {:else if c?.system}
      <div class="menu-note">Capture One's own: changed only there.</div>
    {/if}
  </div>
{/if}

{#if albumDialog}
  <AlbumDialog
    mode={albumDialog.mode}
    kind={albumDialog.kind}
    collection={albumDialog.collection}
    {collections}
    keywords={keywordList}
    picked={targets().length}
    onclose={() => (albumDialog = null)}
    ondone={applyCollection}
  />
{/if}

{#if keywordDialog}
  <KeywordDialog
    mode={keywordDialog.mode}
    start={keywordDialog.start}
    keywords={keywordList}
    onclose={() => (keywordDialog = null)}
    ondone={applyRemap}
  />
{/if}

{#if placeMenu}
  {@const node = placeMenu.node}
  <div class="menu-shield" role="presentation" onclick={() => (placeMenu = null)} oncontextmenu={(e) => { e.preventDefault(); placeMenu = null; }}></div>
  <div class="menu" role="menu" style:left="{placeMenu.x}px" style:top="{placeMenu.y}px">
    <div class="menu-title">{placeText(node.value)}</div>
    <button role="menuitem" onclick={() => { placeDialog = { mode: "rename", path: node.value }; placeMenu = null; }}>
      {node.value.endsWith("—") ? "Set a name…" : "Rename…"}
    </button>
    <button role="menuitem" onclick={() => { placeDialog = { mode: "join", path: node.value }; placeMenu = null; }}>Join into…</button>
  </div>
{/if}

{#if placeDialog}
  <PlaceDialog
    mode={placeDialog.mode}
    path={placeDialog.path}
    onclose={() => (placeDialog = null)}
    ondone={codesFilled}
  />
{/if}

{#if fillingCodes}
  <CountryCodesDialog {filter} onclose={() => (fillingCodes = false)} ondone={codesFilled} />
{/if}

{#if copying}
  <CopyDialog
    photo={copying}
    fields={clipboard?.fields ?? ["country", "country_code", "state", "city", "location", "geo"]}
    oncopy={(fields) => { if (copying) { clipboard = { from: copying, fields }; pasteFields = null; } }}
    onclose={() => (copying = null)}
  />
{/if}

{#if syncing}
  <CopyDialog
    photo={syncing.from}
    sync={picked.size - 1}
    fields={clipboard?.fields ?? ["keywords"]}
    oncopy={(fields, fill) => { if (syncing) sync(syncing, fields, fill); }}
    onclose={() => (syncing = null)}
  />
{/if}

{#if pasting && clipboard}
  {@const rows = pasting}
  <CopyDialog
    photo={clipboard.from}
    paste={rows.length}
    fields={pasteFields ?? clipboard.fields}
    oncopy={(fields, fill) => { pasteFields = fields; if (clipboard) pasteRows(rows, clipboard.from, fields, fill); }}
    onclose={() => (pasting = null)}
  />
{/if}

{#if showBackups}
  <BackupsDialog onclose={() => (showBackups = false)} onrestored={() => { syncNote = ""; refresh(true); }} />
{/if}

{#if writing}
  <WriteDialog onclose={() => (writing = false)} onchanged={() => { refreshKeywords(); edited(); }} />
{/if}

{#if viewing && selected !== null && rowAt(selected) !== undefined}
  <Viewer row={rowAt(selected)!} next={rowAt(selected + 1)} onclose={close} onstep={step}>
    {#if showInfo}{@render details()}{/if}
  </Viewer>
{/if}
