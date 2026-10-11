<script lang="ts">
  import * as api from "./lib/api";
  import { codeFor, nameFor } from "./lib/countries";
  import { COLORS, bytes, camera, date, exposure, folderOf, geoText, parseGeo } from "./lib/format";

  const COLOR_TAGS: api.Color[] = ["Red", "Orange", "Yellow", "Green", "Blue", "Pink", "Purple"];

  // Everything the library holds about one photo. Keywords, albums, the folder and the
  // camera are links: following one filters the grid by it. Stars and colours edit.
  let {
    row, version, picked, queued, collections, keywords, clip, shared, sharedAlbums,
    onrate, oncolor, onaddkeyword, onaddkeywords, onremovekeyword, onkeyword, onalbum, onfolder, oncamera,
    onaddalbum, onremovealbum, onnewalbum,
    onplace, ongeo, oniptc, oncopy, onpaste, onsync,
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
    /** Adds keywords to every selected photo: the ones only some of them have. */
    onaddkeywords: (paths: string[]) => void;
    /** With several photos selected: their keywords, and how many of them carry each. */
    shared: [string, number][] | null;
    onremovekeyword: (path: string) => void;
    onkeyword: (path: string) => void;
    onalbum: (id: string) => void;
    /** With several photos selected: the albums they are in, and how many each holds. */
    sharedAlbums: [string, number][] | null;
    /** Puts the selected photos into an album, or takes them out. */
    onaddalbum: (id: string) => void;
    onremovealbum: (id: string) => void;
    /** Opens the New album dialog, for the selected photos. */
    onnewalbum: () => void;
    onfolder: (path: string) => void;
    oncamera: (model: string) => void;
    /** A location field typed in; "" clears it. */
    /** Location fields typed in; "" clears one. */
    onplace: (fields: Partial<Record<api.PlaceField, string>>) => void;
    ongeo: (value: [number, number] | null) => void;
    /** An IPTC text field typed in; "" clears it. */
    oniptc: (field: api.CaptionField, value: string) => void;
    /** What the clipboard holds, for showing; null when empty. */
    clip: string | null;
    oncopy: () => void;
    /** `fill` only fills what each photo leaves empty. */
    onpaste: (fill: boolean) => void;
    /** Opens the choice of fields to take from this photo onto the other selected ones. */
    onsync: () => void;
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
    if (f.startsWith("collection:")) return `album “${collections.get(f.slice(11))?.name ?? f.slice(11)}”`;
    return ({ color: "colour", place: "location", geo: "GPS", iptc: "IPTC fields" } as Record<string, string>)[f] ?? f;
  }

  const PLACE_FIELDS: [api.PlaceField, string][] = [
    ["country", "Country"], ["country_code", "Code"], ["state", "State"], ["city", "City"], ["location", "Location"],
  ];

  // Typed fields commit on Enter or leaving the field; Escape puts the value back.
  function fieldKeys(e: KeyboardEvent, shown: string) {
    const input = e.currentTarget as HTMLInputElement;
    if (e.key === "Enter") input.blur();
    else if (e.key === "Escape") {
      input.value = shown;
      geoError = null;
      input.blur();
    }
  }

  // Country and ISO code go together: a country fills its code (and clearing it clears
  // the code); a code typed where there is no country fills the country.
  function withCode(field: api.PlaceField, value: string, place: api.Photo["place"]) {
    const fields: Partial<Record<api.PlaceField, string>> = { [field]: value };
    if (field === "country") {
      const code = value.trim() ? codeFor(value) : "";
      if (code !== null) fields.country_code = code;
      // A name the app does not know: the old country's code would now be wrong. A code
      // that never matched the country (set by hand) is left alone.
      else if (place.country_code && codeFor(place.country ?? "") === place.country_code) fields.country_code = "";
    } else if (field === "country_code" && value.trim() && !place.country) {
      const name = nameFor(value);
      if (name) fields.country = name;
    }
    return fields;
  }

  // A description has lines: Enter adds one, Ctrl+Enter (or leaving) saves.
  function textKeys(e: KeyboardEvent, shown: string) {
    const area = e.currentTarget as HTMLTextAreaElement;
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) area.blur();
    else if (e.key === "Escape") {
      area.value = shown;
      area.blur();
    }
  }

  // The IPTC fields, grouped as Capture One groups them. Location has its own section.
  const TITLE_HINT =
    "IPTC Object Name: a short identifier of the image, such as a file or job code, not a caption " +
    "(that is the Description). Capture One shows it under IPTC – Image.";
  const IPTC_GROUPS: [string, [api.CaptionField, string, string?][]][] = [
    ["IPTC – Content", [["headline", "Headline"], ["description", "Description"]]],
    ["IPTC – Image", [["title", "Title", TITLE_HINT]]],
    ["IPTC – Status", [["job_id", "Job ID", "Job identifier (original transmission reference)"],
      ["instructions", "Instructions"], ["source", "Source", "Who holds the original"], ["copyright", "Copyright"]]],
    ["IPTC – Contact", [["creator", "Creator"]]],
  ];

  let geoError = $state<string | null>(null);
  function commitGeo(e: Event) {
    const parsed = parseGeo((e.currentTarget as HTMLInputElement).value);
    if (typeof parsed === "string") geoError = parsed;
    else {
      geoError = null;
      ongeo(parsed);
    }
  }

  // Sorted, so the list keeps still while keywords are added and removed.
  let sortedKeywords = $derived(
    [...(photo?.keywords ?? [])].sort((a, b) => a.join("|").localeCompare(b.join("|"), undefined, { sensitivity: "base" })),
  );

  // One photo shows its keywords; several show every keyword any of them has, those on
  // only some set apart, as Capture One does.
  type KeywordRow = { full: string; path: string[]; n: number };
  let keywordRows: KeywordRow[] = $derived(
    picked > 1 && shared
      ? shared.map(([full, n]) => ({ full, path: full.split("|"), n }))
          .sort((a, b) => a.full.localeCompare(b.full, undefined, { sensitivity: "base" }))
      : sortedKeywords.map((path) => ({ full: path.join("|"), path, n: picked })),
  );
  let partial = $derived(keywordRows.filter((k) => k.n < picked).map((k) => k.full));

  // The albums the photo is in; with several selected, every album any of them is in,
  // those holding only some set apart, as keywords are.
  type AlbumRow = { c: api.Collection; n: number };
  let albumRows: AlbumRow[] = $derived(
    (picked > 1 && sharedAlbums ? sharedAlbums : (photo?.collections ?? []).map((id): [string, number] => [id, picked]))
      .map(([id, n]) => ({ c: collections.get(id)!, n }))
      .filter((r) => !!r.c)
      .sort((a, b) => a.c.path.localeCompare(b.c.path, undefined, { sensitivity: "base" })),
  );
  /** Albums a photo can be put into, by path, for completion. */
  let albumChoices = $derived(
    [...collections.values()].filter((c) => c.kind === "Album").map((c) => c.path).sort((a, b) => a.localeCompare(b)),
  );
  let albumTyped = $state("");
  let albumError = $state<string | null>(null);

  function addAlbum(e: SubmitEvent) {
    e.preventDefault();
    const text = albumTyped.trim().toLowerCase();
    if (!text) return;
    const albums = [...collections.values()].filter((c) => c.kind === "Album");
    // The whole path, else a name only one album has.
    const found = albums.filter((c) => c.path.toLowerCase() === text);
    const named = found.length ? found : albums.filter((c) => c.name.toLowerCase() === text);
    if (named.length === 1) {
      albumError = null;
      albumTyped = "";
      onaddalbum(named[0].id);
    } else if (named.length > 1) albumError = `Several albums are called that: pick one by its path.`;
    else albumError = `No album “${albumTyped.trim()}”. New album… makes one.`;
  }
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
    <div class="clip">
      <button title="Choose what to copy from this photo (Ctrl+Shift+C)" onclick={oncopy}>Copy…</button>
      {#if picked > 1}
        <button title="Choose fields of this photo to put on the other {picked - 1} selected (Ctrl+Shift+S)" onclick={onsync}>
          Sync to {(picked - 1).toLocaleString()}…
        </button>
      {/if}
    </div>
    {#if clip}
      {@const onto = picked > 1 ? `the ${picked} selected photos` : "this photo"}
      <div class="clip">
        <button title="Paste onto {onto}, replacing what they have (Ctrl+Shift+V)" onclick={() => onpaste(false)}>
          Paste{picked > 1 ? ` to ${picked.toLocaleString()}` : ""}
        </button>
        <button title="Fill only the fields {onto} leave empty; keywords are added (Ctrl+Alt+V)" onclick={() => onpaste(true)}>Fill</button>
        <span class="clip-what" title={clip}>{clip}</span>
      </div>
    {/if}
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
    {#if partial.length}
      <p class="partial-note">
        <span>{partial.length} on only some of the {picked.toLocaleString()}</span>
        <button title="Add {partial.length === 1 ? 'it' : `all ${partial.length}`} to every selected photo" onclick={() => onaddkeywords(partial)}>
          Add to all
        </button>
      </p>
    {/if}
    <ul class="links keywords">
      {#each keywordRows as { full, path, n } (full)}
        <li class:queued={queued.includes(`keyword:${full}`)} class:partial={n < picked}>
          {#each path as part, i}
            {#if i}<span class="sep">›</span>{/if}
            <button class="link" onclick={() => onkeyword(path.slice(0, i + 1).join("|"))}>{part}</button>
          {/each}
          {#if n < picked}
            <span class="count" title="On {n.toLocaleString()} of the {picked.toLocaleString()} selected photos">{n.toLocaleString()}/{picked.toLocaleString()}</span>
            <button class="apply" title="Add to all {picked.toLocaleString()} selected photos" aria-label="Add {full} to all" onclick={() => onaddkeyword(full)}>+</button>
          {/if}
          <button
            class="remove"
            class:after={n < picked}
            title={`Remove${keywordRows.some((k) => k.full.startsWith(full + "|")) ? ", with the keywords under it," : ""}${picked > 1 ? ` from the ${picked} selected photos` : ""}`}
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

    <h3>Albums</h3>
    <ul class="links keywords albums">
      {#each albumRows as { c, n } (c.id)}
        <li class:queued={queued.includes(`collection:${c.id}`)} class:partial={n < picked}>
          <button class="link" title={c.kind === "Project" ? "A project" : undefined} onclick={() => onalbum(c.id)}>{c.path}</button>
          {#if n < picked}
            <span class="count" title="Holds {n.toLocaleString()} of the {picked.toLocaleString()} selected photos">{n.toLocaleString()}/{picked.toLocaleString()}</span>
            {#if c.kind === "Album"}
              <button class="apply" title="Put all {picked.toLocaleString()} selected photos in it" aria-label="Put all in {c.path}" onclick={() => onaddalbum(c.id)}>+</button>
            {/if}
          {/if}
          {#if c.kind === "Album"}
            <button
              class="remove"
              class:after={n < picked}
              title="Take {picked > 1 ? `the ${picked} selected photos` : 'the photo'} out of it"
              aria-label="Take out of {c.path}"
              onclick={() => onremovealbum(c.id)}>×</button
            >
          {/if}
        </li>
      {/each}
    </ul>
    <form class="add-keyword add-album" onsubmit={addAlbum}>
      <input
        list="album-list"
        placeholder="Put in an album"
        autocomplete="off"
        spellcheck="false"
        bind:value={albumTyped}
        onkeydown={(e) => { if (e.key === "Escape") (e.currentTarget as HTMLInputElement).blur(); }}
      />
      <button type="button" title="Make an album and put {picked > 1 ? 'these photos' : 'this photo'} in it" onclick={onnewalbum}>New album…</button>
    </form>
    <datalist id="album-list">
      {#each albumChoices as path}<option value={path}></option>{/each}
    </datalist>
    {#if albumError}<p class="error">{albumError}</p>{/if}

    <h3>Location</h3>
    <dl class="fields" class:queued={queued.includes("place")}>
      {#each PLACE_FIELDS as [field, label] (field)}
        {@const shown = p.place[field] ?? ""}
        <dt><label for="place-{field}">{label}</label></dt>
        <dd>
          <input
            id="place-{field}"
            class="field"
            class:code={field === "country_code"}
            value={shown}
            placeholder={picked > 1 ? `set on ${picked} photos` : "—"}
            maxlength={field === "country_code" ? 3 : undefined}
            spellcheck="false"
            onkeydown={(e) => fieldKeys(e, shown)}
            onchange={(e) => onplace(withCode(field, e.currentTarget.value, p.place))}
          />
        </dd>
      {/each}
    </dl>
    <dl class="fields" class:queued={queued.includes("geo")}>
      {#key p.geo}
        {@const shown = p.geo ? geoText(p.geo) : ""}
        <dt><label for="place-geo">GPS</label></dt>
        <dd>
          <input
            id="place-geo"
            class="field"
            value={shown}
            placeholder="latitude, longitude"
            title="Decimal (50.0614, 19.9366), with N/S/E/W, or degrees, minutes and seconds. Empty clears it."
            spellcheck="false"
            onkeydown={(e) => fieldKeys(e, shown)}
            onchange={commitGeo}
          />
        </dd>
      {/key}
    </dl>
    {#if geoError}<p class="error">{geoError}</p>{/if}

    {#each IPTC_GROUPS as [title, fields] (title)}
      <h3>{title}</h3>
      <dl class="fields" class:queued={queued.includes("iptc")}>
        {#each fields as [field, label, hint] (field)}
          {@const shown = p.iptc[field] ?? ""}
          <dt><label for="iptc-{field}" title={hint}>{label}</label></dt>
          <dd>
            {#if field === "description"}
              <textarea
                id="iptc-{field}"
                class="field text"
                rows={Math.min(8, Math.max(2, shown.split("\n").length))}
                value={shown}
                placeholder={picked > 1 ? `set on ${picked} photos` : "—"}
                title="Enter adds a line; Ctrl+Enter or leaving the field saves; Escape puts it back"
                onkeydown={(e) => textKeys(e, shown)}
                onchange={(e) => oniptc(field, e.currentTarget.value)}
              ></textarea>
            {:else}
              <input
                id="iptc-{field}"
                class="field"
                value={shown}
                placeholder={picked > 1 ? `set on ${picked} photos` : field === "title" ? "object name" : "—"}
                title={hint}
                spellcheck={field !== "title" && field !== "job_id"}
                onkeydown={(e) => fieldKeys(e, shown)}
                onchange={(e) => oniptc(field, e.currentTarget.value)}
              />
            {/if}
          </dd>
        {/each}
      </dl>
    {/each}

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
