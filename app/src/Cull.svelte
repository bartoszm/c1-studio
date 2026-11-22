<script lang="ts">
  import { untrack } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import * as api from "./lib/api";
  import type { Filter } from "./lib/api";
  import Compare from "./Compare.svelte";
  import { REST_CHOICES, cullKey, keepers, restLabel, restOf, shownRows, stepGroup, stepPhoto, type RestChoice } from "./lib/cull";
  import { settings } from "./lib/settings.svelte";

  // Choosing the best of alike photos: the duplicate groups or series under the filter, one
  // at a time. The group's photos are compared side by side (four at a time of a big one,
  // all of them in the strip below), the one to keep is suggested, and Enter keeps the
  // active photo and deals with the rest as the settings say.
  let { groups, at, series, filter, version, blocked, busy, done, removes, message, failed, onclose, onat, onseries, onactive, onkeep }: {
    /** The groups' ids, in the order they are gone through. */
    groups: string[];
    /** Which of them is shown. */
    at: number;
    /** Series (bursts), not duplicate groups. */
    series: boolean;
    /** What the grid shows: only a group's photos under it are chosen among. */
    filter: Filter;
    /** Bumped after edits, to read the group again. */
    version: number;
    /** A dialog or menu is open over this: the keys are its own. */
    blocked: boolean;
    /** A pick is being made: the next one waits. */
    busy: boolean;
    /** Groups already picked from, this time. */
    done: string[];
    /** Whether the source has a trash to move the rest to. */
    removes?: api.Removal;
    /** The footer's word (work under way, an error, a note), which this covers. */
    message?: string | null;
    failed?: boolean;
    onclose: () => void;
    onat: (at: number) => void;
    onseries: (series: boolean) => void;
    /** The photo edits land on, by row. */
    onactive: (row: number | undefined) => void;
    /** Keep `keep` of the group and deal with `rest`. */
    onkeep: (group: string, keep: number[], rest: number[]) => void;
  } = $props();

  let id = $derived<string | undefined>(groups[at]);
  let view = $state<api.CullGroup | null>(null);
  let error = $state<string | null>(null);
  /** The photo edits land on and Enter keeps, by row. */
  let active = $state<number | undefined>(undefined);
  /** Photos marked to be kept too (`k`). */
  const marked = new SvelteSet<number>();
  /** The photos side by side: all of a small group, four of a big one. */
  let shown = $state<number[]>([]);

  let rows = $derived(view?.photos.map((p) => p.row) ?? []);
  let byRow = $derived(new Map(view?.photos.map((p) => [p.row, p]) ?? []));
  let best = $derived(view?.order[0]);
  /** Each photo's place in the order of which to keep, from 1. */
  let rank = $derived(new Map(view?.order.map((row, i) => [row, i + 1]) ?? []));

  let asked = 0;
  /** Reads the group. `same`: after an edit, so the active photo and the marks stay. */
  async function read(same: boolean) {
    const mine = ++asked;
    const group = id;
    if (group === undefined) {
      view = null;
      return;
    }
    try {
      const next = await api.cullGroup($state.snapshot(filter) as Filter, group);
      if (mine !== asked) return;
      const now = next.photos.map((p) => p.row);
      if (!same) marked.clear();
      for (const row of [...marked]) if (!now.includes(row)) marked.delete(row);
      if (!same || active === undefined || !now.includes(active)) active = next.order[0];
      shown = shownRows(now, active, same ? shown : []);
      view = next;
      error = null;
    } catch (e) {
      if (mine === asked) error = String(e);
    }
  }
  $effect(() => {
    void id;
    untrack(() => {
      view = null;
      read(false);
    });
  });
  let seen = untrack(() => version);
  $effect(() => {
    const now = version;
    untrack(() => {
      if (now === seen) return;
      seen = now;
      read(true);
    });
  });
  $effect(() => onactive(active));

  function setActive(row: number | undefined) {
    if (row === undefined) return;
    active = row;
    shown = shownRows(rows, row, shown);
  }
  function toggleMark(row = active) {
    if (row === undefined) return;
    if (marked.has(row)) marked.delete(row);
    else marked.add(row);
  }
  function keep() {
    if (busy || !view || id === undefined || active === undefined) return;
    const kept = keepers(rows, active, marked);
    if (!kept.length) return;
    onkeep(id, kept, restOf(rows, kept));
  }
  const go = (delta: 1 | -1) => onat(stepGroup(at, groups.length, delta));

  /** Our keys come before the comparison's: the arrows move through the whole group, not
   *  only the photos side by side. */
  function onkey(e: KeyboardEvent): boolean {
    const action = cullKey(e, blocked);
    if (!action) return false;
    e.preventDefault();
    switch (action.kind) {
      case "keep": keep(); break;
      case "mark": toggleMark(); break;
      case "group": go(action.delta); break;
      case "photo": setActive(stepPhoto(rows, active, action.delta)); break;
      case "best": setActive(best); break;
    }
    return true;
  }

  // The next group's photos are fetched while this one is looked at.
  $effect(() => {
    const next = groups[at + 1];
    if (next === undefined) return;
    const f = $state.snapshot(filter) as Filter;
    api.cullGroup(f, next).then((g) => { for (const p of g.photos.slice(0, 4)) new Image().src = api.largeUrl(p.row); }).catch(() => {});
  });

  let strip: HTMLElement | undefined = $state();
  $effect(() => {
    const row = active;
    void view;
    if (row === undefined) return;
    strip?.querySelector(`[data-row="${row}"]`)?.scrollIntoView({ block: "nearest", inline: "nearest" });
  });

  const kind = $derived(series ? "series" : "duplicate group");
  const judged = $derived.by(() => {
    if (!view || best === undefined) return "";
    const p = byRow.get(best);
    if (!p) return "";
    const why = p.reasons.length ? `: ${p.reasons.join(", ")}` : "";
    return view.close ? `Too close to call; ${p.name} by a hair${why}` : `Suggested: ${p.name}${why}`;
  });
  const JUDGES: Record<api.Judge, string> = {
    cloud: "A model compared these photos",
    local: "By the local model's view of each photo, with sharpness and your own marks",
    sharpness: "By sharpness, clipped tones and your own marks (rating, edits)",
  };
  const restTitle = "What Enter does with the photos of the group you do not keep. More in Settings › Culling.";
