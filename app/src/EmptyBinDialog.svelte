<script lang="ts">
  import * as api from "./lib/api";
  import { source } from "./lib/source.svelte";
  import { removalText } from "./lib/bin";

  // Deleting photos in the trash for good: the whole trash, or the photos picked in it
  // (`rows`). The source deletes their files, or marks them deleted; this cannot be undone
  // here, so it asks first.
  let {
    count,
    rows,
    removes,
    onclose,
    ondone,
  }: { count: number; rows?: number[]; removes: api.Removal; onclose: () => void; ondone: () => void } = $props();

  let status = $state<api.WriteStatus | null>(null);
  let busy = $state(false);
  let failed = $state<string | null>(null);
  let done = $state<api.EmptyReport | null>(null);

  async function check() {
    try {
      status = (await api.backups()).status;
    } catch (e) {
      failed = String(e);
    }
  }
  check();

  let locked = $derived(status && typeof status === "object" ? status.Locked.by : null);
  let going = $derived(rows?.length ?? count);

  async function empty() {
    busy = true;
    failed = null;
    try {
      done = await api.emptyBin(rows);
      ondone();
    } catch (e) {
      failed = String(e);
      await check();
    } finally {
      busy = false;
    }
  }
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && !busy && onclose()} />

<div class="backdrop" role="presentation" onclick={(e) => e.target === e.currentTarget && !busy && onclose()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="empty-title">
    <h2 id="empty-title">{rows ? "Delete for good" : "Empty trash"}</h2>
    {#if done}
      <p class="ok">{done.removed.toLocaleString()} photo{done.removed === 1 ? "" : "s"} deleted for good.</p>
      {#if done.backup}<p class="muted">The {source.noun} was backed up first: <small>{done.backup}</small></p>{/if}
      {#if done.failed.length}
        <p class="warn">{done.failed.length.toLocaleString()} stayed in the trash:</p>
        <ul>
          {#each done.failed as [name, why]}<li><b>{name}</b> <span class="muted">· {why}</span></li>{/each}
        </ul>
      {/if}
      {#each done.warnings as warning}<p class="warn">{warning}</p>{/each}
    {:else}
      <p>
        Delete <b>{going.toLocaleString()} photo{going === 1 ? "" : "s"}</b> {rows ? "picked in the trash" : "in the trash"} for good?
        This cannot be undone here.
      </p>
      <p class="muted">{removalText(going, removes, source.wording)}</p>
      {#if locked}
        <p class="warn">Close {source.owner ?? "the program that has it open"} first: it keeps the {source.noun} in memory and would write it back over the change. <span class="muted">(Held by {locked}.)</span></p>
      {:else if status === "ReadOnly"}
        <p class="warn">This source is not written by the app, so nothing can be deleted in it.</p>
      {/if}
    {/if}
    {#if failed}<p class="error">{failed}</p>{/if}

    <div class="buttons">
      {#if done}
        <button onclick={onclose}>Close</button>
      {:else}
        {#if status !== null && status !== "Ready"}<button onclick={check} disabled={busy}>Check again</button>{/if}
        <button onclick={onclose} disabled={busy}>Cancel</button>
        <button class="primary danger" disabled={busy || status !== "Ready" || !going} onclick={empty}>
          {busy ? "Deleting…" : rows ? "Delete for good" : "Empty trash"}
        </button>
      {/if}
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
    width: min(520px, 100%);
    max-height: calc(100vh - 32px);
    overflow: auto;
    padding: 18px 20px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 6px;
    box-shadow: 0 12px 40px rgb(0 0 0 / 50%);
  }
  h2 { margin: 0 0 10px; font-size: 16px; font-weight: 600; }
  ul { margin: 6px 0 0; padding: 0 0 0 16px; max-height: 200px; overflow: auto; }
  small { word-break: break-all; }
  .buttons button { padding: 3px 10px; border: 1px solid var(--line); border-radius: 4px; }
  .buttons button:hover:not(:disabled) { background: var(--hover); }
  button:disabled { opacity: 0.4; cursor: default; }
  .buttons { display: flex; justify-content: flex-end; gap: 8px; margin-top: 12px; }
  .buttons .primary { border: 0; background: var(--queued); color: #fff; font-weight: 600; }
  .buttons .primary.danger { background: var(--error); }
  .muted { color: var(--muted); }
  .warn { color: #e8b03a; }
  .ok { color: #7cc47c; }
  .error { padding: 0; }
  p { margin: 8px 0; }
</style>
