<script lang="ts">
  import * as maplibregl from "maplibre-gl";
  import "maplibre-gl/dist/maplibre-gl.css";
  // Vite bundles the worker with what it imports; maplibre's own lookup can't in dev.
  import workerUrl from "maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url";
  import { onMount, untrack } from "svelte";
  import * as api from "./lib/api";
  import type { BBox, Cluster, Filter } from "./lib/api";

  let {
    filter, version, focus, dragRows, onview, onpick, onsetgeo, onerror,
  }: {
    /** What the grid is filtered by; the map shows those of its photos that have a position. */
    filter: Filter;
    /** Bumped when photos were edited: positions may have changed. */
    version: number;
    /** The photo the details panel shows. */
    focus: number | null;
    /** Photos being dragged from the grid, to be dropped on the map. */
    dragRows: number[] | null;
    /** What the map shows now (null: not known yet); the grid can follow it. */
    onview: (bounds: BBox) => void;
    onpick: (row: number) => void;
    /** Photos moved to a place: dragged markers, or photos dropped from the grid. */
    onsetgeo: (rows: number[], lat: number, lon: number) => void;
    onerror: (text: string) => void;
  } = $props();

  // OpenFreeMap: free vector tiles, no key. Painted light, turned dark by a CSS filter
  // on the canvas (markers are not on it).
  const STYLE = "https://tiles.openfreemap.org/styles/positron";
  const START = "map-view";
  maplibregl.setWorkerUrl(workerUrl);

  let box: HTMLDivElement;
  let map: maplibregl.Map | undefined;
  let failed = $state<string | null>(null);
  let located = $state(0);
  let inView = $state(0);
  const markers = new Map<string, { marker: maplibregl.Marker; el: HTMLElement }>();
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

  function tile(c: Cluster, zoom: number): HTMLElement {
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
    const img = document.createElement("img");
    img.src = api.thumbUrl(c.best);
    img.draggable = false;
    img.alt = "";
    el.append(img);
    if (c.count > 1) {
      const n = document.createElement("span");
      n.textContent = c.count > 999 ? `${Math.round(c.count / 100) / 10}k` : String(c.count);
      el.append(n);
    }
    return el;
  }

  function draw(clusters: Cluster[], zoom: number) {
    const bucket = zoom < 5 ? "far" : "near";
    const seen = new Set<string>();
    for (const c of clusters) {
      const id = `${bucket}:${c.best}:${c.count}:${c.lat.toFixed(6)}:${c.lon.toFixed(6)}`;
      seen.add(id);
      if (markers.has(id)) continue;
      const el = tile(c, zoom);
      const single = c.count === 1;
      el.title = single ? "" : `${c.count.toLocaleString()} photos`;
      el.dataset.row = String(c.best);
      el.dataset.count = String(c.count);
      const marker = new maplibregl.Marker({ element: el, draggable: single }).setLngLat([c.lon, c.lat]).addTo(map!);
      el.addEventListener("click", (e) => {
        e.stopPropagation();
        if (single) onpick(c.best);
        else zoomTo(c);
      });
      if (single) {
        marker.on("dragend", () => {
          const at = marker.getLngLat();
          onsetgeo([c.best], at.lat, at.lng);
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

  function mark() {
    for (const { el } of markers.values()) {
      el.classList.toggle("focus", focus !== null && el.dataset.row === String(focus) && el.dataset.count === "1");
    }
  }

  onMount(() => {
    const start = remembered();
    try {
      map = new maplibregl.Map({
        container: box,
        style: STYLE,
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
    map.addControl(new maplibregl.NavigationControl({ showCompass: false }), "top-right");
    map.on("moveend", moved);
    map.on("error", (e: unknown) => {
      // A tile or the style failing is the usual sign of no network.
      const text = String((e as { error?: Error }).error?.message ?? e);
      if (/Failed to fetch|NetworkError|style/i.test(text)) failed = "The map's tiles could not be loaded (needs a network).";
    });
    map.once("load", () => { failed = null; moved(); });
    return () => {
      map?.remove();
      map = undefined;
      markers.clear();
    };
  });

  // The filter or an edit changed: draw again.
  $effect(() => {
    void key;
    void version;
    untrack(refresh);
  });
  $effect(() => {
    void focus;
    untrack(mark);
  });
  $effect(() => {
    // Moving the pane's box (the grid below grows or shrinks) leaves the canvas stale.
    const watch = new ResizeObserver(() => map?.resize());
    watch.observe(box);
    return () => watch.disconnect();
  });

  let over = $state(false);
  function dropped(e: DragEvent) {
    over = false;
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
  ondragover={(e) => { if (dragRows?.length) { e.preventDefault(); if (e.dataTransfer) e.dataTransfer.dropEffect = "copy"; over = true; } }}
  ondragleave={() => (over = false)}
  ondrop={dropped}
>
  <div class="canvas" bind:this={box}></div>
  <div class="note">
    {#if failed}{failed}
    {:else if located === 0}No photos here have a position yet. Drag some from below onto the map.
    {:else}{inView.toLocaleString()} of {located.toLocaleString()} photos with a position in view{/if}
  </div>
</div>

<style>
  .map { position: relative; flex: 1 1 55%; min-height: 160px; background: #1b1b1b; }
  .map.over { outline: 2px solid var(--queued, #3b82f6); outline-offset: -2px; }
  .canvas { position: absolute; inset: 0; }
  /* The style is light; this turns it dark without touching the markers. */
  .canvas :global(.maplibregl-canvas) { filter: invert(1) hue-rotate(180deg) brightness(0.85) contrast(0.9); }
  .note {
    position: absolute; left: 8px; bottom: 8px; padding: 2px 8px; font-size: 12px;
    background: rgba(20, 20, 20, 0.8); border-radius: 3px; pointer-events: none;
  }
  .canvas :global(.pin) {
    position: relative; width: 56px; height: 56px; border: 2px solid #fff; border-radius: 4px;
    background: #333; box-shadow: 0 1px 6px rgba(0, 0, 0, 0.6); cursor: pointer;
  }
  .canvas :global(.pin img) { display: block; width: 100%; height: 100%; object-fit: cover; border-radius: 2px; }
  .canvas :global(.pin span) {
    position: absolute; right: -6px; top: -8px; min-width: 20px; padding: 0 5px; border-radius: 10px;
    background: #e5484d; color: #fff; font-size: 11px; font-weight: 600; line-height: 20px; text-align: center;
  }
  .canvas :global(.pin.disc) {
    display: flex; align-items: center; justify-content: center; border-radius: 50%;
    background: rgba(229, 72, 77, 0.85); color: #fff; font-size: 12px; font-weight: 600;
  }
  .canvas :global(.pin.focus) { border-color: #3b82f6; box-shadow: 0 0 0 3px #3b82f6; z-index: 2; }
</style>