</script>

<!-- With nothing to compare, the comparison is not there to pass the keys on: they still
     close this and move to another group. -->
<svelte:window
  onkeydown={(e) => {
    if (view && shown.length) return;
    if (!onkey(e) && e.key === "Escape" && !blocked && !e.defaultPrevented) {
      e.preventDefault();
      onclose();
    }
  }}
/>

{#snippet badge(row: number)}
  {@const p = byRow.get(row)}
  {#if row === best}<span class="best" title={p?.reasons.join(", ")}>suggested</span>{/if}
  {#if marked.has(row)}<span class="keep">keep</span>{:else if p?.kept}<span class="keep was" title="Picked before as the one to keep">kept</span>{/if}
  {#if p && row !== best && p.reasons.length}<span class="why">{p.reasons.join(", ")}</span>{/if}
{/snippet}

<div class="cull">
  <div class="bar">
    <div class="kinds" role="group" aria-label="Kind of group">
      <button class="toggle" class:on={series} aria-pressed={series} title="Bursts: photos from one camera, seconds apart, looking alike" onclick={() => onseries(true)}>Series</button>
      <button class="toggle" class:on={!series} aria-pressed={!series} title="The same frame twice: different images taken at the same instant" onclick={() => onseries(false)}>Duplicates</button>
    </div>
    <span class="where">
      <button title="Previous group (↑)" aria-label="Previous group" disabled={at <= 0} onclick={() => go(-1)}>‹</button>
      {#if groups.length}
        <strong>{kind} {(at + 1).toLocaleString()} of {groups.length.toLocaleString()}</strong>
        {#if view}<span class="muted">· {view.photos.length} photo{view.photos.length === 1 ? "" : "s"}</span>{/if}
        {#if id !== undefined && done.includes(id)}<span class="done">✓ picked</span>{/if}
      {:else}
        <strong>No {series ? "series" : "duplicates"} among these photos</strong>
      {/if}
      <button title="Next group, leaving this one as it is (↓)" aria-label="Next group" disabled={at >= groups.length - 1} onclick={() => go(1)}>›</button>
    </span>
    {#if view}<span class="judge" title={JUDGES[view.judge]}>{judged}</span>{/if}
    <span class="end">
      <label class="rest" title={restTitle}>
        The rest:
        <select
          value={settings.cull.rest}
          onchange={(e) => { settings.cull.rest = e.currentTarget.value as RestChoice; e.currentTarget.blur(); }}
        >
          {#each REST_CHOICES as choice (choice)}
            <option value={choice}>{restLabel(choice, settings.cull, removes)}</option>
          {/each}
        </select>
      </label>
      <button class="primary" disabled={busy || !view || active === undefined} title="Keep the active photo and the ones marked, deal with the rest, go to the next group (Enter)" onclick={keep}>
        Keep {marked.size + (active !== undefined && !marked.has(active) ? 1 : 0) || ""}
      </button>
      <button class="close" title="Close (Esc)" aria-label="Close" onclick={onclose}>×</button>
    </span>
  </div>

  {#if view && shown.length}
    {#key id}
      <Compare
        rows={shown}
        active={Math.max(0, active === undefined ? 0 : shown.indexOf(active))}
        {version}
        {blocked}
        {message}
        {failed}
        embedded
        hint="Enter keep · k keep too · ← → photo · ↑ ↓ group · g suggested · 0–5 - + * Delete edit · z zoom · Esc"
        onkeyfirst={onkey}
        {badge}
        {onclose}
        onactive={(i) => setActive(shown[i])}
        onrows={(next, i) => { shown = next; active = next[i]; }}
      />
    {/key}
  {:else}
    <p class="empty" class:failed={!!error}>
      {#if error}{error}
      {:else if !groups.length}Nothing to choose from here. Try the other kind, or a wider filter.
      {:else if view}None of this group's photos are left under the filter.
      {:else}reading the group…{/if}
    </p>
  {/if}

  {#if view && view.photos.length}
    <div class="strip" bind:this={strip}>
      {#each view.photos as p (p.row)}
        <button
          class="frame"
          class:active={p.row === active}
          class:shown={shown.includes(p.row)}
          data-row={p.row}
          title="{p.name}{p.reasons.length ? ` · ${p.reasons.join(', ')}` : ''}"
          onclick={(e) => { setActive(p.row); e.currentTarget.blur(); }}
          ondblclick={() => toggleMark(p.row)}
        >
          <img src={api.thumbUrl(p.row)} alt="" loading="lazy" decoding="async" draggable="false" />
          <span class="rank" class:best={p.row === best}>{rank.get(p.row) ?? ""}</span>
          {#if marked.has(p.row) || p.kept}<span class="tick" class:was={!marked.has(p.row)}>✓</span>{/if}
          <span class="bar-score" style="width: {Math.round(p.score * 100)}%"></span>
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .cull {
    position: fixed;
    inset: 0;
    z-index: 10;
    display: grid;
    grid-template-rows: auto minmax(0, 1fr) auto;
    background: #111;
  }
  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 14px;
    padding: 6px 12px;
    background: var(--panel);
    border-bottom: 1px solid var(--line);
  }
  .kinds { display: flex; gap: 2px; }
  .where { display: flex; align-items: center; gap: 8px; }
  .where button { padding: 0 8px; font-size: 18px; line-height: 1.2; }
  .where button:disabled { opacity: 0.35; cursor: default; }
  .muted { color: var(--muted); }
  .done { color: var(--queued); }
  .judge { color: var(--muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; flex: 1 1 200px; }
  .end { display: flex; align-items: center; gap: 10px; margin-left: auto; }
  .rest { display: flex; align-items: center; gap: 6px; color: var(--muted); }
  .close { padding: 0 6px; font-size: 20px; line-height: 1; color: var(--muted); }
  .close:hover { color: var(--text); }
  .empty { align-self: center; justify-self: center; color: var(--muted); }
  .empty.failed { color: var(--error); }

  .strip {
    display: flex;
    gap: 4px;
    padding: 6px 8px;
    overflow-x: auto;
    background: var(--panel);
    border-top: 1px solid var(--line);
  }
  .frame {
    position: relative;
    flex: 0 0 auto;
    width: 96px;
    height: 68px;
    padding: 0;
    overflow: hidden;
    background: #111;
    border: 2px solid transparent;
    border-radius: 3px;
    opacity: 0.6;
  }
  .frame.shown { opacity: 1; }
  .frame.active { border-color: var(--queued); }
  .frame img { width: 100%; height: 100%; object-fit: cover; }
  .rank {
    position: absolute;
    top: 2px;
    left: 2px;
    min-width: 16px;
    padding: 0 4px;
    font-size: 11px;
    color: #fff;
    background: rgb(0 0 0 / 65%);
    border-radius: 3px;
  }
  .rank.best { color: #111; background: var(--star); }
  .tick {
    position: absolute;
    top: 2px;
    right: 2px;
    padding: 0 4px;
    font-size: 11px;
    color: #111;
    background: var(--queued);
    border-radius: 3px;
  }
  .tick.was { opacity: 0.6; }
  .bar-score { position: absolute; left: 0; bottom: 0; height: 3px; background: var(--star); }

  .cull :global(.caption .best) { padding: 0 6px; font-size: 11px; color: #111; background: var(--star); border-radius: 3px; }
  .cull :global(.caption .keep) { padding: 0 6px; font-size: 11px; color: #111; background: var(--queued); border-radius: 3px; }
  .cull :global(.caption .keep.was) { opacity: 0.6; }
  .cull :global(.caption .why) { color: var(--muted); font-size: 12px; }
</style>
