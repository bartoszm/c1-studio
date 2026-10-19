<script lang="ts">
  import * as api from "./lib/api";
  import { placeText } from "./lib/tree";

  // Photos placed from the photos taken just before and after them. Fill missing: photos
  // with no position go on the line between two with a position (or by the nearest one, if
  // close), or, with no place either, get the place their neighbours share. Refine: photos
  // with a place get a neighbour's that says the same and more (a location in their city),
  // and ≈ positions (their place's) the GPS of a neighbour taken moments apart. Counts update
  // as the settings change; nothing is written until Apply.
  let { filter, onclose, ondone }: {
    /** The view's filter: only its photos are placed. */
    filter: api.Filter;
    onclose: () => void;
    ondone: (undo: [number, api.Op][]) => Promise<void>;
  } = $props();

  let mode = $state<"fill" | "refine">("fill");
  let gap = $state("30");
  let positions = $state(true);
  let places = $state(true);
  let placeGap = $state("15");
  let positionGap = $state("2");
  let refinePlaces = $state(true);
  let refinePositions = $state(true);
  let plan = $state<api.NeighbourPlan | null>(null);
  let refined = $state<api.RefinePlan | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let asked = 0;

  /** Minutes typed, up to two days: further apart, photos say nothing about each other's place. */
  const minutes = (text: string) => {
    const g = text.trim() === "" || Number.isNaN(+text) ? null : Math.round(+text);
    return g !== null && g > 0 && g <= 2880 ? g : null;
  };
  let settings = $derived.by(() => {
    const g = minutes(gap);
    return g !== null ? { max_gap_minutes: g, positions, places } : null;
  });
  let refineSettings = $derived.by(() => {
    const p = minutes(placeGap), q = minutes(positionGap);
    return p !== null && q !== null
      ? { place_gap_minutes: p, position_gap_minutes: q, places: refinePlaces, positions: refinePositions }
      : null;
  });
  const shown = () => ({ ...$state.snapshot(filter), within: undefined });

  $effect(() => {
    const s = settings;
    if (mode !== "fill" || !s) return;
    const mine = ++asked;
    api.neighbourPlan(shown(), s)
      .then((p) => { if (mine === asked) { plan = p; error = null; } })
      .catch((e) => { if (mine === asked) { plan = null; error = String(e); } });
  });

  $effect(() => {
    const s = refineSettings;
    if (mode !== "refine" || !s) return;
    const mine = ++asked;
    api.refinePlan(shown(), s)
      .then((p) => { if (mine === asked) { refined = p; error = null; } })
      .catch((e) => { if (mine === asked) { refined = null; error = String(e); } });
  });

  let count = $derived(
    mode === "fill"
      ? plan ? (positions ? plan.positions : 0) + (places ? plan.places : 0) : 0
      : refined ? (refinePlaces ? refined.places : 0) + (refinePositions ? refined.positions : 0) : 0,
  );

  async function apply() {
    if (!count || busy) return;
    busy = true;
    try {
      if (mode === "fill" && settings) await ondone(await api.neighbourApply(shown(), settings));
      else if (mode === "refine" && refineSettings) await ondone(await api.refineApply(shown(), refineSettings));
      onclose();
    } catch (e) {
      error = String(e);
      busy = false;
    }
  }
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && onclose()} />

