<script lang="ts">
  import { onMount } from "svelte";
  import * as api from "./lib/api";
  import { source } from "./lib/source.svelte";
  import { codeFor } from "./lib/countries";

  // Photos with a country but no ISO code, by country, with the code the country's
  // name gives; ticked ones get it, as one edit queued for Capture One.
  let { filter, onclose, ondone }: {
    /** The view's filter: only its photos are filled. */
    filter: api.Filter;
    onclose: () => void;
    ondone: (undo: [number, api.Op][]) => Promise<void>;
  } = $props();

  type Row = { country: string; rows: number[]; code: string; on: boolean };
  let list = $state<Row[] | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);

  onMount(async () => {
    try {
      const found = await api.missingCountryCodes($state.snapshot(filter));
      list = found
        .map(([country, rows]) => {
          const code = codeFor(country) ?? "";
          return { country, rows, code, on: !!code };
        })
        .sort((a, b) => b.rows.length - a.rows.length);
    } catch (e) {
      error = String(e);
    }
  });

  const valid = (code: string) => /^[A-Za-z]{2}$/.test(code.trim());
  let chosen = $derived((list ?? []).filter((r) => r.on && valid(r.code)));
  let photos = $derived(chosen.reduce((n, r) => n + r.rows.length, 0));

  async function apply() {
    if (!chosen.length || busy) return;
    busy = true;
    error = null;
    try {
      const undo = await api.setCountryCodes(chosen.map((r) => [r.rows, r.code.trim().toUpperCase()]));
      await ondone(undo);
      onclose();
    } catch (e) {
      error = String(e);
      busy = false;
    }
  }
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && onclose()} />

<div class="backdrop" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="cc-title">
    <h2 id="cc-title">Fill in ISO country codes</h2>
    <p class="muted">
      Photos in view that have a country but no ISO code. The code comes from the country's name;
      change it or untick a row. Queued for {source.word} like any edit; Ctrl+Z takes it back.
    </p>
    {#if error}<p class="error">{error}</p>{/if}
    {#if !list}
      <p class="muted">Looking…</p>
    {:else if !list.length}
      <p>Every photo in view with a country has its code.</p>
    {:else}
      <table>
        <thead><tr><th></th><th>Country</th><th class="n">Photos</th><th>Code</th></tr></thead>
        <tbody>
          {#each list as r (r.country)}
            <tr class:off={!r.on}>
              <td><input type="checkbox" bind:checked={r.on} disabled={!valid(r.code)} aria-label="Fill {r.country}" /></td>
              <td>{r.country}</td>
              <td class="n">{r.rows.length.toLocaleString()}</td>
              <td>
                <input
                  class="code"
                  maxlength="2"
                  placeholder="??"
                  bind:value={r.code}
                  oninput={() => { r.code = r.code.toUpperCase(); r.on = valid(r.code); }}
                  title={r.code ? "" : "Not a country name the app knows: type its two-letter code"}
                />
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
    <div class="buttons">
      <button onclick={onclose}>Cancel</button>
      <button class="primary" disabled={!chosen.length || busy} onclick={apply}>
        {busy ? "Filling…" : `Fill ${photos.toLocaleString()} photo${photos === 1 ? "" : "s"}`}
      </button>
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 20;
    display: grid;
    place-items: center;
    padding: 16px;
    background: rgb(0 0 0 / 55%);
  }
  .dialog {
    width: min(460px, 100%);
    max-height: calc(100vh - 32px);
    overflow: auto;
    padding: 18px 20px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 6px;
    box-shadow: 0 12px 40px rgb(0 0 0 / 50%);
  }
  h2 { margin: 0 0 6px; font-size: 16px; font-weight: 600; }
  p { margin: 6px 0; }
  .muted { color: var(--muted); }
  .error { padding: 0; }
  table { width: 100%; margin-top: 8px; border-collapse: collapse; }
  th { text-align: left; font-weight: 600; color: var(--muted); font-size: 12px; padding: 2px 6px; }
  td { padding: 3px 6px; border-top: 1px solid var(--line); }
  .n { text-align: right; font-variant-numeric: tabular-nums; }
  tr.off td:not(:first-child) { color: var(--muted); }
  input[type="checkbox"] { margin: 0; accent-color: var(--queued); }
  .code {
    width: 3.5em;
    padding: 2px 5px;
    font: inherit;
    text-transform: uppercase;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 3px;
  }
  .code:focus { outline: none; border-color: var(--queued); }
  .buttons { display: flex; justify-content: flex-end; gap: 8px; margin-top: 16px; }
  .buttons button { padding: 4px 12px; border: 1px solid var(--line); border-radius: 4px; }
  .buttons button:hover:not(:disabled) { background: var(--hover); }
  .buttons .primary { border: 0; background: var(--queued); color: #fff; font-weight: 600; }
  .buttons button:disabled { opacity: 0.4; cursor: default; }
</style>
