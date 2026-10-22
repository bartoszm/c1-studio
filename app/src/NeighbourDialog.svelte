<script lang="ts">
  import * as api from "./lib/api";
  import { placeText } from "./lib/tree";

  // Photos placed from the photos taken just before and after them. Positions: photos with
  // none, or only their place's (≈), go on the line between two with GPS (or by the nearest,
  // if close), when that lies in their place. Places: photos with none, or no location, get
  // a neighbour's that says the same and more. Counts update as the settings change;
  // nothing is written until Apply.
  let { filter, onclose, ondone }: {
    /** The view's filter: only its photos are placed. */
    filter: api.Filter;
    onclose: () => void;
    ondone: (undo: [number, api.Op][]) => Promise<void>;
  } = $props();

  let positions = $state(true);
  let positionGap = $state("30");
  let places = $state(true);
  let placeGap = $state("15");
  let plan = $state<api.NeighbourPlan | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let asked = 0;

  /** Minutes typed, up to two days: further apart, photos say nothing about each other's place. */
  const minutes = (text: string) => {
    const g = text.trim() === "" || Number.isNaN(+text) ? null : Math.round(+text);
    return g !== null && g > 0 && g <= 2880 ? g : null;
  };
  // A gap not valid (emptied, say) only matters for the part it is for.
  let settings = $derived.by((): api.NeighbourSettings | null => {
    const p = minutes(positionGap), q = minutes(placeGap);
    if ((positions && p === null) || (places && q === null) || !(positions || places)) return null;
    return { positions, position_gap_minutes: p ?? 30, places, place_gap_minutes: q ?? 15 };
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

  let count = $derived(settings && plan ? (positions ? plan.positions : 0) + (places ? plan.places : 0) : 0);

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
      For the photos the filter selects (the map's area aside): the photos taken just before and after, by any camera in the catalog, say where they were.
    </p>

    <div class="row">
      <label class="check"><input type="checkbox" bind:checked={positions} /> <b>Position</b> from their GPS, taken at most this far apart, minutes</label>
      <input class="short" bind:value={positionGap} inputmode="numeric" aria-label="GPS neighbours at most this far apart, minutes" />
    </div>
    <p class="muted small indent">
      For photos with no position, or only their place's (≈): between two neighbours with GPS, or at the nearest within half that (not between two taken from a plane). Only if it lies in the photo's own place.
    </p>
    {#if positions && plan && settings}
      <p class="result indent">Of {plan.unpositioned.toLocaleString()} photos with no position of their own, <b>{plan.positions.toLocaleString()}</b> get one.</p>
    {/if}

    <div class="row">
      <label class="check"><input type="checkbox" bind:checked={places} /> <b>Place</b> from a neighbour within, minutes</label>
      <input class="short" bind:value={placeGap} inputmode="numeric" aria-label="Place neighbours at most this far apart, minutes" />
    </div>
    <p class="muted small indent">
      For photos with no place, or no location: the nearest neighbour each side that says the same and more (Kraków → Kraków › Wawel), past those that say the same. Both sides must agree: a neighbour somewhere else gives nothing.
    </p>
    {#if places && plan && settings}
      <p class="result indent">Of {plan.unlocated.toLocaleString()} photos with no location, <b>{plan.places.toLocaleString()}</b> get a more specific place.</p>
      {#if plan.place_counts.length}
        <ul class="places indent">
          {#each plan.place_counts.slice(0, 8) as [from, to, n]}
            <li>{from ? placeText(from) : "(no place)"} → {placeText(to)} <span class="muted">{n.toLocaleString()}</span></li>
          {/each}
          {#if plan.place_counts.length > 8}<li class="muted">and {plan.place_counts.length - 8} more</li>{/if}
        </ul>
      {/if}
    {/if}

    {#if error}<p class="error">{error}</p>{/if}
    <p class="muted small">Changes made this way are queued like any edit: check them on the map, undo with Ctrl+Z.</p>
    <div class="buttons">
      <button onclick={onclose}>Cancel</button>
      <button class="primary" disabled={!count || busy} onclick={apply}>
        {busy ? "Placing…" : `Apply ${count.toLocaleString()} change${count === 1 ? "" : "s"}`}
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
  .check { display: flex; justify-content: flex-start; align-items: center; gap: 6px; }
  .short { width: 5em; padding: 2px 5px; font: inherit; color: var(--text); background: var(--bg); border: 1px solid var(--line); border-radius: 3px; text-align: right; }
  .short:focus { outline: none; border-color: var(--queued); }
  input[type="checkbox"] { margin: 0; accent-color: var(--queued); }
  .result { margin-top: 6px; }
  .indent { margin-left: 22px; }
  .places { margin: 4px 0; padding-left: 18px; font-size: 12px; }
  .buttons { display: flex; justify-content: flex-end; gap: 8px; margin-top: 16px; }
  .buttons button { padding: 4px 12px; border: 1px solid var(--line); border-radius: 4px; }
  .buttons button:hover:not(:disabled) { background: var(--hover); }
  .buttons .primary { border: 0; background: var(--queued); color: #fff; font-weight: 600; }
  .buttons button:disabled { opacity: 0.4; cursor: default; }
</style>
