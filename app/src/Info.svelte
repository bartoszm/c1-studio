<script lang="ts">
  import * as api from "./lib/api";
  import { COLORS, ORIGINS, bytes, camera, date, eventText, exposure, folderOf, geoText, parseGeo } from "./lib/format";
  import { allFields, fieldValues, isEmpty, keyOf, treeKeys, withCode, type Place } from "./lib/places";
  import { placeText } from "./lib/tree";
  import PlaceInput from "./PlaceInput.svelte";
  import { tick } from "svelte";
  import { collapse, mergedField, scrollFix } from "./lib/batch";

  const COLOR_TAGS: api.Color[] = ["Red", "Orange", "Yellow", "Green", "Blue", "Pink", "Purple"];

  // Everything the library holds about one photo. Keywords, albums, the folder and the
  // camera are links: following one filters the grid by it. Stars and colours edit.
  let {
    row, version, analysed, picked, queued, queuedPicked, collections, keywords, clip, summary, stale,
    onrate, oncolor, onaddkeyword, onaddkeywords, onremovekeyword, onkeyword, onalbum, onfolder, oncamera,
    onaddalbum, onremovealbum, ongroup,
    onplace, ongeo, onfillplace, ongeofromplace, oniptc, oncopy, onpaste, onpastechoose, onsync,
  }: {
    row: number | undefined;
    /** Bumped after edits, to read the photo again. */
    version: number;
    /** Bumped when the analysis has measured photos again, to read Look again. */
    analysed: number;
    /** How many photos an edit here applies to. */
    picked: number;
    /** Fields of this photo edited here and not yet written to Capture One. */
    queued: string[];
    /** With several photos selected: how many of them have edits waiting. */
    queuedPicked: number;
    collections: Map<string, api.Collection>;
    /** Every keyword path in the catalog, most used first, for completion. */
    keywords: string[];
    onrate: (stars: number | null) => void;
    oncolor: (color: api.Color | null) => void;
    onaddkeyword: (path: string) => void;
    /** Adds keywords to every selected photo: the ones only some of them have. */
    onaddkeywords: (paths: string[]) => void;
    /** With several photos selected: what they have, read together (null until read). */
    summary: api.Selection | null;
    /** `summary` is about the photos picked before: shown dimmed until the new one is in. */
    stale: boolean;
    onremovekeyword: (path: string) => void;
    onkeyword: (path: string) => void;
    onalbum: (id: string) => void;
    /** Puts the selected photos into an album, or takes them out. */
    onaddalbum: (id: string) => void;
    onremovealbum: (id: string) => void;
    /** Shows a duplicate group's or a series' photos, keeping the other filters. */
    ongroup: (id: string) => void;
    onfolder: (path: string) => void;
    oncamera: (model: string) => void;
    /** Location fields typed in, "" clearing one: on `rows`, or on every photo edited. */
    onplace: (fields: Partial<Record<api.PlaceField, string>>, rows?: number[]) => void;
    ongeo: (value: [number, number] | null) => void;
    /** Fills the empty place fields of the photos edited from their positions. */
    onfillplace: () => void;
    /** Gives the photos edited that have no position their place's (approximate). */
    ongeofromplace: () => void;
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
  // Not state: which photo `look` is about. Edits don't change it: only another photo, or
  // the analysis measuring again, reads it again; the same photo keeps it until then.
  let lookOf: number | undefined;
  $effect(() => {
    const current = row;
    void analysed;
    if (current !== lookOf) {
      lookOf = current;
      look = null;
    }
    if (current !== undefined) api.look(current).then((l) => { if (current === row) look = l; }).catch(() => {});
  });
  const hex = (rgb: number[]) => "#" + rgb.map((c) => c.toString(16).padStart(2, "0")).join("");

  /** What the change log holds about the photo, read only while the section is open. */
  let historyOpen = $state(false);
  let history = $state<api.LogEvent[] | null>(null);
  let historyError = $state<string | null>(null);
  /** Names the log knows for albums no longer listed (deleted, or since written). */
  let historyAlbums = $state<Record<string, string>>({});
  // Not state: which photo `history` is about, and the newest read asked for.
  let historyOf: number | undefined;
  let historyAsked = 0;
  $effect(() => {
    const current = row;
    void version;
    // Another photo starts empty; the same one keeps its list until the new one is in.
    if (current !== historyOf) {
      historyOf = current;
      history = null;
      historyError = null;
    }
    if (current === undefined || !historyOpen) return;
    const asked = ++historyAsked;
    api.photoHistory(current).then(
      (h) => {
        if (asked !== historyAsked) return;
        history = h.events;
        historyAlbums = h.albums;
        historyError = null;
      },
      (e) => {
        if (asked === historyAsked) historyError = String(e);
      },
    );
  });
  const loggedAlbumName = (id: string) => `“${collections.get(id)?.path ?? historyAlbums[id] ?? id}”`;

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

  /** Several photos picked: the panel opens with a block of their fields, in a fixed order
   *  and of fixed height, with what varies (lists, the focused photo) below it. */
  let batch = $derived(picked > 1);
  /** The selection as read: kept, dimmed, while a newer pick is read. */
  let fresh = $derived(batch && summary && !stale ? summary : null);

  // Several photos selected: the places they have, set apart as keywords are, and for
  // each field the value they share or how many values they hold. The list's rows are
  // edited by its buttons, so only the pick's own reply is used.
  let placeGroups = $derived(fresh ? (fresh.places as [Place, number[]][]) : null);
  let placeCounts = $derived(placeGroups?.map(([p, rows]): [Place, number] => [p, rows.length]) ?? null);
  let placeKeys = $derived(placeCounts ? treeKeys(placeCounts) : undefined);
  /** The places list: the selection's as read, dimmed while a newer pick is read. */
  let listGroups = $derived(batch ? ((summary?.places ?? []) as [Place, number[]][]) : []);
  let placedGroups = $derived(listGroups.filter(([p]) => !isEmpty(p)));
  let unplaced = $derived(listGroups.filter(([p]) => isEmpty(p)).flatMap(([, rows]) => rows));
  let fillOpen = $state(false);
  const NO_PLACE: Place = { country: null, country_code: null, state: null, city: null, location: null };
  const placeLabel = (place: Place) => (keyOf(place) ? placeText(keyOf(place)) : `ISO code ${place.country_code}`);
  /** A place's own key in a list: the groups differ in some field, any of them. */
  const placeId = (place: Place) => JSON.stringify(allFields(place));

  type FieldView = { shown: string; placeholder: string; title?: string };
  function fieldView(place: Place, field: api.PlaceField): FieldView {
    // Several picked and not read yet: nothing shown as theirs, not even the focused photo's.
    if (batch && !placeCounts) return mergedField(null, picked);
    if (!placeCounts) return { shown: place[field] ?? "", placeholder: "—" };
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
    if (!placeCounts) return batch ? {} : place;
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
    batch
      ? (summary?.keywords ?? []).map(([full, n]) => ({ full, path: full.split("|"), n }))
          .sort((a, b) => a.full.localeCompare(b.full, undefined, { sensitivity: "base" }))
      : sortedKeywords.map((path) => ({ full: path.join("|"), path, n: picked })),
  );
  let partial = $derived(keywordRows.filter((k) => k.n < picked).map((k) => k.full));

  // The albums the photo is in; with several selected, every album any of them is in,
  // those holding only some set apart, as keywords are.
  type AlbumRow = { c: api.Collection; n: number };
  let albumRows: AlbumRow[] = $derived(
    (batch ? (summary?.albums ?? []) : (photo?.collections ?? []).map((id): [string, number] => [id, picked]))
      .map(([id, n]) => ({ c: collections.get(id)!, n }))
      .filter((r) => !!r.c)
      .sort((a, b) => a.c.path.localeCompare(b.c.path, undefined, { sensitivity: "base" })),
  );

  // Long lists of a big selection show their first rows, those on only some first.
  let open = $state({ keywords: false, albums: false, places: false });
  let shownKeywords = $derived(collapse(keywordRows, (k) => k.n < picked, !batch || open.keywords));
  let shownAlbums = $derived(collapse(albumRows, (a) => a.n < picked, !batch || open.albums));
  let shownPlaces = $derived(collapse(placedGroups, ([, rows]) => rows.length < picked, !batch || open.places));

  /** The selection's rating, colour, GPS and IPTC fields; null while it is read. */
  let merged = $derived(fresh);
  const geoOf = ([lat, lon]: [number, number]) => geoText({ lat, lon });

  // The field being typed in stays where it is when what is above it changes (a list
  // read, a note added): the panel scrolls by as much as it moved.
  let panel = $state<HTMLElement>();
  $effect.pre(() => {
    void [summary, stale, photo, look, queued, queuedPicked, keywordRows.length, albumRows.length];
    const el = document.activeElement;
    if (!panel || !(el instanceof HTMLElement) || !panel.contains(el) || el === panel) return;
    const before = el.getBoundingClientRect().top;
    tick().then(() => {
      if (panel && panel.contains(el)) panel.scrollTop = scrollFix(before, el.getBoundingClientRect().top, panel.scrollTop);
    });
  });
</script>

{#snippet marks(p: api.Photo)}
  {@const rating = batch ? (merged?.rating.same ?? null) : p.rating}
  {@const color = batch ? (merged?.color.same ?? null) : p.color}
  <div class="marks">
    <span class="rate" role="group" aria-label="Rating" title={batch && merged && merged.rating.same === null && merged.rating.values ? `${merged.rating.values} ratings among them` : undefined}>
      {#each [1, 2, 3, 4, 5] as n}
        <button
          class="star"
          class:on={(rating ?? 0) >= n}
          title={rating === n ? "Clear the rating (0)" : `${n} star${n > 1 ? "s" : ""} (${n})`}
          onclick={() => onrate(rating === n ? null : n)}>★</button
        >
      {/each}
    </span>
    {#if !batch}
      {#if p.trashed}<span class="tag">in trash</span>{/if}
      {#if p.edit.edited}<span class="tag">edited</span>{/if}
    {/if}
  </div>
  <div class="colors" role="group" aria-label="Colour tag">
    {#each COLOR_TAGS as c}
      <button
        class="dot"
        class:on={color === c}
        style:background={COLORS[c.toLowerCase()]}
        title={color === c ? "Clear the colour" : c}
        aria-label={c}
        onclick={() => oncolor(color === c ? null : c)}
      ></button>
    {/each}
  </div>
{/snippet}

{#snippet clipRows()}
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
{/snippet}

{#snippet exif(p: api.Photo)}
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
{/snippet}

{#snippet keywordList()}
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
    {#each shownKeywords.shown as { full, path, n } (full)}
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
  {@render more(shownKeywords.more, () => (open.keywords = true))}
{/snippet}

{#snippet more(n: number, show: () => void)}
  {#if n}<button class="link more" onclick={show}>+{n.toLocaleString()} more</button>{/if}
{/snippet}

{#snippet keywordInput()}
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
{/snippet}

{#snippet albumList()}
  <h3>Albums</h3>
  <ul class="links keywords albums">
    {#each shownAlbums.shown as { c, n } (c.id)}
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
  {@render more(shownAlbums.more, () => (open.albums = true))}
  {#if !albumRows.length && (!batch || summary)}<p class="hint">In no album. Drag photos onto an album in the sidebar to put them in.</p>{/if}
{/snippet}

{#snippet lookSection()}
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
      <button class="link look-link" title="In the grid, within the other filters" onclick={() => ongroup(group)}>Looks like {n - 1} other photo{n === 2 ? "" : "s"}: show them</button>
    {/if}
    {#if look.series}
      {@const [group, n] = look.series}
      <button class="link look-link" title="In the grid, within the other filters" onclick={() => ongroup(group)}>One of a series of {n}: show it</button>
    {/if}
  {/if}
{/snippet}

{#snippet placeList()}
  {#if unplaced.length}
    <p class="partial-note">
      <span>{unplaced.length.toLocaleString()} of the {picked.toLocaleString()} have no location</span>
      {#if placedGroups.length}
        <button title="Give them one of the places the others have" onclick={() => (fillOpen = !fillOpen)}>Fill…</button>
      {/if}
    </p>
    {#if fillOpen && placedGroups.length}
      <ul class="links keywords fill-places">
        {#each placedGroups as [place, rows] (placeId(place))}
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
    {#each shownPlaces.shown as [place, rows] (placeId(place))}
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
  {@render more(shownPlaces.more, () => (open.places = true))}
{/snippet}

{#snippet locationFields(p: api.Photo)}
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
            selection={placeKeys}
            {picked}
            geo={batch ? null : p.geo}
            {version}
            onfields={(fields) => onplace(fields)}
            ontext={(value) => onplace(withCode(field, value, sharedPlace(p.place)))}
          />
        {/if}
      </dd>
    {/each}
  </dl>
  <dl class="fields" class:queued={queued.includes("geo")}>
    {#key batch ? JSON.stringify(merged?.geo ?? null) : p.geo}
      {@const view = batch ? mergedField(merged?.geo, picked, geoOf) : { shown: p.geo ? geoText(p.geo) : "", placeholder: "latitude, longitude", title: undefined }}
      <dt><label for="place-geo">GPS</label></dt>
      <dd>
        <input
          id="place-geo"
          class="field"
          value={view.shown}
          placeholder={view.placeholder}
          title={`${view.title ? view.title + "\n" : ""}Decimal (50.0614, 19.9366), with N/S/E/W, or degrees and minutes (44°58′N 14°45′E). Empty clears it.`}
          spellcheck="false"
          onkeydown={(e) => fieldKeys(e, view.shown)}
          onchange={commitGeo}
        />
      </dd>
    {/key}
  </dl>
  {#if !batch && geoError}<p class="error">{geoError}</p>{/if}
  {@const named = !!(p.place.city || p.place.state || p.place.country || p.place.location)}
  {#if batch || (p.geo && !(p.place.city && p.place.country)) || (!p.geo && named)}
    <div class="geo-actions">
      {#if batch || p.geo}
        <button
          title="Fill the empty city, region and country fields from the position (built-in GeoNames; offline). Fields already filled stay."
          onclick={onfillplace}
        >Fill place from GPS</button>
      {/if}
      {#if batch || (!p.geo && named)}
        <button
          title="Photos with no GPS get the middle of their city, region or country. Shown as approximate (≈) on the map; can be undone."
          onclick={ongeofromplace}
        >GPS from place ≈</button>
      {/if}
    </div>
  {/if}
{/snippet}

{#snippet iptcFields(p: api.Photo)}
  {#each IPTC_GROUPS as [title, fields] (title)}
    <h3>{title}</h3>
    <dl class="fields" class:queued={queued.includes("iptc")}>
      {#each fields as [field, label, hint] (field)}
        {@const view = batch
          ? mergedField(merged?.iptc[field], picked)
          : { shown: p.iptc[field] ?? "", placeholder: field === "title" ? "object name" : "—", title: hint }}
        {@const shown = view.shown}
        <dt><label for="iptc-{field}" title={hint}>{label}</label></dt>
        <dd>
          {#if field === "description"}
            <textarea
              id="iptc-{field}"
              class="field text"
              rows={batch ? 2 : Math.min(8, Math.max(2, shown.split("\n").length))}
              value={shown}
              placeholder={view.placeholder}
              title={`${batch && view.title ? view.title + "\n" : ""}Enter adds a line; Ctrl+Enter or leaving the field saves; Escape puts it back`}
              onkeydown={(e) => textKeys(e, shown)}
              onchange={(e) => oniptc(field, e.currentTarget.value)}
            ></textarea>
          {:else}
            <input
              id="iptc-{field}"
              class="field"
              value={shown}
              placeholder={view.placeholder}
              title={batch ? view.title : hint}
              spellcheck={field !== "title" && field !== "job_id"}
              onkeydown={(e) => fieldKeys(e, shown)}
              onchange={(e) => oniptc(field, e.currentTarget.value)}
            />
          {/if}
        </dd>
      {/each}
    </dl>
  {/each}
{/snippet}

{#snippet fileSection(p: api.Photo)}
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

  <details class="history" bind:open={historyOpen}>
    <summary><h3>History</h3></summary>
    {#if historyError}
      <p class="hint">Could not read the history: {historyError}</p>
    {:else if history === null}
      <p class="hint">Reading…</p>
    {:else if !history.length}
      <p class="hint">No changes seen since the app started keeping a history.</p>
    {:else}
      <ol>
        {#each history as e}
          <li>
            <span class="when">{date(e.at)}</span>
            <span class="origin from-{e.origin.toLowerCase()}">{ORIGINS[e.origin]}</span>
            <span class="what">{eventText(e, loggedAlbumName)}</span>
          </li>
        {/each}
      </ol>
    {/if}
  </details>
{/snippet}

<aside class="info-panel" bind:this={panel}>
  {#if failed}
    <p class="error">{failed}</p>
  {:else if !photo}
    <p class="hint">Select a photo to see its details.</p>
  {:else if batch}
    {@const p = photo}
    <!-- The fields of the selection, in a fixed order and of fixed height: nothing above
         a field changes as the selection is read or edited. -->
    <section class="batch" aria-label="Edit {picked} photos">
      <h2>Edit {picked.toLocaleString()} photos</h2>
      {@render marks(p)}
      {@render keywordInput()}
      <h3>Location</h3>
      {@render locationFields(p)}
      {@render iptcFields(p)}
    </section>
    <p class="queued-note badge" class:none={!queuedPicked}>
      {#if queuedPicked}{queuedPicked.toLocaleString()} of the {picked.toLocaleString()} edited here, not yet written to Capture One.{/if}
    </p>
    {#if typedError}<p class="error">{typedError}</p>{/if}
    {#if geoError}<p class="error">{geoError}</p>{/if}
    {@render clipRows()}
    <div class="lists" class:stale={stale && !!summary}>
      {#if !summary}<p class="hint">Reading the {picked.toLocaleString()} photos…</p>{/if}
      {@render keywordList()}
      {@render albumList()}
      <h3>Places</h3>
      {@render placeList()}
    </div>
    <h3 class="focused" title={p.file.name}>Focused: {p.file.name}</h3>
    {@render exif(p)}
    {@render lookSection()}
    {@render fileSection(p)}
  {:else}
    {@const p = photo}
    <h2 title={p.file.name}>{p.file.name}</h2>
    {@render marks(p)}
    {@render clipRows()}
    {#if queued.length}
      <p class="queued-note">Edited here ({queued.map(fieldName).join(", ")}), not yet written to Capture One.</p>
    {/if}
    {@render exif(p)}
    {@render keywordList()}
    {@render keywordInput()}
    {#if typedError}<p class="error">{typedError}</p>{/if}
    {@render albumList()}
    {@render lookSection()}
    <h3>Location</h3>
    {@render locationFields(p)}
    {@render iptcFields(p)}
    {@render fileSection(p)}
  {/if}
</aside>
