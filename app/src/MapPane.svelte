<script lang="ts">
  import * as maplibregl from "maplibre-gl";
  import "maplibre-gl/dist/maplibre-gl.css";
  // Vite bundles the worker with what it imports; maplibre's own lookup can't in dev.
  import workerUrl from "maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url";
  import { onMount, tick, untrack } from "svelte";
  import * as api from "./lib/api";
  import type { BBox, Cluster, Filter, PlaceCluster } from "./lib/api";
  import { parseGeo } from "./lib/format";
  import { hitView, mapPadding, parseHits, routeStep, viewBox, wrapLon, type Hit, type NominatimRow } from "./lib/mapview";
  import { movingOf } from "./lib/pins";
  import { pointFor } from "./lib/placeHints";
  import { settings } from "./lib/settings.svelte";
  import UnplacedTray from "./UnplacedTray.svelte";

  let {
    filter, version, focus, dragRows, pickedRows, tile, trip, onbox, onview, ontile, onpick, onsetgeo, onerror,
    onplace, onpromote, ondragrows, onneighbours, onunplaced,
  }: {
    /** What the grid is filtered by; the map shows those of its photos that have a position. */
    filter: Filter;
    /** Bumped when photos were edited: positions may have changed. */
    version: number;
    /** The photo the details panel shows. */
    focus: number | null;
    /** Photos being dragged from the grid, to be dropped on the map. */
    dragRows: number[] | null;
    /** The photos picked in the strip: *Put them here* places them at a searched place. */
    pickedRows: number[];
    /** The tile picked: the grid shows just its photos. */
    tile: BBox | null;
    /** The trip the grid shows: its route can be drawn and played. */
    trip: { name: string } | null;
    /** Shift+drag picked an area: the grid shows just its photos. */
    onbox: (bounds: BBox) => void;
    /** A tile clicked (its photos' bounds, and the photo when it is one), or let go (null). */
    ontile: (bounds: BBox | null, row: number | null) => void;
    /** What the map shows now (null: not known yet); the grid can follow it. */
    onview: (bounds: BBox) => void;
    onpick: (row: number) => void;
    /** Photos moved to a place: dragged markers, or photos dropped from the grid. */
    /** Photos placed; `from` is the tile they were moved from, when a pin was dragged. */
    onsetgeo: (rows: number[], lat: number, lon: number, from?: BBox) => void;
    onerror: (text: string) => void;
    /** Show the photos at a place (a `place_tree` path) in the grid. */
    onplace: (path: string) => void;
    /** Give the photos at a place (of those with no position) the place's position. */
    onpromote: (paths: string[]) => void;
    /** Photos dragged from the tray of photos not on the map (null: let go). */
    ondragrows: (rows: number[] | null) => void;
    /** Open placing from neighbours in time. */
    onneighbours: () => void;
    /** Show the photos with no position, with a place (true) or without, in the grid. */
    onunplaced: (placed: boolean) => void;
  } = $props();

  // OpenFreeMap: free vector tiles, no key. Painted light, turned dark by a CSS filter
  // on the canvas (markers are not on it).
  const STYLE = "https://tiles.openfreemap.org/styles/positron";
  /** Without online services: a plain ground with a line every 10°, made here. */
  const OFFLINE: maplibregl.StyleSpecification = {
    version: 8,
    sources: { graticule: { type: "geojson", data: graticule(10) } },
    layers: [
      { id: "ground", type: "background", paint: { "background-color": "#e9e7e1" } },
      { id: "graticule", type: "line", source: "graticule", paint: { "line-color": "#c9c5bb", "line-width": 1 } },
    ],
  };
  function graticule(step: number): maplibregl.GeoJSONSourceSpecification["data"] {
    const lines: [number, number][][] = [];
    for (let lon = -180; lon <= 180; lon += step) lines.push([[lon, -85], [lon, 0], [lon, 85]]);
    for (let lat = -80; lat <= 80; lat += step) lines.push([[-180, lat], [0, lat], [180, lat]]);
    return { type: "Feature", properties: {}, geometry: { type: "MultiLineString", coordinates: lines } };
  }
  const START = "map-view";
  maplibregl.setWorkerUrl(workerUrl);

  let box: HTMLDivElement;
  let map: maplibregl.Map | undefined;
  let failed = $state<string | null>(null);
  let located = $state(0);
  let inView = $state(0);
  /** Photos placed only by their place's name: how many, in view, and hidden by the zoom. */
  let approx = $state({ located: 0, inView: 0, hidden: 0, placeOnly: 0, unresolved: 0, unplaced: 0 });
  const markers = new Map<string, { marker: maplibregl.Marker; el: HTMLElement }>();
  const approxMarkers = new Map<string, { marker: maplibregl.Marker; el: HTMLElement }>();
  /** The diameter of the dot a photo's pin stands on (matches `.pin .dot`). */
  const DOT = 10;
  let asked = 0;

  // What covers the map, measured: zooms and flights keep the photos clear of it. Where each
  // sits is as in the styles below: the find box 8px from the top, the note 8px, the tray
  // 32px and the route bar 30px from the bottom.
  let width = $state(0);
  let height = $state(0);
  let find: HTMLFormElement;
  let findHeight = $state(0);
  let trayHeight = $state(0);
  let routeHeight = $state(0);
  let noteHeight = $state(0);
  let padding = $derived(
    mapPadding({
      width,
      height,
      findBottom: 8 + findHeight,
      // The tray shows without a trip, the route bar with one.
      coverBottom: Math.max(8 + noteHeight, trip ? 30 + routeHeight : 32 + trayHeight),
    }),
  );

  /** The filter without the area: moving the map must not refetch what it shows by itself. */
  let key = $derived(JSON.stringify({ ...filter, within: undefined }));

  function remembered(): { center: [number, number]; zoom: number } {
    try {
      const v = JSON.parse(localStorage.getItem(START) ?? "null");
      // Stored unwrapped before (a centre of 499.77 after panning round the world).
      if (Array.isArray(v?.center) && typeof v.zoom === "number") return { center: [wrapLon(v.center[0]), v.center[1]], zoom: v.zoom };
    } catch {
      // Starts over the world instead.
    }
    return { center: [15, 30], zoom: 1.5 };
  }

  function view(): { bounds: BBox; zoom: number } {
    const b = map!.getBounds();
    return { bounds: viewBox(b.getWest(), b.getSouth(), b.getEast(), b.getNorth()), zoom: map!.getZoom() };
  }

  async function refresh() {
    if (!map) return;
    const mine = ++asked;
    const v = view();
    try {
      const r = await api.map(untrack(() => $state.snapshot(filter)), v);
      if (mine !== asked) return;
      located = r.located;
      inView = r.in_view;
      approx = {
        located: r.approx_located,
        inView: r.approx_in_view,
        hidden: r.approx_hidden,
        placeOnly: r.place_only,
        unresolved: r.unresolved,
        unplaced: r.unplaced,
      };
      draw(r.clusters, v.zoom);
      drawApprox(settings.approxPlaces ? r.approx : [], v.zoom);
      // Places known only coarsely (a city, for a street in it) are worth asking too.
      if (r.place_only > 0) lookUp();
    } catch (e) {
      if (mine === asked) onerror(String(e));
    }
  }

  // Places the gazetteer does not know well enough: looked up online, a few at a time
  // (Nominatim takes one request a second), once per filter. Requests failing one after
  // another (offline, blocked) stop the round; the next is tried after a pause.
  let lookedUp = "";
  let lookUpRunning = false;
  let retryAt = 0;
  const FAILS_IN_A_ROW = 3;
  const RETRY_MS = 5 * 60_000;
  /** Places still to look up, for the note. */
  let lookingUp = $state(0);
  async function lookUp() {
    const k = key;
    if (lookUpRunning || lookedUp === k || Date.now() < retryAt || !settings.onlineMap || !settings.approxPlaces) return;
    lookedUp = k;
    lookUpRunning = true;
    let done = false;
    try {
      const list = await api.unresolvedPlaces(untrack(() => ({ ...$state.snapshot(filter), within: undefined })));
      lookingUp = list.length;
      let batch: { path: string; found: api.Approx | null }[] = [];
      let fails = 0;
      for (const u of list) {
        if (k !== key || !settings.onlineMap || !map) break;
        const found = await pointFor(u.place, u.have);
        lookingUp--;
        // A failed request is asked again another time.
        if (found === undefined) {
          if (++fails >= FAILS_IN_A_ROW) {
            retryAt = Date.now() + RETRY_MS;
            break;
          }
          continue;
        }
        fails = 0;
        batch.push({ path: u.path, found });
        if (batch.length >= 5) {
          await api.setPlacePoints(batch);
          batch = [];
          refresh();
        }
      }
      if (batch.length) {
        await api.setPlacePoints(batch);
        refresh();
      }
      done = k === key && retryAt <= Date.now();
    } catch (e) {
      retryAt = Date.now() + RETRY_MS;
      onerror(String(e));
    } finally {
      lookingUp = 0;
      lookUpRunning = false;
      // Not finished for this filter (failed, or the filter changed meanwhile): the next
      // refresh asks again; a changed filter is asked at once.
      if (!done) lookedUp = "";
      if (k !== key) lookUp();
    }
  }

  const LEVEL_WORD: Record<api.Level, string> = { Country: "country", State: "region", City: "city", Location: "place" };
  /** `Poland|Malopolska|Kraków` → `Kraków, Malopolska, Poland`. */
  function placeLabel(path: string | null): string {
    return (path ?? "").split("|").filter((l) => l && l !== "—").reverse().join(", ");
  }
  const short = (n: number) => (n > 999 ? `${Math.round(n / 100) / 10}k` : String(n));

  /** Photos placed by name: hollow, dashed and marked ≈, never draggable. */
  function drawApprox(list: PlaceCluster[], zoom: number) {
    const bucket = zoom < 5 ? "far" : "near";
    const seen = new Set<string>();
    for (const c of list) {
      const id = `${bucket}:${c.best}:${c.count}:${c.level}:${c.place ?? ""}:${c.lat.toFixed(5)}:${c.lon.toFixed(5)}`;
      seen.add(id);
      if (approxMarkers.has(id)) continue;
      const el = document.createElement("div");
      el.className = "pin approx";
      if (bucket === "far") {
        const px = Math.round(22 + 9 * Math.log10(Math.max(1, c.count)));
        el.classList.add("disc");
        el.style.width = el.style.height = `${px}px`;
        el.textContent = `≈${short(c.count)}`;
      } else {
        const card = document.createElement("div");
        card.className = "card";
        const img = document.createElement("img");
        img.src = api.thumbUrl(c.best);
        img.draggable = false;
        img.alt = "";
        const n = document.createElement("span");
        n.textContent = `≈${short(c.count)}`;
        card.append(img, n);
        el.append(card);
      }
      const atOne = !!c.place && !!c.paths?.length;
      el.title = atOne
        ? `≈ ${placeLabel(c.place)}${c.places > 1 ? ` (${c.places} places)` : ""}: ${c.count.toLocaleString()} photo${c.count === 1 ? "" : "s"} placed by name, shown at their ${LEVEL_WORD[c.level]} · click for more`
        : `≈ ${c.places} places, ${c.count.toLocaleString()} photos placed by name · click to zoom in`;
      // A card hangs below its point, so photos with GPS standing on theirs above stay in sight.
      const marker = new maplibregl.Marker({ element: el, anchor: bucket === "far" ? "center" : "top", offset: [0, bucket === "far" ? 0 : 4] })
        .setLngLat([c.lon, c.lat])
        .addTo(map!);
      el.addEventListener("click", (e) => {
        e.stopPropagation();
        if (atOne) {
          const p = map!.project([c.lon, c.lat]);
          popup = { cluster: c, x: p.x, y: p.y, confirm: false };
        } else {
          const { min_lat, max_lat, min_lon, max_lon } = c.bounds;
          map!.fitBounds([[min_lon, min_lat], [max_lon, max_lat]], { maxZoom: 11, padding, absolutePadding: true });
        }
      });
      el.addEventListener("dblclick", (e) => e.stopPropagation());
      approxMarkers.set(id, { marker, el });
    }
    for (const [id, m] of approxMarkers) {
      if (!seen.has(id)) {
        m.marker.remove();
        approxMarkers.delete(id);
      }
    }
  }

  /** A place's marker clicked: what can be done with its photos. */
  let popup = $state<{ cluster: PlaceCluster; x: number; y: number; confirm: boolean } | null>(null);
  function zoomToPlace(c: PlaceCluster) {
    popup = null;
    const next = { Country: 5, State: 8, City: 11, Location: 14 }[c.level];
    map?.easeTo({ center: [c.lon, c.lat], zoom: Math.max(map.getZoom(), next), padding });
  }

  function moved(e?: maplibregl.MapMovementEvent & { padded?: boolean }) {
    // Only the padding changed: the map shows what it showed.
    if (e?.padded) return;
    // Panned round the world: back to the same place within -180..180, without a move to
    // see. The jump ends in another `moveend`, which does the rest.
    // (Not `LngLat.wrap()`: it changes the last digits of a longitude already in range.)
    const at = map!.getCenter();
    if (at.lng < -180 || at.lng > 180) {
      map!.jumpTo({ center: [wrapLon(at.lng), at.lat] });
      return;
    }
    try {
      // The middle of the whole canvas, not of the padded part: what is left free changes
      // with the tray and the find box, so the view comes back where it was.
      const c = map!.unproject([box.clientWidth / 2, box.clientHeight / 2]);
      localStorage.setItem(START, JSON.stringify({ center: [c.lng, c.lat], zoom: map!.getZoom() }));
    } catch {
      // Not remembered, that's all.
    }
    onview(view().bounds);
    refresh();
  }

  function zoomTo(c: Cluster) {
    const { min_lat, max_lat, min_lon, max_lon } = c.bounds;
    if (min_lat === max_lat && min_lon === max_lon) {
      map!.easeTo({ center: [c.lon, c.lat], zoom: Math.min(19, map!.getZoom() + 3), padding });
    } else {
      map!.fitBounds([[min_lon, min_lat], [max_lon, max_lat]], { maxZoom: 18, padding, absolutePadding: true });
    }
  }

  function pinFor(c: Cluster, zoom: number): HTMLElement {
    const el = document.createElement("div");
    el.className = "pin";
    if (zoom < 5 && c.count > 1) {
      // Far out: a disc, its size following the number of photos.
      const px = Math.round(22 + 9 * Math.log10(c.count));
      el.classList.add("disc");
      el.style.width = el.style.height = `${px}px`;
      el.textContent = c.count > 999 ? `${Math.round(c.count / 100) / 10}k` : String(c.count);
      return el;
    }
    // Near: the photo on a stem, the dot at its foot being exactly where it is.
    const card = document.createElement("div");
    card.className = "card";
    const img = document.createElement("img");
    img.src = api.thumbUrl(c.best);
    img.draggable = false;
    img.alt = "";
    card.append(img);
    if (c.count > 1) {
      const n = document.createElement("span");
      n.textContent = c.count > 999 ? `${Math.round(c.count / 100) / 10}k` : String(c.count);
      card.append(n);
    }
    const stem = document.createElement("div");
    stem.className = "stem";
    const dot = document.createElement("div");
    dot.className = "dot";
    el.append(card, stem, dot);
    return el;
  }

  function draw(clusters: Cluster[], zoom: number) {
    const bucket = zoom < 5 ? "far" : "near";
    const seen = new Set<string>();
    for (const c of clusters) {
      const id = `${bucket}:${c.best}:${c.count}:${c.lat.toFixed(6)}:${c.lon.toFixed(6)}`;
      seen.add(id);
      if (markers.has(id)) continue;
      const el = pinFor(c, zoom);
      const single = c.count === 1;
      // A disc sits on its place; a photo stands on it, its dot (DOT px) on the spot.
      const pinned = !el.classList.contains("disc");
      // One photo, or a stack of them all at one spot, moves by its pin; photos spread
      // over a tile do not, as dropping them would heap them on one spot.
      const moves = single ? [c.best] : pinned ? (c.rows ?? []) : [];
      el.title = single
        ? "Drag to move; the dot is where it goes"
        : moves.length
          ? `${c.count.toLocaleString()} photos at one spot · drag to move them together, or pick some of them below to move just those`
          : `${c.count.toLocaleString()} photos`;
      el.dataset.row = String(c.best);
      el.dataset.count = String(c.count);
      el.dataset.bounds = JSON.stringify(c.bounds);
      const marker = new maplibregl.Marker({
        element: el,
        draggable: moves.length > 0,
        anchor: pinned ? "bottom" : "center",
        offset: pinned ? [0, DOT / 2] : [0, 0],
      })
        .setLngLat([c.lon, c.lat])
        .addTo(map!);
      // A click picks the tile (or lets it go); a double click also zooms in to it.
      let dragged = false;
      el.addEventListener("click", (e) => {
        e.stopPropagation();
        if (dragged) return;
        if (e.detail > 1) return;
        // The picked tile itself lets go; one of its photos, zoomed in, is picked instead.
        if (tile && sameBox(tile, c.bounds)) ontile(null, null);
        else ontile(c.bounds, single ? c.best : null);
      });
      el.addEventListener("dblclick", (e) => {
        e.stopPropagation();
        ontile(c.bounds, single ? c.best : null);
        if (!single) zoomTo(c);
      });
      if (moves.length) {
        marker.on("dragstart", () => {
          dragged = true;
          el.classList.add("moving");
        });
        marker.on("dragend", () => {
          el.classList.remove("moving");
          // The click that ends a drag comes after this; it is not a click on the tile.
          setTimeout(() => (dragged = false));
          const at = marker.getLngLat();
          // Some of the stack picked in the grid: they go, the rest stay (and so does
          // the picked tile).
          const some = movingOf(moves, pickedRows);
          onsetgeo(some, at.lat, ((at.lng + 540) % 360) - 180, some === moves ? c.bounds : undefined);
        });
      }
      markers.set(id, { marker, el });
    }
    for (const [id, m] of markers) {
      if (!seen.has(id)) {
        m.marker.remove();
        markers.delete(id);
      }
    }
    mark();
  }

  function sameBox(a: BBox, b: BBox): boolean {
    return a.min_lat === b.min_lat && a.max_lat === b.max_lat && a.min_lon === b.min_lon && a.max_lon === b.max_lon;
  }

  /** `outer` holds all of `inner`. */
  function covers(outer: BBox, inner: BBox): boolean {
    return inner.min_lat >= outer.min_lat && inner.max_lat <= outer.max_lat && inner.min_lon >= outer.min_lon && inner.max_lon <= outer.max_lon;
  }

  function mark() {
    for (const { el } of markers.values()) {
      el.classList.toggle("focus", focus !== null && el.dataset.row === String(focus) && el.dataset.count === "1");
      // Zoomed in, the picked tile's photos stand in several tiles, zoomed out in a bigger
      // one: all of them show it. (Not by their average place, which rounding can put a
      // hair outside the bounds of photos all at one spot.)
      const b = JSON.parse(el.dataset.bounds ?? "null") as BBox | null;
      el.classList.toggle("chosen", !!tile && !!b && (covers(tile, b) || covers(b, tile)));
    }
  }

  onMount(() => {
    const start = remembered();
    try {
      map = new maplibregl.Map({
        container: box,
        style: settings.onlineMap ? STYLE : OFFLINE,
        center: start.center,
        zoom: start.zoom,
        maxZoom: 19,
        attributionControl: { compact: true },
        dragRotate: false,
        pitchWithRotate: false,
        touchPitch: false,
      });
    } catch (e) {
      failed = String(e);
      return;
    }
    map.touchZoomRotate.disableRotation();
    map.boxZoom.disable();
    box.addEventListener("mousedown", startBox, true);
    map.addControl(new maplibregl.NavigationControl({ showCompass: false }), "top-right");
    map.on("moveend", moved);
    map.on("movestart", () => (popup = null));
    // The remembered centre is the canvas's: it stays where it is as the padding is set.
    pad();
    // A click on the map itself lets the picked tile go (Shift+drag draws an area instead).
    map.on("click", (e) => {
      popup = null;
      if (!e.originalEvent.shiftKey && untrack(() => tile)) ontile(null, null);
    });
    map.on("error", (e: unknown) => {
      // A tile or the style failing is the usual sign of no network.
      const text = String((e as { error?: Error }).error?.message ?? e);
      if (/Failed to fetch|NetworkError|style/i.test(text)) failed = "The map's tiles could not be loaded (needs a network).";
    });
    map.once("load", () => { failed = null; moved(); drawRoute(); });
    return () => {
      // Answers still on the way are for a map that is gone: they draw nothing.
      asked++;
      routeAsked++;
      box.removeEventListener("mousedown", startBox, true);
      stop();
      head = undefined;
      map?.remove();
      map = undefined;
      markers.clear();
      approxMarkers.clear();
    };
  });

  // Online services turned on or off in Settings: the ground changes, the pins stay.
  let online = settings.onlineMap;
  $effect(() => {
    if (settings.onlineMap === online) return;
    online = settings.onlineMap;
    failed = null;
    hits = [];
    searched = false;
    chosen = null;
    map?.setStyle(online ? STYLE : OFFLINE);
    // A new style drops the route's layers: draw them again once it is in.
    map?.once("idle", drawRoute);
  });

  // The filter or an edit changed, or places by name were turned on or off: draw again.
  $effect(() => {
    void key;
    void version;
    void settings.approxPlaces;
    popup = null;
    untrack(refresh);
  });
  $effect(() => {
    void focus;
    void tile;
    untrack(mark);
  });
  // The map's padding follows what covers it: easeTo, flyTo and fitBounds centre in what is
  // left. Setting it stops a move, so while the map moves it waits for the move to end.
  let padLater = false;
  function pad() {
    if (!map) return;
    if (map.isMoving()) {
      if (!padLater) {
        padLater = true;
        map.once("moveend", () => {
          padLater = false;
          pad();
        });
      }
      return;
    }
    const p = padding;
    const was = map.getPadding();
    if (p.top === was.top && p.bottom === was.bottom && p.left === was.left && p.right === was.right) return;
    // The map stays where it is on screen: the new centre is what is under it now.
    const spot: [number, number] = [(p.left + box.clientWidth - p.right) / 2, (p.top + box.clientHeight - p.bottom) / 2];
    const at = map.unproject(spot);
    map.jumpTo({ center: at, padding: p }, { padded: true });
    // At the poles the map holds the centre back, so the view did move: that is reported.
    const now = map.project(at);
    if (Math.hypot(now.x - spot[0], now.y - spot[1]) > 0.5 && map.loaded()) moved();
  }
  $effect(() => {
    void padding;
    untrack(pad);
  });
  $effect(() => {
    // Moving the pane's box (the grid below grows or shrinks) leaves the canvas stale.
    const watch = new ResizeObserver(() => map?.resize());
    watch.observe(box);
    return () => watch.disconnect();
  });

  // A trip's route: its photos with a position joined in the order they were taken. Playing
  // moves along it photo by photo, drawing the way behind and showing each photo in turn.
  let route = $state.raw<api.RoutePoint[]>([]);
  /** How far along, in photos (a fraction is part of the way to the next). */
  let along = $state(0);
  let playing = $state(false);
  let head: maplibregl.Marker | undefined;
  let headImg: HTMLImageElement | undefined;
  let routeAsked = 0;
  /** The trip whose route is shown: only another one is fitted and played from its end. */
  let routeOf: string | null = null;
  // The canvas is painted inverted (see the style below): these read as grey and amber.
  const ROUTE_ALL = "#8a8a8a";
  const ROUTE_DONE = "#7a4f00";

  async function loadRoute() {
    const mine = ++routeAsked;
    const next = tripKey;
    if (next !== routeOf) stop();
    if (!trip) {
      routeOf = null;
      route = [];
      drawRoute();
      return;
    }
    try {
      const got = await api.route(untrack(() => $state.snapshot({ ...filter, within: undefined })));
      if (mine !== routeAsked) return;
      // After an edit the same trip is read again: the view and the playhead stay.
      const step = routeStep(routeOf, next, along, got.length);
      routeOf = next;
      route = got;
      along = step.along;
      if (got.length < 2) stop();
      drawRoute();
      if (step.fresh) {
        // Once the route bar is in (and the tray gone): the fit keeps clear of it.
        await tick();
        if (mine === routeAsked) fitRoute();
      }
    } catch (e) {
      if (mine === routeAsked) onerror(String(e));
    }
  }
  // By name: the trip object is made afresh whenever the trips are counted again.
  let tripKey = $derived(trip?.name ?? null);
  $effect(() => {
    void tripKey;
    void key;
    void version;
    untrack(loadRoute);
  });

  /** Where the way has got to: between two photos, part of the way to the next. */
  function position(): [number, number] | null {
    if (!route.length) return null;
    const i = Math.min(Math.floor(along), route.length - 1);
    const a = route[i];
    const b = route[Math.min(i + 1, route.length - 1)];
    const f = along - i;
    return [a.lon + (b.lon - a.lon) * f, a.lat + (b.lat - a.lat) * f];
  }

  function line(points: [number, number][]) {
    return { type: "Feature" as const, properties: {}, geometry: { type: "LineString" as const, coordinates: points } };
  }

  /** Whether the route's layers are there; until the style can take them, it draws
   *  again when the map next settles. (`isStyleLoaded` is false while tiles load too.) */
  let waiting = false;
  function ensureLayers(): boolean {
    if (!map) return false;
    if (map.getSource("route")) return true;
    try {
      map.addSource("route", { type: "geojson", data: line([]) });
      map.addSource("route-done", { type: "geojson", data: line([]) });
      map.addLayer({ id: "route", type: "line", source: "route", paint: { "line-color": ROUTE_ALL, "line-width": 2, "line-dasharray": [2, 2] } });
      map.addLayer({ id: "route-done", type: "line", source: "route-done", layout: { "line-cap": "round", "line-join": "round" }, paint: { "line-color": ROUTE_DONE, "line-width": 4 } });
      return true;
    } catch {
      // The style is not in yet (adding throws only then): undo what got added.
      for (const id of ["route-done", "route"]) if (map.getLayer(id)) map.removeLayer(id);
      for (const id of ["route-done", "route"]) if (map.getSource(id)) map.removeSource(id);
      if (!waiting) {
        waiting = true;
        map.once("idle", () => {
          waiting = false;
          drawRoute();
        });
      }
      return false;
    }
  }

  function drawRoute() {
    if (!ensureLayers() || !map) return;
    const all = route.map((p) => [p.lon, p.lat] as [number, number]);
    (map.getSource("route") as maplibregl.GeoJSONSource).setData(line(all.length > 1 ? all : []));
    const at = position();
    const done = at ? [...all.slice(0, Math.floor(along) + 1), at] : [];
    (map.getSource("route-done") as maplibregl.GeoJSONSource).setData(line(done.length > 1 ? done : []));
    if (!at || route.length < 2) {
      head?.remove();
      head = undefined;
      return;
    }
    if (!head) {
      const el = document.createElement("div");
      el.className = "route-head";
      headImg = document.createElement("img");
      headImg.alt = "";
      el.append(headImg);
      head = new maplibregl.Marker({ element: el }).setLngLat(at).addTo(map);
    }
    head.setLngLat(at);
    const row = route[Math.min(Math.round(along), route.length - 1)].row;
    const src = api.thumbUrl(row);
    if (headImg && headImg.getAttribute("src") !== src) headImg.src = src;
  }

  function fitRoute() {
    if (!map || route.length < 2) return;
    const lons = route.map((p) => p.lon);
    const lats = route.map((p) => p.lat);
    map.fitBounds([[Math.min(...lons), Math.min(...lats)], [Math.max(...lons), Math.max(...lats)]], {
      maxZoom: 15,
      padding,
      absolutePadding: true,
    });
  }

  let frame = 0;
  function play() {
    if (route.length < 2) return;
    if (along >= route.length - 1) along = 0;
    playing = true;
    // About twenty seconds for the whole way, but no faster than three photos a second.
    const perMs = Math.min(3, (route.length - 1) / 20) / 1000;
    let last = performance.now();
    const step = (now: number) => {
      if (!playing) return;
      along = Math.min(route.length - 1, along + (now - last) * perMs);
      last = now;
      drawRoute();
      if (along >= route.length - 1) playing = false;
      else frame = requestAnimationFrame(step);
    };
    frame = requestAnimationFrame(step);
  }
  function stop() {
    playing = false;
    cancelAnimationFrame(frame);
  }
  let alongText = $derived.by(() => {
    const p = route[Math.min(Math.round(along), route.length - 1)];
    if (!p) return "";
    const when = new Date(p.at).toLocaleString(undefined, { weekday: "short", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
    return `${when} · ${Math.round(along) + 1} of ${route.length}`;
  });

  // Shift+drag draws an area; its photos become the grid's filter.
  let rubber = $state<{ x: number; y: number; w: number; h: number } | null>(null);
  function startBox(e: MouseEvent) {
    if (!e.shiftKey || e.button !== 0 || !map) return;
    e.stopPropagation();
    e.preventDefault();
    const r = box.getBoundingClientRect();
    const [x0, y0] = [e.clientX - r.left, e.clientY - r.top];
    const at = (m: MouseEvent) => {
      const [x, y] = [Math.min(Math.max(m.clientX - r.left, 0), r.width), Math.min(Math.max(m.clientY - r.top, 0), r.height)];
      return { x: Math.min(x, x0), y: Math.min(y, y0), w: Math.abs(x - x0), h: Math.abs(y - y0) };
    };
    const move = (m: MouseEvent) => (rubber = at(m));
    const up = (m: MouseEvent) => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      const a = at(m);
      rubber = null;
      if (a.w < 6 || a.h < 6 || !map) return;
      const nw = map.unproject([a.x, a.y]);
      const se = map.unproject([a.x + a.w, a.y + a.h]);
      onbox({ min_lat: se.lat, max_lat: nw.lat, min_lon: nw.lng, max_lon: se.lng });
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  }

  // Finding a place by name (OpenStreetMap's Nominatim): to look at it, or to put photos there.
  let query = $state("");
  let hits = $state<Hit[]>([]);
  let searching = $state(false);
  let searched = $state(false);
  let chosen = $state<Hit | null>(null);
  async function search(e: Event) {
    e.preventDefault();
    const q = query.trim();
    if (!q) return;
    // Coordinates go straight there, online or not.
    const at = /\d/.test(q) ? parseGeo(q) : null;
    if (Array.isArray(at)) {
      const [lat, lon] = at;
      chosen = { name: `${lat.toFixed(5)}, ${lon.toFixed(5)}`, lat, lon, box: null };
      hits = [];
      searched = false;
      await findSettled();
      map?.flyTo({ center: [lon, lat], zoom: Math.max(map.getZoom(), 13), padding });
      return;
    }
    if (!settings.onlineMap) {
      searched = true;
      hits = [];
      return;
    }
    searching = true;
    try {
      // jsonv2 gives each place's rank and kind (`place_rank`, `addresstype`): the zoom
      // when its box is no guide.
      const res = await fetch(`https://nominatim.openstreetmap.org/search?format=jsonv2&limit=6&q=${encodeURIComponent(q)}`);
      hits = parseHits((await res.json()) as NominatimRow[]);
      searched = true;
    } catch {
      hits = [];
      onerror("The place search needs a network.");
    } finally {
      searching = false;
    }
  }
  async function go(h: Hit) {
    chosen = h;
    hits = [];
    searched = false;
    await findSettled();
    if (!map) return;
    // The zoom fits what is left free, and the flight centres in it.
    const p = padding;
    map.flyTo({ ...hitView(h, { width: width - p.left - p.right, height: height - p.top - p.bottom }, 0), padding: p });
  }
  /** The find box's height once the list has gone (and *Put…* come), not when it is next
   *  reported: the flight that follows gets the room. */
  async function findSettled() {
    await tick();
    // Gone, when the map closed meanwhile.
    findHeight = find?.offsetHeight ?? findHeight;
  }

  let over = $state(false);
  /** Where photos dragged from the grid would land: the cursor's tip, in the pane. */
  let aim = $state<{ x: number; y: number } | null>(null);
  function aiming(e: DragEvent) {
    if (!dragRows?.length) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
    over = true;
    const r = box.getBoundingClientRect();
    aim = { x: e.clientX - r.left, y: e.clientY - r.top };
  }
  function left(e: DragEvent) {
    // Moving onto a child of the pane is not leaving it.
    const to = e.relatedTarget;
    if (to instanceof Node && e.currentTarget instanceof Node && e.currentTarget.contains(to)) return;
    over = false;
    aim = null;
  }
  function dropped(e: DragEvent) {
    over = false;
    aim = null;
    if (!dragRows?.length || !map) return;
    e.preventDefault();
    const r = box.getBoundingClientRect();
    const at = map.unproject([e.clientX - r.left, e.clientY - r.top]);
    onsetgeo(dragRows, at.lat, ((at.lng + 540) % 360) - 180);
  }
</script>

<div
  class="map"
  class:over
  role="application"
  aria-label="Map"
  ondragover={aiming}
  ondragleave={left}
  ondrop={dropped}
>
  <div class="canvas" bind:this={box} bind:clientWidth={width} bind:clientHeight={height}></div>
  {#if aim}<div class="aim" style:left="{aim.x}px" style:top="{aim.y}px"></div>{/if}
  {#if rubber}<div class="rubber" style:left="{rubber.x}px" style:top="{rubber.y}px" style:width="{rubber.w}px" style:height="{rubber.h}px"></div>{/if}
  <form class="find" onsubmit={search} bind:this={find} bind:offsetHeight={findHeight}>
    <input
      type="search"
      placeholder={settings.onlineMap ? "Find a place, or type 44°58′N 14°45′E" : "Type coordinates: 44°58′N 14°45′E"}
      title="A place's name (online), or coordinates: 50.0614, 19.9366 · 44°58′N 14°45′E · N44°58′ E14°45′"
      spellcheck="false"
      bind:value={query}
      oninput={() => { searched = false; hits = []; }}
      onkeydown={(e) => { if (e.key === "Escape") { query = ""; hits = []; chosen = null; e.currentTarget.blur(); } }}
    />
    {#if searching}<span class="muted">searching…</span>{/if}
    {#if hits.length}
      <ul>{#each hits as h}<li><button type="button" onclick={() => go(h)}>{h.name}</button></li>{/each}</ul>
    {:else if searched}<span class="muted">{settings.onlineMap ? "No such place." : "Offline, only coordinates can be found (Settings)."}</span>{/if}
    {#if chosen && pickedRows.length}
      <button
        type="button"
        class="put"
        title={chosen.name}
        onclick={() => { onsetgeo(pickedRows, chosen!.lat, chosen!.lon); chosen = null; }}
      >Put {pickedRows.length.toLocaleString()} picked photo{pickedRows.length === 1 ? "" : "s"} here</button>
    {/if}
  </form>
  {#if popup}
    {@const c = popup.cluster}
    <div class="popup" style:left="{popup.x}px" style:top="{popup.y}px" role="dialog" aria-label="Photos at {placeLabel(c.place)}">
      <b>≈ {placeLabel(c.place)}</b>
      <span class="muted">
        {c.count.toLocaleString()} photo{c.count === 1 ? "" : "s"} placed by name, shown at the middle of their {LEVEL_WORD[c.level]}{#if c.places > 1}{" "}(also {(c.paths ?? []).filter((p) => p !== c.place).map((p) => p.split("|").pop()).join(", ")}, known no closer){/if}
      </span>
      {#if popup.confirm}
        <span>
          Give {c.count === 1 ? "it" : `all ${c.count.toLocaleString()}`} this position? In Capture One the position belongs to
          the image, so its variants move too. They keep showing as ≈ here, and can be undone.
        </span>
        <div class="buttons">
          <button class="primary" onclick={() => { onpromote(c.paths ?? []); popup = null; }}>Set GPS</button>
          <button onclick={() => (popup = popup && { ...popup, confirm: false })}>Cancel</button>
        </div>
      {:else}
        <div class="buttons">
          <button onclick={() => { onplace(c.place!); popup = null; }}>Show in grid</button>
          <button title="Zoom in to about how far the {LEVEL_WORD[c.level]} reaches" onclick={() => zoomToPlace(c)}>Zoom in</button>
          <button title="Set their GPS to this point, marked as approximate" onclick={() => (popup = popup && { ...popup, confirm: true })}>Give them this position…</button>
        </div>
      {/if}
    </div>
  {/if}
  {#if !trip}
    <UnplacedTray
      bind:height={trayHeight}
      {filter}
      {version}
      counts={{ none: approx.unplaced, place: approx.placeOnly }}
      {ondragrows}
      onsetgeo={(rows, lat, lon) => onsetgeo(rows, lat, lon)}
      {onneighbours}
      onshow={onunplaced}
      {onpick}
      {onerror}
    />
  {/if}
  {#if trip && route.length > 1}
    <div class="route" role="group" aria-label="Route of {trip.name}" bind:offsetHeight={routeHeight}>
      <button title={playing ? "Pause" : "Play the route, photo by photo"} onclick={() => (playing ? stop() : play())}>{playing ? "❚❚" : "▶"}</button>
      <input
        type="range"
        min="0"
        max={route.length - 1}
        step="0.01"
        bind:value={along}
        oninput={() => { stop(); drawRoute(); }}
        aria-label="How far along the route"
      />
      <span class="when">{alongText}</span>
      <button title="Show the whole route" onclick={fitRoute}>⤢</button>
    </div>
  {:else if trip && route.length < 2}
    <div class="route quiet" bind:offsetHeight={routeHeight}>Too few photos of “{trip.name}” have a position to draw its route.</div>
  {/if}
  <div class="note" bind:offsetHeight={noteHeight}>
    {#if failed}{failed}
    {:else if !settings.onlineMap && !aim && !tile && located > 0}Offline: no map tiles or place search (Settings) · {inView.toLocaleString()} of {located.toLocaleString()} photos with a position in view
    {:else if aim}They go where the cross is; zoom in to place them closer.
    {:else if tile}The grid shows the picked tile's photos · pick some and drag the pin (or them) to move just those · click it again, the map or Esc to show the area
    {:else if located === 0 && !approx.located}No photos here have a position yet. Drag some from below onto the map.
    {:else}{inView.toLocaleString()} of {located.toLocaleString()} photos with a position in view{#if settings.approxPlaces && approx.located}{" "}· ≈ {approx.inView.toLocaleString()} by place name{#if approx.hidden}{" "}({approx.hidden.toLocaleString()} hidden at this zoom){/if}{/if}{#if lookingUp}{" "}· looking up {lookingUp.toLocaleString()} places online…{/if} · Click a tile for its photos, double-click to zoom in · Drag a photo by its pin · Shift+drag picks an area{/if}
  </div>
</div>

<style>
  .map { position: relative; flex: 1 1 55%; min-height: 160px; background: #1b1b1b; }
  .map.over { outline: 2px solid var(--queued, #3b82f6); outline-offset: -2px; }
  .canvas { position: absolute; inset: 0; }
  /* The style is light; this turns it dark without touching the markers. */
  .canvas :global(.maplibregl-canvas) { filter: invert(1) hue-rotate(180deg) brightness(0.85) contrast(0.9); }
  .rubber { position: absolute; border: 2px dashed #3b82f6; background: rgba(59, 130, 246, 0.15); pointer-events: none; z-index: 3; }
  .find { position: absolute; left: 8px; top: 8px; width: 280px; z-index: 4; display: flex; flex-direction: column; gap: 4px; }
  .find input { width: 100%; padding: 4px 8px; border-radius: 3px; border: 1px solid var(--line, #444); background: rgba(20, 20, 20, 0.9); color: inherit; font: inherit; }
  .find ul { margin: 0; padding: 0; list-style: none; background: rgba(20, 20, 20, 0.95); border: 1px solid var(--line, #444); border-radius: 3px; }
  .find li button { display: block; width: 100%; padding: 4px 8px; text-align: left; font-size: 12px; }
  .find li button:hover { background: var(--hover, #333); }
  .find .muted { font-size: 12px; padding: 2px 8px; background: rgba(20, 20, 20, 0.8); border-radius: 3px; }
  .find .put { padding: 4px 8px; border-radius: 3px; background: var(--queued, #3b82f6); color: #fff; font-weight: 600; }
  .note {
    position: absolute; left: 8px; bottom: 8px; padding: 2px 8px; font-size: 12px;
    background: rgba(20, 20, 20, 0.8); border-radius: 3px; pointer-events: none;
    max-width: calc(100% - 16px); box-sizing: border-box; overflow: hidden; white-space: nowrap; text-overflow: ellipsis;
  }
  .popup {
    position: absolute; z-index: 6; transform: translate(-50%, calc(-100% - 22px)); width: 260px;
    display: flex; flex-direction: column; gap: 4px; padding: 8px 10px; font-size: 12px;
    background: rgba(20, 20, 20, 0.96); border: 1px dashed #fbbf24; border-radius: 4px; box-shadow: 0 4px 16px rgba(0, 0, 0, 0.6);
  }
  .popup .muted { color: var(--muted, #8a8a8a); }
  .popup .buttons { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 4px; }
  .popup button { padding: 2px 8px; border-radius: 3px; border: 1px solid var(--line, #444); }
  .popup button:hover { background: var(--hover, #333); }
  .popup .primary { border: 0; background: var(--queued, #3b82f6); color: #fff; font-weight: 600; }
  /* Photos placed by their place's name: dashed, see-through, never mistaken for a GPS pin. */
  .canvas :global(.pin.approx .card) { border: 2px dashed #fbbf24; opacity: 0.85; }
  .canvas :global(.pin.approx span) { background: #b7791f; }
  .canvas :global(.pin.approx.disc) { background: rgba(251, 191, 36, 0.25); border: 2px dashed #fbbf24; color: #fde68a; }
  .route {
    position: absolute; right: 8px; bottom: 30px; z-index: 4; display: flex; align-items: center; gap: 6px;
    width: min(460px, calc(100% - 16px)); padding: 4px 8px; font-size: 12px;
    background: rgba(20, 20, 20, 0.9); border: 1px solid var(--line, #444); border-radius: 4px;
  }
  .route.quiet { width: auto; color: var(--muted, #8a8a8a); }
  .route button { flex: none; min-width: 26px; padding: 2px 6px; border-radius: 3px; border: 1px solid var(--line, #444); }
  .route button:hover { background: var(--hover, #333); }
  .route input { flex: 1; min-width: 60px; accent-color: #fbbf24; }
  .route .when { flex: none; color: var(--muted, #8a8a8a); font-variant-numeric: tabular-nums; }
  .canvas :global(.route-head) {
    width: 44px; height: 44px; border: 2px solid #fbbf24; border-radius: 50%; overflow: hidden;
    background: #333; box-shadow: 0 0 0 3px rgba(251, 191, 36, 0.35), 0 1px 6px rgba(0, 0, 0, 0.7); pointer-events: none; z-index: 5;
  }
  .canvas :global(.route-head img) { width: 100%; height: 100%; object-fit: cover; }
  /* A photo stands on a stem; the dot at its foot is the place. */
  .canvas :global(.pin) { display: flex; flex-direction: column; align-items: center; cursor: pointer; }
  /* Only the photo, stem and dot catch the mouse; the room beside the stem is the map's. */
  .canvas :global(.pin:not(.disc)) { pointer-events: none; }
  .canvas :global(.pin:not(.disc) > *) { pointer-events: auto; }
  .canvas :global(.pin .card) {
    position: relative; width: 56px; height: 56px; border: 2px solid #fff; border-radius: 4px;
    background: #333; box-shadow: 0 1px 6px rgba(0, 0, 0, 0.6);
  }
  .canvas :global(.pin img) { display: block; width: 100%; height: 100%; object-fit: cover; border-radius: 2px; }
  .canvas :global(.pin span) {
    position: absolute; right: -6px; top: -8px; min-width: 20px; padding: 0 5px; border-radius: 10px;
    background: #e5484d; color: #fff; font-size: 11px; font-weight: 600; line-height: 20px; text-align: center;
  }
  .canvas :global(.pin .stem) { width: 2px; height: 10px; background: #fff; box-shadow: 0 0 2px rgba(0, 0, 0, 0.8); }
  .canvas :global(.pin .dot) {
    width: 10px; height: 10px; box-sizing: border-box; border-radius: 50%;
    background: #e5484d; border: 2px solid #fff; box-shadow: 0 0 3px rgba(0, 0, 0, 0.8);
  }
  .canvas :global(.pin.disc) {
    justify-content: center; border: 2px solid #fff; border-radius: 50%; box-shadow: 0 1px 6px rgba(0, 0, 0, 0.6);
    background: rgba(229, 72, 77, 0.85); color: #fff; font-size: 12px; font-weight: 600;
  }
  .canvas :global(.pin.focus) { z-index: 2; }
  /* The picked tile. */
  .canvas :global(.pin.chosen) { z-index: 2; }
  .canvas :global(.pin.chosen .card) { border-color: #fbbf24; box-shadow: 0 0 0 3px #fbbf24, 0 1px 6px rgba(0, 0, 0, 0.6); }
  .canvas :global(.pin.chosen .dot) { background: #fbbf24; }
  .canvas :global(.pin.disc.chosen) { border-color: #fbbf24; box-shadow: 0 0 0 3px #fbbf24; }
  .canvas :global(.pin.focus .card) { border-color: #3b82f6; box-shadow: 0 0 0 3px #3b82f6; }
  .canvas :global(.pin.focus .dot) { background: #3b82f6; }
  /* Moving: see through the photo to what is under it; the dot carries a cross. */
  .canvas :global(.pin.moving) { cursor: grabbing; z-index: 3; }
  .canvas :global(.pin.moving .card) { opacity: 0.45; }
  .canvas :global(.pin.moving .dot) { position: relative; background: #fbbf24; }
  .canvas :global(.pin.moving .dot::before),
  .canvas :global(.pin.moving .dot::after),
  .aim::before,
  .aim::after {
    content: ""; position: absolute; left: 50%; top: 50%; background: #fbbf24; box-shadow: 0 0 2px rgba(0, 0, 0, 0.9);
  }
  .canvas :global(.pin.moving .dot::before),
  .aim::before { width: 44px; height: 1px; transform: translate(-50%, -50%); }
  .canvas :global(.pin.moving .dot::after),
  .aim::after { width: 1px; height: 44px; transform: translate(-50%, -50%); }
  /* Photos dragged from the grid land at the cross. */
  .aim {
    position: absolute; width: 10px; height: 10px; margin: -5px 0 0 -5px; border-radius: 50%; box-sizing: border-box;
    background: #fbbf24; border: 2px solid #fff; pointer-events: none; z-index: 4; box-shadow: 0 0 3px rgba(0, 0, 0, 0.9);
  }
</style>
