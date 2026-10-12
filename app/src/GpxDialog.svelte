<script lang="ts">
  import { onMount } from "svelte";
  import * as api from "./lib/api";

  // A recorded track places photos by when they were taken. Capture One keeps the time on
  // the camera's clock, so the clock's distance from UTC is asked for; the counts update
  // as it changes, and nothing is written until Apply.
  let { filter, onclose, ondone }: {
    /** The view's filter: only its photos are placed. */
    filter: api.Filter;
    onclose: () => void;
    ondone: (undo: [number, api.Op][]) => Promise<void>;
  } = $props();

  let path = $state<string | null>(null);
  /** Hours east of UTC, as typed; the computer's zone to begin with. */
  let hours = $state(String(-new Date().getTimezoneOffset() / 60));
  let gap = $state("15");
  let overwrite = $state(false);
  let plan = $state<api.GpxPlan | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let asked = 0;

  onMount(async () => {
    try {
      path = await api.pickGpx();
    } catch (e) {
      error = String(e);
    }
    if (!path) onclose();
  });

  const num = (text: string) => (text.trim() === "" || Number.isNaN(+text) ? null : +text);
  let settings = $derived.by(() => {
    const h = num(hours);
    const g = num(gap);
    return path && h !== null && g !== null && g > 0
      ? { path, offset_minutes: Math.round(h * 60), max_gap_minutes: Math.round(g), overwrite }
      : null;
  });

  $effect(() => {
    const s = settings;
    if (!s) return;
    const mine = ++asked;
    api.gpxPlan($state.snapshot(filter), s)
      .then((p) => { if (mine === asked) { plan = p; error = null; } })
      .catch((e) => { if (mine === asked) { plan = null; error = String(e); } });
  });

  const day = (ms: number) => new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short", timeZone: "UTC" });

  async function apply() {
    if (!settings || !plan?.matched || busy) return;
    busy = true;
    try {
      await ondone(await api.gpxApply($state.snapshot(filter), settings));
      onclose();
    } catch (e) {
      error = String(e);
      busy = false;
    }
  }
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && onclose()} />

{#if path}
  <div class="backdrop" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose()}>
    <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="gpx-title">
      <h2 id="gpx-title">Place photos from a GPX track</h2>
      <p class="muted file" title={path}>{path}</p>
      {#if plan}
        <p>
          {plan.fixes.toLocaleString()} points, {day(plan.from)} to {day(plan.to)} UTC.
        </p>
      {/if}
      <label class="row">
        Camera clock, hours east of UTC
        <input class="short" bind:value={hours} inputmode="decimal" />
      </label>
      <p class="muted small">
        Poland is 1 in winter, 2 in summer. If the camera was never set to local time, use what it was set to.
      </p>
      <label class="row">
        Longest gap to bridge, minutes
        <input class="short" bind:value={gap} inputmode="numeric" />
      </label>
      <label class="row check">
        <input type="checkbox" bind:checked={overwrite} /> Also move photos that already have a position
      </label>
      {#if error}<p class="error">{error}</p>{/if}
      {#if plan}
        <p class="result">
          {#if plan.matched}
            <b>{plan.matched.toLocaleString()}</b> of {plan.candidates.toLocaleString()} photos in view taken during the track get a position.
          {:else}
            No photos in view fall on the track{plan.candidates ? "" : " (none in view were taken during it)"}. Check the clock's offset.
          {/if}
        </p>
      {/if}
      <div class="buttons">
        <button onclick={onclose}>Cancel</button>
        <button class="primary" disabled={!plan?.matched || busy} onclick={apply}>
          {busy ? "Placing…" : `Place ${(plan?.matched ?? 0).toLocaleString()} photos`}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .backdrop { position: fixed; inset: 0; z-index: 20; display: grid; place-items: center; padding: 16px; background: rgb(0 0 0 / 55%); }
  .dialog { width: min(480px, 100%); max-height: calc(100vh - 32px); overflow: auto; padding: 18px 20px; background: var(--panel); border: 1px solid var(--line); border-radius: 6px; box-shadow: 0 12px 40px rgb(0 0 0 / 50%); }
  h2 { margin: 0 0 6px; font-size: 16px; font-weight: 600; }
  p { margin: 6px 0; }
  .muted { color: var(--muted); }
  .small { font-size: 12px; margin-top: 0; }
  .file { overflow: hidden; white-space: nowrap; text-overflow: ellipsis; direction: rtl; text-align: left; }
  .row { display: flex; justify-content: space-between; align-items: center; gap: 12px; margin-top: 10px; }
  .row.check { justify-content: flex-start; gap: 6px; }
  .short { width: 5em; padding: 2px 5px; font: inherit; color: var(--text); background: var(--bg); border: 1px solid var(--line); border-radius: 3px; text-align: right; }
  .short:focus { outline: none; border-color: var(--queued); }
  input[type="checkbox"] { margin: 0; accent-color: var(--queued); }
  .error { padding: 0; }
  .result { margin-top: 14px; }
  .buttons { display: flex; justify-content: flex-end; gap: 8px; margin-top: 16px; }
  .buttons button { padding: 4px 12px; border: 1px solid var(--line); border-radius: 4px; }
  .buttons button:hover:not(:disabled) { background: var(--hover); }
  .buttons .primary { border: 0; background: var(--queued); color: #fff; font-weight: 600; }
  .buttons button:disabled { opacity: 0.4; cursor: default; }
</style>
