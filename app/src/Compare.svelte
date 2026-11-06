<script lang="ts">
  import { untrack } from "svelte";
  import { SvelteMap, SvelteSet } from "svelte/reactivity";
  import * as api from "./lib/api";
  import { COLORS, date, exposure } from "./lib/format";
  import { FIT, MODES, clampView, compareKey, dropRow, oneToOne, panBy, zoomAt, type CompareMode, type View } from "./lib/compare";

  let { rows, active, version, blocked, onclose, onactive, onrows }: {
    /** The photos compared, 2 to 4, in their order on screen. */
    rows: number[];
    /** Index in `rows` of the photo edits land on. */
    active: number;
    /** Bumped after edits, to read the photos again. */
    version: number;
    /** A dialog or menu is open over the comparison: the keys are its own. */
    blocked: boolean;
    onclose: () => void;
    onactive: (i: number) => void;
    /** The photos reordered or one taken out, with the active one after it. */
    onrows: (rows: number[], active: number) => void;
  } = $props();

  function remembered(): CompareMode {
    try {
      const m = localStorage.getItem("compare");
      if (MODES.some((x) => x.mode === m)) return m as CompareMode;
    } catch {
      // Side by side, then.
    }
    return "side";
  }
  let chosen = $state<CompareMode>(remembered());
  $effect(() => {
    try {
      localStorage.setItem("compare", chosen);
    } catch {
      // Not remembered, that's all.
    }
  });
  /** Three or four photos are always side by side. */
  let mode = $derived<CompareMode>(rows.length === 2 ? chosen : "side");
  /** One pane with both photos in it, or one pane per photo. */
  let stacked = $derived(mode !== "side");
  let panes = $derived(stacked ? 1 : rows.length);

  /** Zoom and pan are the same in every pane, unless unlocked. */
  let sync = $state(true);
  let views = $state<View[]>([FIT, FIT, FIT, FIT]);
  /** Where the split line is, 0..1 of the pane's width. */
  let split = $state(0.5);
  /** How opaque the second photo is in the overlay, 0..1. */
  let opacity = $state(0.5);

  function setView(i: number, v: View) {
    if (sync) views = views.map(() => v);
    else views[i] = v;
  }
  // Another layout, other pane sizes: start from the whole photos.
  $effect(() => {
    void panes;
    views = [FIT, FIT, FIT, FIT];
  });

  // Details of each photo, read again after edits. Only the newest read's reply is used.
  const infos = new SvelteMap<number, api.Photo>();
  const looks = new SvelteMap<number, api.LookView>();
  let asked = 0;
  $effect(() => {
    const now = [...rows];
    void version;
    const mine = ++asked;
    for (const row of now) {
      api.photo(row).then((p) => { if (mine === asked && p) infos.set(row, p); }).catch(() => {});
      api.look(row).then((l) => { if (mine === asked && l) looks.set(row, l); }).catch(() => {});
    }
  });
  /** The sharpest of the photos, when every one is measured and they differ. */
  let sharpest = $derived.by(() => {
    const d = rows.map((r) => looks.get(r)?.analysis.sharpness);
    if (d.some((x) => x === undefined)) return undefined;
    const top = Math.max(...(d as number[]));
    return d.filter((x) => x === top).length === 1 ? rows[d.indexOf(top)] : undefined;
  });

  // Capture One's thumbnail shows at once; the rendered preview replaces it when ready.
  const sharp = new SvelteSet<number>();
  /** Each preview's own size, for a 1:1 zoom. */
  const natural = new Map<number, [number, number]>();
  function loaded(row: number, e: Event) {
    const img = e.currentTarget as HTMLImageElement;
    natural.set(row, [img.naturalWidth, img.naturalHeight]);
    sharp.add(row);
  }

  let stage: HTMLElement;
  const paneAt = (el: EventTarget | null) => (el as HTMLElement | null)?.closest<HTMLElement>("[data-pane]");
  const paneIndex = (el: HTMLElement) => +(el.dataset.pane ?? 0);

  // The wheel zooms about the pointer. Not a passive listener: the page must not scroll.
  $effect(() => {
    const wheel = (e: WheelEvent) => {
      const el = paneAt(e.target);
      if (!el || !e.deltaY) return;
      e.preventDefault();
      const i = paneIndex(el);
      const r = el.getBoundingClientRect();
      setView(i, zoomAt(views[i], Math.exp(-e.deltaY * 0.0015), e.clientX - r.left, e.clientY - r.top, r.width, r.height));
    };
    stage.addEventListener("wheel", wheel, { passive: false });
    return () => stage.removeEventListener("wheel", wheel);
  });

  /** A press on a pane: dragging pans it (or moves the split line); a click without moving
   *  makes the photo under it the active one. */
  let drag: { i: number; x: number; y: number; moved: boolean; line: boolean; el: HTMLElement } | null = null;
  function down(e: PointerEvent) {
    const el = paneAt(e.target);
    if (!el || e.button !== 0) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    const line = !!(e.target as HTMLElement).closest(".line");
    drag = { i: paneIndex(el), x: e.clientX, y: e.clientY, moved: false, line, el };
  }
  function move(e: PointerEvent) {
    if (!drag) return;
    const dx = e.clientX - drag.x;
    const dy = e.clientY - drag.y;
    if (!drag.moved && Math.hypot(dx, dy) < 3) return;
    drag.moved = true;
    const r = drag.el.getBoundingClientRect();
    if (drag.line) {
      split = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
      return;
    }
    drag.x = e.clientX;
    drag.y = e.clientY;
    setView(drag.i, panBy(views[drag.i], dx, dy, r.width, r.height));
  }
  function up(e: PointerEvent) {
    const was = drag;
    drag = null;
    if (!was || was.moved) return;
    if (!stacked) onactive(was.i);
    else if (mode === "split") {
      const r = was.el.getBoundingClientRect();
      onactive((e.clientX - r.left) / r.width < split ? 0 : 1);
    }
  }
  /** Double-click: the whole photo, or 1:1 about the point clicked. */
  function dbl(e: MouseEvent) {
    const el = paneAt(e.target);
    if (!el || (e.target as HTMLElement).closest(".line")) return;
    const r = el.getBoundingClientRect();
    zoom(paneIndex(el), e.clientX - r.left, e.clientY - r.top);
  }
  function zoom(i: number, px?: number, py?: number) {
    const el = stage.querySelector<HTMLElement>(`[data-pane="${i}"]`);
    if (!el) return;
    const r = el.getBoundingClientRect();
    if (views[i].s > 1) return setView(i, FIT);
    const row = stacked ? rows[0] : rows[i];
    const [nw, nh] = natural.get(row) ?? [0, 0];
    const s = oneToOne(nw, nh, r.width, r.height, window.devicePixelRatio || 1);
    setView(i, zoomAt(views[i], s, px ?? r.width / 2, py ?? r.height / 2, r.width, r.height));
  }

  function swap() {
    // In Blink the photo shown is the first, and edits land on it.
    onrows([rows[1], rows[0]], mode === "blink" ? 0 : 1 - active);
  }
  // In Blink edits land on the photo shown, which is the first: the active one goes first.
  $effect(() => {
    if (mode === "blink" && active !== 0) untrack(swap);
  });
  function drop(i = active) {
    const next = dropRow(rows, i);
    if (next) onrows(next.rows, next.active);
  }

  function onkey(e: KeyboardEvent) {
    const action = compareKey(e, blocked, rows.length);
    if (!action) return;
    e.preventDefault();
    switch (action.kind) {
      case "close": return onclose();
      case "swap": return swap();
      case "active": {
        // In Blink the photo active is the one shown: stepping shows the other.
        if (mode === "blink") return swap();
        return onactive((active + action.delta + rows.length) % rows.length);
      }
      case "drop": return drop();
      case "zoom": return zoom(stacked ? 0 : active);
      case "sync": return toggleSync();
      case "mode": chosen = action.mode; return;
    }
  }
  function toggleSync() {
    sync = !sync;
    // Locked again: every pane takes the active one's view.
    if (sync) views = views.map(() => views[stacked ? 0 : active]);
  }

  /** The transform of a photo in pane `i`. */
  const transform = (v: View) => `transform: translate(${v.x}px, ${v.y}px) scale(${v.s})`;
  /** How the second photo sits over the first in one pane. */
  function over(): string {
    if (mode === "split") return `clip-path: inset(0 0 0 ${split * 100}%)`;
    if (mode === "overlay") return `opacity: ${opacity}`;
    if (mode === "difference") return "mix-blend-mode: difference";
    return "display: none";
  }
  const pct = (v: View) => `${Math.round(v.s * 100)}%`;
