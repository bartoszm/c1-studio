<script lang="ts">
  import * as api from "./lib/api";
  import { source } from "./lib/source.svelte";

  // The backups made before writes, newest first, and putting one back. A restore backs
  // the catalog up first, so it shows up here and can be undone the same way.
  let { onclose, onrestored }: { onclose: () => void; onrestored: () => void } = $props();

  let list = $state<api.BackupInfo[] | null>(null);
  let status = $state<api.WriteStatus>("ReadOnly");
  let failed = $state<string | null>(null);
  /** The backup asked to be restored, waiting for a yes. */
  let asking = $state<api.BackupInfo | null>(null);
  let busy = $state(false);
  let done = $state<api.BackupInfo | null>(null);

  async function load() {
    try {
      const b = await api.backups();
      list = b.list;
      status = b.status;
    } catch (e) {
      failed = String(e);
    }
  }
  load();

  let locked = $derived(typeof status === "object" ? status.Locked.by : null);

  async function restore(b: api.BackupInfo) {
    busy = true;
    failed = null;
    try {
      done = await api.restoreBackup(b.id);
      asking = null;
      onrestored();
    } catch (e) {
      failed = String(e);
    } finally {
      busy = false;
    }
    await load();
  }

  const when = (ms: number) =>
    new Date(ms).toLocaleString(undefined, { day: "numeric", month: "short", year: "numeric", hour: "2-digit", minute: "2-digit" });
  function ago(ms: number) {
    const min = Math.round((Date.now() - ms) / 60000);
    if (min < 1) return "just now";
    if (min < 60) return `${min} min ago`;
    const h = Math.round(min / 60);
    if (h < 48) return `${h} h ago`;
    return `${Math.round(h / 24)} days ago`;
  }
  const mb = (n: number) => `${Math.round(n / 1e6).toLocaleString()} MB`;
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && !busy && onclose()} />

<div class="backdrop" role="presentation" onclick={(e) => e.target === e.currentTarget && !busy && onclose()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="backups-title">
    <h2 id="backups-title">Backups</h2>
    <p class="muted">
      Before the app writes, it keeps a copy of the {source.backup}; the newest five are kept. Restoring one brings that copy
      back.
    </p>

    {#if done}
      <p class="ok">Restored. What it replaced is kept as the newest backup below, so this can be undone the same way.</p>
    {/if}
    {#if locked}
      <p class="warn">Close {source.owner ?? "the program that has it open"} to restore: it keeps the {source.noun} in memory and would write it back over the restored one. <span class="muted">(Held by {locked}.)</span></p>
    {:else if status === "ReadOnly"}
      <p class="warn">This {source.noun} is not written by the app, so it cannot be restored here.</p>
    {/if}

    {#if list === null && !failed}
      <p class="muted">Reading…</p>
    {:else if list && !list.length}
      <p class="muted">No backups yet: one is made the first time the app writes to this {source.noun}.</p>
    {:else if list}
      <ul>
        {#each list as b (b.id)}
          <li class:asking={asking?.id === b.id}>
            <span class="what">
              <span><b>{when(b.created_ms)}</b> <span class="muted">· {ago(b.created_ms)} · {mb(b.size)}</span></span>
              {#if b.note}<span class="note">{b.note}</span>{/if}
              <small title={b.path}>{b.id}</small>
            </span>
            {#if asking?.id !== b.id}
              <button disabled={status !== "Ready" || busy} onclick={() => (asking = b)}>Restore…</button>
            {/if}
          </li>
          {#if asking?.id === b.id}
            <li class="confirm">
              <p>
                Go back to the {source.backup} as of <b>{when(b.created_ms)}</b>? Everything changed since{#if source.owner}, in
                {source.owner} or written from here,{/if} is undone.
              </p>
              <p class="muted">
                What is there now is backed up first and listed here. Edits queued in this app stay queued.
              </p>
              <div class="buttons">
                <button onclick={() => (asking = null)} disabled={busy}>Cancel</button>
                <button class="primary danger" disabled={busy} onclick={() => restore(b)}>{busy ? "Restoring…" : "Restore"}</button>
              </div>
            </li>
          {/if}
        {/each}
      </ul>
    {/if}
    {#if failed}<p class="error">{failed}</p>{/if}

    <div class="buttons">
      {#if status !== "Ready"}<button onclick={load} disabled={busy}>Check again</button>{/if}
      <button onclick={onclose} disabled={busy}>Close</button>
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
    width: min(600px, 100%);
    max-height: calc(100vh - 32px);
    overflow: auto;
    padding: 18px 20px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 6px;
    box-shadow: 0 12px 40px rgb(0 0 0 / 50%);
  }
  h2 { margin: 0 0 10px; font-size: 16px; font-weight: 600; }
  ul { margin: 10px 0 0; padding: 0; list-style: none; }
  li { display: flex; align-items: center; gap: 10px; padding: 7px 0; border-top: 1px solid var(--line); }
  li.asking { border-bottom: 0; }
  li.confirm { display: block; padding: 4px 0 10px 12px; border-top: 0; }
  .what { flex: 1; display: grid; gap: 2px; min-width: 0; }
  .what small { overflow: hidden; white-space: nowrap; text-overflow: ellipsis; color: #777; }
  .note { color: #e8b03a; font-size: 12px; }
  li > button, .buttons button { padding: 3px 10px; border: 1px solid var(--line); border-radius: 4px; }
  li > button:hover:not(:disabled), .buttons button:hover:not(:disabled) { background: var(--hover); }
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
