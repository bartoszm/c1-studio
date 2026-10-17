<script lang="ts">
  import { onMount, untrack } from "svelte";
  import * as api from "./lib/api";
  import type { Filter, GroupPage, GroupSort } from "./lib/api";
  import { GROUPS, sameSettings, shown, withField } from "./lib/classes";
  import { groupBadge, groupCount, moreLabel } from "./lib/analysis";
  import SortControl from "./SortControl.svelte";

  // Duplicates and series in one place: the groups of alike photos under the current filter,
  // in the order asked for, and the lines that decide what counts as one (applied on request;
  // the photos are sorted again, not measured again).
  let {
    filter, version, total, counts, onflag, ongroup, onclasses,
  }: {
    filter: Filter;
    /** Bumped when photos were edited or measured again. */
    version: number;
    total: number;
    /** Photos under the filter that are in a duplicate group, and in a series. */
    counts: { duplicates: number; series: number };
    onflag: (flag: "duplicates" | "series") => void;
    /** Show one group's photos in the grid. */
    ongroup: (id: string) => void;
    /** The lines moved: classes and groups changed. */
    onclasses: () => void;
  } = $props();

  type Kind = "duplicates" | "series";
  const KINDS: [Kind, string, string][] = [
    ["duplicates", "Duplicates", "The same frame twice: different images taken at the same instant that look alike."],
    ["series", "Series", "Bursts: photos from one camera, seconds apart, looking alike."],
  ];
  /** The order of the groups: a field and, reversed, the other way round. */
  const SORTS: { key: GroupSort; label: string; first: string; second: string }[] = [
    { key: "Matched", label: "Matching photos", first: "Most first", second: "Fewest first" },
    { key: "Size", label: "Group size", first: "Biggest first", second: "Smallest first" },
    { key: "Newest", label: "Date", first: "Newest first", second: "Oldest first" },
    { key: "Sharpest", label: "Sharpness", first: "Sharpest first", second: "Softest first" },
  ];
  const remembered = <T extends string>(key: string, allowed: readonly T[], fallback: T): T => {
    try {
      const v = localStorage.getItem(key) as T | null;
      return v && allowed.includes(v) ? v : fallback;
    } catch {
      return fallback;
    }
  };
  const remember = (key: string, value: string) => {
    try { localStorage.setItem(key, value); } catch { /* not remembered */ }
  };
  let kind = $state<Kind>(remembered("alikeKind", ["duplicates", "series"], "duplicates"));
  let sort = $state<GroupSort>(remembered("alikeSort", SORTS.map((s) => s.key), "Matched"));
  let reversed = $state(remembered("alikeReversed", ["1", "0"], "0") === "1");
  const sortInfo = $derived(SORTS.find((s) => s.key === sort) ?? SORTS[0]);

  // The groups, a page at a time: read again from the first page when the filter, the kind or
  // the order changes; "Show more" adds the next. Not state: the newest read asked for, and
  // what `list` holds.
  let list = $state<GroupPage>({ total: 0, groups: [] });
  let adding = $state(false);
  let loadedFor = "";
  let listKey = "";
  let listFilter = "";
  let loaded: Filter = {};
  function read() {
    // Picks of the groups themselves and stacking don't change which groups exist.
    const asked = { ...$state.snapshot(filter), group: undefined, stacked: undefined };
    const filterKey = JSON.stringify(asked) + ":" + kind + ":" + sort + ":" + reversed;
    const key = filterKey + ":" + version + ":" + counts.duplicates + ":" + counts.series;
    if (key === loadedFor) return;
    loadedFor = key;
    // The same read again (an edit, photos measured): as many groups as are shown, so a list
    // opened further stays open. Anything else starts from the first page.
    const count = filterKey === listFilter ? Math.max(api.GROUP_PAGE, untrack(() => list.groups.length)) : api.GROUP_PAGE;
    api.groups(asked, kind === "series", 0, count, sort, reversed)
      .then((page) => {
        if (key !== loadedFor) return;
        list = page;
        loaded = asked;
        listKey = key;
        listFilter = filterKey;
      })
      .catch(() => {});
  }
  $effect(() => {
    // Read here so the effect depends on all of them.
    void [filter, kind, sort, reversed, version, counts.duplicates, counts.series];
    untrack(read);
  });
  async function more() {
    // Not while the list is being read again: the next page would be the old one's.
    if (adding || listKey !== loadedFor) return;
    const key = listKey;
    adding = true;
    try {
      const page = await api.groups(loaded, kind === "series", list.groups.length, api.GROUP_PAGE, sort, reversed);
      if (key !== loadedFor || key !== listKey) return;
      const have = new Set(list.groups.map((g) => g.id));
      list = { total: page.total, groups: [...list.groups, ...page.groups.filter((g) => !have.has(g.id))] };
    } catch {
      // The button stays for another try.
    } finally {
      adding = false;
    }
  }

  // The lines: sliders over the Alike fields, with how many photos they would group.
  const FIELDS = GROUPS.find((g) => g.title === "Alike")!.fields;
  let saved = $state<api.ClassSettings | null>(null);
  let draft = $state<api.ClassSettings | null>(null);
  let preview = $state<api.ClassPreview | null>(null);
  let busy = $state(false);
  let error = $state("");
  let timer: ReturnType<typeof setTimeout> | undefined;
  let asked = 0;
  let changed = $derived(!!saved && !!draft && !sameSettings(saved, draft));
  onMount(() => {
    (async () => {
      try {
        saved = await api.classSettings();
        draft = { ...saved };
      } catch (e) {
        error = String(e);
      }
    })();
    return () => clearTimeout(timer);
  });
  // Grouping reads every photo, so the preview waits for the slider to rest.
  function edit(next: api.ClassSettings) {
    draft = next;
    clearTimeout(timer);
    const mine = ++asked;
    timer = setTimeout(async () => {
      try {
        const got = await api.previewClasses(next);
        if (mine === asked) preview = got;
      } catch (e) {
        error = String(e);
      }
    }, 300);
  }
  async function apply() {
    if (!draft || busy) return;
    busy = true;
    error = "";
    try {
      saved = await api.setClassSettings(draft);
      draft = { ...saved };
      preview = null;
      onclasses();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
  async function reset() {
    try {
      const d = await api.defaultClassSettings();
      if (draft) edit({ ...draft, ...Object.fromEntries(FIELDS.map((f) => [f.key, d[f.key]])) });
    } catch (e) {
      error = String(e);
    }
  }
  const would = $derived(preview ? `would group ${preview.duplicates.toLocaleString()} photos as duplicates and ${preview.series.toLocaleString()} in series` : "");
</script>

<div class="alike">
  <div class="top">
    <div class="kinds" role="group" aria-label="Kind of group">
      {#each KINDS as [key, label] (key)}
        <button class:on={kind === key} aria-pressed={kind === key} onclick={() => { kind = key; remember("alikeKind", key); }}>
          {label}
          <span class="n">{counts[key].toLocaleString()}</span>
        </button>
      {/each}
    </div>
    <SortControl
      fields={SORTS.map((s) => ({ key: s.key, label: s.label }))}
      field={sort}
      words={reversed ? sortInfo.second : sortInfo.first}
      label="Sort groups by"
      onfield={(key) => { sort = key as GroupSort; reversed = false; remember("alikeSort", sort); remember("alikeReversed", "0"); }}
      onflip={() => { reversed = !reversed; remember("alikeReversed", reversed ? "1" : "0"); }}
    />
  </div>

  {#if draft}
    <details class="lines">
      <summary>Lines: what counts as {kind === "series" ? "a series" : "a duplicate"}{would && changed ? ` · ${would}` : ""}</summary>
      <div class="sliders">
        {#each FIELDS as f (f.key)}
          <label class="slider">
            <span class="name">{f.label}</span>
            <input
              type="range"
              min={f.min}
              max={f.max}
              step={f.step}
              value={shown(f, draft)}
              oninput={(e) => edit(withField(draft!, f, Number(e.currentTarget.value)))}
            />
            <output>{shown(f, draft)} {f.unit}</output>
            {#if f.hint}<small class="muted">{f.hint}</small>{/if}
          </label>
        {/each}
        {#if error}<p class="error" role="alert">{error}</p>{/if}
        <div class="actions">
          <button disabled={busy} onclick={reset}>Defaults</button>
          <button disabled={!changed || busy} onclick={apply}>{busy ? "Applying…" : "Apply"}</button>
        </div>
      </div>
    </details>
  {/if}

  {#each KINDS.filter(([key]) => key === kind) as [key, label, hint] (key)}
    <h3>
      {label}
      <span class="totals">
        <button class="link" class:on={filter[key]} title="Only these photos in the grid" onclick={() => onflag(key)}>
          {counts[key].toLocaleString()} photo{counts[key] === 1 ? "" : "s"}
        </button>
        · <span>{groupCount(list.total)}</span>
      </span>
    </h3>
    <p class="muted hint">{hint}</p>
    <div class="groups">
      {#each list.groups as g (g.id)}
        <button
          class="group"
          class:on={filter.group === g.id}
          title={g.matched < g.size
            ? `Show the ${g.matched} of these ${g.size} photos the filter picks in the grid`
            : `Show these ${g.size} photos in the grid`}
          onclick={() => ongroup(g.id)}
        >
          <span class="thumbs">
            {#each g.rows as row (row)}<img src={api.thumbUrl(row)} alt="" loading="lazy" decoding="async" />{/each}
          </span>
          <span class="size">{groupBadge(g)}</span>
        </button>
      {:else}
        <p class="muted">None{filter.group || total === 0 ? "" : " found"}.</p>
      {/each}
    </div>
    {#if list.total > list.groups.length}
      <button class="more" disabled={adding} onclick={more}>{moreLabel(list.total, list.groups.length)}</button>
    {/if}
  {/each}
</div>

<style>
  .alike { display: flex; flex-direction: column; gap: 8px; }
  .top { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .kinds { display: flex; gap: 4px; }
  .kinds button { padding: 3px 12px; border: 1px solid var(--line, #444); border-radius: 3px; }
  .kinds button.on { background: var(--on, #2a3a55); border-color: #5b8def; }
  .kinds .n { margin-left: 6px; color: var(--muted, #999); font-variant-numeric: tabular-nums; }
  .lines { border: 1px solid var(--line, #333); border-radius: 4px; padding: 4px 10px; background: rgba(255, 255, 255, 0.03); }
  .lines summary { cursor: pointer; font-size: 12px; padding: 2px 0; }
  .sliders { display: grid; grid-template-columns: repeat(auto-fit, minmax(300px, 1fr)); gap: 6px 18px; padding: 6px 0; }
  .slider { display: grid; grid-template-columns: 1fr 90px 64px; align-items: center; gap: 2px 8px; }
  .slider small { grid-column: 1 / -1; }
  .slider input { min-width: 0; accent-color: #5b8def; }
  .slider output { text-align: right; font-variant-numeric: tabular-nums; font-size: 12px; }
  .name { font-size: 13px; }
  .muted { color: var(--muted, #999); font-size: 12px; }
  .error { color: #e5484d; font-size: 12px; grid-column: 1 / -1; margin: 0; }
  .actions { grid-column: 1 / -1; display: flex; justify-content: flex-end; gap: 8px; }
  .actions button, .more { padding: 3px 12px; border: 1px solid var(--line, #444); border-radius: 3px; }
  .actions button:hover:not(:disabled), .more:hover { background: var(--hover, #333); }
  button:disabled { opacity: 0.35; cursor: default; }
  h3 { margin: 4px 0 0; font-size: 12px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em; color: #999; display: flex; justify-content: space-between; gap: 8px; align-items: baseline; }
  h3 .totals { text-transform: none; letter-spacing: 0; font-weight: 400; }
  .link { color: #e8b03a; font-size: 12px; }
  .link:hover, .link.on { text-decoration: underline; }
  .link.on { font-weight: 600; }
  .hint { margin: 0; }
  .groups { display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: 8px; }
  .group { display: block; width: 100%; min-width: 0; overflow: hidden; position: relative; padding: 4px; border: 1px solid var(--line, #333); border-radius: 3px; text-align: left; }
  .group:hover { background: var(--hover, #333); }
  .group.on { border-color: #5b8def; }
  .more { align-self: flex-start; }
  .thumbs { display: flex; gap: 3px; height: 72px; width: 100%; }
  .thumbs img { flex: 1 1 0; width: 0; min-width: 0; height: 100%; object-fit: cover; border-radius: 2px; background: #222; }
  .size { position: absolute; right: 2px; top: 2px; min-width: 20px; padding: 0 6px; border-radius: 10px; background: #e5484d; color: #fff; font-size: 11px; font-weight: 600; line-height: 18px; text-align: center; }
</style>