<div class="backdrop" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="neighbours-title">
    <h2 id="neighbours-title">Place photos from their neighbours</h2>
    <div class="tabs" role="tablist">
      <button role="tab" aria-selected={mode === "fill"} class:on={mode === "fill"} onclick={() => (mode = "fill")}>Fill missing</button>
      <button role="tab" aria-selected={mode === "refine"} class:on={mode === "refine"} onclick={() => (mode = "refine")}>Refine</button>
    </div>
    {#if mode === "fill"}
      <p class="muted small">
        For the photos the filter selects (the map's area aside) that have no position: the photos taken just before and after, by any camera in the catalog, say where
        they were.
      </p>
      <label class="row">
        Neighbours at most this far apart, minutes
        <input class="short" bind:value={gap} inputmode="numeric" />
      </label>
      <label class="row check">
        <input type="checkbox" bind:checked={positions} /> A position between theirs (or the nearest, within half that)
      </label>
      <label class="row check">
        <input type="checkbox" bind:checked={places} /> For photos with no place: their neighbours' place, when they agree
      </label>
    {:else}
      <p class="muted small">
        For the photos the filter selects (the map's area aside) that are placed only roughly: the photos taken just before and after, by any camera in the catalog,
        say more.
      </p>
      <div class="row">
        <label class="check"><input type="checkbox" bind:checked={refinePlaces} /> A more specific place, from a neighbour within, minutes</label>
        <input class="short" bind:value={placeGap} inputmode="numeric" aria-label="Place neighbours at most this far apart, minutes" />
      </div>
      <div class="row">
        <label class="check"><input type="checkbox" bind:checked={refinePositions} /> ≈ positions: their neighbours' GPS, within, minutes</label>
        <input class="short" bind:value={positionGap} inputmode="numeric" aria-label="GPS neighbours at most this far apart, minutes" />
      </div>
      <p class="muted small">
        A place is refined when the nearest photo before or after says the same and more (a location in its city); a position only when it lies in the photo's own place.
      </p>
    {/if}
    {#if error}<p class="error">{error}</p>{/if}
    {#if mode === "fill" && plan}
      <p class="result">
        Of {plan.candidates.toLocaleString()} photos with no position,
        {#if positions}<b>{plan.positions.toLocaleString()}</b> get a position{/if}{#if positions && places}{" "}and{/if}
        {#if places}<b>{plan.places.toLocaleString()}</b> get a place{/if}.
      </p>
      {#if places && plan.place_counts.length}
        <ul class="places">
          {#each plan.place_counts.slice(0, 8) as [path, n]}<li>{placeText(path)} <span class="muted">{n.toLocaleString()}</span></li>{/each}
          {#if plan.place_counts.length > 8}<li class="muted">and {plan.place_counts.length - 8} more</li>{/if}
        </ul>
      {/if}
      <p class="muted small">Positions given this way are queued like any edit: check them on the map, undo with Ctrl+Z.</p>
    {:else if mode === "refine" && refined}
      {#if refinePlaces}
        <p class="result">Of {refined.placed.toLocaleString()} photos with a place, <b>{refined.places.toLocaleString()}</b> get a more specific one.</p>
      {/if}
      {#if refinePositions}
        <p class="result">Of {refined.approx.toLocaleString()} with a ≈ position, <b>{refined.positions.toLocaleString()}</b> move to their neighbours' GPS.</p>
      {/if}
      {#if refinePlaces && refined.place_counts.length}
        <ul class="places">
          {#each refined.place_counts.slice(0, 8) as [from, to, n]}
            <li>{placeText(from)} → {placeText(to)} <span class="muted">{n.toLocaleString()}</span></li>
          {/each}
          {#if refined.place_counts.length > 8}<li class="muted">and {refined.place_counts.length - 8} more</li>{/if}
        </ul>
      {/if}
      <p class="muted small">Changes made this way are queued like any edit: check them on the map, undo with Ctrl+Z.</p>
    {/if}
    <div class="buttons">
      <button onclick={onclose}>Cancel</button>
      <button class="primary" disabled={!count || busy} onclick={apply}>
        {#if mode === "fill"}{busy ? "Placing…" : `Place ${count.toLocaleString()} photos`}{:else}{busy ? "Refining…" : `Refine ${count.toLocaleString()} photos`}{/if}
      </button>
    </div>
  </div>
</div>

<style>
  .backdrop { position: fixed; inset: 0; z-index: 20; display: grid; place-items: center; padding: 16px; background: rgb(0 0 0 / 55%); }
  .dialog { width: min(480px, 100%); max-height: calc(100vh - 32px); overflow: auto; padding: 18px 20px; background: var(--panel); border: 1px solid var(--line); border-radius: 6px; box-shadow: 0 12px 40px rgb(0 0 0 / 50%); }
  h2 { margin: 0 0 6px; font-size: 16px; font-weight: 600; }
  p { margin: 6px 0; }
  .muted { color: var(--muted); }
  .small { font-size: 12px; }
  .tabs { display: inline-flex; gap: 2px; margin: 4px 0 6px; }
  .tabs button { padding: 2px 10px; border: 1px solid var(--line); border-radius: 4px; }
  .tabs button:hover:not(.on) { background: var(--hover); }
  .tabs .on { background: var(--queued); border-color: var(--queued); color: #fff; }
  .row { display: flex; justify-content: space-between; align-items: center; gap: 12px; margin-top: 10px; }
  .row.check, .check { display: flex; justify-content: flex-start; align-items: center; gap: 6px; }
  .short { width: 5em; padding: 2px 5px; font: inherit; color: var(--text); background: var(--bg); border: 1px solid var(--line); border-radius: 3px; text-align: right; }
  .short:focus { outline: none; border-color: var(--queued); }
  input[type="checkbox"] { margin: 0; accent-color: var(--queued); }
  .result { margin-top: 14px; }
  .places { margin: 4px 0; padding-left: 18px; font-size: 12px; }
  .buttons { display: flex; justify-content: flex-end; gap: 8px; margin-top: 16px; }
  .buttons button { padding: 4px 12px; border: 1px solid var(--line); border-radius: 4px; }
  .buttons button:hover:not(:disabled) { background: var(--hover); }
  .buttons .primary { border: 0; background: var(--queued); color: #fff; font-weight: 600; }
  .buttons button:disabled { opacity: 0.4; cursor: default; }
</style>
