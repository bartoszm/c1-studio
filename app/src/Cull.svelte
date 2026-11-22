<script lang="ts">
  import { onDestroy, onMount, untrack } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { listen } from "@tauri-apps/api/event";
  import * as api from "./lib/api";
  import type { Filter } from "./lib/api";
  import Compare from "./Compare.svelte";
  import { REST_CHOICES, cullKey, keepers, lookedAt, restLabel, restOf, shownRows, stepGroup, stepPhoto, unseenNote, type RestChoice } from "./lib/cull";
  import { settings } from "./lib/settings.svelte";

  // Choosing the best of alike photos: the duplicate groups or series under the filter, one
  // at a time. The group's photos are compared side by side (four at a time of a big one,
  // all of them in the strip below), the one to keep is suggested, and Enter keeps the
  // active photo and deals with the rest as the settings say.
  let { groups, at, series, filter, version, blocked, busy, done, removes, message, failed, onclose, onat, onstale, onseries, onactive, onkeep, onsettings }: {
    /** Open Settings › AI, where the local model is downloaded. */
    onsettings: () => void;
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
    /** The group shown has no photos left under the filter: the groups may be named
     *  otherwise by now, and are to be listed again. `row`: a photo that was in it. */
    onstale: (row: number | undefined) => void;
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

  /** `view` is the group shown, not still the one before it while the next is read. */
  let current = $derived(view !== null && view.id === id);
  let rows = $derived(view?.photos.map((p) => p.row) ?? []);
  let byRow = $derived(new Map(view?.photos.map((p) => [p.row, p]) ?? []));
  let best = $derived(view?.order[0]);
  /** Each photo's place in the order of which to keep, from 1. */
  let rank = $derived(new Map(view?.order.map((row, i) => [row, i + 1]) ?? []));

  let asked = 0;
  /** The next group, read while this one is looked at: good until an edit (`version`). */
  const ahead = new Map<string, api.CullGroup>();
  /** Groups found empty and reported, so that one is reported once. */
  const reported = new Set<string>();
  /** Shows a group as read. `same`: the one shown already, after an edit, so the active
   *  photo and the marks stay. */
  function show(next: api.CullGroup, same: boolean) {
    const now = next.photos.map((p) => p.row);
    if (!same) marked.clear();
    for (const row of [...marked]) if (!now.includes(row)) marked.delete(row);
    if (!same || active === undefined || !now.includes(active)) active = next.order[0];
    shown = shownRows(now, active, same ? shown : []);
    view = next;
    error = null;
  }
  /** Reads the group. `same`: after an edit. The group before stays on screen until this
   *  one is there (no empty screen between two groups), but takes no pick and no edit. */
  async function read(same: boolean) {
    const mine = ++asked;
    const group = id;
    if (group === undefined) {
      view = null;
      return;
    }
    const ready = same ? undefined : ahead.get(group);
    if (ready) {
      ahead.delete(group);
      return show(ready, false);
    }
    try {
      const next = await api.cullGroup($state.snapshot(filter) as Filter, group);
      if (mine !== asked) return;
      const was = view?.id === group;
      if (!next.photos.length && !reported.has(group)) {
        reported.add(group);
        onstale(was ? active : undefined);
      }
      show(next, same && was);
    } catch (e) {
      if (mine !== asked) return;
      if (view?.id !== group) view = null;
      error = String(e);
    }
  }
  let seen = untrack(() => version);
  $effect(() => {
    void id;
    untrack(() => {
      // An edit that came with the change of group: what was read ahead is of before it.
      if (version !== seen) ahead.clear();
      seen = version;
      read(false);
    });
  });
  $effect(() => {
    const now = version;
    untrack(() => {
      if (now === seen) return;
      seen = now;
      ahead.clear();
      read(true);
    });
  });
  // While the next group is read, the photo of the one before takes no edit.
  $effect(() => onactive(current ? active : undefined));

  /** The local model is looking at this group. */
  let looking = $state(false);
  /** It is looking at every group, in the background: how far it has got. */
  let ranking = $state<api.RankState | null>(null);
  /** Why the last look could not be made (the model would not load, another run goes on):
   *  said in the bar, since the group stays on screen. */
  let lookError = $state<string | null>(null);
  /** The group it was last asked to look at by hand. */
  let lookedGroup = $state<string | undefined>(undefined);
  /** The photos of the group shown it could not look at, in words. */
  let unseen = $derived(view && current ? unseenNote(view.unseen, view.photos.length, lookedGroup === view.id) : "");
  async function rankThis() {
    const group = id;
    if (looking || !view || group === undefined) return;
    if (!view.model) return onsettings();
    looking = true;
    lookError = null;
    try {
      const next = await api.rankGroup($state.snapshot(filter) as Filter, group);
      // The answer is of the group asked about: another may be shown by now.
      ahead.delete(group);
      lookedGroup = group;
      if (id === group) {
        const same = view?.id === group;
        const follows = active === best;
        show(next, same);
        // The active photo follows the suggestion while it was on it.
        if (same && follows) setActive(next.order[0]);
      }
    } catch (e) {
      lookError = String(e);
    } finally {
      looking = false;
    }
  }
  async function rankAll() {
    if (view && !view.model) return onsettings();
    lookError = null;
    try {
      if (ranking?.running) await api.stopRanking();
      else await api.startRanking($state.snapshot(filter) as Filter, series);
    } catch (e) {
      lookError = String(e);
    }
  }
  onMount(() => {
    api.rankingState().then((s) => (ranking = s)).catch(() => {});
    const heard = [
      // The event names the group just looked at: only when that is the one shown is it
      // read again, not for every group of a long run. One read ahead is of before the look.
      listen<api.RankState>("ranking", (e) => {
        const before = ranking;
        ranking = e.payload;
        if (e.payload.group !== null) ahead.delete(e.payload.group);
        if (lookedAt(before, e.payload, id)) read(true);
      }),
      // A run stopped in the middle of the group shown has looked at a part of it.
      listen("ranking-done", () => { if (view?.unseen) read(true); }),
    ];
    return () => heard.forEach((h) => h.then((off) => off()));
  });
  // The model is let go with the screen (a run in the background keeps it until it ends).
  onDestroy(() => { api.cullDone().catch(() => {}); });
  // Back from Settings, where the model is downloaded: it may be there now.
  let covered = untrack(() => blocked);
  $effect(() => {
    const now = blocked;
    untrack(() => {
      if (covered && !now && view && !view.model) read(true);
      covered = now;
    });
  });

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
    if (busy || !view || !current || id === undefined || active === undefined) return;
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
      case "rank": if (action.all) rankAll(); else rankThis(); break;
    }
    return true;
  }

  // The next group is read, and the photos it will show side by side fetched, while this one
  // is looked at: once the group shown has stayed a moment, so that going through many with
  // a key held down reads none of them ahead.
  $effect(() => {
    const next = groups[at + 1];
    const edit = version;
    if (next === undefined || ahead.has(next)) return;
    const f = $state.snapshot(filter) as Filter;
    // Gone on from, or edited since: what comes back then is not kept (a group read before a
    // pick from it would show the photos the pick sent away).
    let left = false;
    const timer = setTimeout(() => {
      api.cullGroup(f, next).then((g) => {
        if (left || edit !== seen || !g.photos.length) return;
        ahead.clear();
        ahead.set(next, g);
        for (const row of shownRows(g.photos.map((p) => p.row), g.order[0])) new Image().src = api.largeUrl(row);
      }).catch(() => {});
    }, AHEAD_MS);
    return () => {
      left = true;
      clearTimeout(timer);
    };
  });

  let strip: HTMLElement | undefined = $state();
  $effect(() => {
    const row = active;
    void view;
    if (row === undefined) return;
    strip?.querySelector(`[data-row="${row}"]`)?.scrollIntoView({ block: "nearest", inline: "nearest" });
  });

  /** How long a group is shown before the next one is read ahead. */
  const AHEAD_MS = 150;
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
  const pct = (v: number) => Math.round(v * 100);
  const restTitle ="What Enter does with the photos of the group you do not keep. More in Settings › Culling.";
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
  {#if row === best}<span class="badge best" title={p?.reasons.join(", ")}>suggested</span>{/if}
  {#if marked.has(row)}<span class="badge keep">keep</span>{:else if p?.kept}<span class="badge keep was" title="Picked before as the one to keep">kept</span>{/if}
  {#if p && row !== best && p.reasons.length}<span class="why">{p.reasons.join(", ")}</span>{/if}
  {#if p?.aesthetic}
    {@const a = p.aesthetic}
    <span class="why" title="How the local model saw this photo, 0 to 100, above 50 the better side. It tells the frames of a group apart; it is no rating.">
      composition {pct(a.composition)} · moment {pct(a.moment)} · mood {pct(a.mood)} · technical {pct(a.technical)}
    </span>
  {/if}
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
        {#if view && current}<span class="muted">· {view.photos.length} photo{view.photos.length === 1 ? "" : "s"}</span>{/if}
        {#if id !== undefined && done.includes(id)}<span class="done">✓ picked</span>{/if}
      {:else}
        <strong>No {series ? "series" : "duplicates"} among these photos</strong>
      {/if}
      <button title="Next group, leaving this one as it is (↓)" aria-label="Next group" disabled={at >= groups.length - 1} onclick={() => go(1)}>›</button>
    </span>
    {#if view && current}<span class="judge" title={JUDGES[view.judge]}>{judged}</span>{/if}
    <span class="end">
      <span class="model">
        {#if view && !view.model}
          <button class="link" title="The local model (SigLIP 2, about 1.5 GB, run on this computer) can weigh composition, moment and mood too: get it in Settings › AI" onclick={onsettings}>Get the local model…</button>
        {:else}
          <button
            class="toggle"
            disabled={looking || !view}
            title="Have the local model look at this group's photos: composition, moment and mood count beside sharpness, where they tell the frames apart (r)"
            onclick={rankThis}
          >{looking ? "Looking…" : "Look"}</button>
          <button
            class="toggle"
            class:on={ranking?.running}
            title={ranking?.running
              ? "The local model is looking at every group, in the background: click to stop (Shift+R). What it saw so far is kept."
              : "Have the local model look at every group here, in the background, oldest first (Shift+R)"}
            onclick={rankAll}
          >{ranking?.running ? `${ranking.done.toLocaleString()} of ${ranking.total.toLocaleString()} · Stop` : "Look at all"}</button>
          {#if ranking && !ranking.running && ranking.error}<span class="failed" title={ranking.error}>stopped: {ranking.error}</span>{/if}
          {#if lookError}<span class="failed" title={lookError}>{lookError}</span>{/if}
          {#if ranking?.failed}
            <span class="unseen" title="Photos Look at all could not look at: they have no thumbnail or preview yet (it makes none), or could not be read. Their groups are still decided by sharpness; Look on a group makes what is missing. events.log in the catalog's folder says why of each.">{ranking.failed.toLocaleString()} failed</span>
          {/if}
          {#if unseen}
            <span class="unseen" title="The local model could not look at every photo of this group (no picture of it could be read): its view of the others does not count until it has seen them all. events.log in the catalog's folder says why.">{unseen}</span>
          {/if}
        {/if}
      </span>
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
      <button class="primary" disabled={busy || !view || !current || active === undefined} title="Keep the active photo and the ones marked, deal with the rest, go to the next group (Enter)" onclick={keep}>
        Keep {marked.size + (active !== undefined && !marked.has(active) ? 1 : 0) || ""}
      </button>
      <button class="close" title="Close (Esc)" aria-label="Close" onclick={onclose}>×</button>
    </span>
  </div>

  {#if view && shown.length}
    {#key view.id}
      <Compare
        rows={shown}
        active={Math.max(0, active === undefined ? 0 : shown.indexOf(active))}
        {version}
        {blocked}
        {message}
        {failed}
        embedded
        hint="Enter keep · k keep too · ← → photo · ↑ ↓ group · g suggested · r look · 0–5 - + * Delete edit · z zoom · Esc"
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
  .model { display: flex; align-items: center; gap: 4px; }
  .model .failed { color: var(--error); max-width: 220px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .model .unseen { color: var(--muted); white-space: nowrap; }
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

  /* What this screen says of a photo in the comparison's caption: its own, styled here. */
  .badge { padding: 0 6px; font-size: 11px; color: #111; border-radius: 3px; }
  .badge.best { background: var(--star); }
  .badge.keep { background: var(--queued); }
  .badge.was { opacity: 0.6; }
  .why { color: var(--muted); font-size: 12px; }
</style>
