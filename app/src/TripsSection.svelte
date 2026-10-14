<script lang="ts">
  import { SvelteSet } from "svelte/reactivity";
  import * as api from "./lib/api";
  import type { Trip } from "./lib/api";
  import { tripName, tripDays, tripDates } from "./lib/trips";

  // The Trips section of the sidebar: trips found from when and where photos were taken,
  // newest first by year. A click filters the grid to one; the menu renames, keeps,
  // merges, splits, dismisses or exports it.
  let { list, selected, focusRow, onpick, onchanged, onexport, onerror }: {
    /** The trips under the current filter (App loads them: the timeline shows them too). */
    list: api.TripList | null;
    /** The trip the grid shows. */
    selected: number | null;
    /** The focused photo, to split a trip before it. */
    focusRow: number | undefined;
    onpick: (trip: Trip | null) => void;
    /** Trips or their settings changed: load them again. */
    onchanged: () => Promise<api.TripList | null>;
    onexport: (trip: Trip) => void;
    onerror: (text: string) => void;
  } = $props();

  let showSuggested = $state(true);
  let showDismissed = $state(false);
  let closedYears = new SvelteSet<string>();
  let menu = $state<{ x: number; y: number; trip: Trip; splitAt: number | null } | null>(null);
  let renaming = $state<{ id: number; text: string } | null>(null);
  let settingHome = $state<{ country: string; city: string; gap: number } | null>(null);

  let shown = $derived(
    (list?.trips ?? []).filter(
      (t) => (t.kind !== "suggested" || showSuggested) && (t.kind !== "dismissed" || showDismissed),
    ),
  );
  let suggested = $derived(list?.trips.filter((t) => t.kind === "suggested").length ?? 0);
  let dismissed = $derived(list?.trips.filter((t) => t.kind === "dismissed").length ?? 0);
  /** Newest year first, newest trip first within it. */
  let years = $derived.by(() => {
    const by = new Map<string, Trip[]>();
    for (const t of [...shown].reverse()) {
      const y = String(new Date(t.start).getFullYear());
      by.set(y, [...(by.get(y) ?? []), t]);
    }
    return [...by];
  });

  async function edit(change: api.TripEdit) {
    menu = null;
    const shown = selected;
    const touched = change.op === "merge" ? change.ids : change.op === "create" ? [] : [change.id];
    try {
      const keep = await api.editTrips(change);
      const now = await onchanged();
      // Only an edit to the trip the grid shows moves the grid. A renamed, kept or split
      // one keeps its id (App follows its new span); a merged one is a new trip; a
      // dismissed or forgotten one is shown no more.
      if (shown === null || !touched.includes(shown)) return;
      if (change.op === "dismiss" || change.op === "forget") onpick(null);
      else if (change.op === "merge") onpick(now?.trips.find((t) => t.id === keep) ?? null);
    } catch (e) {
      onerror(String(e));
    }
  }

  /** The trips around `t` (dismissed ones aside), for merging with a neighbour. */
  function neighbours(t: Trip): [Trip | undefined, Trip | undefined] {
    const live = (list?.trips ?? []).filter((x) => x.kind !== "dismissed");
    const i = live.findIndex((x) => x.id === t.id);
    return [live[i - 1], live[i + 1]];
  }

  async function openMenu(t: Trip, e: MouseEvent) {
    e.preventDefault();
    menu = { x: e.clientX, y: e.clientY, trip: t, splitAt: null };
    // Splitting goes before the focused photo, when it is one of this trip's, past its first.
    if (focusRow === undefined) return;
    const photo = await api.photo(focusRow).catch(() => null);
    const at = photo?.captured_at;
    if (menu?.trip.id === t.id && at != null && at > t.start && at <= t.end) menu = { ...menu, splitAt: at };
  }

  function rename(t: Trip) {
    menu = null;
    renaming = { id: t.id, text: t.name ?? t.label };
  }
  function commitRename() {
    const r = renaming;
    renaming = null;
    if (r) edit({ op: "rename", id: r.id, name: r.text.trim() || null });
  }

  function openHome() {
    const home = list?.settings.home ?? list?.home_guess;
    settingHome = { country: home?.country ?? "", city: home?.city ?? "", gap: list?.settings.gap_hours ?? 36 };
  }
  async function saveHome(home: api.Home | null, gap?: number) {
    if (!list) return;
    settingHome = null;
    try {
      await api.setTripSettings({ ...list.settings, home, gap_hours: gap ?? list.settings.gap_hours });
      await onchanged();
    } catch (e) {
      onerror(String(e));
    }
  }
  function submitHome(e: Event) {
    e.preventDefault();
    const h = settingHome;
    if (!h || !h.country.trim()) return;
    saveHome({ country: h.country.trim(), city: h.city.trim() || null }, Math.max(1, h.gap || 36));
  }
  const homeText = (h: api.Home) => [h.city, h.country].filter(Boolean).join(", ");
  const autofocus = (el: HTMLInputElement) => { el.focus(); el.select(); };
