<script lang="ts">
  import * as api from "./lib/api";
  import type { AnalysisState, Dim, Filter, Group } from "./lib/api";
  import { HUES } from "./lib/format";

  type Key = "shape" | "hues" | "light" | "focus";
  let {
    filter, facets, total, analysis, version, ontoggle, onflag, ongroup, onshow,
  }: {
    filter: Filter;
    /** Counts over the photos the filter selects. */
    facets: Partial<Record<Dim, [string, number][]>>;
    total: number;
    analysis: AnalysisState | null;
    /** Bumped when photos were edited or measured again. */
    version: number;
    ontoggle: (key: Key, value: string) => void;
    onflag: (flag: "duplicates" | "series" | "analysed") => void;
    /** Show one group's photos in the grid. */
    ongroup: (id: string) => void;
    onshow: () => void;
  } = $props();

  /** Each chart: dimension, title, filter field, value order (empty: by count). */
  const CHARTS: [Dim, string, Key, string[]][] = [
    ["shape", "Shape", "shape", ["landscape", "portrait", "square", "panorama"]],
    ["hue", "Main colour", "hues", []],
    ["light", "Light", "light", ["dark", "medium", "bright", "clipped highlights", "blocked shadows", "low contrast"]],
    ["focus", "Focus", "focus", ["sharp", "soft", "blurry"]],
  ];

  function values(dim: Dim, order: string[]): [string, number][] {
    const all = facets[dim] ?? [];
    if (!order.length) return all;
    const rank = (v: string) => (order.includes(v) ? order.indexOf(v) : order.length);
    return [...all].sort((a, b) => rank(a[0]) - rank(b[0]));
  }
  const flag = (name: string) => facets.flag?.find(([v]) => v === name)?.[1] ?? 0;
  let measured = $derived(flag("analysed"));

  // The groups of alike photos under the current filter, fetched when it changes.
  let dups = $state<Group[]>([]);
  let series = $state<Group[]>([]);
  let loadedFor = "";
  $effect(() => {
    // The area, picks of the groups themselves and the alike flags don't change which groups exist
    // enough to refetch for; everything else does.
    const asked = $state.snapshot(filter);
    const key = JSON.stringify({ ...asked, group: undefined }) + ":" + version + ":" + measured;
    if (key === loadedFor) return;
    loadedFor = key;
    Promise.all([api.groups(asked, false), api.groups(asked, true)])
      .then(([d, s]) => { if (key === loadedFor) { dups = d; series = s; } })
      .catch(() => {});
  });
</script>