</script>

<svelte:window onkeydown={onkey} onresize={() => (views = [FIT, FIT, FIT, FIT])} />

{#snippet layer(row: number, v: View, style = "")}
  <div class="layer" {style}>
    <div class="zoom" style={transform(v)}>
      <img class="base" src={api.thumbUrl(row)} alt="" draggable="false" />
      <img class="full" class:shown={sharp.has(row)} src={api.previewUrl(row)} alt="" draggable="false" onload={(e) => loaded(row, e)} />
    </div>
  </div>
{/snippet}

{#snippet caption(i: number)}
  {@const row = rows[i]}
  {@const p = infos.get(row)}
  {@const l = looks.get(row)}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="caption" class:active={i === active} onclick={() => onactive(i)} title="Edits land on this photo (click, Tab or ← →)">
    <span class="tag">{String.fromCharCode(65 + i)}</span>
    {#if p}
      <strong>{p.file.name}</strong>
      {#if p.color}<i class="dot" style="background: {COLORS[p.color.toLowerCase()] ?? 'transparent'}"></i>{/if}
      <span class="stars">{"★".repeat(p.rating ?? 0)}<span class="off">{"★".repeat(5 - (p.rating ?? 0))}</span></span>
      {#if p.captured_at}<span>{date(p.captured_at)}</span>{/if}
      <span class="muted">{exposure(p)}</span>
    {/if}
    {#if l}
      <span class="muted" title="Focus class and the detail measured in the photo">{l.focus} · detail {Math.round(l.analysis.sharpness).toLocaleString()}</span>
      {#if sharpest === row}<span class="best">sharpest</span>{/if}
    {/if}
    {#if rows.length > 2}
      <button class="drop" title="Take out of the comparison (x)" onclick={(e) => { e.stopPropagation(); drop(i); }}>×</button>
    {/if}
  </div>
{/snippet}

<div class="compare" role="dialog" aria-label="Compare photos">
  <div class="bar">
    <div class="modes" role="group" aria-label="How to compare">
      {#each MODES as m (m.mode)}
        <button
          class="toggle"
          class:on={mode === m.mode}
          disabled={m.mode !== "side" && rows.length !== 2}
          title={m.mode !== "side" && rows.length !== 2 ? `${m.title}: for two photos` : m.title}
          onclick={() => (chosen = m.mode)}
        >{m.label}</button>
      {/each}
    </div>
    {#if mode === "overlay"}
      <label class="slider" title="How opaque B is over A">
        A
        <input type="range" min="0" max="1" step="0.01" bind:value={opacity} onpointerup={(e) => e.currentTarget.blur()} />
        B
      </label>
    {/if}
    {#if rows.length === 2}
      <button class="toggle" title="Swap A and B (Space)" onclick={swap}>⇄ Swap</button>
    {/if}
    <button class="toggle" class:on={sync} aria-pressed={sync} title="Zoom and move all the photos together (l)" onclick={toggleSync}>
      {sync ? "🔒" : "🔓"} Zoom together
    </button>
    <button class="toggle" title="The whole photo, or 1:1 (z or double-click); the wheel zooms, dragging moves" onclick={() => zoom(stacked ? 0 : active)}>
      {views[stacked ? 0 : active].s > 1 ? `${pct(views[stacked ? 0 : active])} · Fit` : "1:1"}
    </button>
    <span class="hint">wheel zoom · drag move · Tab / ← → active photo · 0–5 and - + * edit it · Esc</span>
    <button class="close" title="Close (Esc)" onclick={onclose}>×</button>
  </div>

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="board"
        style="--cols: {panes === 4 ? 2 : panes}"
    bind:this={stage}
    onpointerdown={down}
    onpointermove={move}
    onpointerup={up}
    onpointercancel={() => (drag = null)}
    ondblclick={dbl}
  >
    {#if stacked}
      <div class="slot">
        <div class="frame" class:moving={views[0].s > 1} data-pane="0">
          {@render layer(rows[0], views[0])}
          {#if mode !== "blink"}{@render layer(rows[1], views[0], over())}{/if}
          {#if mode === "split"}
            <div class="line" style="left: {split * 100}%" title="Drag to move the line"><span>A │ B</span></div>
          {/if}
          {#if mode === "blink"}<span class="which">{infos.get(rows[0])?.file.name ?? ""} · Space: the other</span>{/if}
        </div>
        <div class="captions" class:one={mode === "blink"}>
          {#if mode === "blink"}
            {@render caption(0)}
          {:else}
            {@render caption(0)}{@render caption(1)}
          {/if}
        </div>
      </div>
    {:else}
      {#each rows as row, i (row)}
        <div class="slot" class:active={i === active}>
          <div class="frame" class:moving={views[i].s > 1} data-pane={i}>
            {@render layer(row, views[i])}
          </div>
          {@render caption(i)}
        </div>
      {/each}
    {/if}
  </div>
</div>

<style>
  .compare {
    position: fixed;
    inset: 0;
    z-index: 10;
    display: grid;
    grid-template-rows: auto 1fr;
    background: #111;
  }
  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 10px;
    padding: 6px 12px;
    background: var(--panel);
    border-bottom: 1px solid var(--line);
  }
  .modes { display: flex; gap: 2px; }
  .toggle:disabled { opacity: 0.4; cursor: default; }
  .slider { display: flex; align-items: center; gap: 6px; color: var(--muted); }
  .slider input { width: 140px; accent-color: var(--accent); }
  .hint { margin-left: auto; color: var(--muted); font-size: 12px; }
  .close { padding: 0 6px; font-size: 20px; line-height: 1; color: var(--muted); }
  .close:hover { color: var(--text); }

  .board {
    display: grid;
    grid-template-columns: repeat(var(--cols), minmax(0, 1fr));
    grid-auto-rows: minmax(0, 1fr);
    gap: 4px;
    padding: 4px;
    min-height: 0;
    user-select: none;
  }
  .slot {
    display: grid;
    grid-template-rows: minmax(0, 1fr) auto;
    min-height: 0;
    outline: 1px solid transparent;
  }
  .slot.active { outline-color: var(--queued); }
  .frame {
    position: relative;
    overflow: hidden;
    /* The difference is taken between this pane's photos only. */
    isolation: isolate;
    background: #111;
    cursor: zoom-in;
    touch-action: none;
  }
  .frame.moving { cursor: grab; }
  .layer { position: absolute; inset: 0; overflow: hidden; }
  .zoom { position: absolute; inset: 0; transform-origin: 0 0; }
  .zoom img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: contain; }
  .zoom .base { filter: blur(0.5px); }
  .zoom .full { opacity: 0; transition: opacity 120ms; }
  .zoom .full.shown { opacity: 1; }

  .line {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 13px;
    margin-left: -6px;
    cursor: ew-resize;
    z-index: 1;
  }
  .line::before {
    content: "";
    position: absolute;
    top: 0;
    bottom: 0;
    left: 6px;
    width: 1px;
    background: rgb(255 255 255 / 75%);
    box-shadow: 0 0 2px #000;
  }
  .line span {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    padding: 2px 6px;
    font-size: 11px;
    white-space: nowrap;
    color: #fff;
    background: rgb(0 0 0 / 60%);
    border-radius: 3px;
  }
  .which {
    position: absolute;
    top: 8px;
    left: 8px;
    padding: 2px 8px;
    color: #fff;
    background: rgb(0 0 0 / 60%);
    border-radius: 3px;
  }

  .captions { display: grid; grid-template-columns: 1fr 1fr; }
  .captions.one { grid-template-columns: 1fr; }
  .caption {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 2px 12px;
    padding: 6px 10px;
    background: var(--panel);
    border-top: 2px solid transparent;
    cursor: pointer;
  }
  .caption.active { border-top-color: var(--queued); }
  .caption .tag { font-weight: 700; color: var(--muted); }
  .caption.active .tag { color: var(--queued); }
  .caption strong { overflow-wrap: anywhere; }
  .caption .muted { color: var(--muted); }
  .caption .stars { color: var(--star); letter-spacing: 1px; }
  .caption .stars .off { color: #444; }
  .caption .dot { display: inline-block; width: 10px; height: 10px; border-radius: 50%; }
  .caption .best { padding: 0 6px; font-size: 11px; color: #111; background: var(--star); border-radius: 3px; }
  .caption .drop { margin-left: auto; padding: 0 4px; font-size: 16px; line-height: 1; color: var(--muted); }
  .caption .drop:hover { color: var(--error); }
</style>