</script>

{#if list && !list.settings.home && !settingHome}
  <div class="intro">
    <p>Trips are found from when and where photos were taken. Photos taken at home are never part of one.</p>
    {#if list.home_guess}
      <button class="primary" onclick={() => saveHome(list!.home_guess)}>Home is {homeText(list.home_guess)}</button>
    {/if}
    <button onclick={openHome}>{list.home_guess ? "Somewhere else…" : "Set home…"}</button>
  </div>
{/if}

{#if settingHome}
  <form class="home" onsubmit={submitHome}>
    <label>Country <input bind:value={settingHome.country} placeholder="Poland" use:autofocus /></label>
    <label>City <input bind:value={settingHome.city} placeholder="empty: the whole country" /></label>
    <label title="A longer gap without photos starts a new trip (a few days are allowed inside one country)">
      New trip after <input type="number" min="1" bind:value={settingHome.gap} class="gap" /> h
    </label>
    <div class="actions">
      <button type="button" onclick={() => (settingHome = null)}>Cancel</button>
      <button type="submit" class="primary" disabled={!settingHome.country.trim()}>Save</button>
    </div>
  </form>
{/if}

{#if list?.settings.home}
  {#each years as [year, trips] (year)}
    <button class="year" onclick={() => (closedYears.has(year) ? closedYears.delete(year) : closedYears.add(year))}>
      <span class="chev" class:open={!closedYears.has(year)}>›</span>{year}
      <span class="n">{trips.length}</span>
    </button>
    {#if !closedYears.has(year)}
      {#each trips as t (t.id)}
        {#if renaming?.id === t.id}
          <form class="rename" onsubmit={(e) => { e.preventDefault(); commitRename(); }}>
            <input
              bind:value={renaming.text}
              use:autofocus
              onkeydown={(e) => { if (e.key === "Escape") renaming = null; }}
              onblur={commitRename}
            />
          </form>
        {:else}
          <div
            class="trip {t.kind}"
            class:on={selected === t.id}
            class:empty={t.count === 0}
            role="treeitem"
            aria-selected={selected === t.id}
            tabindex="-1"
            oncontextmenu={(e) => openMenu(t, e)}
          >
            <button
              class="label"
              title="{tripName(t)} · {tripDates(t, true)} · {tripDays(t)} days · {t.photos.toLocaleString()} photos{t.kind === 'suggested' ? '\nSuggested: busy days with no place. Keep it as a trip (✓) or dismiss it (×); right-click for more.' : ''}"
              onclick={() => onpick(selected === t.id ? null : t)}
              ondblclick={() => rename(t)}
            >
              {#if t.cover !== null}<img src={api.thumbUrl(t.cover)} alt="" loading="lazy" />{:else}<i class="nocover"></i>{/if}
              <span class="text">
                <span class="name">{tripName(t)}</span>
                <span class="dates">{tripDates(t)}</span>
              </span>
              <span class="n">{t.count.toLocaleString()}</span>
            </button>
            <span class="quick">
              {#if t.kind === "suggested"}
                <button title="Keep as a trip" onclick={() => edit({ op: "keep", id: t.id })}>✓</button>
                <button title="Not a trip" onclick={() => edit({ op: "dismiss", id: t.id })}>×</button>
              {:else if t.kind === "dismissed"}
                <button title="Find it again" onclick={() => edit({ op: "forget", id: t.id })}>↺</button>
              {/if}
            </span>
          </div>
        {/if}
      {/each}
    {/if}
  {/each}
  {#if !shown.length}<p class="none">No trips{list.trips.length ? " shown" : " found yet"}.</p>{/if}
  <div class="foot">
    <button title="Home and when a new trip starts" onclick={openHome}>Home: {homeText(list.settings.home)}</button>
    {#if suggested}
      <button class:on={showSuggested} onclick={() => (showSuggested = !showSuggested)}>{suggested} suggested</button>
    {/if}
    {#if dismissed}
      <button class:on={showDismissed} onclick={() => (showDismissed = !showDismissed)}>{dismissed} dismissed</button>
    {/if}
  </div>
{/if}

{#if menu}
  {@const t = menu.trip}
  {@const [before, after] = neighbours(t)}
  <div class="menu-shield" role="presentation" onclick={() => (menu = null)} oncontextmenu={(e) => { e.preventDefault(); menu = null; }}></div>
  <div class="menu" role="menu" style:left="{menu.x}px" style:top="{menu.y}px">
    <div class="menu-title">{tripName(t)} · {tripDates(t, true)}</div>
    <button role="menuitem" onclick={() => rename(t)}>Rename…</button>
    {#if t.kind === "found" || t.kind === "suggested"}
      <button role="menuitem" onclick={() => edit({ op: "keep", id: t.id })}>Keep as a trip</button>
    {/if}
    {#if t.kind !== "dismissed"}
      {#if before}
        <button role="menuitem" onclick={() => edit({ op: "merge", ids: [before.id, t.id] })}>Merge with “{tripName(before)}” before it</button>
      {/if}
      {#if after}
        <button role="menuitem" onclick={() => edit({ op: "merge", ids: [t.id, after.id] })}>Merge with “{tripName(after)}” after it</button>
      {/if}
      {#if menu.splitAt !== null}
        {@const at = menu.splitAt}
        <button role="menuitem" onclick={() => edit({ op: "split", id: t.id, at })}>Split before the focused photo</button>
      {:else}
        <div class="menu-note">To split it, focus the first photo of the second part.</div>
      {/if}
      <button role="menuitem" onclick={() => { const trip = t; menu = null; onexport(trip); }}>Make an album of it</button>
      <button role="menuitem" class="danger" onclick={() => edit({ op: "dismiss", id: t.id })}>Not a trip</button>
    {/if}
    {#if t.kind === "kept" || t.kind === "dismissed"}
      <button role="menuitem" title="Forget the name and changes: it is found again as detection sees it" onclick={() => edit({ op: "forget", id: t.id })}>Forget my changes</button>
    {/if}
  </div>
{/if}

<style>
  .intro, .home { display: grid; gap: 6px; padding: 4px 8px 6px 26px; }
  .intro p { margin: 0; font-size: 12px; color: var(--muted); }
  .intro button, .home button, .foot button {
    padding: 2px 8px;
    border: 1px solid var(--line);
    border-radius: 3px;
    color: var(--muted);
    text-align: left;
  }
  .intro button:hover, .home button:hover, .foot button:hover { color: var(--text); background: var(--hover); }
  .intro .primary, .home .primary { color: var(--text); background: var(--on); }
  .home label { display: flex; align-items: center; justify-content: space-between; gap: 8px; color: var(--muted); }
  .home input, .rename input {
    width: 140px;
    padding: 2px 6px;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 3px;
    font: inherit;
  }
  .home input:focus, .rename input:focus { outline: none; border-color: var(--queued); }
  .home .gap { width: 56px; }
  .home .actions { display: flex; justify-content: flex-end; gap: 6px; }
  .year {
    display: flex;
    align-items: center;
    width: 100%;
    padding: 2px 8px 2px 6px;
    color: var(--muted);
  }
  .year:hover { color: var(--text); background: var(--hover); }
  .year .n { margin-left: auto; font-variant-numeric: tabular-nums; }
  .trip { display: flex; align-items: center; padding-right: 4px; }
  .trip:hover { background: var(--hover); }
  .trip.on { background: var(--on); }
  .trip .label { display: flex; flex: 1; align-items: center; gap: 6px; min-width: 0; padding: 2px 4px 2px 22px; text-align: left; }
  .trip img, .trip .nocover { flex: none; width: 26px; height: 26px; object-fit: cover; border-radius: 2px; background: var(--line); }
  .trip .text { display: grid; flex: 1; min-width: 0; line-height: 1.2; }
  .trip .name, .trip .dates { overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
  .trip .dates { font-size: 11px; color: var(--muted); }
  .trip .n { color: var(--muted); font-variant-numeric: tabular-nums; }
  .trip.suggested .name { font-style: italic; color: var(--muted); }
  .trip.suggested img { opacity: 0.6; }
  .trip.dismissed .name { text-decoration: line-through; color: #666; }
  .trip.empty .name, .trip.empty .n { color: #666; }
  /* The same room on every row, so the counts line up; the buttons show on hover. */
  .quick { display: flex; flex: none; justify-content: flex-end; width: 40px; visibility: hidden; }
  .trip:hover .quick { visibility: visible; }
  .quick button { width: 20px; height: 20px; border-radius: 3px; color: var(--muted); }
  .quick button:hover { color: var(--text); background: var(--on); }
  .rename { padding: 2px 8px 2px 54px; }
  .rename input { width: 100%; }
  .none { margin: 2px 8px 2px 26px; font-size: 12px; color: #777; }
  .foot { display: flex; flex-wrap: wrap; gap: 4px; padding: 6px 8px 2px 26px; }
  .foot button { font-size: 11px; }
  .foot button.on { color: var(--text); background: var(--on); }
</style>
