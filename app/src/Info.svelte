<script lang="ts">
  import * as api from "./lib/api";
  import { COLORS, bytes, camera, date, exposure, folderOf } from "./lib/format";

  // Everything the library holds about one photo. Keywords, albums, the folder and the
  // camera are links: following one filters the grid by it.
  let { row, collections, onkeyword, onalbum, onfolder, oncamera }: {
    row: number | undefined;
    collections: Map<string, api.Collection>;
    onkeyword: (path: string) => void;
    onalbum: (id: string) => void;
    onfolder: (path: string) => void;
    oncamera: (model: string) => void;
  } = $props();

  let photo = $state<api.Photo | null>(null);
  let failed = $state<string | null>(null);

  $effect(() => {
    const current = row;
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
      {#if p.rating}<span class="stars">{"★".repeat(p.rating)}</span>{:else}<span class="muted">unrated</span>{/if}
      {#if p.color}<i class="dot" style:background={COLORS[p.color.toLowerCase()]} title={p.color}></i>{/if}
      {#if p.trashed}<span class="tag">in trash</span>{/if}
      {#if p.edit.edited}<span class="tag">edited</span>{/if}
    </div>

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

    {#if p.keywords.length}
      <h3>Keywords</h3>
      <ul class="links">
        {#each p.keywords as path}
          <li>
            {#each path as part, i}
              {#if i}<span class="sep">›</span>{/if}
              <button class="link" onclick={() => onkeyword(path.slice(0, i + 1).join("|"))}>{part}</button>
            {/each}
          </li>
        {/each}
      </ul>
    {/if}

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
