<script lang="ts">
  import type { Snippet } from "svelte";
  import * as api from "./lib/api";
  import { exposure } from "./lib/format";
  import { typingIn } from "./lib/keys";

  let { row, next, version, onclose, onstep, children }: {
    row: number;
    next?: number;
    /** Bumped after edits, to read the photo again. */
    version: number;
    onclose: () => void;
    onstep: (delta: number) => void;
    /** A side panel, when shown. */
    children?: Snippet;
  } = $props();

  let info = $state<api.Photo | null>(null);
  // Capture One's thumbnail shows at once; the rendered preview replaces it when ready.
  let sharp = $state(false);

  // Not state: which photo `info` is about. An edit reads the same photo again and keeps
  // the old details until the new ones are in; another photo starts empty.
  let infoOf: number | undefined;
  $effect(() => {
    const current = row;
    void version;
    if (current !== infoOf) {
      infoOf = current;
      sharp = false;
      info = null;
    }
    api.photo(current).then((p) => {
      if (current === row) info = p;
    });
  });

  // Warm the next preview while this one is being looked at.
  $effect(() => {
    if (next !== undefined) new Image().src = api.previewUrl(next);
  });

  function onkey(e: KeyboardEvent) {
    // Arrows and Escape in a field of the panel move the caret or leave the field.
    if (e.defaultPrevented || typingIn(e)) return;
    if (e.key === "Escape") onclose();
    else if (e.key === "ArrowRight") onstep(1);
    else if (e.key === "ArrowLeft") onstep(-1);
  }
</script>

<svelte:window onkeydown={onkey} />

<div class="viewer" class:side={!!children} role="dialog" aria-label="Photo">
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="stage" onclick={onclose}>
    {#key row}
      <img class="base" src={api.thumbUrl(row)} alt="" />
      <img class="full" class:shown={sharp} src={api.previewUrl(row)} alt="" onload={() => (sharp = true)} />
    {/key}
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
    <span class="hint">{sharp ? "preview" : "thumbnail"} · ← → Esc · i</span>
  </div>
  {@render children?.()}
</div>
