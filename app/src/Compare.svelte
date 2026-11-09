<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { SvelteMap, SvelteSet } from "svelte/reactivity";
  import * as api from "./lib/api";
  import { COLORS, date, exposure } from "./lib/format";
  import { FIT, MODES, compareKey, dropRow, oneToOne, panBy, zoomAt, type CompareMode, type View } from "./lib/compare";

  let { rows, active, version, blocked, album = false, message, failed, onclose, onactive, onrows }: {
    /** The photos compared, 2 to 4, in their order on screen. */
    rows: number[];
    /** Index in `rows` of the photo edits land on. */
    active: number;
    /** Bumped after edits, to read the photos again. */
    version: number;
    /** A dialog or menu is open over the comparison: the keys are its own. */
    blocked: boolean;
    /** An album is shown: Delete takes the photo out of it, as in the grid. */
    album?: boolean;
    /** The footer's word (work under way, an error, a note), which the comparison covers. */
    message?: string | null;
    /** `message` is an error. */
    failed?: boolean;
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
  // Views are in fractions of the pane, so they hold through any change of its size. `live`
  // takes every wheel and move at once; the screen follows it once a frame.
  const fits = () => [FIT, FIT, FIT, FIT];
  let live: View[] = fits();
  let views = $state<View[]>(fits());
  let frame = 0;
  function setView(i: number, v: View) {
    if (sync) live = live.map(() => v);
    else live[i] = v;
    frame ||= requestAnimationFrame(() => {
      frame = 0;
      views = live.slice();
    });
  }
  function resetViews() {
    cancelAnimationFrame(frame);
    frame = 0;
    live = fits();
    views = fits();
  }
  /** The photos side by side when the layout last changed: whose view each pane holds. */
  let laid: number[] = [];
  // Another layout: start from the whole photos. A photo taken out leaves the others as they were.
  $effect(() => {
    void panes;
    untrack(() => {
      const was = laid;
      laid = stacked ? [] : rows.slice();
      if (stacked || !rows.every((r) => was.includes(r))) return resetViews();
      cancelAnimationFrame(frame);
      frame = 0;
      live = [...rows.map((r) => live[was.indexOf(r)]), ...fits()].slice(0, 4);
      views = live.slice();
    });
  });
  /** Where the split line is, 0..1 of the pane's width. */
  let split = $state(0.5);
  /** How opaque the second photo is in the overlay, 0..1. */
  let opacity = $state(0.5);

  // Details of each photo: read once, and the photo's own again after an edit (its
  // analysis does not change with edits). Only the newest read of a photo is used.
  const infos = new SvelteMap<number, api.Photo>();
  const looks = new SvelteMap<number, api.LookView>();
  const reading = new Map<number, number>();
  const lookAsked = new Set<number>();
  let reads = 0;
  function readPhoto(row: number) {
    const mine = ++reads;
    reading.set(row, mine);
    api.photo(row).then((p) => { if (reading.get(row) === mine && p) infos.set(row, p); }).catch(() => {});
  }
  // Not the order, nor the active photo: only which photos are compared.
  let compared = $derived([...rows].sort((a, b) => a - b).join());
  $effect(() => {
    void compared;
    untrack(() => {
      for (const row of rows) {
        if (!reading.has(row)) readPhoto(row);
        if (lookAsked.has(row)) continue;
        lookAsked.add(row);
        api.look(row).then((l) => { if (l) looks.set(row, l); }).catch(() => {});
      }
    });
  });
  let seen = untrack(() => version);
  $effect(() => {
    const now = version;
    untrack(() => {
      if (now === seen) return;
      seen = now;
      for (const row of rows) readPhoto(row);
    });
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
  /** Previews that could not be read: their thumbnail stays. */
  const broken = new SvelteSet<number>();
  /** Every preview is in. Until then one photo over another waits: a thumbnail against a
   *  preview would differ everywhere. */
  let ready = $derived(rows.every((r) => sharp.has(r)));
  /** A photo compared whose preview could not be read. */
  let lost = $derived(rows.find((r) => broken.has(r)));

  let root: HTMLElement;
  let stage: HTMLElement;
  const paneAt = (el: EventTarget | null) => (el as HTMLElement | null)?.closest<HTMLElement>("[data-pane]");
  const paneIndex = (el: HTMLElement) => +(el.dataset.pane ?? 0);

  // Keys come here, not to the button that opened the comparison: Enter or Space on it
  // would open it again.
  onMount(() => root.focus());
  /** After a click on the bar, its button lets go of the keys. */
  function refocus(e: MouseEvent) {
    if (!(e.target as HTMLElement).closest("input")) root.focus();
  }

  // Each pane's size, read when it changes rather than on every wheel or move.
  const sizes = new Map<Element, DOMRect>();
  $effect(() => {
    void panes;
    void rows.length;
    sizes.clear();
    const observer = new ResizeObserver((entries) => {
      for (const e of entries) sizes.delete(e.target);
    });
    for (const el of stage.querySelectorAll("[data-pane]")) observer.observe(el);
    return () => observer.disconnect();
  });
  function rect(el: HTMLElement): DOMRect {
    let r = sizes.get(el);
    if (!r) sizes.set(el, (r = el.getBoundingClientRect()));
    return r;
  }
  // Panes move when the bar wraps: their place is read again with each new press or zoom.
  const where = (el: HTMLElement) => {
    sizes.delete(el);
    return rect(el);
  };

  // The wheel zooms about the pointer. Not a passive listener: the page must not scroll.
  $effect(() => {
    const wheel = (e: WheelEvent) => {
      const el = paneAt(e.target);
      if (!el || !e.deltaY) return;
      e.preventDefault();
      const i = paneIndex(el);
      const r = rect(el);
      setView(i, zoomAt(live[i], Math.exp(-e.deltaY * 0.0015), (e.clientX - r.left) / r.width, (e.clientY - r.top) / r.height, 1, 1));
    };
    stage.addEventListener("wheel", wheel, { passive: false });
    return () => stage.removeEventListener("wheel", wheel);
  });

  /** A press on a pane: dragging pans it (or moves the split line); a click without moving
   *  makes the photo under it the active one. One pointer at a time. */
  let drag = $state<{ id: number; i: number; x: number; y: number; moved: boolean; line: boolean; r: DOMRect } | null>(null);
  function down(e: PointerEvent) {
    const el = paneAt(e.target);
    if (drag || !el || e.button !== 0) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    const line = !!(e.target as HTMLElement).closest(".line");
    drag = { id: e.pointerId, i: paneIndex(el), x: e.clientX, y: e.clientY, moved: false, line, r: where(el) };
  }
  function move(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.id) return;
    const dx = e.clientX - drag.x;
    const dy = e.clientY - drag.y;
    if (!drag.moved && Math.hypot(dx, dy) < 3) return;
    drag.moved = true;
    const r = drag.r;
    if (drag.line) {
      split = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
      return;
    }
    drag.x = e.clientX;
    drag.y = e.clientY;
    setView(drag.i, panBy(live[drag.i], dx / r.width, dy / r.height, 1, 1));
  }
  function up(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.id) return;
    const was = drag;
    drag = null;
    if (was.moved) return;
    if (!stacked) onactive(was.i);
    else if (mode === "split") onactive((e.clientX - was.r.left) / was.r.width < split ? 0 : 1);
  }
  function cancel(e: PointerEvent) {
    if (drag?.id === e.pointerId) drag = null;
  }
  /** Double-click: the whole photo, or 1:1 about the point clicked. The pane is found by
   *  the point: with the pointer captured, the click's target is the board. */
  function dbl(e: MouseEvent) {
    const under = document.elementsFromPoint(e.clientX, e.clientY);
    const el = under.find((x) => x.matches("[data-pane]")) as HTMLElement | undefined;
    if (!el || under.some((x) => x.matches(".line"))) return;
    const r = where(el);
    zoom(paneIndex(el), (e.clientX - r.left) / r.width, (e.clientY - r.top) / r.height);
  }
  function zoom(i: number, px = 0.5, py = 0.5) {
    const el = stage.querySelector<HTMLElement>(`[data-pane="${i}"]`);
    if (!el) return;
    if (live[i].s > 1) return setView(i, FIT);
    const r = where(el);
    const row = stacked ? rows[mode === "blink" ? active : 0] : rows[i];
    const [nw, nh] = natural.get(row) ?? [0, 0];
    const s = oneToOne(nw, nh, r.width, r.height, window.devicePixelRatio || 1);
    setView(i, zoomAt(live[i], s, px, py, 1, 1));
  }

  function swap() {
    // In Blink the photo shown is the active one: Space shows the other, in the same order.
    if (mode === "blink") return onactive(1 - active);
    onrows([rows[1], rows[0]], 1 - active);
  }
  function drop(i = active) {
    const next = dropRow(rows, i, active);
    if (next) onrows(next.rows, next.active);
  }

  function onkey(e: KeyboardEvent) {
    const action = compareKey(e, blocked, rows.length);
    if (!action) {
      // Enter and Space never press a button left focused behind the keys.
      if (!blocked && !e.defaultPrevented && (e.key === "Enter" || e.key === " ") && !(e.target as HTMLElement | null)?.closest?.("input")) e.preventDefault();
      return;
    }
    e.preventDefault();
    switch (action.kind) {
      case "close": return onclose();
      case "swap": return swap();
      case "active": return onactive((active + action.delta + rows.length) % rows.length);
      case "drop": return drop();
      case "zoom": return zoom(stacked ? 0 : active);
      case "sync": return toggleSync();
      case "mode": chosen = action.mode; return;
    }
  }
  function toggleSync() {
    sync = !sync;
    // Locked again: every pane takes the active one's view.
    if (sync) setView(stacked ? 0 : active, live[stacked ? 0 : active]);
  }

  /** The transform of a photo in pane `i`: a shift in fractions of the pane, then the scale. */
  const transform = (v: View) => `transform: translate(${v.x * 100}%, ${v.y * 100}%) scale(${v.s})`;
  /** How photo `i` sits in the one pane: the second over the first, or in Blink only the active
   *  one (the other waits, loaded, out of sight). The second waits too until both previews are in. */
  function over(i: number): string {
    if (mode === "blink") return i === active ? "" : "visibility: hidden";
    if (i === 0) return "";
    if (!ready) return "visibility: hidden";
    if (mode === "split") return `clip-path: inset(0 0 0 ${split * 100}%)`;
    if (mode === "overlay") return `opacity: ${opacity}`;
    if (mode === "difference") return "mix-blend-mode: difference";
    return "visibility: hidden";
  }
  const pct = (v: View) => `${Math.round(v.s * 100)}%`;
</script>

<svelte:window onkeydown={onkey} />

{#snippet layer(row: number, v: View, style = "")}
  <div class="layer" {style}>
    <div class="zoom" style={transform(v)}>
      <img class="base" src={api.thumbUrl(row)} alt="" draggable="false" />
      <img class="full" class:shown={sharp.has(row)} src={api.largeUrl(row)} alt="" draggable="false" onload={(e) => loaded(row, e)} onerror={() => broken.add(row)} />
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
      <button class="drop" title="Take this photo out of the comparison (x for the active one)" onclick={(e) => { e.stopPropagation(); drop(i); root.focus(); }}>×</button>
    {/if}
  </div>
{/snippet}

<div class="compare" role="dialog" aria-modal="true" aria-label="Compare photos" tabindex="-1" bind:this={root}>
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="bar" onclick={refocus}>
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
        <!-- The keys go back to the comparison after a click, even one that changes nothing; Esc closes from here too. -->
        <input
          type="range"
          min="0"
          max="1"
          step="0.01"
          bind:value={opacity}
          onchange={() => root.focus()}
          onpointerup={() => root.focus()}
          onkeydown={(e) => { if (e.key === "Escape") { e.preventDefault(); onclose(); } }}
        />
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
    {#if message}
      <span class="status" class:failed>{message}</span>
    {:else}
      <span class="hint">wheel zoom · drag move · Tab / ← → active photo · 0–5 and - + * edit it · Delete {album ? "out of the album" : "to the trash"} · Esc</span>
    {/if}
    <button class="close" title="Close (Esc)" onclick={onclose}>×</button>
  </div>

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="board"
    class:dragging={drag?.moved && !drag.line}
    style="--cols: {panes === 4 ? 2 : panes}"
    bind:this={stage}
    onpointerdown={down}
    onpointermove={move}
    onpointerup={up}
    onpointercancel={cancel}
    ondblclick={dbl}
  >
    {#if stacked}
      <div class="slot">
        <div class="frame" class:moving={views[0].s > 1} data-pane="0">
          <!-- Keyed by photo: a swap moves the images rather than loading them again. -->
          {#each rows as row, i (row)}
            {@render layer(row, views[0], over(i))}
          {/each}
          {#if mode === "split"}
            <div class="line" style="left: {split * 100}%" title="Drag to move the line"><span>A │ B</span></div>
          {/if}
          {#if mode === "blink"}<span class="which">{infos.get(rows[active])?.file.name ?? ""} · Space: the other</span>{/if}
          {#if !ready}
            <span class="wait" class:failed={lost !== undefined}>
              {lost === undefined ? "loading the previews…" : `the preview of ${infos.get(lost)?.file.name ?? "a photo"} could not be read`}
            </span>
          {/if}
        </div>
        <div class="captions" class:one={mode === "blink"}>
          {#if mode === "blink"}
            {@render caption(active)}
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
            {#if broken.has(row)}<span class="wait failed">the preview could not be read: the thumbnail shows</span>{/if}
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
  .compare:focus { outline: none; }
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
  .status { margin-left: auto; font-size: 12px; color: var(--text); }
  .status.failed { color: var(--error); }
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
  /* A layer of its own only while dragging: a scale held in one would look soft. */
  .board.dragging .zoom { will-change: transform; }
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
  .which, .wait {
    position: absolute;
    top: 8px;
    left: 8px;
    padding: 2px 8px;
    color: #fff;
    background: rgb(0 0 0 / 60%);
    border-radius: 3px;
  }
  .wait { top: auto; bottom: 8px; }
  .wait.failed { color: var(--error); }

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
