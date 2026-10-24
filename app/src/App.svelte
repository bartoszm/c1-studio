<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { onMount, tick, untrack } from "svelte";
  import { SvelteMap, SvelteSet } from "svelte/reactivity";
  import * as api from "./lib/api";
  import type { Dim, Filter, QueryResult } from "./lib/api";
  import { COLORS, FIELD_NAMES, HUES, stars } from "./lib/format";
  import { dateTree, weekText } from "./lib/dates";
  import { albumTree, cityWithoutLocation, pathTree, placeText, placeTree, searchTree, type Node } from "./lib/tree";
  import { allowPlaceLookups } from "./lib/placeHints";
  import { SECTION_DIMS, type SectionKey } from "./lib/sections";
  import {
    LIGHT_PROBLEMS, RATING_OPS, allOfPicks, compile, criteriaCount, cycle, pickOf, ratingFor, ratingParts, ratingText, sameRating,
    type AllOf, type Match, type Pick, type RatingOp,
  } from "./lib/criteria";
  import { settings } from "./lib/settings.svelte";
  import Grid, { DEFAULT_TILES, MAX_COUNT, MIN_TILE, type Layout, type Shape, type Tiles } from "./Grid.svelte";
  import AlbumDialog, { rows as ruleRows } from "./AlbumDialog.svelte";
  import BackupsDialog from "./BackupsDialog.svelte";
  import EmptyBinDialog from "./EmptyBinDialog.svelte";
  import StartupDialog from "./StartupDialog.svelte";
  import CopyDialog from "./CopyDialog.svelte";
  import CountryCodesDialog from "./CountryCodesDialog.svelte";
  import PlaceDialog from "./PlaceDialog.svelte";
  import Info from "./Info.svelte";
  import MapPane from "./MapPane.svelte";
  import AnalysisPane from "./AnalysisPane.svelte";
  import HistoryPane from "./HistoryPane.svelte";
  import { pickText } from "./lib/history";
  import AiPane from "./AiPane.svelte";
  import GpxDialog from "./GpxDialog.svelte";
  import NeighbourDialog from "./NeighbourDialog.svelte";
  import SourceMenu from "./SourceMenu.svelte";
  import { source } from "./lib/source.svelte";
  import { offlineText, progressText } from "./lib/folders";
  import KeywordDialog from "./KeywordDialog.svelte";
  import Tree from "./Tree.svelte";
  import Timeline from "./Timeline.svelte";
  import TripsSection from "./TripsSection.svelte";
  import { tripDates, tripName } from "./lib/trips";
  import { shouldFollow } from "./lib/mapview";
  import { PAGE, refreshPages, rowAt as rowIn } from "./lib/pages";
  import { editingNote, followStep, keptPicks, sameRows, wantedRows, type Picks } from "./lib/batch";
  import { followFilter } from "./lib/follow";
  import { Trail, type Shown } from "./lib/navigation";
  import { hiddenNote, hides, withHidden } from "./lib/hidden";
  import { binNote, deleteAction, withBin } from "./lib/bin";
  import { STRIP_DEFAULT, STRIP_MIN, clampStrip, dragged, storedStrip } from "./lib/splitter";
  import { FIELDS, effectiveField, matchAvailable, restore, toSort, words, type SortField } from "./lib/sorting";
  import SortControl from "./SortControl.svelte";
  import { LOCK_HELP, isLockError } from "./lib/lock";
  import { typingIn } from "./lib/keys";
  import SettingsDialog from "./SettingsDialog.svelte";
  import Viewer from "./Viewer.svelte";
  import WriteDialog from "./WriteDialog.svelte";

  /** Groups counted whatever the sidebar shows: the analysis tab and the Location fill-in need them. */
  const ALWAYS: Dim[] = ["country", "flag", "shape", "hue", "light", "focus"];
  /** The sidebar's sections as set in Settings, the hidden ones left out. */
  let shownSections = $derived(settings.sections.filter((s) => !s.hidden).map((s) => s.key));
  /** The groups each query counts: only those something shows. */
  let dims = $derived([...new Set([...shownSections.flatMap((k) => SECTION_DIMS[k]), ...ALWAYS])]);
  let albumsShown = $derived(shownSections.includes("albums"));

  let status = $state<api.Status | null>(null);
  /** How far the source's scan has got (`source-progress` events), while one runs. */
  let progress = $state<api.SourceProgress | null>(null);
  /** Rows of photos whose originals are out of reach, for dimming their tiles. */
  /** The source menu in the header; open by itself when nothing could be opened. */
  let choosingSource = $state(false);
  /** How far opening a source has got, while it is under way. */
  let startup = $state<api.Startup | null>(null);
  /** The window has read the source the app started with. It may start before it is open. */
  let loaded = $state(false);
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
  /** The photo at `selected`. It stays while a refresh reads its page again, so the panel and
   *  the viewer never lose it in between. */
  let focusRow = $state<number | undefined>(undefined);
  let viewing = $state(false);
  let syncNote = $state("syncing…");
  /** Why reading the source failed, in full: shown in the grid when it is left empty. */
  let syncFailed = $state<string | null>(null);
  let grid = $state<Grid>();

  // Four views of the same filter: the grid, the map (with a strip of photos under it),
  // the analysis, the history and the AI suggestions. While the map follows, the grid shows
  // what the map shows.
  type Tab = "photos" | "map" | "analysis" | "history" | "ai";
  function rememberedTab(): Tab {
    try {
      const t = localStorage.getItem("tab");
      if (t === "map" || t === "analysis" || t === "history" || t === "ai") return t;
    } catch {
      // Opens on the photos.
    }
    return "photos";
  }
  let tab = $state<Tab>(rememberedTab());
  let mapOn = $derived(tab === "map");
  /** The grid is a strip under the view: on the map, the analysis, the history and the AI tab. */
  let stripped = $derived(mapOn || tab === "analysis" || tab === "history" || tab === "ai");
  let follow = $state(!remembered("map").includes("free"));
  /** A photo picked on the map: the panel shows it and edits reach it when nothing else is picked. */
  let mapRow = $state<number | null>(null);
  $effect(() => {
    try {
      localStorage.setItem("map", JSON.stringify(follow ? [] : ["free"]));
      localStorage.setItem("tab", tab);
    } catch {
      // Not remembered, that's all.
    }
  });
  let followTimer: ReturnType<typeof setTimeout>;
  /** What the map shows now, for when a picked tile is let go. */
  let mapView = $state.raw<api.BBox | null>(null);
  /** A tile picked on the map: the grid shows just its photos until it is let go. */
  let mapTile = $state.raw<api.BBox | null>(null);
  /** The area the grid kept to before the tile was picked (a drawn one, or none). */
  let beforeTile: api.BBox | undefined;
  function mapMoved(bounds: api.BBox) {
    mapView = bounds;
    // Photos with no position are in no area: while the grid shows them, it does not follow.
    if (!follow || mapTile || filter.gps === false || filter.approx) return;
    clearTimeout(followTimer);
    followTimer = setTimeout(() => {
      if (!mapOn || !follow) return;
      filter = { ...filter, within: bounds };
      run(true, true);
    }, 250);
  }
  /** Picking a tile (its photos' bounds: tiles never overlap, so exactly its photos), or
   *  letting go of it (null): the grid goes back to the map's area. */
  async function mapTilePick(bounds: api.BBox | null, row: number | null) {
    clearTimeout(followTimer);
    if (bounds) {
      if (!mapTile) beforeTile = filter.within;
      mapTile = bounds;
      filter = { ...filter, within: bounds };
    } else {
      if (!mapTile) return;
      mapTile = null;
      filter = { ...filter, within: follow ? (mapView ?? undefined) : beforeTile };
    }
    // Other photos altogether: the photos picked stay picked where the tile has them (by
    // row, not by their place in the grid); a photo clicked is picked instead. Tiles
    // clicked one after another are one step back.
    await run(false, false, bounds ? { kind: "tile" } : {});
    if (row !== null) await mapSelect(row);
  }
  function sameBox(a: api.BBox | undefined | null, b: api.BBox | undefined | null): boolean {
    return !!a && !!b && a.min_lat === b.min_lat && a.max_lat === b.max_lat && a.min_lon === b.min_lon && a.max_lon === b.max_lon;
  }
  // Anything else that changes the area (a drawn one, the chip, Clear filters) lets go.
  $effect(() => {
    if (mapTile && !sameBox(filter.within, mapTile)) mapTile = null;
  });
  /** The filter with the map's view as its area, when the grid follows the map. */
  function following(f: Filter): Filter {
    return mapView && shouldFollow({ follow, mapOn, tile: !!mapTile, filter: f }) ? { ...f, within: mapView } : f;
  }
  // Following turned on, or possible again (a tile let go, Clear filters, a chip that kept
  // the grid off the map dropped): the grid takes what the map shows now, not at its next move.
  $effect(() => {
    if (!mapView || !shouldFollow({ follow, mapOn, tile: !!mapTile, filter })) return;
    untrack(() => {
      // The map's own report of this view, waiting to be followed, would only run it again.
      clearTimeout(followTimer);
      filter = following(filter);
      run(true, true);
    });
  });
  /** Letting go of the map's area. */
  function mapReleased() {
    clearTimeout(followTimer);
    if (filter.within) {
      filter = { ...filter, within: undefined };
      run(true, true);
    }
  }
  function showTab(next: Tab) {
    // The map reports its view again when it opens: one kept from before would be followed first.
    if (next !== "map") mapView = null;
    tab = next;
    mapRow = null;
    viewing = false;
  }
  function toggleFollow() {
    follow = !follow;
    if (!follow) mapReleased();
    else if (mapView && !mapTile && filter.within) {
      // A drawn area gives way to what the map shows.
      filter = { ...filter, within: undefined };
      run(true, true);
    }
  }
  /** Photos the map cannot show by position (no GPS): the grid lets go of the map's area. */
  function showOffMap(extra: Filter) {
    clearTimeout(followTimer);
    mapTile = null;
    filter = { ...filter, within: undefined, ...extra };
    run();
  }
  /** An area drawn on the map: the grid keeps to it, whatever the map shows next. */
  function mapBox(bounds: api.BBox) {
    follow = false;
    clearTimeout(followTimer);
    filter = { ...filter, within: bounds };
    run(true, true);
  }
  function mapPick(row: number) {
    picked.clear();
    selected = null;
    mapRow = row;
  }
  /** A photo focused that the map does not move to: one picked on it (it is under the
   *  pointer), or kept while the map moved. Only for that change of focus. */
  let holdMap: number | null = null;
  /** A photo picked on the map: picked in the grid too, scrolled to; one the grid does not
   *  show is only in the panel. */
  async function mapSelect(row: number) {
    const mine = generation;
    const i = await api.position(asking($state.snapshot(filter)), { sort: sortBy }, row).catch(() => null);
    if (mine !== generation) return;
    if (i === null) return mapPick(row);
    await need(i, i + 1);
    if (mine !== generation) return;
    holdMap = row;
    await pick(i);
    await tick();
    holdMap = null;
    grid?.reveal(i);
  }
  /** With the grid and map in sync, the map centres on the photo picked in the grid. */
  let mapCenter = $state.raw<{ row: number; lat: number; lon: number } | null>(null);
  $effect(() => {
    const row = focusRow;
    if (row === undefined || !mapOn || !follow) return;
    if (untrack(() => holdMap) === row) return;
    api.photo(row).then((p) => {
      if (focusRow === row && p?.geo) mapCenter = { row, lat: p.geo.lat, lon: p.geo.lon };
    }).catch(() => {});
  });
  async function mapSetGeo(rows: number[], lat: number, lon: number, from?: api.BBox) {
    dragging = null;
    const at: [number, number] = [Math.round(lat * 1e6) / 1e6, Math.round(lon * 1e6) / 1e6];
    await applyEdit({ Geo: at }, rows);
    if (editError) return;
    note(`${rows.length.toLocaleString()} photo${rows.length === 1 ? "" : "s"} placed at ${at[0].toFixed(4)}, ${at[1].toFixed(4)}`);
    void offerFill(rows);
    // The picked tile moved as a whole: the pick goes with it, so the grid keeps its photos.
    if (from && sameBox(from, mapTile)) {
      mapTile = { min_lat: at[0], max_lat: at[0], min_lon: at[1], max_lon: at[1] };
      filter = { ...filter, within: mapTile };
      run(true);
    }
  }

  /** Rows of the photos picked for editing; a click picks one, Ctrl and Shift add. */
  const picked = new SvelteSet<number>();
  /** Where a Shift-click range starts. */
  let anchor: number | null = null;
  /** Row → fields edited here and not yet written to Capture One. */
  let queued = $state.raw(new Map<number, string[]>());
  let queuedCount = $state(0);
  /** Bumped after every edit, so the details panel reads the photo again. */
  let version = $state(0);
  /** Bumped when the analysis has measured photos again: the panel's Look reads them again. */
  let analysed = $state(0);

  /** Photos the user hid are left out of every view; "Hidden" in the header lets them in. */
  let showHidden = $state(false);
  /** Of the photos shown, the hidden ones (only while they are shown): their tiles are marked. */
  let hiddenHere = $state.raw(new Set<number>());
  /** The trash is shown: the photos deleted here, and only those. */
  let inBin = $state(false);
  /** Asking before deleting photos in the trash for good: all of them, or the picked `rows`. */
  let emptying = $state<{ rows?: number[] } | null>(null);
  /** How tall the strip of photos under the map and the analysis is: dragged, remembered. */
  let stripH = $state(storedStrip((() => { try { return localStorage.getItem("strip"); } catch { return null; } })()));
  let paneEl = $state<HTMLDivElement>();
  let paneH = $state(0);
  /** The strip as shown: the stored height kept within what this window leaves room for. */
  let stripShown = $derived(clampStrip(stripH, paneH || 900));
  let splitting = $state(false);
  function setStrip(px: number, keep = true) {
    stripH = clampStrip(px, paneEl?.clientHeight ?? 900);
    if (keep) try { localStorage.setItem("strip", String(stripH)); } catch { /* not remembered */ }
  }
  function splitStart(e: PointerEvent) {
    const handle = e.currentTarget as HTMLElement;
    handle.setPointerCapture(e.pointerId);
    const [startY, start] = [e.clientY, stripShown];
    splitting = true;
    const move = (ev: PointerEvent) => setStrip(dragged(start, startY, ev.clientY, paneEl?.clientHeight ?? 900), false);
    const end = () => {
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", end);
      handle.removeEventListener("pointercancel", end);
      splitting = false;
      setStrip(stripH);
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", end);
    handle.addEventListener("pointercancel", end);
  }
  function splitKey(e: KeyboardEvent) {
    const step = e.shiftKey ? 80 : 20;
    if (e.key === "ArrowUp") setStrip(stripShown + step);
    else if (e.key === "ArrowDown") setStrip(stripShown - step);
    else if (e.key === "Home") setStrip(STRIP_DEFAULT);
    else return;
    e.preventDefault();
  }

  /** The order of the grid and of the strip under the map and the analysis. */
  const storedSort = (() => {
    try {
      return restore(localStorage.getItem("sortBy"), localStorage.getItem("sort"));
    } catch {
      return restore(null, null);
    }
  })();
  let sortField = $state<SortField>(storedSort.field);
  /** The direction is the other way round from the field's usual one. */
  let sortReversed = $state(storedSort.reversed);
  /** What the index sorts by: Best match with nothing picked to match shows as the date. */
  let sortShown = $derived(effectiveField(sortField, filter));
  let sortBy = $derived(toSort(sortShown, sortShown === sortField && sortReversed));
  const rememberSort = () => {
    // Best match depends on what is picked, and nothing is picked at the next start.
    const stored = sortField === "match" ? { field: "date", reversed: false } : { field: sortField, reversed: sortReversed };
    try { localStorage.setItem("sortBy", JSON.stringify(stored)); } catch { /* not remembered */ }
  };
  /** A new field starts in its usual direction. */
  function setSortField(next: SortField) {
    sortField = next;
    sortReversed = false;
    rememberSort();
    run();
  }
  function flipSort() {
    sortReversed = !sortReversed;
    rememberSort();
    run();
  }
  // Best match with nothing left to match (the picks were cleared): the date takes over for good.
  $effect(() => {
    if (sortField === "match" && !matchAvailable(filter)) {
      untrack(() => {
        sortField = "date";
        sortReversed = false;
        rememberSort();
      });
    }
  });
  /** One photo per burst or duplicate group: its sharpest. */
  let stack = $state((() => { try { return localStorage.getItem("stack") === "1"; } catch { return false; } })());
  /** The filter with Match applied, and the stacking when it is on: what the grid shows. */
  const asking = (f: Filter): Filter => ({ ...compiled(f), ...(stack ? { stacked: true } : {}) });
  function toggleStack() {
    stack = !stack;
    try { localStorage.setItem("stack", stack ? "1" : "0"); } catch { /* not remembered */ }
    run();
  }
  let writing = $state(false);
  let showBackups = $state(false);
  /** Write the queue to Capture One by itself; off unless turned on. */
  let autosaveOn = $state(false);
  /** The History tab is on for this catalog (Settings › History). Off, the tab is hidden, a
   *  History pick leaves the filter, and the log keeps only what 0.5 kept. Shown once the
   *  catalog says it is on, so a tab remembered from last time asks nothing of it before. */
  let historyOn = $state(false);
  let autosaveState = $state<api.AutosaveState>({ state: "off" });
  let editError = $state<string | null>(null);
  // An error stays a while (longer than a note), then goes: what went wrong before is no
  // longer what the footer says now. A click lets it go sooner.
  $effect(() => {
    if (!editError) return;
    const gone = setTimeout(() => (editError = null), 10_000);
    return () => clearTimeout(gone);
  });
  /** A word on what an edit did, shown a few seconds in the footer. */
  let editNote = $state<string | null>(null);
  let noteTimer: ReturnType<typeof setTimeout>;
  function note(text: string) {
    editNote = text;
    clearTimeout(noteTimer);
    noteTimer = setTimeout(() => (editNote = null), 4000);
  }
  /** The background analysis, for the footer. */
  let analysis = $state<api.AnalysisState | null>(null);
  /** Photos being described by a vision model, for the footer and the AI tab. */
  let describing = $state<api.DescribeState | null>(null);
  /** The photos an AI run would describe: what an edit would reach. */
  let aiTargets = $derived.by(() => {
    void picked.size;
    return targets();
  });
  let thumbs = $state<api.ThumbsState | null>(null);
  let thumbsRetried = 0;
  let countsAt = 0;
  /** What is being dragged: photos (onto an album) or an album (onto a group). */
  let dragging = $state<{ rows: number[] } | { album: string } | null>(null);
  // Each entry undoes one edit (one key press or click, any number of photos).
  type Done =
    | { ops: [number, api.Op][] }
    | { remap: number; request: api.RemapRequest }
    | { collection: number; change: api.CollectionChange }
    | { hidden: number[]; on: boolean }
    | { binned: number[]; on: boolean };
  type Undone =
    | { ops: [number, api.Op][] }
    | { request: api.RemapRequest }
    | { change: api.CollectionChange }
    | { hidden: number[]; on: boolean }
    | { binned: number[]; on: boolean };
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
  let showTimeline = $state(!remembered("folded").includes("timeline"));
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
      const hidden = [...(showInfo ? [] : ["info"]), ...(showTimeline ? [] : ["timeline"])];
      localStorage.setItem("folded", JSON.stringify([...folded, ...hidden]));
    } catch {
      // Storage unavailable: the layout just isn't remembered.
    }
  });

  /** The groups the result on screen was counted with. */
  let counted: Dim[] = [];
  // A section shown again needs counts that result doesn't have. (Before the first result,
  // or after a failed query, the next query counts it anyway.)
  $effect(() => {
    const now = dims;
    untrack(() => { if (result && now.some((d) => !counted.includes(d))) run(true); });
  });
  $effect(() => allowPlaceLookups(settings.onlineMap));
  let showSettings = $state(false);
  /** The Settings tab it opens on: AI from the AI tab, else the sidebar's sections. */
  let settingsTab = $state<"sidebar" | "ai">("sidebar");
  /** A section header's right-click menu. */
  let sectionMenu = $state<{ x: number; y: number; key: SectionKey } | null>(null);
  function hideSection(key: SectionKey) {
    const s = settings.sections.find((s) => s.key === key);
    if (s) s.hidden = true;
  }

  // Each query gets a number; answers to older ones are dropped.
  let generation = 0;
  // The result's photos, fetched a page at a time as the grid reaches them. A new result
  // swaps in a new map, so the variable itself must be reactive too: cells already on
  // screen would otherwise keep reading the old one and show the previous result's photos.
  let pages = $state.raw(new SvelteMap<number, number[]>());
  /** Page → positions in it of the photos whose file is offline; swapped with `pages`. */
  let offlinePages = $state.raw(new SvelteMap<number, Set<number>>());
  let loading = new Map<number, Promise<void>>();

  /** Runs the filter. `keep` re-runs it in place after an edit: same scroll, same picks.
   *  `byRow`: a single pick stays on its photo, wherever it now is (the map moved),
   *  rather than on its place in the grid. Otherwise it is a new filter: the photos picked
   *  (or `how.restore`'s) stay picked where it still shows them, and the filter left is a
   *  step back (see `Step`). */
  async function run(keep = false, byRow = false, how: Step = {}) {
    const mine = ++generation;
    // Overtaking a new filter still looking for its picks (the map moved, a count was
    // needed): this run finishes that, as the same step.
    if (keep && carried) {
      keep = byRow = false;
      how = { restore: carried, travelling: true };
    }
    // A new filter: what was picked is looked for in it, and what was shown is kept to go
    // back to. Picks a run overtaken was still looking for are this one's to look for.
    const was = keep ? null : (how.restore ?? carried ?? untrack(pickedNow));
    if (was) {
      carried = was;
      untrack(() => leaving(was, how));
    }
    untrack(() => (filter = following(filter)));
    const asked = asking($state.snapshot(filter));
    const keepRow = keep && byRow && picked.size <= 1 && selected !== null ? focusRow : undefined;
    const kept = keepRow === undefined ? null : api.position(asked, { sort: sortBy }, keepRow).catch(() => null);
    let where: number | null = null;
    const groups = dims;
    // Which of the photos shown are hidden, to mark their tiles: none are while hidden.
    if (showHidden || inBin) api.hiddenRows(asked).then((rows) => { if (mine === generation) hiddenHere = new Set(rows); }).catch(() => {});
    else hiddenHere = new Set();
    // Smart albums are counted apart: one query each, so they never hold up the grid.
    if (albumsShown) api.smartCounts(asked).then((c) => { if (mine === generation) smart = new Map(c); }).catch(() => {});
    try {
      const first = api.query(asked, {
        sort: sortBy, limit: PAGE, offset: 0, facets: groups, facets_ignore_own: true,
      });
      // A photo picked while this ran is the one kept: by its row, as the grid it was picked
      // in goes. Several picked go as after an edit.
      let keptRow = keepRow;
      if (kept) {
        where = await kept;
        if (mine !== generation) return;
        if (picked.size > 1) keptRow = undefined;
        else if (focusRow !== keepRow) {
          keptRow = focusRow;
          where = keptRow === undefined ? null : await api.position(asked, { sort: sortBy }, keptRow).catch(() => null);
          if (mine !== generation) return;
        }
      }
      // In place, the pages on screen and the focused photo's (and the next, for the viewer)
      // are read again alongside the first and swapped in with the result in one go: until
      // then the grid and the details panel keep the old ones.
      const shown = grid?.range() ?? [0, 0];
      const offline = new Map<number, Set<number>>();
      const { result: r, pages: fresh } = keep
        ? await refreshPages(
            first,
            (total) => {
              const at = Math.min(where ?? selected ?? 0, total - 1);
              return [shown, [at, at + 2]];
            },
            result?.total ?? 0,
            (page) => api.query(asked, { sort: sortBy, limit: PAGE, offset: page * PAGE }).then((p) => {
              if (p.offline.length) offline.set(page, new Set(p.offline));
              return p.rows;
            }),
          )
        : await first.then((r) => ({ result: r, pages: new Map([[0, r.rows]]) }));
      if (mine !== generation) return;
      if (r.offline.length) offline.set(0, new Set(r.offline));
      result = r;
      counted = groups;
      pages = new SvelteMap(fresh);
      offlinePages = new SvelteMap(offline);
      loading = new Map();
      error = null;
      onScreen = shownNow();
      if (keptRow !== undefined) {
        // Set with the pages, so the focus never passes over the photo now at its old place;
        // one gone from the result leaves nothing picked, not whatever took its place.
        picked.clear();
        selected = anchor = null;
        if (where === null) return;
        if (rowAt(where) === undefined) {
          await need(where, where + 1);
          if (mine !== generation) return;
        }
        holdMap = keptRow;
        selected = anchor = where;
        picked.add(keptRow);
        await tick();
        holdMap = null;
        if (mine === generation) grid?.reveal(where);
      } else if (keep) {
        if (selected !== null && selected >= r.total) selected = r.total ? r.total - 1 : null;
        await followPicks(mine, asked);
      } else if (was) {
        const lost = await keepPicks(mine, asked, was);
        // Said for a filter picked here; going back or forward, Back would not find them.
        if (mine === generation && lost && was.picks.length > 1 && !how.travelling) {
          notice = { kind: "outside", lost, total: was.picks.length };
        }
      }
    } catch (e) {
      if (mine === generation) {
        error = String(e);
        carried = null;
      }
    }
  }

  const pickedNow = (): Picks => ({ picks: [...picked], focus: selected === null ? undefined : focusRow });
  /** The picks a new filter is looking for, until it has them in: a newer run that overtakes
   *  it looks for these, not for the selection it found cleared. Picking anew drops them. */
  let carried: Picks | null = null;

  /**
   * After a new filter ran: the photos picked before that it still shows stay picked, and
   * the focus stays on the focused one, or moves to the first of them, scrolled to; with
   * none of them left, the grid starts at the top with nothing picked. Says how many of the
   * picked photos it no longer shows.
   */
  async function keepPicks(mine: number, asked: Filter, was: Picks): Promise<number> {
    picked.clear();
    selected = anchor = null;
    const found = await api.positions(asked, { sort: sortBy }, wantedRows(was)).catch(() => []);
    if (mine !== generation) return 0;
    const { kept, at, lost } = keptPicks(was, found);
    if (at === null) {
      carried = null;
      viewing = false;
      grid?.reset();
      return lost;
    }
    await need(at, at + 1);
    if (mine !== generation) return 0;
    carried = null;
    const row = rowAt(at);
    for (const r of kept) picked.add(r);
    // The map stays where it is: the photo was picked before, not just now.
    holdMap = row ?? null;
    selected = anchor = at;
    focusRow = row;
    if (row !== was.focus) viewing = false;
    await tick();
    holdMap = null;
    if (mine === generation) grid?.center(at);
    return lost;
  }

  // Back and forward through the filters shown (Alt+← and Alt+→).
  const trail = new Trail();
  let canBack = $state(false);
  let canForward = $state(false);
  /** The filter on screen, once its result is in: what a new one leaves. */
  let onScreen: Shown | null = null;
  /** How a new filter came: `restore` picks these instead of the photos picked now;
   *  `travelling` is no new step and nothing said (going back or forward, or finishing a
   *  run overtaken); steps of one `kind` in a row
   *  (typing a search, clicking tiles) are one step back. */
  type Step = { restore?: Picks; travelling?: boolean; kind?: "search" | "tile" };
  const shownNow = (): Shown => ({
    filter: $state.snapshot(filter), sortField, sortReversed, match, bin: inBin, hidden: showHidden,
    tile: mapTile, follow, picks: [],
  });
  function leaving(was: Picks, how: Step) {
    notice = null;
    if (!onScreen || how.travelling) return;
    trail.leave({ ...onScreen, ...was }, shownNow(), how.kind);
    canBack = trail.back.length > 0;
    canForward = trail.forward.length > 0;
  }
  function travel(back: boolean) {
    const now = { ...shownNow(), ...pickedNow() };
    const to = back ? trail.goBack(now) : trail.goForward(now);
    canBack = trail.back.length > 0;
    canForward = trail.forward.length > 0;
    if (!to) return;
    clearTimeout(followTimer);
    clearTimeout(typing);
    // The area as it was: a tile stays the tile, a drawn area keeps the grid off the map's
    // moves (as drawing it did).
    mapTile = to.tile;
    if (!to.follow && to.filter.within && !to.tile) follow = false;
    inBin = to.bin;
    showHidden = to.hidden;
    filter = to.filter;
    search = to.filter.text ?? "";
    sortField = to.sortField;
    sortReversed = to.sortReversed;
    match = to.match;
    rememberSort();
    run(false, false, { restore: { picks: to.picks, focus: to.focus }, travelling: true });
  }
  /** The footer's word on the selection: picked photos the new filter left out, or the
   *  filter having followed an edit (and how many of the photos it does not show). */
  let notice = $state<{ kind: "outside" | "followed"; lost: number; total: number } | null>(null);
  $effect(() => {
    if (notice?.kind !== "followed") return;
    const gone = setTimeout(() => (notice = null), 8000);
    return () => clearTimeout(gone);
  });

  /**
   * After an edit re-ran the filter in place: photos that left the result leave the
   * selection, and a single selection follows the focus to whatever photo now sits there
   * (removing `animal` under an `animal` filter moves on to the next photo).
   */
  async function followPicks(mine: number, asked: Filter) {
    if (picked.size > 1) {
      const all = new Set((await api.positions(asked, { sort: sortBy }, [...picked])).map(([row]) => row));
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

  const rowAt = (i: number) => rowIn(pages, i);
  // The focused photo follows `selected` once its page is in.
  $effect(() => {
    if (selected === null) focusRow = undefined;
    else {
      const row = rowAt(selected);
      if (row !== undefined) focusRow = row;
    }
  });

  function load(page: number): Promise<void> {
    const pending = loading.get(page);
    if (pending) return pending;
    const mine = generation;
    const into = pages;
    const intoOffline = offlinePages;
    const p = api.query(asking($state.snapshot(filter)), { sort: sortBy, limit: PAGE, offset: page * PAGE })
      .then((r) => {
        if (mine !== generation) return;
        if (r.offline.length) intoOffline.set(page, new Set(r.offline));
        into.set(page, r.rows);
      })
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
      source.wording = status.wording;
      progress = status.progress;
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

  /** Opens the catalog this window was started on again: after another window that held its
   *  library is closed, or whatever else made opening it fail. */
  let retrying = $state(false);
  async function retryOpen() {
    retrying = true;
    try {
      const current = (await api.sources()).current;
      if (!current) {
        choosingSource = true;
        return;
      }
      await api.openSource(current);
      location.reload();
    } catch (e) {
      error = String(e);
    } finally {
      retrying = false;
    }
  }

  /** Reads what the open source holds; at start, once it is open. */
  function loadSource() {
    loaded = true;
    refresh();
    api.autosave().then(([on, state]) => { autosaveOn = on; autosaveState = state; }).catch(() => {});
    api.historyEnabled().then(showsHistory).catch(() => {});
    api.settingsProblem().then((p) => { if (p) editError = `Settings: ${p}`; }).catch(() => {});
    api.analysisState().then((s) => (analysis = s)).catch(() => {});
    api.describeState().then((s) => (describing = s)).catch(() => {});
    api.thumbsState().then((s) => (thumbs = s)).catch(() => {});
  }

  onMount(() => {
    const unwatch = api.watchBrokenImages();
    const off = [
      // The dialog while a source opens. The one the app started with has no session yet:
      // the window reads it when it is ready.
      listen<api.Startup | null>("startup-progress", (e) => {
        startup = e.payload;
        if (!e.payload && !loaded) loadSource();
      }),
      listen<api.ThumbsState>("thumbs", (e) => {
        thumbs = e.payload;
        // While a run makes them, tiles that showed nothing pick up their new thumbnails
        // every few seconds. (Photos that failed are not tried again until "Try again".)
        if (e.payload.running && Date.now() - thumbsRetried > 3000) {
          thumbsRetried = Date.now();
          api.retryBrokenImages();
        }
      }),
      listen("thumbs-done", () => api.retryBrokenImages()),
      // The details panel reads the photo (and its history) again: Capture One may have changed it.
      listen("library-changed", () => { syncNote = ""; syncFailed = null; version++; refresh(true); }),
      listen<api.SourceProgress | null>("source-progress", (e) => { progress = e.payload; }),
      listen<string>("sync-failed", (e) => { syncNote = "sync failed"; syncFailed = e.payload; }),
      listen<api.AutosaveState>("autosave", (e) => { autosaveState = e.payload; }),
      listen("queue-written", () => { edited(); }),
      listen<api.AnalysisState>("analysis", (e) => {
        analysis = e.payload;
        // Counts catch up now and then while photos are measured.
        if (Date.now() - countsAt > 20000) {
          countsAt = Date.now();
          run(true);
        }
      }),
      listen("analysis-done", () => { version++; analysed++; refresh(true); }),
      listen<api.DescribeState>("describe", (e) => { describing = e.payload; }),
      listen("describe-done", () => { version++; }),
    ];
    // Listening first, so the end of start-up is not missed between the question and the answer.
    Promise.all(off)
      .then(() => api.startupProgress())
      .then((p) => {
        if (!p) loadSource();
        else if (!loaded) startup = p;
      })
      .catch(() => loadSource());
    // The startup sync reports only changes; an unchanged catalog says nothing.
    const quiet = setTimeout(() => { if (syncNote === "syncing…") syncNote = ""; }, 3000);
    return () => { clearTimeout(quiet); unwatch(); off.forEach((p) => p.then((f) => f())); };
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
    for (const key of ["collections", "not_in_collections"] as const) {
      const kept = filter[key]?.filter((id) => ids.has(id));
      if (kept && kept.length !== filter[key]?.length) filter = { ...filter, [key]: kept.length ? kept : undefined };
    }
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
    const row = selected === null ? mapRow ?? undefined : focusRow;
    return row === undefined ? [] : [row];
  }

  /** What an edit changed: its photos, and the edit itself when it is one. */
  type EditDone = { rows: number[]; edit?: api.Edit };

  async function edited(done?: EditDone) {
    const focus = selected === null ? undefined : focusRow;
    version++;
    await refreshQueued();
    // Asked before the filter runs again, so an edit it follows shows no empty grid first.
    if (done?.rows.length && (await followEdit(done, focus))) return;
    await run(true);
  }

  /** The edit took every photo out of the filter: the filter goes where they went, with them
   *  picked, if it can (see lib/follow.ts). Says whether it did. */
  async function followEdit(done: EditDone, focus: number | undefined): Promise<boolean> {
    if (inBin) return false;
    const mine = generation;
    try {
      const now = $state.snapshot(filter);
      if ((await api.query(asking(now), { sort: sortBy, limit: 0 })).total) return false;
      const sel = await api.selectionSummary(done.rows);
      if (mine !== generation) return false;
      const next = followFilter(now, sel, done.rows.length, done.edit, match === "any");
      if (!next) return false;
      // Only to a filter that shows them: one the edit left empty for another reason stays.
      const found = await api.positions(asking(next), { sort: sortBy }, done.rows);
      if (!found.length || mine !== generation) return false;
      if (next.place_under) openPlace(next.place_under);
      filter = next;
      const ran = run(false, false, { restore: { picks: done.rows, focus: focus !== undefined && done.rows.includes(focus) ? focus : done.rows[0] } });
      const its = generation;
      await ran;
      if (its === generation) notice = { kind: "followed", lost: done.rows.length - found.length, total: done.rows.length };
      return true;
    } catch (e) {
      editError = String(e);
      return false;
    }
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
  /** What the picked photos have, read together; null for one. A reply for the pick before
   *  stays while the new one is read, so the panel dims it rather than going blank. */
  let selection = $state.raw<{ rows: number[]; data: api.Selection } | null>(null);
  let selectionError = $state<string | null>(null);
  // Not state: one read at a time. Picks changing faster (Shift+arrow over a big range) wait
  // for it, and then the newest pick is read once.
  let selectionReading = false;
  let selectionAgain = false;
  function readSelection() {
    if (selectionReading) {
      selectionAgain = true;
      return;
    }
    if (picked.size < 2 || !showInfo) {
      selection = null;
      return;
    }
    selectionReading = true;
    const rows = [...picked];
    api.selectionSummary(rows)
      .then((data) => {
        selection = { rows, data };
        selectionError = null;
      })
      .catch((e) => (selectionError = String(e)))
      .finally(() => {
        selectionReading = false;
        if (selectionAgain) {
          selectionAgain = false;
          readSelection();
        }
      });
  }
  $effect(() => {
    void [...picked];
    void version;
    void showInfo;
    untrack(readSelection);
  });
  /** Whether `selection` is about the photos picked now. */
  let selectionNow = $derived(!!selection && sameRows(selection.rows, new Set(picked)));
  /** How many of the picked photos have edits waiting. */
  let queuedPicked = $derived(picked.size > 1 ? [...picked].filter((r) => queued.has(r)).length : 0);
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
    const row = selected === null ? undefined : focusRow;
    if (row === undefined) return;
    copying = await api.photo(row);
  }

  async function openSync() {
    const row = selected === null ? undefined : focusRow;
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
    await edited({ rows });
  }

  /** What runs now, for the footer: an edit on many photos takes a while. */
  let busy = $state<string | null>(null);
  let running = 0;
  async function working(note: string, run: () => Promise<void>) {
    running++;
    busy = note;
    try {
      await run();
    } finally {
      if (--running === 0) busy = null;
    }
  }
  /** How many photos an undo or redo of these ops touches. */
  const opRows = (ops: [number, api.Op][]) => new Set(ops.map(([row]) => row)).size;
  /** The photos ops touch, for `edited` to follow. */
  const rowsOf = (ops: [number, api.Op][]): EditDone => ({ rows: [...new Set(ops.map(([row]) => row))] });

  async function applyEdit(edit: api.Edit, rows = targets()) {
    if (!rows.length) return;
    await working(editingNote("edit", rows.length), async () => {
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
      await edited({ rows, edit });
    });
  }

  /** How many photos an undo or redo touches, when it is about photos. */
  const doneRows = (d: Done | Undone) =>
    "ops" in d ? opRows(d.ops) : "hidden" in d ? d.hidden.length : "binned" in d ? d.binned.length : null;

  function undo() {
    const last = undoStack.at(-1);
    if (last) return working(editingNote("undo", doneRows(last)), undoNow);
  }

  function redo() {
    const last = redoStack.at(-1);
    if (last) return working(editingNote("redo", doneRows(last)), redoNow);
  }

  /** Hides the photos edited here from every view (`on`), or shows them again. A mark of
   *  C1 Studio's own: nothing is queued for Capture One. */
  async function setHidden(on: boolean, rows = targets()) {
    if (!rows.length) return;
    await working(on ? `hiding ${rows.length.toLocaleString()}…` : `showing ${rows.length.toLocaleString()}…`, async () => {
      try {
        const changed = await api.setHidden(rows, on);
        if (changed.length) {
          undoStack.push({ hidden: changed, on });
          redoStack = [];
          syncNote = hiddenNote(changed.length, on);
        }
        editError = null;
      } catch (e) {
        editError = String(e);
      }
      await edited();
    });
  }
  /** `h`: hides the photos edited here, or shows them again when all of them are hidden. Asks
   *  the library which are hidden: the tiles' marks are only known while hidden ones are shown,
   *  and may still be on their way. */
  let hiding = false;
  async function toggleHidden() {
    if (hiding) return;
    hiding = true;
    try {
      const rows = targets();
      const hidden = new Set(await api.hiddenOf(rows));
      await setHidden(hides(rows, hidden), rows);
    } catch (e) {
      editError = String(e);
    } finally {
      hiding = false;
    }
  }
  /** Whether every photo edited here is hidden: what the Hide button offers. */
  let targetsHidden = $state(false);
  $effect(() => {
    const rows = targets();
    void version;
    if (!rows.length) {
      targetsHidden = false;
      return;
    }
    api.hiddenOf(rows).then((h) => { if (targets().join() === rows.join()) targetsHidden = h.length === rows.length; }).catch(() => {});
  });
  /** Moves the photos edited here to the trash (`on`), out of every view but the trash's, or
   *  restores them. Nothing reaches the source until the trash is emptied. */
  async function setBinned(on: boolean, rows = targets()) {
    if (!rows.length) return;
    await working(on ? `deleting ${rows.length.toLocaleString()}…` : `restoring ${rows.length.toLocaleString()}…`, async () => {
      try {
        const changed = await api.setBinned(rows, on);
        if (changed.length) {
          undoStack.push({ binned: changed, on });
          redoStack = [];
          syncNote = binNote(changed.length, on);
        }
        editError = null;
      } catch (e) {
        editError = String(e);
      }
      await binChanged();
    });
  }
  /** What the Delete keys move to the trash: in the viewer, only the photo shown (the next one
   *  takes its place), else the photos edited here. */
  const deleting = () => (viewing && focusRow !== undefined ? [focusRow] : targets());
  /** After photos went into the trash or out of it: the views and its count follow. */
  async function binChanged() {
    await edited();
    api.status().then((s) => (status = s)).catch(() => {});
    if (viewing && !result?.total) viewing = false;
  }
  /** Shows the trash, or leaves it for the library. */
  function toggleBin() {
    inBin = !inBin;
    run();
  }

  /** Not remembered across starts: hidden photos are out again whenever the app opens. */
  function toggleShowHidden() {
    showHidden = !showHidden;
    run();
  }

  async function undoNow() {
    const last = undoStack.pop();
    if (!last) return;
    // An undo can empty the filter as the edit did: it follows the photos back.
    const done = "ops" in last ? rowsOf(last.ops) : undefined;
    try {
      if ("hidden" in last) {
        await api.setHidden(last.hidden, !last.on);
        redoStack.push(last);
      } else if ("binned" in last) {
        await api.setBinned(last.binned, !last.on);
        redoStack.push(last);
        await binChanged();
        return;
      } else if ("ops" in last) redoStack.push({ ops: await api.editOps(last.ops) });
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
    await edited(done);
  }

  async function redoNow() {
    const last = redoStack.pop();
    if (!last) return;
    const done = "ops" in last ? rowsOf(last.ops) : undefined;
    try {
      if ("hidden" in last) {
        await api.setHidden(last.hidden, last.on);
        undoStack.push(last);
      } else if ("binned" in last) {
        await api.setBinned(last.binned, last.on);
        undoStack.push(last);
        await binChanged();
        return;
      } else if ("ops" in last) undoStack.push({ ops: await api.editOps(last.ops) });
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
    await edited(done);
  }

  async function pick(i: number, how?: { add: boolean; range: boolean }) {
    const row = rowAt(i);
    notice = null;
    carried = null;
    if (how?.range && anchor !== null) {
      const [lo, hi] = anchor < i ? [anchor, i] : [i, anchor];
      await need(lo, hi + 1);
      if (!how.add) picked.clear();
      for (let k = lo; k <= hi; k++) {
        const r = rowAt(k);
        if (r !== undefined) picked.add(r);
      }
    } else if (how?.add && row !== undefined) {
      if (picked.size === 0 && selected !== null && focusRow !== undefined) picked.add(focusRow);
      if (picked.has(row)) picked.delete(row);
      else picked.add(row);
      anchor = i;
    } else {
      picked.clear();
      if (row !== undefined) picked.add(row);
      anchor = i;
    }
    selected = i;
    focusRow = row;
    mapRow = null;
  }

  async function pickAll() {
    if (!result) return;
    const all = await api.query(asking($state.snapshot(filter)), { sort: sortBy });
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
      run(false, false, { kind: "search" });
    }, 150);
  }

  // Flat sidebar values ↔ filter fields. A value is off → in → out → off.
  const cap = (v: string) => v[0].toUpperCase() + v.slice(1);
  /** The colours picked in, or none. */
  const colorsIn = () => (typeof filter.color === "object" ? filter.color.Values : undefined);
  function pickState(dim: Dim, value: string): Pick {
    switch (dim) {
      case "rating": {
        const r = filter.rating;
        if (value === "none") return r === "None" ? "in" : r === "Any" ? "out" : null;
        return sameRating(r, ratingFor(ratingOp, +value)) ? "in" : null;
      }
      case "color": {
        const c = filter.color;
        if (value === "none") return c === "None" ? "in" : c === "Any" ? "out" : null;
        return pickOf(colorsIn(), filter.colors_not, cap(value));
      }
      case "camera": return pickOf(filter.cameras, filter.cameras_not, value);
      case "hue": return filter.hues?.includes(value) ? "in" : null;
      case "year": return filter.year === +value ? "in" : null;
      case "country": return filter.country === value ? "in" : null;
      default: return null;
    }
  }

  function toggle(dim: Dim, value: string) {
    // Main colours add up, as in the analysis tab: any of those picked.
    if (dim === "hue") return toggleList("hues", value);
    switch (dim) {
      case "rating":
        if (value === "none") {
          const r = filter.rating;
          filter = { ...filter, rating: r === "None" ? "Any" : r === "Any" ? undefined : "None" };
        } else {
          filter = { ...filter, rating: pickState(dim, value) ? undefined : ratingFor(ratingOp, +value) };
        }
        break;
      case "color":
        if (value === "none") {
          const c = filter.color;
          filter = { ...filter, color: c === "None" ? "Any" : c === "Any" ? undefined : "None" };
        } else {
          const { inc, exc } = cycle(colorsIn(), filter.colors_not, cap(value));
          filter = { ...filter, color: inc && { Values: inc as api.Color[] }, colors_not: exc as api.Color[] | undefined };
        }
        break;
      case "camera": {
        const { inc, exc } = cycle(filter.cameras, filter.cameras_not, value);
        filter = { ...filter, camera: undefined, cameras: inc, cameras_not: exc };
        break;
      }
      case "year": filter = { ...filter, year: pickState(dim, value) ? undefined : +value }; break;
      case "country": filter = { ...filter, country: pickState(dim, value) ? undefined : value }; break;
      default: return;
    }
    run();
  }

  let match = $state<Match>("all");
  let ratingOp = $state<RatingOp>("≥");
  /** Analysis cards asking for every pick ("red and blue") instead of any. */
  let allOf = $state<AllOf>({});
  /** The filter as the index is asked: Match applied, and the "all" cards' picks combined. */
  function compiled(f: Filter): Filter {
    return withBin(withHidden(allOfPicks(compile(f, match), allOf), showHidden), inBin);
  }
  function toggleAllOf(card: keyof AllOf) {
    allOf = { ...allOf, [card]: !allOf[card] };
    run();
  }
  // A card's picks all cleared: it goes back to "any", so the next picks are not silently "all".
  $effect(() => {
    const hues = !!filter.hues?.length;
    const problems = !!filter.light?.some((l) => LIGHT_PROBLEMS.includes(l));
    untrack(() => {
      if ((allOf.hues && !hues) || (allOf.problems && !problems)) {
        allOf = { hues: allOf.hues && hues, problems: allOf.problems && problems };
      }
    });
  });
  /** What the rest of the app asks the index: the filter with Match applied. */
  let eff = $derived(compiled(filter));
  function setMatch(m: Match) {
    match = m;
    run();
  }
  function setRatingOp(op: RatingOp) {
    const was = ratingParts(filter.rating);
    ratingOp = op;
    if (was) filter = { ...filter, rating: ratingFor(op, was.n) };
    run();
  }
  /** "No album" / "No place" / "No location" rows: off → none → has some → off. */
  function toggleNone(key: "uncollected" | "placed" | "located") {
    const none = key === "uncollected";
    const v = filter[key];
    filter = { ...filter, [key]: v === undefined ? none : v === none ? !none : undefined };
    run();
  }
  function toggleUnkeyworded() {
    filter = { ...filter, unkeyworded: filter.unkeyworded ? undefined : true };
    run();
  }

  // Trees: picking a node filters by it, picking it again lets go.
  function pickAlbum(n: Node) {
    // A smart album can only be included: the index has no "not this rule".
    if (byId.get(n.value)?.kind === "Smart") {
      const on = filter.collections?.includes(n.value);
      const left = (filter.collections ?? []).filter((v) => v !== n.value);
      const next = on ? left : [...left, n.value];
      filter = { ...filter, collections: next.length ? next : undefined };
    } else {
      const { inc, exc } = cycle(filter.collections, filter.not_in_collections, n.value);
      filter = { ...filter, collections: inc, not_in_collections: exc };
    }
    run();
  }
  /** A value of a many-valued filter field on or off (the analysis' groups). */
  function toggleList(key: "shape" | "hues" | "light" | "focus", value: string) {
    const list = filter[key] ?? [];
    const next = list.includes(value) ? list.filter((v) => v !== value) : [...list, value];
    filter = { ...filter, [key]: next.length ? next : undefined };
    run();
  }
  function toggleFlag(flag: "duplicates" | "series" | "analysed") {
    const on = flag === "analysed" ? filter.analysed === false : !!filter[flag];
    filter = { ...filter, [flag]: on ? undefined : flag === "analysed" ? false : true };
    run();
  }
  function pickFolder(n: Node) {
    filter = { ...filter, folder: filter.folder === n.value ? undefined : n.value };
    run();
  }
  function pickPlace(n: Node) {
    const city = cityWithoutLocation(n.value);
    if (city) {
      // A city's photos with no location: the city, and No location.
      const on = filter.place_under === city && filter.located === false;
      filter = { ...filter, place_under: on ? undefined : city, located: on ? undefined : false };
    } else filter = { ...filter, place_under: filter.place_under === n.value ? undefined : n.value };
    run();
  }
  /** The place tree's entry the filter picks: a city with No location is its "(no location)". */
  let placePicked = $derived.by(() => {
    const p = filter.place_under;
    if (!p) return undefined;
    const noLocation = filter.located === false && places.some(function has(n): boolean {
      return n.value === `${p}|—` || n.children.some(has);
    });
    return "p:" + (noLocation ? `${p}|—` : p);
  });
  /** Photos in view with a country but no ISO code (from the index's flags). */
  let missingCodes = $derived(result?.facets.flag?.find(([v]) => v === "no_country_code")?.[1] ?? 0);
  let importingGpx = $state(false);
  let fillingCodes = $state(false);
  async function codesFilled(undo: [number, api.Op][]) {
    if (undo.length) {
      undoStack.push({ ops: undo });
      redoStack = [];
    }
    await edited(rowsOf(undo));
  }
  /** Photos given positions in bulk (a track, their neighbours): offer their places too. */
  async function placedMany(undo: [number, api.Op][]) {
    await codesFilled(undo);
    const placed = undo.filter(([, op]) => "SetGeo" in op).map(([row]) => row);
    if (placed.length) void offerFill(placed);
  }
  let placingFromNeighbours = $state(false);

  /** Empty place fields that photos just given a position could have filled from it: filled
   *  at once if Settings say so, else offered. */
  let fillOffer = $state<{ groups: api.FillGroup[]; rows: number[] } | null>(null);
  async function offerFill(rows: number[]) {
    try {
      const groups = await api.fillPlan(rows);
      if (!groups.length) return;
      if (settings.fillPlaces) await fillPlaces(groups.flatMap((g) => g.rows));
      else fillOffer = { groups, rows: groups.flatMap((g) => g.rows) };
    } catch (e) {
      editError = String(e);
    }
  }
  async function fillPlaces(rows: number[]) {
    fillOffer = null;
    let undo: [number, api.Op][] = [];
    try {
      undo = await api.fillPlaces(rows);
      if (undo.length) {
        undoStack.push({ ops: undo });
        redoStack = [];
        const n = new Set(undo.map(([r]) => r)).size;
        note(`Place filled in for ${n.toLocaleString()} photo${n === 1 ? "" : "s"}`);
      }
      editError = null;
    } catch (e) {
      editError = String(e);
    }
    await edited(rowsOf(undo));
  }
  function placeLine(p: api.Photo["place"]): string {
    return [p.city, p.state, p.country].filter(Boolean).join(", ");
  }
  /** The photos edited that have no position get their place's (approximate). */
  async function geoFromPlace() {
    let undo: [number, api.Op][] = [];
    try {
      undo = await api.promoteRows(targets());
      if (undo.length) {
        undoStack.push({ ops: undo });
        redoStack = [];
        const n = new Set(undo.map(([r]) => r)).size;
        note(`${n.toLocaleString()} photo${n === 1 ? "" : "s"} given the position of ${n === 1 ? "its" : "their"} place (approximate)`);
      } else {
        note("No position to give: the photos have one already, or no place known on the map");
      }
      editError = null;
    } catch (e) {
      editError = String(e);
    }
    await edited(rowsOf(undo));
  }
  /** The photos at a place on the map (of those with no position) get its position. */
  async function promotePlace(paths: string[]) {
    let undo: [number, api.Op][] = [];
    try {
      undo = await api.promotePlaces({ ...compiled($state.snapshot(filter)), within: undefined }, paths);
      if (undo.length) {
        undoStack.push({ ops: undo });
        redoStack = [];
        const n = new Set(undo.map(([r]) => r)).size;
        note(`${n.toLocaleString()} photo${n === 1 ? "" : "s"} given the position of ${placeText(paths[0])}${paths.length > 1 ? ` and ${paths.length - 1} more places` : ""} (approximate)`);
      } else {
        note("Nothing to do: these photos have a position already");
      }
      editError = null;
    } catch (e) {
      editError = String(e);
    }
    await edited(rowsOf(undo));
  }
  function pickKeyword(n: Node) {
    const { inc, exc } = cycle(filter.keywords_under, filter.keywords_under_not, n.value);
    filter = { ...filter, keywords_under: inc, keywords_under_not: exc };
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

  // Trips: loaded here for both the sidebar and the timeline, counted under the filter.
  let tripList = $state.raw<api.TripList | null>(null);
  let tripsAsked = 0;
  async function refreshTrips(): Promise<api.TripList | null> {
    const mine = ++tripsAsked;
    try {
      const got = await api.trips(compiled($state.snapshot(filter)));
      if (mine === tripsAsked) tripList = got;
    } catch (e) {
      if (mine === tripsAsked) editError = String(e);
    }
    return tripList;
  }
  // The counts set the picked trip aside (see Library::trips): picking one asks nothing.
  let tripKey = $derived(JSON.stringify({ ...filter, trip: undefined }));
  $effect(() => {
    void tripKey;
    void version;
    if (status) untrack(refreshTrips);
  });
  /** The trip picked last, by id: its span may change when trips are found again. */
  let pickedTrip: number | null = null;
  /** The trip the filter picks: the one picked, else one with just that span. */
  let shownTrip = $derived.by(() => {
    const span = filter.trip;
    if (!span || !tripList) return null;
    return (
      tripList.trips.find((t) => t.id === pickedTrip) ??
      tripList.trips.find((t) => t.start === span.from && t.end === span.to) ??
      null
    );
  });
  // Found again with another span (photos added, a split): the grid follows the trip.
  $effect(() => {
    const t = shownTrip;
    if (t && filter.trip && (t.start !== filter.trip.from || t.end !== filter.trip.to)) {
      untrack(() => {
        filter = { ...filter, trip: { from: t.start, to: t.end } };
        run(true);
      });
    }
  });
  /** Picking a trip lets go of picked days, which may lie outside it. */
  function pickTrip(t: api.Trip | null) {
    pickedTrip = t?.id ?? null;
    filter = t
      ? { ...filter, trip: { from: t.start, to: t.end }, captured: undefined, year: undefined, month: undefined, week: undefined }
      : { ...filter, trip: undefined };
    run();
  }
  function pickDays(range: api.TimeRange | null) {
    filter = { ...filter, captured: range ?? undefined, year: undefined, month: undefined, week: undefined };
    run();
  }
  /** A new album at the top level, named after the trip, holding its photos. */
  async function exportTrip(t: api.Trip) {
    try {
      const rows = (await api.query({ trip: { from: t.start, to: t.end } }, { sort: "Date" })).rows;
      const name = `${tripName(t)} · ${tripDates(t, true)}`;
      const [seq, change] = await api.changeCollection({ Create: { id: "", name, kind: "Album", parent: null, rule: null } });
      undoStack.push({ collection: seq, change });
      redoStack = [];
      await refreshCollections();
      if ("Create" in change) await applyEdit({ AddToAlbum: change.Create.id }, rows);
      if (!editError) note(`Album “${name}” made with ${rows.length.toLocaleString()} photos; Write sends it to ${source.word}`);
    } catch (e) {
      editError = String(e);
    }
  }

  function clearAll() {
    mapTile = null;
    filter = {};
    search = "";
    run();
  }

  /** The values a flat group shows. */
  function values(dim: Dim): [string, number][] {
    const all = result?.facets[dim] ?? [];
    if (dim === "year" || dim === "rating") return [...all].sort((a, b) => b[0].localeCompare(a[0]));
    if (dim === "hue") return all;
    return all.slice(0, 10);
  }

  function label(dim: Dim, value: string) {
    if (dim === "rating") return value === "none" ? "unrated" : "★".repeat(+value);
    return value;
  }

  const TITLES: Record<string, string> = {
    rating: "Rating", color: "Colour", hue: "Main colour", year: "Year", date: "Date", camera: "Camera", country: "Country",
    albums: "Albums", folders: "Folders", keywords: "Keywords", places: "Location", look: "Look", trips: "Trips",
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

  // Not state: where a step still waiting for its page goes. Quick presses count from
  // it, so none is lost; only the newest step moves the viewer.
  let stepping: number | null = null;
  async function step(d: number) {
    const from = stepping ?? selected;
    if (from === null || !result) return;
    const to = from + d;
    if (to < 0 || to >= result.total) return;
    stepping = to;
    try {
      await need(to, to + 2);
    } catch {
      // The page could not be read: stay where we are.
      if (stepping === to) stepping = null;
      return;
    }
    if (stepping !== to) return;
    stepping = null;
    selected = to;
    focusRow = rowAt(to);
    if (followStep(picked, focusRow)) anchor = to;
  }

  function close() {
    if (selected !== null) grid?.reveal(selected);
    viewing = false;
  }

  /** The History tab shown or not; hidden, it is left, and a pick from it goes. */
  function showsHistory(on: boolean) {
    historyOn = on;
    if (on) return;
    if (tab === "history") showTab("photos");
    if (filter.history) {
      filter = { ...filter, history: undefined };
      run();
    }
  }
  async function toggleHistory(on: boolean) {
    try {
      await api.setHistory(on);
      showsHistory(on);
    } catch (e) {
      editError = String(e);
    }
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
      case "pending": return [`saving to ${source.word} when you pause…`, false];
      case "waiting":
        return [/captureone|imgcore/i.test(s.by) ? `autosave waits for ${source.owner ?? s.by} to close` : `autosave waits: the ${source.noun} is open in another program`, false];
      case "attention": {
        const parts = [s.conflicts && `${s.conflicts} conflict${s.conflicts > 1 ? "s" : ""}`, s.blocked && `${s.blocked} that cannot be written`];
        return [`autosave paused: ${parts.filter(Boolean).join(", ")}`, true];
      }
      case "saved": {
        const time = new Date(s.at).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
        return [`saved ${s.written} edit${s.written === 1 ? "" : "s"} to ${source.word} at ${time}`, false];
      }
      case "failed": return [`autosave failed: ${s.error}`, true];
      default: return ["", false];
    }
  }

  let menuOpen = $derived(!!(keywordMenu || placeMenu || albumMenu || sectionMenu));
  /** A dialog is open: its keys are its own, not the grid's or the viewer's. */
  let dialogOpen = $derived(
    !!(writing || showBackups || emptying || showSettings || sectionMenu || pasting || keywordDialog || placeDialog || albumDialog || fillingCodes || importingGpx || placingFromNeighbours || copying || syncing),
  );

  function onkey(e: KeyboardEvent) {
    if (e.key === "Escape" && menuOpen) {
      keywordMenu = null;
      placeMenu = null;
      albumMenu = null;
      sectionMenu = null;
      // Handled: the viewer, listening after us, must not close too.
      e.preventDefault();
      return;
    }
    if (dialogOpen || typingIn(e) || e.defaultPrevented) return;
    const ctrl = e.ctrlKey || e.metaKey;
    const key = e.key.toLowerCase();
    if (ctrl && e.altKey && !e.shiftKey && key === "v") {
      pasteClip(true);
      e.preventDefault();
      return;
    }
    if (ctrl && !e.altKey) {
      if (e.key === "Delete" && deleteAction({ ctrl, album: !!shownAlbum, inBin, removes: status?.removes }) === "bin") setBinned(true, deleting());
      else if (key === "z" && !e.shiftKey) undo();
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
    if (e.altKey && !ctrl && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
      travel(e.key === "ArrowLeft");
      e.preventDefault();
      return;
    }
    if (e.altKey) return;
    if (/^[0-5]$/.test(e.key)) applyEdit({ Rating: +e.key || null });
    else if (e.key in COLOR_KEYS) applyEdit({ Color: COLOR_KEYS[e.key] });
    else if (e.key === "i") showInfo = !showInfo;
    else if (e.key === "h" && !e.repeat) toggleHidden();
    else if ((e.key === "Delete" || e.key === "Backspace") && shownAlbum && !viewing && !inBin) {
      const album = shownAlbum;
      const n = targets().length;
      applyEdit({ RemoveFromAlbum: album.id }).then(() => {
        if (!editError) note(`${n.toLocaleString()} photo${n === 1 ? "" : "s"} taken out of “${album.name}”`);
      });
    }
    else if (e.key === "Delete" && deleteAction({ ctrl: false, album: false, inBin, removes: status?.removes }) === "bin") {
      setBinned(true, deleting());
    }
    else if (viewing) return; // the viewer has its own keys
    else if (e.key === "Escape" && mapOn && mapTile && picked.size <= 1) mapTilePick(null, null);
    else if (e.key === "Escape" && picked.size > 1) {
      picked.clear();
      const row = selected === null ? undefined : focusRow;
      if (row !== undefined) picked.add(row);
    } else if (e.key === "Enter" && selected !== null) viewing = true;
    // On Analysis, History and AI the page keys scroll the view; arrows move in the strip.
    else if ((tab === "analysis" || tab === "history" || tab === "ai") && ["PageUp", "PageDown", "Home", "End"].includes(e.key)) return;
    else if (grid?.move(e.key, e.shiftKey)) e.preventDefault();
  }

  // Links in the info panel: filter by what was clicked.
  function only(f: Filter) {
    filter = { ...filter, ...f };
    run();
  }
  /** A duplicate group's or a series' photos, from the Analysis tab or the panel: within the
   *  other filters, which stay. The Analysis tab shows them in its strip; elsewhere, the grid. */
  function showGroup(group: string) {
    filter = { ...filter, group };
    if (tab !== "analysis") showTab("photos");
    run();
  }
  /** The photos a part of the change log is about, from the History tab: within the other
   *  filters, which stay. The tab keeps them in its strip. */
  function showHistory(history: api.HistoryPick) {
    filter = { ...filter, history };
    run();
  }
  function showKeyword(path: string) {
    // Open the branches above it so the pick is visible in the tree.
    const parts = path.split("|");
    for (let i = 1; i < parts.length; i++) expanded.add("k:" + parts.slice(0, i).join("|"));
    only({ keywords_under: [path] });
  }
  /** Opens the Location tree's branches above a place, so its pick is in sight. */
  function openPlace(path: string) {
    const parts = path.split("|");
    for (let i = 1; i < parts.length; i++) expanded.add("p:" + parts.slice(0, i).join("|"));
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
  type Chip = { out?: boolean; what?: string; text: string; title?: string; dot?: string; stars?: boolean; rest?: unknown };
  const without = <T,>(list: T[], v: T) => {
    const left = list.filter((x) => x !== v);
    return left.length ? left : undefined;
  };
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
    trip: (v) => [{ what: "Trip", text: shownTrip ? tripName(shownTrip) : rangeText(v), title: rangeText(v) }],
    camera: (v) => [{ what: "Camera", text: v }],
    cameras: (v) => v.map((c) => ({ what: "Camera", text: c, rest: without(v, c) })),
    cameras_not: (v) => v.map((c) => ({ what: "Camera", text: `not ${c}`, out: true, rest: without(v, c) })),
    colors_not: (v) =>
      v.map((c) => ({ what: "Colour", text: `not ${c.toLowerCase()}`, out: true, dot: COLORS[c.toLowerCase()], rest: without(v, c) })),
    country: (v) => [{ what: "Country", text: v }],
    keywords_under: (v) => v.map((p) => ({ what: "Keyword", text: p.split("|").join(" › "), title: p, rest: without(v, p) })),
    keywords_under_not: (v) =>
      v.map((p) => ({ what: "Keyword", text: `not ${p.split("|").join(" › ")}`, out: true, title: p, rest: without(v, p) })),
    unkeyworded: (v) => (v ? [{ text: "no keywords" }] : []),
    not_in_collections: (v) =>
      v.map((id) => ({ what: "Album", text: `not ${byId.get(id)?.name ?? id}`, out: true, title: byId.get(id)?.path, rest: without(v, id) })),
    uncollected: (v) => [{ what: "Album", text: v ? "none" : "any" }],
    all_of: () => [],
    any_of: () => [],
    collections: (v) =>
      v.map((id) => ({ what: "Album", text: byId.get(id)?.name ?? id, title: byId.get(id)?.path, rest: without(v, id) })),
    folder: (v) => [{ what: "Folder", text: v }],
    place_under: (v) => [{ what: "Place", text: placeText(v) }],
    gps: (v) => [{ what: "GPS", text: v ? "has a position" : "no position" }],
    placed: (v) => [{ what: "Place", text: v ? "has a place" : "no place" }],
    located: (v) => [{ what: "Location", text: v ? "has a location" : "no location" }],
    approx: (v) => [{ what: "GPS", text: v ? "none of its own" : "its own" }],
    no_country_code: (v) => (v ? [{ text: "no ISO country code" }] : []),
    shape: (v) => v.map((x) => ({ what: "Shape", text: x, rest: without(v, x) })),
    hues: (v) => v.map((x) => ({ what: allOf.hues && v.length > 1 ? "Main colour, all of" : "Main colour", text: x, rest: without(v, x) })),
    light: (v) => v.map((x) => ({ what: allOf.problems && LIGHT_PROBLEMS.includes(x) && v.filter((p) => LIGHT_PROBLEMS.includes(p)).length > 1 ? "Light, all of" : "Light", text: x, rest: without(v, x) })),
    focus: (v) => v.map((x) => ({ what: "Focus", text: x, rest: without(v, x) })),
    history: (v) => [{ what: "History", text: pickText(v, (id) => (byId.get(id) ? `“${byId.get(id)?.name}”` : id)), title: "Photos the change log says changed" }],
    group: (v) => [{ text: v.startsWith("dup:") ? "one photo's duplicates" : "one series" }],
    duplicates: (v) => (v ? [{ text: "duplicates" }] : []),
    series: (v) => (v ? [{ text: "in a series" }] : []),
    analysed: (v) => [{ text: v ? "measured" : "not measured yet" }],
    // Set by the header's Hidden button, not a filter the user picks.
    hidden: () => [],
    // Set by the Trash row, whose bar says so; the source's own trash is never shown.
    bin: () => [],
    trashed: () => [],
    // Set from the header's Stack button, not a filter the user picks.
    stacked: () => [],
    offline: (v) => [{ text: v ? "offline" : "online" }],
    within: (v) => [{ what: "Map", text: sameBox(v, mapTile) ? "one tile" : "area" }],
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
    // A picked tile dropped: the grid goes back to the map's view. The map's area dropped:
    // the grid stops following it, or it would come straight back.
    if (key === "within") {
      if (mapTile) mapTile = null;
      else follow = false;
    }
    filter = { ...filter, [key]: rest };
    if (key === "text") search = "";
    run();
  }

  function fold(key: string) {
    if (folded.has(key)) folded.delete(key);
    else folded.add(key);
  }
</script>

{#snippet head(key: SectionKey)}
  <button
    class="head"
    onclick={() => fold(key)}
    oncontextmenu={(e) => { e.preventDefault(); sectionMenu = { x: e.clientX, y: e.clientY, key }; }}
  >
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
  {@const row = selected === null ? mapRow ?? undefined : focusRow}
  <Info
    {row}
    {version}
    {analysed}
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
    onaddkeywords={(paths) => applyEdit({ AddKeywords: paths })}
    collections={byId}
    onkeyword={showKeyword}
    onalbum={showAlbum}
    onaddalbum={(id) => applyEdit({ AddToAlbum: id })}
    onremovealbum={(id) => applyEdit({ RemoveFromAlbum: id })}
    ongroup={showGroup}
    onfolder={showFolder}
    oncamera={(camera) => only({ cameras: [camera] })}
    summary={selection?.data ?? null}
    stale={!selectionNow}
    summaryError={selectionError}
    {queuedPicked}
    onplace={(fields, rows) => applyEdit({ Place: fields }, rows ?? targets())}
    ongeo={async (value) => {
      const rows = targets();
      await applyEdit({ Geo: value }, rows);
      if (value && !editError) void offerFill(rows);
    }}
    onfillplace={() => fillPlaces(targets())}
    ongeofromplace={geoFromPlace}
    oniptc={(field, value) => applyEdit({ Iptc: { [field]: value } })}
    clip={clipText}
    oncopy={openCopy}
    onpaste={pasteClip}
    onpastechoose={() => { if (clipboard) pasting = targets(); }}
    onsync={openSync}
    allHidden={targetsHidden}
    onhide={toggleHidden}
    {inBin}
    ondelete={status && status.removes !== "None" ? () => setBinned(!inBin) : undefined}
  />
{/snippet}

<svelte:window onkeydown={onkey} ondragend={() => (dragging = null)} />

<div class="app" class:with-info={showInfo} class:busy={!!busy} aria-busy={!!busy}>
  <header>
    <SourceMenu bind:open={choosingSource} onbackups={() => (showBackups = true)} onchanged={() => refresh(true)} />
    <span class="trail">
      <button aria-label="Back" title="Back to the filter before, with its photos picked (Alt+←)" disabled={!canBack} onclick={() => travel(true)}>‹</button>
      <button aria-label="Forward" title="Forward again (Alt+→)" disabled={!canForward} onclick={() => travel(false)}>›</button>
    </span>
    <input class="search" placeholder="Search keywords, captions, places, filenames" bind:value={search} oninput={onSearch} />
    <nav class="tabs" aria-label="View">
      <button class:on={tab === "photos"} onclick={() => showTab("photos")}>Photos</button>
      <button class:on={tab === "map"} title="Where the photos were taken" onclick={() => showTab("map")}>Map</button>
      <button class:on={tab === "analysis"} title="What the photos look like: colour, light, focus, duplicates" onclick={() => showTab("analysis")}>Analysis</button>
      {#if historyOn}
        <button class:on={tab === "history"} title="The {source.noun} over time: what changed between two dates, and the change log" onclick={() => showTab("history")}>History</button>
      {/if}
      <button class:on={tab === "ai"} title="Captions, keywords and ratings suggested by a vision model, to review" onclick={() => showTab("ai")}>AI</button>
    </nav>
    <div class="sort-slot" title="The order of the photos in the grid and in the strip under the map and the analysis">
      <SortControl
        fields={FIELDS.map((f) => ({
          key: f.key,
          label: f.label,
          disabled: f.key === "match" && !matchAvailable(filter),
          hint: f.key === "match" ? "pick a colour, light or focus filter" : undefined,
        }))}
        field={sortShown}
        words={words(sortShown, sortShown === sortField && sortReversed)}
        label="Sort photos by"
        onfield={(key) => setSortField(key as SortField)}
        onflip={flipSort}
      />
    </div>
    <button
      class="toggle"
      class:on={stack}
      aria-pressed={stack}
      title="Show one photo per burst or duplicate group (its sharpest). With a filter on, a frame is hidden only when the sharpest frame of its group matches too; open a group to see all its frames."
      onclick={toggleStack}
    >Stack</button>
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
    <!-- Its own slot, there or not, so the header doesn't move as photos are picked. -->
    <span class="copy-slot">
      <button
        class="copy-to"
        class:hidden={!(picked.size > 1 && selected !== null)}
        title="Copy fields of the focused photo into the other selected photos: choose which, overwrite or merge (Ctrl+Shift+S)"
        onclick={openSync}
      >⇉ Copy to {Math.max(1, picked.size - 1).toLocaleString()} selected</button>
    </span>
    <button
      class="toggle"
      class:on={showHidden}
      aria-pressed={showHidden}
      title={showHidden ? "Hidden photos are shown, marked on their tiles: click to leave them out again" : "Show the photos hidden with Hide (h); they are left out of every view and count until then"}
      onclick={toggleShowHidden}
    >Hidden</button>
    <button class="toggle" class:on={showTimeline} title="Photos over time, with trips: drag across it to pick days" onclick={() => (showTimeline = !showTimeline)}>Timeline</button>
    <button class="toggle" class:on={showInfo} title="Details (i)" onclick={() => (showInfo = !showInfo)}>Details</button>
    <button class="toggle settings" title="Settings: the sidebar's sections, online map services, the History tab, AI" aria-label="Settings" onclick={() => (showSettings = true)}>
      <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.3">
        <circle cx="8" cy="8" r="2.2" />
        <path d="M8 1.5v2M8 12.5v2M1.5 8h2M12.5 8h2M3.4 3.4l1.4 1.4M11.2 11.2l1.4 1.4M3.4 12.6l1.4-1.4M11.2 4.8l1.4-1.4" stroke-linecap="round" />
        <circle cx="8" cy="8" r="4.6" />
      </svg>
    </button>
    <span class="count">
      {#if result}{result.total.toLocaleString()} photos · {(result.micros / 1000).toFixed(1)} ms{/if}
    </span>
  </header>

  <aside>
    {#each shownSections as key (key)}
      {#if key === "albums"}
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
            <button
              class="none-row"
              class:on={filter.uncollected === true}
              class:out={filter.uncollected === false}
              title="Photos in no album · click again for photos in some album"
              onclick={() => toggleNone("uncollected")}
            ><span class="name">{filter.uncollected === false ? "In an album" : "No album"}</span></button>
            <Tree
              nodes={albums}
              selected={undefined}
              mark={(n) => pickOf(filter.collections, filter.not_in_collections, n.value)}
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
      {:else if key === "date"}
        {@render dates()}
      {:else if key === "trips"}
        <section>
          {@render head("trips")}
          {#if !folded.has("trips")}
            <TripsSection
              list={tripList}
              selected={shownTrip?.id ?? null}
              focusRow={selected === null ? undefined : focusRow}
              onpick={pickTrip}
              onchanged={refreshTrips}
              onexport={exportTrip}
              onerror={(text) => (editError = text)}
            />
          {/if}
        </section>
      {:else if key === "places"}
        <section>
          {@render head("places")}
          {#if !folded.has("places")}
            <button
              class="none-row"
              class:on={filter.placed === false}
              class:out={filter.placed === true}
              title="Photos without a place · click again for photos with one"
              onclick={() => toggleNone("placed")}
            ><span class="name">{filter.placed === true ? "With a place" : "No place"}</span></button>
            <button
              class="none-row"
              class:on={filter.located === false}
              class:out={filter.located === true}
              title="Photos with nothing in the Location field · click again for photos with a location"
              onclick={() => toggleNone("located")}
            ><span class="name">{filter.located === true ? "With a location" : "No location"}</span></button>
            <Tree
              nodes={places}
              selected={placePicked}
              {expanded}
              onpick={pickPlace}
              oncontext={(node, e) => { if (!cityWithoutLocation(node.value)) placeMenu = { x: e.clientX, y: e.clientY, node }; }}
            />
            {#if missingCodes}
              <button class="tree-action" onclick={() => (fillingCodes = true)} title="Fill in the ISO codes of these photos' countries">
                {missingCodes.toLocaleString()} without an ISO country code · Fill in…
              </button>
            {/if}
          {/if}
        </section>
      {:else if key === "folders"}
        <section>
          {@render head("folders")}
          {#if !folded.has("folders")}
            <Tree nodes={folders} selected={filter.folder && "f:" + filter.folder} {expanded} onpick={pickFolder} />
          {/if}
        </section>
      {:else if key === "keywords"}
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
            <button
              class="none-row"
              class:on={filter.unkeyworded}
              title="Photos without any keyword"
              onclick={toggleUnkeyworded}
            ><span class="name">No keywords</span></button>
            <Tree
              nodes={keywordView.nodes}
              selected={undefined}
              mark={(n) => pickOf(filter.keywords_under, filter.keywords_under_not, n.value)}
              expanded={keywordSearch.trim() ? searchOpen : expanded}
              onpick={pickKeyword}
              oncontext={(node, e) => (keywordMenu = { x: e.clientX, y: e.clientY, node })}
            />
            {#if keywordSearch.trim() && !keywordView.nodes.length}<p class="tree-none">No keyword matches.</p>{/if}
          {/if}
        </section>
      {:else}
        {@const dim = key}
        {@const list = values(dim)}
        {#if list.length}
          <section>
            {@render head(dim)}
            {#if !folded.has(dim)}
              {#if dim === "rating"}
                <div class="rating-op">
                  Stars
                  <span class="seg" role="group" aria-label="Rating comparison">
                    {#each RATING_OPS as op}
                      <button class:on={ratingOp === op} onclick={() => setRatingOp(op)} title={op === "≥" ? "At least" : op === "≤" ? "At most" : "Exactly"}>{op}</button>
                    {/each}
                  </span>
                </div>
              {/if}
              {#each list as [value, n]}
                {@const state = pickState(dim, value)}
                <button
                  class="facet"
                  class:on={state === "in"}
                  class:out={state === "out"}
                  title="Click to include · again to exclude"
                  onclick={() => toggle(dim, value)}
                >
                  {#if dim === "color"}<i class="dot" style:background={COLORS[value]}></i>{/if}
                  {#if dim === "hue"}<i class="dot" style:background={HUES[value] ?? HUES.muted}></i>{/if}
                  <span class="name" class:stars={dim === "rating" && value !== "none"}>{label(dim, value)}</span>
                  <span class="n">{n.toLocaleString()}</span>
                </button>
              {/each}
            {/if}
          </section>
        {/if}
      {/if}
    {/each}
    {#if status && status.removes !== "None"}
      <button
        class="trash-row"
        class:on={inBin}
        aria-pressed={inBin}
        title={inBin ? "Back to the library" : "Photos deleted here (Delete): restore them, or empty the trash to delete them for good"}
        onclick={toggleBin}
      ><span class="name">Trash</span><span class="n">{status.binned.toLocaleString()}</span></button>
    {/if}
  </aside>

  {#if active || inBin}
    <div class="filters" role="group" aria-label="Filters">
      {#if inBin && status}
        <span class="bin-bar">
          <b>Trash</b>
          <span class="muted">deleted here; {status.removes === "Files" ? "their files stay" : `${source.name} keeps them`} until the trash is emptied</span>
          <button disabled={!targets().length} title="Back into every view (Ctrl+Z after a delete does too)" onclick={() => setBinned(false)}>Restore</button>
          <button disabled={!targets().length} title="Delete the picked photos for good, now" onclick={() => (emptying = { rows: targets() })}>Delete for good…</button>
          <button class="danger" disabled={!status.binned} title="Delete every photo in the trash for good" onclick={() => (emptying = {})}>Empty trash…</button>
          <button title="Back to the library" onclick={toggleBin}>Close</button>
        </span>
      {/if}
      {#each chips as chip}
        <span class="chip" class:out={chip.out} title={chip.title}>
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
      {#if criteriaCount(filter) >= 2}
        <span class="match">
          Match
          <span class="seg" role="group" aria-label="Match all or any">
            <button class:on={match === "all"} onclick={() => setMatch("all")} title="Photos must satisfy every criterion">All</button>
            <button class:on={match === "any"} onclick={() => setMatch("any")} title="Photos may satisfy any one of the rating, colour, camera, keyword, album and place picks; the rest still apply">Any</button>
          </span>
        </span>
      {/if}
      {#if active}<button class="clear" onclick={clearAll}>Clear filters</button>{/if}
    </div>
  {/if}

  <div class="pane" class:strip={stripped} class:splitting bind:this={paneEl} bind:clientHeight={paneH} style:--strip="{stripShown}px">
  {#if showTimeline && tab !== "analysis" && tab !== "history" && tab !== "ai" && status}
    <Timeline
      filter={eff}
      {version}
      trips={tripList?.settings.home ? tripList.trips : []}
      trip={shownTrip}
      onrange={pickDays}
      ontrip={pickTrip}
      onerror={(text) => (editError = text)}
    />
  {/if}
  {#if mapOn}
    <!-- The map's own tools, by the map: the header has no room for them beside the rest. -->
    <div class="map-tools" role="toolbar" aria-label="Map">
      <label class="follow" title="The grid shows only the photos the map shows, and the map centres on the photo picked in the grid. Off: the grid keeps its own filter, and the map is just a place to look and to drop photos.">
        <input type="checkbox" checked={follow} onchange={toggleFollow} /> sync grid and map
      </label>
      <button class="toggle" title="Give the photos in view positions from a recorded GPX track, by when they were taken" onclick={() => (importingGpx = true)}>GPX…</button>
      <button class="toggle" title="Place the filtered photos from the photos taken just before and after them: positions from their GPS, places from theirs" onclick={() => (placingFromNeighbours = true)}>Neighbours…</button>
    </div>
    <MapPane
      filter={eff}
      geoPerAsset={status?.geo_per_asset}
      {version}
      focus={selected === null ? mapRow : (focusRow ?? null)}
      dragRows={dragging && "rows" in dragging ? dragging.rows : null}
      pickedRows={[...picked]}
      tile={mapTile}
      trip={shownTrip && { name: tripName(shownTrip) }}
      onbox={mapBox}
      onview={mapMoved}
      center={mapCenter}
      ontile={mapTilePick}
      onselect={mapSelect}
      onsetgeo={mapSetGeo}
      onerror={(text) => (editError = text)}
      onplace={(path) => showOffMap({ place_under: path, approx: true, gps: undefined, placed: undefined })}
      onpromote={promotePlace}
      ondragrows={(rows) => (dragging = rows && { rows })}
      onneighbours={() => (placingFromNeighbours = true)}
      onunplaced={(placed) =>
        showOffMap(
          placed
            ? { placed, approx: true, gps: undefined, place_under: undefined }
            : { placed, gps: false, approx: undefined, place_under: undefined },
        )}
    />
  {/if}
  {#if tab === "analysis"}
    <AnalysisPane
      filter={eff}
      facets={result?.facets ?? {}}
      total={result?.total ?? 0}
      {analysis}
      {version}
      {stack}
      {allOf}
      picked={filter}
      ontoggle={toggleList}
      onflag={toggleFlag}
      ongroup={showGroup}
      onshow={() => showTab("photos")}
      onstack={toggleStack}
      onclasses={() => { version++; analysed++; refresh(true); }}
      onallof={toggleAllOf}
    />
  {/if}
  {#if tab === "history" && historyOn}
    <HistoryPane
      filter={eff}
      {version}
      wording={source.wording}
      albumName={(id) => byId.get(id)?.name}
      picked={filter.history}
      onpick={showHistory}
      onerror={(text) => (editError = text)}
    />
  {/if}
  <!-- Once the source is open: before, there is no library to read the settings from. -->
  {#if tab === "ai" && loaded}
    <AiPane
      picked={aiTargets}
      describe={describing}
      {version}
      settingsOpen={showSettings}
      onsettings={() => { settingsTab = "ai"; showSettings = true; }}
      onselect={(row) => { picked.clear(); selected = null; mapRow = row; }}
      onaccepted={async (undo) => {
        if (undo.length) {
          undoStack.push({ ops: undo });
          redoStack = [];
          note(`Queued suggestions for ${opRows(undo).toLocaleString()} photo${opRows(undo) === 1 ? "" : "s"}`);
        }
        editError = null;
        await edited(rowsOf(undo));
      }}
      onerror={(m) => (editError = m)}
    />
  {/if}
  {#if stripped}
    <!-- A window splitter: a separator that can be focused and moved with the arrow keys. -->
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <div
      class="splitter"
      role="separator"
      aria-orientation="horizontal"
      aria-label="Height of the photo strip"
      aria-valuemin={STRIP_MIN}
      aria-valuenow={stripShown}
      tabindex="0"
      title="Drag to resize; arrow keys too, Home resets"
      onpointerdown={splitStart}
      onkeydown={splitKey}
      ondblclick={() => setStrip(STRIP_DEFAULT)}
    ></div>
  {/if}
  <Grid
    bind:this={grid}
    total={result?.total ?? 0}
    {rowAt}
    {need}
    {selected}
    picked={(i) => picked.size > 1 && picked.has(rowAt(i) ?? -1)}
    queued={(i) => queued.has(rowAt(i) ?? -1)}
    hidden={(i) => hiddenHere.has(rowAt(i) ?? -1)}
    offline={(i) => offlinePages.get(Math.floor(i / PAGE))?.has(i % PAGE) ?? false}
    onselect={pick}
    ondragcell={dragCell}
    onopen={(i) => { pick(i); viewing = true; }}
    {tiles}
    {shape}
    ontiles={(t) => (tiles = t)}
    bind:layout
  >
    {#if error}
      <p class="error">{isLockError(error) ? LOCK_HELP : error}</p>
      {#if isLockError(error)}<p class="error small">{error}</p>{/if}
      {#if !status}
        <p class="empty">
          <button class="link" disabled={retrying} onclick={retryOpen}>{retrying ? "Trying…" : "Try again"}</button>
          ·
          <button class="link" onclick={() => (choosingSource = true)}>Open a catalog or photo folders…</button>
        </p>
      {/if}
    {:else if syncFailed && status?.photos === 0}
      <p class="error">Nothing could be read from this source.<br />{syncFailed}</p>
    {:else if result && result.total === 0}
      <p class="empty">Nothing matches.</p>
    {/if}
  </Grid>
  </div>

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
      {#if source.owner}
        <span>{source.owner}: {typeof status.writes === "object" ? "open" : "closed"}</span>
      {/if}
      {#if status.offline_roots.length}
        <span class="offline-roots" title={`Not reachable at the last scan:\n${status.offline_roots.join("\n")}\nTheir photos stay, dimmed, with their edits.`}>
          {offlineText(status.offline_roots)}
        </span>
      {/if}
      {#if progress}
        <span class="sync" title={progress.note ?? ""}>{progressText(progress)}</span>
      {:else if status.can_rescan}
        <button class="rescan" title="Look at the folders again for new, moved and removed photos" onclick={() => api.rescan().catch((e) => (editError = String(e)))}>Rescan</button>
      {/if}
    {/if}
    {#if fillOffer}
      {@const first = fillOffer.groups[0]}
      <span class="fill-offer" title={fillOffer.groups.map((g) => `${placeLine(g.place)}: ${g.rows.length}`).join("\n")}>
        Fill in the place of {fillOffer.rows.length.toLocaleString()} photo{fillOffer.rows.length === 1 ? "" : "s"}:
        {placeLine(first.place)}{fillOffer.groups.length > 1 ? ` and ${fillOffer.groups.length - 1} more` : ""}?
        <button onclick={() => fillPlaces(fillOffer!.rows)}>Fill</button>
        <button title="Fill places from new positions from now on without asking (Settings)" onclick={() => { settings.fillPlaces = true; fillPlaces(fillOffer!.rows); }}>Always</button>
        <button onclick={() => (fillOffer = null)}>No</button>
      </span>
    {/if}
    {#if busy}<span class="busy">{busy}</span>{:else if editError}<button class="edit-error" title="Click to dismiss" onclick={() => (editError = null)}>{editError}</button>{:else if editNote}<span class="edit-note">{editNote}</span>{/if}
    {#if notice?.kind === "followed"}
      <span class="fill-offer edit-note">
        The filter followed your edit{#if notice.lost}; {notice.lost.toLocaleString()} of the {notice.total.toLocaleString()} photos are not in it{/if}
        <button title="Back to the filter before (Alt+←)" onclick={() => { notice = null; travel(true); }}>Back</button>
      </span>
    {:else if notice}
      <span class="fill-offer edit-note">
        {notice.lost.toLocaleString()} of {notice.total.toLocaleString()} picked photos are outside this filter
        <button title="Back to the filter before, with all of them picked (Alt+←)" onclick={() => travel(true)}>Show them</button>
        <button aria-label="Dismiss" onclick={() => (notice = null)}>×</button>
      </span>
    {/if}
    {#if syncNote}<span class="sync" class:failed={!!syncFailed} title={syncFailed ?? ""}>{syncNote}</span>{/if}
    {#if thumbs?.running}
      <span class="sync" title="From each photo's preview, or its original when there is no preview">
        making thumbnails {thumbs.done.toLocaleString()} / {thumbs.total.toLocaleString()}
      </span>
    {:else if thumbs?.missing}
      {#if thumbs.failed}
        <button
          class="warn"
          title={[
            `${thumbs.failed.toLocaleString()} photo${thumbs.failed === 1 ? "" : "s"} could not get a thumbnail:`,
            ...thumbs.reasons.slice(0, 6).map(([why, n]) => `  ${n.toLocaleString()} — ${why}`),
            "",
            "Click to open the events log: each photo, where its original was looked for, and why.",
          ].join("\n")}
          onclick={() => api.openLog().catch((e) => (editError = String(e)))}
        >⚠ {thumbs.failed.toLocaleString()} failed</button>
      {/if}
      <button
        class="thumbs"
        title={thumbs.total && thumbs.failed
          ? "Try again, for instance once the drive with the originals is connected."
          : `${source.name} has no thumbnail for these. Make them from each photo's preview, or its original when there is none.`}
        onclick={() => api.fillThumbnails().catch((e) => (editError = String(e)))}
      >{thumbs.missing.toLocaleString()} photo{thumbs.missing === 1 ? "" : "s"} without a thumbnail · {thumbs.total ? "Try again" : "Make them"}</button>
    {/if}
    {#if analysis?.running && analysis.total}
      <span class="sync" title="Shape, colours, light, focus and a likeness hash, from each photo's thumbnail">
        measuring photos {analysis.done.toLocaleString()} / {analysis.total.toLocaleString()}
      </span>
    {/if}
    {#if describing?.running && describing.total}
      <button class="sync" title="Photos described by the vision model; the AI tab shows what it suggests" onclick={() => showTab("ai")}>
        {#if describing.units[0]?.sheet}
          {@const unit = describing.units[0]}
          describing photos: sheet {unit.call} / {describing.calls_total.toLocaleString()} · {unit.phase === "preparing"
            ? "making it"
            : unit.phase === "retrying" ? "trying again" : "sent"}
        {:else}
          describing photos {describing.done.toLocaleString()} / {describing.total.toLocaleString()}{describing.working.length ? ` · ${describing.working.length} at work` : ""}
        {/if}
      </button>
    {/if}
    <span class="autosave" class:alone={!syncNote}>
      {#if autosaveOn}
        {@const [note, attention] = autosaveText(autosaveState)}
        {#if attention}
          <button class="attention" onclick={() => (writing = true)}>{note}</button>
        {:else if note}
          <span>{note}</span>
        {/if}
      {/if}
      <label title={`Write queued edits to ${source.word} by itself, a few seconds after you pause${source.owner ? `, whenever ${source.owner} is closed` : ""}`}>
        <input type="checkbox" checked={autosaveOn} onchange={(e) => toggleAutosave(e.currentTarget.checked)} />
        Autosave
      </label>
    </span>
    {#if queuedCount}
      <button class="queue" onclick={() => (writing = true)}>
        {queuedCount.toLocaleString()} edit{queuedCount === 1 ? "" : "s"} waiting · Write to {source.word}…
      </button>
    {/if}
    {#if status}
      <button
        class="log"
        title="Events log: what was read from and written to {source.word}, thumbnails made, and what failed"
        aria-label="Open the events log"
        onclick={() => api.openLog().catch((e) => (editError = String(e)))}
      >ⓘ Log</button>
    {/if}
  </footer>
</div>

{#if sectionMenu}
  {@const key = sectionMenu.key}
  <div class="menu-shield" role="presentation" onclick={() => (sectionMenu = null)} oncontextmenu={(e) => { e.preventDefault(); sectionMenu = null; }}></div>
  <div class="menu" role="menu" style:left="{sectionMenu.x}px" style:top="{sectionMenu.y}px">
    <div class="menu-title">{TITLES[key]}</div>
    <button role="menuitem" onclick={() => { hideSection(key); sectionMenu = null; }}>Hide this section</button>
    <button role="menuitem" onclick={() => { showSettings = true; sectionMenu = null; }}>Sidebar settings…</button>
  </div>
{/if}

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
      <div class="menu-note">{source.owner ? `${source.owner}'s own: changed only there.` : `Made from ${source.word}: changed only on disk.`}</div>
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

{#if importingGpx}
  <GpxDialog filter={eff} onclose={() => (importingGpx = false)} ondone={placedMany} />
{/if}

{#if placingFromNeighbours}
  <NeighbourDialog filter={eff} onclose={() => (placingFromNeighbours = false)} ondone={placedMany} />
{/if}

{#if fillingCodes}
  <CountryCodesDialog filter={eff} onclose={() => (fillingCodes = false)} ondone={codesFilled} />
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

{#if startup}
  <StartupDialog progress={startup} />
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

{#if showSettings}
  <SettingsDialog
    titles={TITLES}
    history={historyOn}
    onhistory={toggleHistory}
    initial={settingsTab}
    onclose={() => { showSettings = false; settingsTab = "sidebar"; }}
    onclasses={() => { version++; analysed++; refresh(true); }}
  />
{/if}

{#if emptying && status}
  <EmptyBinDialog
    count={status.binned}
    rows={emptying.rows}
    removes={status.removes}
    onclose={() => (emptying = null)}
    ondone={() => { picked.clear(); binChanged(); }}
  />
{/if}

{#if showBackups}
  <BackupsDialog onclose={() => (showBackups = false)} onrestored={() => { syncNote = ""; refresh(true); }} />
{/if}

{#if writing}
  <WriteDialog onclose={() => (writing = false)} onchanged={() => { refreshKeywords(); edited(); }} />
{/if}

{#if viewing && selected !== null && focusRow !== undefined}
  <Viewer row={focusRow} next={rowAt(selected + 1)} {version} blocked={dialogOpen || menuOpen} onclose={close} onstep={step}>
    {#if showInfo}{@render details()}{/if}
  </Viewer>
{/if}
