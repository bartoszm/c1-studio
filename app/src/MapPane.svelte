<script lang="ts">
  import * as maplibregl from "maplibre-gl";
  import "maplibre-gl/dist/maplibre-gl.css";
  // Vite bundles the worker with what it imports; maplibre's own lookup can't in dev.
  import workerUrl from "maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url";
  import { onMount, untrack } from "svelte";
  import * as api from "./lib/api";
  import type { BBox, Cluster, Filter } from "./lib/api";
  import { movingOf } from "./lib/pins";
  import { settings } from "./lib/settings.svelte";

  let {
    filter, version, focus, dragRows, pickedRows, tile, onbox, onview, ontile, onpick, onsetgeo, onerror,
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
  const markers = new Map<string, { marker: maplibregl.Marker; el: HTMLElement }>();
  /** The diameter of the dot a photo's pin stands on (matches `.pin .dot`). */
  const DOT = 10;
  let asked = 0;

  /** The filter without the area: moving the map must not refetch what it shows by itself. */
  let key = $derived(JSON.stringify({ ...filter, within: undefined }));

  function remembered(): { center: [number, number]; zoom: number } {
    try {
      const v = JSON.parse(localStorage.getItem(START) ?? "null");
      if (Array.isArray(v?.center) && typeof v.zoom === "number") return v;
    } catch {
      // Starts over the world instead.
    }
    return { center: [15, 30], zoom: 1.5 };
  }

  function view(): { bounds: BBox; zoom: number } {
    const b = map!.getBounds();
    const zoom = map!.getZoom();
    // Zoomed far out the map shows the world more than once.
    const wide = b.getEast() - b.getWest() >= 360;
    return {
      bounds: {
        min_lat: Math.max(-90, b.getSouth()),
        max_lat: Math.min(90, b.getNorth()),
        min_lon: wide ? -180 : Math.max(-180, b.getWest()),
        max_lon: wide ? 180 : Math.min(180, b.getEast()),
      },
      zoom,
    };
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
      draw(r.clusters, v.zoom);
    } catch (e) {
      if (mine === asked) onerror(String(e));
    }
  }

  function moved() {
    try {
      const c = map!.getCenter();
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
      map!.easeTo({ center: [c.lon, c.lat], zoom: Math.min(19, map!.getZoom() + 3) });
    } else {
      map!.fitBounds([[min_lon, min_lat], [max_lon, max_lat]], { padding: 90, maxZoom: 18 });
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
    // A click on the map itself lets the picked tile go (Shift+drag draws an area instead).
    map.on("click", (e) => {
      if (!e.originalEvent.shiftKey && untrack(() => tile)) ontile(null, null);
    });
    map.on("error", (e: unknown) => {
      // A tile or the style failing is the usual sign of no network.
      const text = String((e as { error?: Error }).error?.message ?? e);
      if (/Failed to fetch|NetworkError|style/i.test(text)) failed = "The map's tiles could not be loaded (needs a network).";
    });
    map.once("load", () => { failed = null; moved(); });
    return () => {
      box.removeEventListener("mousedown", startBox, true);
      map?.remove();
      map = undefined;
      markers.clear();
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
  });

  // The filter or an edit changed: draw again.
  $effect(() => {
    void key;
    void version;
    untrack(refresh);
  });
  $effect(() => {
    void focus;
    void tile;
    untrack(mark);
  });
  $effect(() => {
    // Moving the pane's box (the grid below grows or shrinks) leaves the canvas stale.
    const watch = new ResizeObserver(() => map?.resize());
    watch.observe(box);
    return () => watch.disconnect();
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
  type Hit = { name: string; lat: number; lon: number; box: [number, number, number, number] | null };
  let query = $state("");
  let hits = $state<Hit[]>([]);
  let searching = $state(false);
  let searched = $state(false);
  let chosen = $state<Hit | null>(null);
  async function search(e: Event) {
    e.preventDefault();
    const q = query.trim();
    if (!q || !settings.onlineMap) return;
    searching = true;
    try {
      const res = await fetch(`https://nominatim.openstreetmap.org/search?format=jsonv2&limit=6&q=${encodeURIComponent(q)}`);
      const list = (await res.json()) as { display_name: string; lat: string; lon: string; boundingbox?: string[] }[];
      hits = list.map((h) => ({
        name: h.display_name,
        lat: +h.lat,
        lon: +h.lon,
        box: h.boundingbox ? [+h.boundingbox[2], +h.boundingbox[0], +h.boundingbox[3], +h.boundingbox[1]] : null,
      }));
      searched = true;
    } catch {
      hits = [];
      onerror("The place search needs a network.");
    } finally {
      searching = false;
    }
  }
  function go(h: Hit) {
    chosen = h;
    hits = [];
    searched = false;
    if (h.box && map) map.fitBounds([[h.box[0], h.box[1]], [h.box[2], h.box[3]]], { padding: 40, maxZoom: 17 });
    else map?.flyTo({ center: [h.lon, h.lat], zoom: 14 });
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
  <div class="canvas" bind:this={box}></div>
  {#if aim}<div class="aim" style:left="{aim.x}px" style:top="{aim.y}px"></div>{/if}
  {#if rubber}<div class="rubber" style:left="{rubber.x}px" style:top="{rubber.y}px" style:width="{rubber.w}px" style:height="{rubber.h}px"></div>{/if}
  {#if settings.onlineMap}
    <form class="find" onsubmit={search}>
      <input
        type="search"
        placeholder="Find a place"
        spellcheck="false"
        bind:value={query}
        oninput={() => { searched = false; hits = []; }}
        onkeydown={(e) => { if (e.key === "Escape") { query = ""; hits = []; chosen = null; e.currentTarget.blur(); } }}
      />
      {#if searching}<span class="muted">searching…</span>{/if}
      {#if hits.length}
        <ul>{#each hits as h}<li><button type="button" onclick={() => go(h)}>{h.name}</button></li>{/each}</ul>
      {:else if searched}<span class="muted">No such place.</span>{/if}
      {#if chosen && pickedRows.length}
        <button
          type="button"
          class="put"
          title={chosen.name}
          onclick={() => { onsetgeo(pickedRows, chosen!.lat, chosen!.lon); chosen = null; }}
        >Put {pickedRows.length.toLocaleString()} picked photo{pickedRows.length === 1 ? "" : "s"} here</button>
      {/if}
    </form>
  {/if}
  <div class="note">
    {#if failed}{failed}
    {:else if !settings.onlineMap && !aim && !tile && located > 0}Offline: no map tiles or place search (Settings) · {inView.toLocaleString()} of {located.toLocaleString()} photos with a position in view
    {:else if aim}They go where the cross is; zoom in to place them closer.
    {:else if tile}The grid shows the picked tile's photos · pick some and drag the pin (or them) to move just those · click it again, the map or Esc to show the area
    {:else if located === 0}No photos here have a position yet. Drag some from below onto the map.
    {:else}{inView.toLocaleString()} of {located.toLocaleString()} photos with a position in view · Click a tile for its photos, double-click to zoom in · Drag a photo by its pin; the dot is where it is · Shift+drag picks an area{/if}
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
  }
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