<div class="analysis">
  <header class="bar">
    <div class="progress">
      {#if analysis?.running && analysis.total}
        Measuring photos: {analysis.done.toLocaleString()} / {analysis.total.toLocaleString()}
        <progress max={analysis.total} value={analysis.done}></progress>
      {:else}
        {measured.toLocaleString()} of {total.toLocaleString()} photos measured
      {/if}
      {#if total > measured}
        <button class="link" class:on={filter.analysed === false} onclick={() => onflag("analysed")}>
          {(total - measured).toLocaleString()} not measured yet
        </button>
      {/if}
    </div>
    <button class="show" onclick={onshow}>Show {total.toLocaleString()} photos</button>
  </header>

  <div class="charts">
    {#each CHARTS as [dim, title, key, order] (dim)}
      {@const list = values(dim, order)}
      {@const max = Math.max(1, ...list.map(([, n]) => n))}
      <section class="card">
        <h3>{title}</h3>
        {#each list as [value, n] (value)}
          <button class="bar-row" class:on={filter[key]?.includes(value)} onclick={() => ontoggle(key, value)}>
            <span class="name">
              {#if dim === "hue"}<i class="dot" style:background={HUES[value] ?? "#888"}></i>{/if}{value}
            </span>
            <span class="track"><span class="fill" style:width="{(100 * n) / max}%" style:background={dim === "hue" ? (HUES[value] ?? "#888") : null}></span></span>
            <span class="n">{n.toLocaleString()}</span>
          </button>
        {:else}
          <p class="none">Nothing measured here yet.</p>
        {/each}
      </section>
    {/each}
  </div>

  {#each [
    ["Duplicates", "The same frame twice: different images taken at the same instant that look alike.", dups, "duplicates", flag("duplicate")],
    ["Series", "Bursts: three or more photos from one camera, seconds apart, looking alike.", series, "series", flag("series")],
  ] as const as [title, hint, groups, field, count] (title)}
    <section class="alike">
      <h3>
        {title}
        <button class="link" class:on={filter[field]} title="Only these photos in the grid" onclick={() => onflag(field)}>
          {count.toLocaleString()} photos in {groups.length >= 300 ? "300+" : groups.length} groups
        </button>
      </h3>
      <p class="hint">{hint}</p>
      <div class="groups">
        {#each groups as g (g.id)}
          <button class="group" title="Show these {g.size} photos in the grid" onclick={() => ongroup(g.id)}>
            <span class="thumbs">
              {#each g.rows as row (row)}<img src={api.thumbUrl(row)} alt="" />{/each}
            </span>
            <span class="size">{g.size.toLocaleString()}</span>
          </button>
        {:else}
          <p class="none">None{filter.group || total === 0 ? "" : " found"}.</p>
        {/each}
      </div>
    </section>
  {/each}
</div>

<style>
  .analysis { flex: 1; min-height: 0; overflow-y: auto; padding: 12px 16px 24px; }
  .bar { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 12px; }
  .progress { display: flex; align-items: center; gap: 10px; color: var(--muted, #999); }
  progress { width: 160px; }
  .link { color: #e8b03a; font-size: 12px; }
  .link:hover, .link.on { text-decoration: underline; }
  .link.on { font-weight: 600; }
  .show { padding: 3px 12px; border: 1px solid var(--line, #444); border-radius: 3px; }
  .show:hover { background: var(--hover, #333); }
  .charts { display: grid; grid-template-columns: repeat(auto-fill, minmax(240px, 1fr)); gap: 12px; }
  .card, .alike { background: rgba(255, 255, 255, 0.03); border: 1px solid var(--line, #333); border-radius: 4px; padding: 8px 10px; }
  h3 { margin: 0 0 6px; font-size: 12px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em; color: #999; display: flex; justify-content: space-between; gap: 8px; align-items: baseline; }
  h3 .link { text-transform: none; letter-spacing: 0; font-weight: 400; }
  .bar-row { display: grid; grid-template-columns: 110px 1fr 48px; gap: 8px; align-items: center; width: 100%; padding: 2px 4px; border-radius: 3px; text-align: left; }
  .bar-row:hover { background: var(--hover, #333); }
  .bar-row.on { background: var(--on, #2a3a55); }
  .name { overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
  .dot { display: inline-block; width: 9px; height: 9px; margin-right: 6px; border-radius: 50%; border: 1px solid #555; }
  .track { height: 8px; background: rgba(255, 255, 255, 0.07); border-radius: 4px; overflow: hidden; }
  .fill { display: block; height: 100%; background: #5b8def; border-radius: 4px; }
  .n { text-align: right; color: var(--muted, #999); font-variant-numeric: tabular-nums; }
  .alike { margin-top: 12px; }
  .hint, .none { margin: 0 0 8px; font-size: 12px; color: #888; }
  .groups { display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: 8px; }
  .group { display: block; width: 100%; min-width: 0; overflow: hidden; position: relative; padding: 4px; border: 1px solid var(--line, #333); border-radius: 3px; text-align: left; }
  .group:hover { background: var(--hover, #333); }
  .thumbs { display: flex; gap: 3px; height: 72px; width: 100%; }
  .thumbs img { flex: 1 1 0; width: 0; min-width: 0; height: 100%; object-fit: cover; border-radius: 2px; background: #222; }
  .size { position: absolute; right: 2px; top: 2px; min-width: 20px; padding: 0 6px; border-radius: 10px; background: #e5484d; color: #fff; font-size: 11px; font-weight: 600; line-height: 18px; text-align: center; }
</style>
