<script lang="ts">
  import * as api from "./lib/api";
  import { COLORS, bytes, camera, date, exposure, folderOf } from "./lib/format";

  const COLOR_TAGS: api.Color[] = ["Red", "Orange", "Yellow", "Green", "Blue", "Pink", "Purple"];

  // Everything the library holds about one photo. Keywords, albums, the folder and the
  // camera are links: following one filters the grid by it. Stars and colours edit.
  let {
    row, version, picked, queued, collections, keywords,
    onrate, oncolor, onaddkeyword, onremovekeyword, onkeyword, onalbum, onfolder, oncamera,
  }: {
    row: number | undefined;
    /** Bumped after edits, to read the photo again. */
    version: number;
    /** How many photos an edit here applies to. */
    picked: number;
    /** Fields of this photo edited here and not yet written to Capture One. */
    queued: string[];
    collections: Map<string, api.Collection>;
    /** Every keyword path in the catalog, most used first, for completion. */
    keywords: string[];
    onrate: (stars: number | null) => void;
    oncolor: (color: api.Color | null) => void;
    onaddkeyword: (path: string) => void;
    onremovekeyword: (path: string) => void;
    onkeyword: (path: string) => void;
    onalbum: (id: string) => void;
    onfolder: (path: string) => void;
    oncamera: (model: string) => void;
  } = $props();

  let photo = $state<api.Photo | null>(null);
  let failed = $state<string | null>(null);

  $effect(() => {
    const current = row;
    void version;
    failed = null;
    if (current === undefined) {
      photo = null;
      return;
    }
    api.photo(current).then(
      (p) => { if (current === row) photo = p; },
      (e) => { if (current === row) failed = String(e); },
    );
  });

  let typed = $state("");
  let typedError = $state<string | null>(null);

  function addTyped(e: SubmitEvent) {
    e.preventDefault();
    const path = typed.split("|").map((l) => l.trim()).join("|");
    if (!path) return;
    if (path.split("|").some((l) => !l)) typedError = "A keyword level is empty.";
    else if (path.includes(",")) typedError = "Keyword names cannot contain a comma.";
    else {
      typedError = null;
      typed = "";
      onaddkeyword(path);
    }
  }

  /** How a queued edit reads: `rating`, `colour`, `keyword "street"`. */
  function fieldName(f: string) {
    if (f.startsWith("keyword:")) return `keyword “${f.slice(8)}”`;
    return f === "color" ? "colour" : f;
  }

  // Sorted, so the list keeps still while keywords are added and removed.
  let sortedKeywords = $derived(
    [...(photo?.keywords ?? [])].sort((a, b) => a.join("|").localeCompare(b.join("|"), undefined, { sensitivity: "base" })),
  );

  let albums = $derived(
    (photo?.collections ?? []).map((id) => collections.get(id)).filter((c): c is api.Collection => !!c),
  );
  let place = $derived(
    photo ? [photo.place.location, photo.place.city, photo.place.state, photo.place.country].filter(Boolean) : [],
  );
</script>

