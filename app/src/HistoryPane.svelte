<script lang="ts">
  import { untrack } from "svelte";
  import * as api from "./lib/api";
  import type { Compare, Filter, HistoryPick, LogKind, NetWhat } from "./lib/api";
  import { date, originLabel } from "./lib/format";
  import { KIND_LABELS, spanText } from "./lib/history";
  import GrowthChart from "./GrowthChart.svelte";
  import LogList from "./LogList.svelte";

  // The History tab: the library over time, two dates compared (every number picks those
  // photos in the grid), and the change log.
  let { filter, version, wording, albumName, picked, onpick, onerror }: {
    filter: Filter;
    version: number;
    wording: api.Wording;
    albumName: (id: string) => string | undefined;
    picked: HistoryPick | undefined;
    onpick: (pick: HistoryPick) => void;
    onerror: (text: string) => void;
  } = $props();

  let range = $state<{ from: number; to: number } | null>(null);
  let compare = $state.raw<Compare | null>(null);
  let asked = 0;
  $effect(() => {
    void version;
    const r = range;
    untrack(async () => {
      const mine = ++asked;
      if (!r) {
        compare = null;
        return;
      }
      try {
        const got = await api.historyCompare(r.from, r.to);
        if (mine === asked) compare = got;
      } catch (e) {
        if (mine === asked) onerror(String(e));
      }
    });
  });

  const n = (v: number) => v.toLocaleString();
  const path = (p: string) => p.split("|").join(" › ");
  const album = (id: string, name: string | null) => albumName(id) ?? name ?? id;
  /** A number that picks the photos it counts. */
  function changed(what: NetWhat) {
    if (range) onpick({ Changed: { from: range.from, to: range.to, what } });
  }
  const FIELDS: LogKind[] = ["rating", "color", "keywords", "iptc", "place", "geo", "albums"];
  let fields = $derived(new Map(compare?.fields ?? []));
</script>

