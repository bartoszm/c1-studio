<script lang="ts">
  import * as api from "./lib/api";
  import { COLORS, bytes, camera, date, exposure, folderOf, geoText, parseGeo } from "./lib/format";
  import { allFields, fieldValues, isEmpty, keyOf, treeKeys, withCode, type Place } from "./lib/places";
  import { placeText } from "./lib/tree";
  import PlaceInput from "./PlaceInput.svelte";

  const COLOR_TAGS: api.Color[] = ["Red", "Orange", "Yellow", "Green", "Blue", "Pink", "Purple"];

  // Everything the library holds about one photo. Keywords, albums, the folder and the
  // camera are links: following one filters the grid by it. Stars and colours edit.
  let {
    row, version, picked, queued, collections, keywords, clip, shared, sharedAlbums, sharedPlaces, places,
    onrate, oncolor, onaddkeyword, onaddkeywords, onremovekeyword, onkeyword, onalbum, onfolder, oncamera,
    onaddalbum, onremovealbum, onlook,
    onplace, ongeo, oniptc, oncopy, onpaste, onpastechoose, onsync,
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
    /** Filters by something the analysis found (a duplicate group, a series). */
    onlook: (f: api.Filter) => void;
    onfolder: (path: string) => void;
    oncamera: (model: string) => void;
    /** With several photos selected: their places, each with the photos that have it. */
    sharedPlaces: [Place, number[]][] | null;
    /** Every place in the catalog (`place_tree` keys with counts), for suggestions. */
    places: [string, number][];
    /** Location fields typed in, "" clearing one: on `rows`, or on every photo edited. */
    onplace: (fields: Partial<Record<api.PlaceField, string>>, rows?: number[]) => void;
    ongeo: (value: [number, number] | null) => void;
    /** An IPTC text field typed in; "" clears it. */
    oniptc: (field: api.CaptionField, value: string) => void;
    /** What the clipboard holds, for showing; null when empty. */
    clip: string | null;
    oncopy: () => void;
    /** `fill` only fills what each photo leaves empty. */
    onpaste: (fill: boolean) => void;
    /** Opens the choice of what to paste, and how. */
    onpastechoose: () => void;
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

  /** What the analysis found, once the photo has been measured. */
  let look = $state<api.LookView | null>(null);
  $effect(() => {
    const current = row;
    void version;
    look = null;
    if (current !== undefined) api.look(current).then((l) => { if (current === row) look = l; }).catch(() => {});
  });
  const hex = (rgb: number[]) => "#" + rgb.map((c) => c.toString(16).padStart(2, "0")).join("");

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

  // Several photos selected: the places they have, set apart as keywords are, and for
  // each field the value they share or how many values they hold.
  let placeGroups = $derived(picked > 1 && sharedPlaces ? sharedPlaces : null);
  let placeCounts = $derived(placeGroups?.map(([p, rows]): [Place, number] => [p, rows.length]) ?? null);
  let placeKeys = $derived(placeCounts ? treeKeys(placeCounts) : undefined);
  let placedGroups = $derived((placeGroups ?? []).filter(([p]) => !isEmpty(p)));
  let unplaced = $derived(placeGroups?.find(([p]) => isEmpty(p))?.[1] ?? []);
  let fillOpen = $state(false);
  const NO_PLACE: Place = { country: null, country_code: null, state: null, city: null, location: null };
  const placeLabel = (place: Place) => placeText(keyOf(place));

  type FieldView = { shown: string; placeholder: string; title?: string };
  function fieldView(place: Place, field: api.PlaceField): FieldView {
    if (!placeCounts) return { shown: place[field] ?? "", placeholder: picked > 1 ? `set on ${picked} photos` : "—" };
    const { values, empty } = fieldValues(placeCounts, field);
    if (values.length === 1 && !empty) return { shown: values[0][0], placeholder: "" };
    if (!values.length) return { shown: "", placeholder: `empty on ${picked.toLocaleString()}` };
    const lines = values.slice(0, 12).map(([v, n]) => `${v}: ${n.toLocaleString()}`);
    if (values.length > 12) lines.push(`… ${values.length - 12} more`);
    if (empty) lines.push(`(empty): ${empty.toLocaleString()}`);
    lines.push(`Type to set it on all ${picked.toLocaleString()}`);
    const placeholder = values.length === 1
      ? `${values[0][0]} (${values[0][1].toLocaleString()}/${picked.toLocaleString()})`
      : `${values.length} values${empty ? ` (${empty.toLocaleString()} empty)` : ""}`;
    return { shown: "", placeholder, title: lines.join("\n") };
  }

  /** The levels the selection shares (or the photo has): where suggestions start from. */
  function sharedPlace(place: Place): Partial<Place> {
    if (!placeCounts) return place;
    const out: Partial<Place> = {};
    for (const [field] of PLACE_FIELDS) {
      const { values, empty } = fieldValues(placeCounts, field);
      if (values.length === 1 && !empty) out[field] = values[0][0];
    }
    return out;
  }

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
          Copy to {(picked - 1).toLocaleString()}…
        </button>
      {/if}
    </div>
    {#if clip}
      {@const onto = picked > 1 ? `the ${picked} selected photos` : "this photo"}
      <div class="clip">
        <button title="Paste onto {onto}, replacing what they have (Ctrl+Shift+V)" onclick={() => onpaste(false)}>
          Paste{picked > 1 ? ` to ${picked.toLocaleString()}` : ""}
        </button>
        <button title="Merge: fill only the fields {onto} leave empty, add keywords (Ctrl+Alt+V)" onclick={() => onpaste(true)}>Merge</button>
        <button title="Choose what to paste, and whether to overwrite or merge (Ctrl+Shift+V)" onclick={onpastechoose}>Paste…</button>
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
    {#if !albumRows.length}<p class="hint">In no album. Drag photos onto an album in the sidebar to put them in.</p>{/if}

    {#if look}
      {@const a = look.analysis}
      <h3>Look</h3>
      <div class="palette" title="The main colours, by share of the picture">
        {#each a.palette as s}
          <i style:background={hex(s.rgb)} style:flex-grow={s.share} title="{hex(s.rgb)} · {Math.round(s.share * 100)}%"></i>
        {/each}
      </div>
      <dl class="fields look">
        <dt>Shape</dt><dd>{look.shape}</dd>
        <dt>Colour</dt><dd>{look.hues.join(", ")}</dd>
        <dt>Light</dt><dd title="Mean brightness {Math.round(a.brightness * 100)}%, contrast {Math.round(a.contrast * 100)}%">{look.light.join(", ")}</dd>
        <dt>Focus</dt><dd title="Detail in the sharpest part: {Math.round(a.sharpness)}">{look.focus}</dd>
      </dl>
      {#if look.duplicates}
        {@const [group, n] = look.duplicates}
        <button class="link look-link" onclick={() => onlook({ group })}>Looks like {n - 1} other photo{n === 2 ? "" : "s"}: show them</button>
      {/if}
      {#if look.series}
        {@const [group, n] = look.series}
        <button class="link look-link" onclick={() => onlook({ group })}>One of a series of {n}: show it</button>
      {/if}
    {/if}

    <h3>Location</h3>
    {#if placeGroups}
      {#if unplaced.length}
        <p class="partial-note">
          <span>{unplaced.length.toLocaleString()} of the {picked.toLocaleString()} have no location</span>
          {#if placedGroups.length}
            <button title="Give them one of the places the others have" onclick={() => (fillOpen = !fillOpen)}>Fill…</button>
          {/if}
        </p>
        {#if fillOpen && placedGroups.length}
          <ul class="links keywords fill-places">
            {#each placedGroups as [place, rows] (keyOf(place) + place.country_code)}
              <li>
                <button
                  class="link"
                  title="Put this place on the {unplaced.length.toLocaleString()} without one"
                  onclick={() => { fillOpen = false; onplace(allFields(place), unplaced); }}>{placeLabel(place)}</button
                >
                <span class="count">{rows.length.toLocaleString()}</span>
              </li>
            {/each}
          </ul>
        {/if}
      {/if}
      <ul class="links keywords places">
        {#each placedGroups as [place, rows] (keyOf(place) + place.country_code)}
          {@const n = rows.length}
          <li class:partial={n < picked}>
            <span class="link" title={place.country_code ? `ISO code ${place.country_code}` : "No ISO country code"}>{placeLabel(place)}</span>
            {#if n < picked}
              <span class="count" title="On {n.toLocaleString()} of the {picked.toLocaleString()} selected photos">{n.toLocaleString()}/{picked.toLocaleString()}</span>
              <button class="apply" title="Put this place on all {picked.toLocaleString()} selected photos" aria-label="Put {placeLabel(place)} on all" onclick={() => onplace(allFields(place))}>+</button>
            {/if}
            <button
              class="remove"
              class:after={n < picked}
              title="Clear the location of the {n === 1 ? 'photo' : `${n.toLocaleString()} photos`} with this place"
              aria-label="Clear {placeLabel(place)}"
              onclick={() => onplace(allFields(NO_PLACE), rows)}>×</button
            >
          </li>
        {/each}
      </ul>
    {/if}
    <dl class="fields" class:queued={queued.includes("place")}>
      {#each PLACE_FIELDS as [field, label] (field)}
        {@const view = fieldView(p.place, field)}
        <dt><label for="place-{field}">{label}</label></dt>
        <dd>
          {#if field === "country_code"}
            <input
              id="place-{field}"
              class="field code"
              value={view.shown}
              placeholder={view.placeholder}
              title={view.title}
              maxlength={3}
              spellcheck="false"
              onkeydown={(e) => fieldKeys(e, view.shown)}
              onchange={(e) => onplace(withCode(field, e.currentTarget.value, sharedPlace(p.place)))}
            />
          {:else}
            <PlaceInput
              id="place-{field}"
              level={field}
              shown={view.shown}
              placeholder={view.placeholder}
              title={view.title}
              context={sharedPlace(p.place)}
              catalog={places}
              selection={placeKeys}
              {picked}
              geo={p.geo}
              {version}
              onfields={(fields) => onplace(fields)}
              ontext={(value) => onplace(withCode(field, value, sharedPlace(p.place)))}
            />
          {/if}
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