<aside class="info-panel">
  {#if failed}
    <p class="error">{failed}</p>
  {:else if !photo}
    <p class="hint">Select a photo to see its details.</p>
  {:else}
    {@const p = photo}
    <h2 title={p.file.name}>{p.file.name}</h2>
    <div class="marks">
      <span class="rate" role="group" aria-label="Rating">
        {#each [1, 2, 3, 4, 5] as n}
          <button
            class="star"
            class:on={(p.rating ?? 0) >= n}
            title={p.rating === n ? "Clear the rating (0)" : `${n} star${n > 1 ? "s" : ""} (${n})`}
            onclick={() => onrate(p.rating === n ? null : n)}>★</button
          >
        {/each}
      </span>
      {#if p.trashed}<span class="tag">in trash</span>{/if}
      {#if p.edit.edited}<span class="tag">edited</span>{/if}
    </div>
    <div class="colors" role="group" aria-label="Colour tag">
      {#each COLOR_TAGS as c}
        <button
          class="dot"
          class:on={p.color === c}
          style:background={COLORS[c.toLowerCase()]}
          title={p.color === c ? "Clear the colour" : c}
          aria-label={c}
          onclick={() => oncolor(p.color === c ? null : c)}
        ></button>
      {/each}
    </div>
    {#if picked > 1}<p class="hint">Edits apply to the {picked.toLocaleString()} selected photos.</p>{/if}
    {#if queued.length}
      <p class="queued-note">Edited here ({queued.map(fieldName).join(", ")}), not yet written to Capture One.</p>
    {/if}

    <dl>
      {#if p.captured_at !== null}<dt>Captured</dt><dd>{date(p.captured_at)}</dd>{/if}
      {#if camera(p)}
        <dt>Camera</dt>
        <dd><button class="link" onclick={() => p.exif.camera_model && oncamera(p.exif.camera_model)}>{camera(p)}</button></dd>
      {/if}
      {#if p.exif.lens}<dt>Lens</dt><dd>{p.exif.lens}</dd>{/if}
      {#if exposure(p)}<dt>Exposure</dt><dd>{exposure(p)}</dd>{/if}
      {#if p.exif.width && p.exif.height}
        <dt>Size</dt>
        <dd>{p.exif.width} × {p.exif.height} · {((p.exif.width * p.exif.height) / 1e6).toFixed(1)} MP</dd>
      {/if}
    </dl>

    <h3>Keywords</h3>
    <ul class="links keywords">
      {#each sortedKeywords as path (path.join("|"))}
        {@const full = path.join("|")}
        <li class:queued={queued.includes(`keyword:${full}`)}>
          {#each path as part, i}
            {#if i}<span class="sep">›</span>{/if}
            <button class="link" onclick={() => onkeyword(path.slice(0, i + 1).join("|"))}>{part}</button>
          {/each}
          <button
            class="remove"
            title={picked > 1 ? `Remove from the ${picked} selected photos` : "Remove"}
            aria-label="Remove {full}"
            onclick={() => onremovekeyword(full)}>×</button
          >
        </li>
      {/each}
    </ul>
    <form class="add-keyword" onsubmit={addTyped}>
      <input
        id="keyword-input"
        list="keyword-list"
        placeholder={picked > 1 ? `Add a keyword to ${picked} photos` : "Add a keyword (Ctrl+K)"}
        autocomplete="off"
        spellcheck="false"
        bind:value={typed}
        onkeydown={(e) => { if (e.key === "Escape") (e.currentTarget as HTMLInputElement).blur(); }}
      />
    </form>
    <datalist id="keyword-list">
      {#each keywords as k}<option value={k}></option>{/each}
    </datalist>
    {#if typedError}<p class="error">{typedError}</p>{/if}

    {#if albums.length}
      <h3>Albums</h3>
      <ul class="links">
        {#each albums as c}
          <li><button class="link" onclick={() => onalbum(c.id)}>{c.path}</button></li>
        {/each}
      </ul>
    {/if}

    {#if place.length || p.geo}
      <h3>Place</h3>
      <dl>
        {#if place.length}<dt>Where</dt><dd>{place.join(", ")}</dd>{/if}
        {#if p.geo}
          <dt>GPS</dt>
          <dd>
            {p.geo.lat.toFixed(5)}, {p.geo.lon.toFixed(5)}{#if p.geo.alt !== null} · {Math.round(p.geo.alt)} m{/if}
            <span class="muted">({p.geo.origin.toLowerCase()})</span>
          </dd>
        {/if}
      </dl>
    {/if}

    {#if p.iptc.title || p.iptc.headline || p.iptc.description || p.iptc.creator || p.iptc.copyright}
      <h3>Caption</h3>
      <dl>
        {#if p.iptc.title}<dt>Title</dt><dd>{p.iptc.title}</dd>{/if}
        {#if p.iptc.headline}<dt>Headline</dt><dd>{p.iptc.headline}</dd>{/if}
        {#if p.iptc.description}<dt>Description</dt><dd class="text">{p.iptc.description}</dd>{/if}
        {#if p.iptc.creator}<dt>Creator</dt><dd>{p.iptc.creator}</dd>{/if}
        {#if p.iptc.copyright}<dt>Copyright</dt><dd>{p.iptc.copyright}</dd>{/if}
      </dl>
    {/if}

    <h3>File</h3>
    <dl>
      {#if p.file.path}
        <dt>Folder</dt>
        <dd><button class="link path" onclick={() => p.file.path && onfolder(folderOf(p.file.path))}>{folderOf(p.file.path)}</button></dd>
      {/if}
      {#if p.file.size !== null}<dt>File size</dt><dd>{bytes(p.file.size)}</dd>{/if}
      {#if p.variant_index > 0}<dt>Variant</dt><dd>{p.variant_index + 1}</dd>{/if}
      {#if p.imported_at !== null}<dt>Imported</dt><dd>{date(p.imported_at)}</dd>{/if}
      {#if p.edit.styles.length}<dt>Styles</dt><dd>{p.edit.styles.join(", ")}</dd>{/if}
      {#if p.edit.local_layers || p.edit.retouch_layers}
        <dt>Layers</dt><dd>{p.edit.local_layers} local · {p.edit.retouch_layers} retouch</dd>
      {/if}
      <dt>Catalog id</dt><dd class="muted">{p.id.ext}</dd>
    </dl>
  {/if}
</aside>
