<script lang="ts">
  import type { Snippet } from "svelte";
  import * as api from "./lib/api";
  import { exposure } from "./lib/format";
  import { viewerKey } from "./lib/keys";
  import { GUIDE_NAMES, fitted, guideLines, nextGuide, storedGuide, type Guide } from "./lib/guides";

  let { row, next, version, blocked, onclose, onstep, children }: {
    row: number;
    next?: number;
    /** Bumped after edits, to read the photo again. */
    version: number;
    /** A dialog or menu is open over the viewer: the keys are its own. */
    blocked: boolean;
    onclose: () => void;
    onstep: (delta: number) => void;
    /** A side panel, when shown. */
    children?: Snippet;
  } = $props();

  let info = $state<api.Photo | null>(null);
  // Capture One's thumbnail shows at once; the rendered preview replaces it when ready.
  let sharp = $state(false);

  // Not state: which photo `info` is about, and the newest read asked for. An edit reads
  // the same photo again and keeps the old details until the new ones are in; another
  // photo starts empty. Only the newest read's reply is used.
  let infoOf: number | undefined;
  let asked = 0;
  $effect(() => {
    const current = row;
    void version;
    if (current !== infoOf) {
      infoOf = current;
      sharp = false;
      info = null;
    }
    const mine = ++asked;
    api.photo(current).then((p) => {
      if (mine === asked) info = p;
    });
  });

  // Warm the next preview while this one is being looked at.
  $effect(() => {
    if (next !== undefined) new Image().src = api.previewUrl(next);
  });

  // The composition grid over the photo, remembered between photos and sessions.
  let guide = $state<Guide>(storedGuide((() => { try { return localStorage.getItem("guide"); } catch { return null; } })()));
  function stepGuide(back = false) {
    guide = nextGuide(guide, back);
    try { localStorage.setItem("guide", guide); } catch { /* not remembered */ }
  }

  // The grid covers the picture itself, not the stage around it: the size of the image
  // shown (the thumbnail can be framed unlike the preview) fitted into the stage.
  const INSET = 12;
  let stageW = $state(0);
  let stageH = $state(0);
  let baseSize = $state<[number, number] | null>(null);
  let fullSize = $state<[number, number] | null>(null);
  $effect(() => {
    void row;
    baseSize = null;
    fullSize = null;
  });
  let frame = $derived.by(() => {
    const size = (sharp && fullSize) || baseSize || fullSize;
    if (guide === "off" || !size) return null;
    const box = fitted(size[0], size[1], stageW - 2 * INSET, stageH - 2 * INSET);
    return box && { ...box, left: box.left + INSET, top: box.top + INSET };
  });
  let lines = $derived(frame ? guideLines(guide, frame.width, frame.height) : []);
  const natural = (e: Event): [number, number] => {
    const img = e.currentTarget as HTMLImageElement;
    return [img.naturalWidth, img.naturalHeight];
  };

  function onkey(e: KeyboardEvent) {
    // Arrows and Escape in a field of the panel move the caret or leave the field.
    const action = viewerKey(e, blocked);
    if (action === "close") onclose();
    else if (action === "grid" || action === "grid-back") stepGuide(action === "grid-back");
    else if (action) onstep(action);
  }
</script>

<svelte:window onkeydown={onkey} />

<div class="viewer" class:side={!!children} role="dialog" aria-label="Photo">
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="stage" onclick={onclose} bind:clientWidth={stageW} bind:clientHeight={stageH}>
    {#key row}
      <img class="base" src={api.thumbUrl(row)} alt="" onload={(e) => (baseSize = natural(e))} />
      <img class="full" class:shown={sharp} src={api.previewUrl(row)} alt="" onload={(e) => { fullSize = natural(e); sharp = true; }} />
    {/key}
    {#if frame}
      <svg
        class="guides"
        style:left="{frame.left}px"
        style:top="{frame.top}px"
        width={frame.width}
        height={frame.height}
        aria-hidden="true"
      >
        {#each lines as [x1, y1, x2, y2]}
          <line {x1} {y1} {x2} {y2} class="shade" />
          <line {x1} {y1} {x2} {y2} />
        {/each}
      </svg>
    {/if}
  </div>
  <div class="info">
    {#if info}
      <strong>{info.file.name}</strong>
      {#if info.captured_at}<span>{new Date(info.captured_at).toLocaleString()}</span>{/if}
      <span>{[info.exif.camera_model, info.exif.lens].filter(Boolean).join("  ·  ")}</span>
      <span>{exposure(info)}</span>
      {#if info.rating}<span class="stars">{"★".repeat(info.rating)}</span>{/if}
      {#if info.place.city || info.place.country}<span>{[info.place.city, info.place.country].filter(Boolean).join(", ")}</span>{/if}
      {#if info.keywords.length}<span class="kw">{info.keywords.map((k) => k.join(" › ")).join("   ")}</span>{/if}
    {/if}
    <button class="grid-pick" title="Grid over the photo (g, Shift+G)" onclick={() => stepGuide()}>{GUIDE_NAMES[guide]}</button>
    <span class="hint">{sharp ? "preview" : "thumbnail"} · ← → Esc · i · g</span>
  </div>
  {@render children?.()}
</div>
