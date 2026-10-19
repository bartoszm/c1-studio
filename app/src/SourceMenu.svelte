<script lang="ts">
  import { onMount } from "svelte";
  import * as api from "./lib/api";
  import { LOCK_HELP, isLockError } from "./lib/lock";
  import { FOLDERS } from "./lib/folders";
  import FoldersDialog from "./FoldersDialog.svelte";

  // What is open, and the way to open something else: a recent source, one of a
  // provider's kind through the system dialog, or a typed path. The window reloads on
  // the new source.
  let { open = $bindable(false), onbackups, onchanged }: {
    open?: boolean;
    /** Opens the list of backups made before writes. */
    onbackups?: () => void;
    /** The open source's folders were changed and saved: the window should read it again. */
    onchanged?: () => void;
  } = $props();

  /** The photo folders dialog, when shown: the definition to edit, or none for a new source. */
  let folders = $state<{ at: string | null } | null>(null);

  let view = $state<api.SourcesView | null>(null);
  let busy = $state<string | null>(null);
  let failed = $state<string | null>(null);
  let typed = $state("");
  let kinds = $state(false);

  const SUPPORT: Record<api.Support, string> = { Tested: "checked", Untested: "not checked, tried", Off: "off" };

  async function load() {
    try {
      view = await api.sources();
    } catch (e) {
      failed = String(e);
    }
  }
  onMount(load);
  $effect(() => {
    if (open) load();
  });

  async function go(spec: api.SourceSpec) {
    busy = spec.location;
    failed = null;
    try {
      await api.openSource(spec);
      location.reload();
    } catch (e) {
      failed = String(e);
      busy = null;
    }
  }

  async function browse(p: api.ProviderView) {
    failed = null;
    try {
      const path = await api.pickSource(p.id);
      if (path) await go({ provider: p.id, location: path });
    } catch (e) {
      failed = String(e);
    }
  }

  function editFolders(at: string | null) {
    open = false;
    folders = { at };
  }

  async function rescan() {
    open = false;
    try {
      await api.rescan();
    } catch (e) {
      failed = String(e);
      open = true;
    }
  }

  async function forget(spec: api.SourceSpec) {
    await api.forgetSource(spec);
    await load();
  }

  async function offer(id: string, box: HTMLInputElement) {
    if (!view) return;
    const ids = view.kinds.filter((k) => (k.id === id ? box.checked : k.offered)).map((k) => k.id);
    failed = null;
    try {
      await api.setProviders(ids);
    } catch (e) {
      failed = String(e);
      // Nothing was saved: the box goes back.
      box.checked = !box.checked;
    }
    await load();
  }

  function openTyped(e: SubmitEvent) {
    e.preventDefault();
    // No provider named: the first that recognises the path opens it.
    if (typed.trim()) go({ provider: "", location: typed.trim() });
  }
</script>

<svelte:window onkeydown={(e) => { if (open && e.key === "Escape") open = false; }} />

<button
  class="source-button"
  title={view?.current ? `${view.current.kind}: ${view.current.location}` : "Open a catalog or photo folders"}
  onclick={() => (open = !open)}
>
  <span class="brand">C1 Studio</span>
  <span class="current" class:none={!view?.current}>{view?.current?.name ?? "nothing open"}</span>
  <span class="caret">▾</span>
</button>

{#if open && view}
  <div class="menu-shield" role="presentation" onclick={() => (open = false)}></div>
  <div class="menu source-menu" role="menu">
    {#if view.current}
      <div class="menu-title">Open now</div>
      <div class="source-row current">
        <span class="what"><b>{view.current.name}</b><small>{view.current.kind} · {view.current.location}</small></span>
      </div>
      {#if view.compatibility}
        {@const c = view.compatibility}
        <div class="compat-detail">
          <div>{c.format}</div>
          <div>
            Reading: <b class={c.read}>{SUPPORT[c.read]}</b> · Writing: <b class={c.write}>{SUPPORT[c.write]}</b>
          </div>
          {#if c.note}<div class="note">{c.note}</div>{/if}
        </div>
      {/if}
      {#if view.current.provider === FOLDERS}
        <button role="menuitem" onclick={() => editFolders(view!.current!.location)}>Edit folders…</button>
        <button role="menuitem" onclick={rescan}>Rescan</button>
      {/if}
      {#if onbackups}
        <button role="menuitem" onclick={() => { open = false; onbackups(); }}>Backups…</button>
      {/if}
    {/if}
    {#if view.error}
      <p class="source-note error">{isLockError(view.error) ? LOCK_HELP : view.error}</p>
      {#if isLockError(view.error)}<p class="source-note error small">{view.error}</p>{/if}
      <!-- Opening the same source again: it works once what blocked it is gone. -->
      {#if view.current}
        <button role="menuitem" disabled={!!busy} onclick={() => go(view!.current!)}>Try again</button>
      {/if}
    {/if}
    {#if view.recent.length}
      <div class="menu-title">Recent</div>
      {#each view.recent as r (r.provider + r.location)}
        <div class="source-row">
          <button
            class="what"
            role="menuitem"
            disabled={!r.available || !!busy}
            title={r.available ? `Open ${r.location}` : `${r.location} is not there any more`}
            onclick={() => go(r)}
          >
            <b>{r.name}</b><small>{r.kind} · {r.location}</small>
          </button>
          <button class="forget" title="Take it off this list" aria-label="Forget {r.name}" onclick={() => forget(r)}>×</button>
        </div>
      {/each}
    {/if}
    <div class="menu-title">Open</div>
    {#each view.providers as p (p.id)}
      {#if p.id === FOLDERS}
        <button role="menuitem" disabled={!!busy} onclick={() => editFolders(null)}>New photo folders…</button>
        <button role="menuitem" disabled={!!busy} onclick={() => browse(p)}>Open photo folders…</button>
      {:else}
        <button role="menuitem" disabled={!!busy} onclick={() => browse(p)}>{p.name}…</button>
      {/if}
    {/each}
    <form class="source-path" onsubmit={openTyped}>
      <input placeholder="or type a path, then Enter" spellcheck="false" bind:value={typed} disabled={!!busy} />
    </form>
    <button role="menuitem" aria-expanded={kinds} onclick={() => (kinds = !kinds)}>Kinds of source…</button>
    {#if kinds}
      <div class="source-kinds">
        {#each view.kinds as k (k.id)}
          {@const last = k.offered && view.kinds.filter((x) => x.offered).length === 1}
          <label title={last ? "At least one kind stays on" : `Offer ${k.name} in this menu`}>
            <input type="checkbox" checked={k.offered} disabled={last} onchange={(e) => offer(k.id, e.currentTarget)} />
            {k.name}
          </label>
        {/each}
      </div>
    {/if}
    {#if busy}<p class="source-note">Opening {busy}…</p>{/if}
    {#if failed}<p class="source-note error">{failed}</p>{/if}
  </div>
{/if}

{#if folders}
  <FoldersDialog
    at={folders.at}
    onclose={() => (folders = null)}
    onsaved={() => onchanged?.()}
  />
{/if}
