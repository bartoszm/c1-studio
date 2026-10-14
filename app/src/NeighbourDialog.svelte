<script lang="ts">
  import * as api from "./lib/api";
  import { placeText } from "./lib/tree";

  // Photos with no position, placed from the photos taken just before and after them: on
  // the line between two with a position (or by the nearest one, if close), or, for photos
  // with no place either, the place their neighbours share. Counts update as the gap
  // changes; nothing is written until Apply.
  let { filter, onclose, ondone }: {
    /** The view's filter: only its photos are placed. */
    filter: api.Filter;
    onclose: () => void;
    ondone: (undo: [number, api.Op][]) => Promise<void>;
  } = $props();

  let gap = $state("30");
  let positions = $state(true);
  let places = $state(true);
  let plan = $state<api.NeighbourPlan | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let asked = 0;

  let settings = $derived.by(() => {
    const g = gap.trim() === "" || Number.isNaN(+gap) ? null : Math.round(+gap);
    return g !== null && g > 0 ? { max_gap_minutes: g, positions, places } : null;
  });
  const shown = () => ({ ...$state.snapshot(filter), within: undefined });

  $effect(() => {
    const s = settings;
    if (!s) return;
    const mine = ++asked;
    api.neighbourPlan(shown(), s)
      .then((p) => { if (mine === asked) { plan = p; error = null; } })
      .catch((e) => { if (mine === asked) { plan = null; error = String(e); } });
  });

  let count = $derived(plan ? (positions ? plan.positions : 0) + (places ? plan.places : 0) : 0);

  async function apply() {
    if (!settings || !count || busy) return;
    busy = true;
    try {
      await ondone(await api.neighbourApply(shown(), settings));
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
    {#if error}<p class="error">{error}</p>{/if}
    {#if plan}
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
    {/if}
    <div class="buttons">
      <button onclick={onclose}>Cancel</button>
      <button class="primary" disabled={!count || busy} onclick={apply}>
        {busy ? "Placing…" : `Place ${count.toLocaleString()} photos`}
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
  .row { display: flex; justify-content: space-between; align-items: center; gap: 12px; margin-top: 10px; }
  .row.check { justify-content: flex-start; gap: 6px; }
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
