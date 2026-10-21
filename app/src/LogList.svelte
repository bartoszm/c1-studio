<script lang="ts">
  import { untrack } from "svelte";
  import * as api from "./lib/api";
  import type { HistoryPick, LogGroup, LogKind, Origin } from "./lib/api";
  import { date, originLabel } from "./lib/format";
  import { LIST_KINDS, KIND_LABELS, groupText } from "./lib/history";

  // The change log in groups, newest first: a batch of one kind from one origin is one
  // row. Limited to the days picked on the chart; "More" reads the next page. A row
  // picks the photos its entries are about.
  let { range, version, provider, albumName, picked, onpick, onerror }: {
    range: { from: number; to: number } | null;
    version: number;
    provider: string;
    albumName: (id: string) => string | undefined;
    /** The pick the grid shows now. */
    picked: HistoryPick | undefined;
    onpick: (pick: HistoryPick) => void;
    onerror: (text: string) => void;
  } = $props();

  const ORIGINS: Origin[] = ["App", "Source", "Gpx", "Restore"];
  let origins = $state<Origin[]>([]);
  let kinds = $state<LogKind[]>([]);
  let groups = $state.raw<LogGroup[]>([]);
  let names = $state.raw<Record<string, string>>({});
  let next = $state<number | null>(null);
  let loaded = $state(false);

  const query = $derived({ from: range?.from, to: range?.to, origins: [...origins], kinds: [...kinds], limit: 50 });
  let asked = 0;
  $effect(() => {
    void version;
    const q = $state.snapshot(query);
    untrack(() => read(q, null));
  });

  async function read(q: api.LogQuery, before: number | null) {
    const mine = ++asked;
    try {
      const page = await api.changeLogPage(before === null ? q : { ...q, before });
      if (mine !== asked) return;
      groups = before === null ? page.groups : [...groups, ...page.groups];
      names = before === null ? page.albums : { ...names, ...page.albums };
      next = page.next;
      loaded = true;
    } catch (e) {
      if (mine === asked) onerror(String(e));
    }
  }

  function toggle<T>(list: T[], v: T): T[] {
    return list.includes(v) ? list.filter((x) => x !== v) : [...list, v];
  }

  const album = (id: string) => (albumName(id) ? `“${albumName(id)}”` : names[id] ? `“${names[id]}”` : id);
  /** Kinds that are about photos; an album or keyword tree entry is when it kept some. */
  const pickable = (g: LogGroup) => !["written", "undone"].includes(g.kind) && (g.kind !== "collection" && g.kind !== "remap" || !!g.sample.has_before);
  const isPicked = (g: LogGroup) => !!picked && "Entries" in picked && picked.Entries.first === g.first && picked.Entries.last === g.last;
</script>

<div class="log">
  <div class="chips" role="group" aria-label="Show only">
    {#each ORIGINS as o}
      <button class="chip" class:on={origins.includes(o)} onclick={() => (origins = toggle(origins, o))}>{originLabel(o, provider)}</button>
    {/each}
    <span class="gap"></span>
    {#each LIST_KINDS as k}
      <button class="chip" class:on={kinds.includes(k)} onclick={() => (kinds = toggle(kinds, k))}>{KIND_LABELS[k]}</button>
    {/each}
  </div>
  {#if !loaded}
    <p class="hint">Reading…</p>
  {:else if !groups.length}
    <p class="hint">Nothing was logged{range || origins.length || kinds.length ? " for this" : " yet"}.</p>
  {:else}
    <ol>
      {#each groups as g (g.first)}
        <li class:on={isPicked(g)}>
          <button class="row" disabled={!pickable(g)} title={pickable(g) ? "Show the photos of this entry in the grid" : ""} onclick={() => onpick({ Entries: { first: g.first, last: g.last, kind: g.kind } })}>
            <span class="when">{date(g.at)}</span>
            <span class="origin from-{g.origin.toLowerCase()}">{originLabel(g.origin, provider)}</span>
            <span class="what">{groupText(g, album, provider)}</span>
          </button>
        </li>
      {/each}
    </ol>
    {#if next !== null}<button class="more" onclick={() => read($state.snapshot(query), next)}>More</button>{/if}
  {/if}
</div>

<style>
  .log { padding: 6px 12px 12px; font-size: 12px; }
  .chips { display: flex; flex-wrap: wrap; gap: 4px; margin-bottom: 6px; }
  .gap { width: 10px; }
  .chip { padding: 0 7px; font-size: 11px; color: var(--muted); border: 1px solid var(--line); border-radius: 9px; background: none; }
  .chip:hover { color: var(--text); }
  .chip.on { color: var(--text); background: var(--on); }
  ol { margin: 0; padding: 0; list-style: none; }
  li { border-bottom: 1px solid var(--line); }
  li.on { background: rgba(79, 163, 224, 0.12); }
  .row { display: flex; flex-wrap: wrap; align-items: baseline; gap: 2px 8px; width: 100%; padding: 4px 2px; text-align: left; color: inherit; background: none; border: 0; cursor: pointer; }
  .row:disabled { cursor: default; }
  .row:not(:disabled):hover { background: rgba(255, 255, 255, 0.04); }
  .when { color: var(--muted); font-variant-numeric: tabular-nums; }
  .origin { font-size: 10px; padding: 0 5px; border-radius: 3px; background: var(--on); color: var(--muted); }
  .origin.from-app { color: var(--queued); }
  .what { flex: 1 1 240px; min-width: 0; overflow-wrap: anywhere; }
  .more { margin-top: 8px; padding: 2px 12px; font-size: 12px; }
  .hint { color: var(--muted); }
</style>