<div class="history-pane">
  <GrowthChart {filter} {version} {range} onrange={(r) => (range = r)} {onerror} />
  <div class="scroll">
    {#if compare && range}
      <section class="compare">
        <h3>{spanText(range.from, range.to)}</h3>
        {#if compare.log_start !== null && range.from < compare.log_start}
          <p class="note">The log starts on {date(compare.log_start)}: what changed before it is not known.</p>
        {/if}
        {#if compare.logged_off}
          <p class="note">
            {n(compare.logged_off)} of these {compare.logged_off === 1 ? "entry was" : "entries were"} logged while History was
            off: what they replaced was not kept, so keyword tree changes and album deletes among them are not counted by photo.
          </p>
        {:else if compare.keywords_unkept}
          <p class="note">
            {n(compare.keywords_unkept)} keyword tree {compare.keywords_unkept === 1 ? "change" : "changes"} in these days did not
            keep the photos {compare.keywords_unkept === 1 ? "it" : "they"} rewrote (logged while History was off, or before it
            existed): the keyword numbers fall short by what {compare.keywords_unkept === 1 ? "it" : "they"} did.
          </p>
        {/if}
        {#if !compare.entries}
          <p class="hint">Nothing was logged on these days.</p>
        {:else}
          <p class="line">
            {n(compare.entries)} log {compare.entries === 1 ? "entry" : "entries"}:
            {#each Object.entries(compare.by_origin) as [o, count], i}{i ? ", " : ""}{n(count)} {originLabel(o as api.Origin, wording)}{/each}{compare.written ? `, ${n(compare.written)} written` : ""}
          </p>
          <div class="numbers">
            {#if compare.changed}
              <button class="num" class:on={picked && "Changed" in picked && picked.Changed.what === "Any"} onclick={() => changed("Any")}><b>{n(compare.changed)}</b> photos changed</button>
            {/if}
            {#if compare.added}<button class="num" onclick={() => changed("Added")}><b>{n(compare.added)}</b> added</button>{/if}
            {#if compare.removed}<span class="num plain"><b>{n(compare.removed)}</b> removed{compare.deleted ? `, ${n(compare.deleted)} deleted for good` : ""}</span>{/if}
            {#each FIELDS.filter((f) => fields.has(f)) as f}
              <button class="num" onclick={() => changed({ Field: f })}><b>{n(fields.get(f) ?? 0)}</b> {KIND_LABELS[f]}</button>
            {/each}
          </div>
          {#if compare.keywords_added.length || compare.keywords_removed.length}
            <div class="lists">
              {#if compare.keywords_added.length}
                <div>
                  <h4>Keywords added</h4>
                  {#each compare.keywords_added as [kw, count]}
                    <button class="num" onclick={() => changed({ KeywordAdded: kw })}><b>{n(count)}</b> {path(kw)}</button>
                  {/each}
                </div>
              {/if}
              {#if compare.keywords_removed.length}
                <div>
                  <h4>Keywords removed</h4>
                  {#each compare.keywords_removed as [kw, count]}
                    <button class="num" onclick={() => changed({ KeywordRemoved: kw })}><b>{n(count)}</b> {path(kw)}</button>
                  {/each}
                </div>
              {/if}
            </div>
          {/if}
          {#if compare.albums.length}
            <h4>Albums</h4>
            <div class="numbers">
              {#each compare.albums as a (a.id)}
                {#if a.photos_in}<button class="num" onclick={() => changed({ AlbumIn: a.id })}><b>+{n(a.photos_in)}</b> into “{album(a.id, a.name)}”</button>{/if}
                {#if a.photos_out}<button class="num" onclick={() => changed({ AlbumOut: a.id })}><b>−{n(a.photos_out)}</b> out of “{album(a.id, a.name)}”</button>{/if}
              {/each}
            </div>
          {/if}
          {#if compare.album_changes.length || compare.keyword_changes.length}
            <h4>Albums and keyword tree</h4>
            <ul class="changes">
              {#each [...compare.album_changes, ...compare.keyword_changes].sort((a, b) => a.at - b.at) as e}
                <li><span class="when">{date(e.at)}</span> {"collection" in e ? e.collection.label : "remap" in e ? e.remap.label : ""}</li>
              {/each}
            </ul>
          {/if}
        {/if}
      </section>
    {:else}
      <p class="hint pad">Drag across the chart to pick days, or click a bar. Each number below then shows those photos in the grid.</p>
    {/if}
    <LogList {range} {version} {wording} {albumName} {picked} {onpick} {onerror} />
  </div>
</div>

<style>
  .history-pane { flex: 1 1 auto; display: flex; flex-direction: column; min-height: 0; }
  .scroll { flex: 1 1 auto; min-height: 0; overflow: auto; }
  .compare { padding: 8px 12px 0; font-size: 12px; }
  h3 { margin: 0 0 4px; font-size: 13px; }
  h4 { margin: 10px 0 4px; font-size: 11px; font-weight: 600; color: var(--muted); text-transform: uppercase; letter-spacing: 0.04em; }
  .line, .note { margin: 2px 0 6px; color: var(--muted); }
  .numbers { display: flex; flex-wrap: wrap; gap: 4px; }
  .lists { display: flex; flex-wrap: wrap; gap: 4px 24px; }
  .lists > div { display: flex; flex-direction: column; align-items: flex-start; gap: 2px; min-width: 200px; }
  .num { padding: 1px 8px; font-size: 12px; color: var(--text); background: none; border: 1px solid var(--line); border-radius: 10px; cursor: pointer; }
  .lists .num { border-color: transparent; padding-left: 0; }
  .num:hover { background: var(--on); }
  .num.on { background: var(--on); }
  .num.plain { cursor: default; color: var(--muted); }
  .num.plain:hover { background: none; }
  b { font-variant-numeric: tabular-nums; }
  .changes { margin: 0; padding: 0; list-style: none; }
  .changes li { padding: 1px 0; }
  .when { color: var(--muted); font-variant-numeric: tabular-nums; }
  .hint { color: var(--muted); font-size: 12px; }
  .pad { margin: 8px 12px; }
</style>
