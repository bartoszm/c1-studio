<script lang="ts">
  import { onMount } from "svelte";
  import * as api from "./lib/api";

  // What is open, and the way to open something else: a recent source, one of a
  // provider's kind through the system dialog, or a typed path. The window reloads on
  // the new source.
  let { open = $bindable(false), onbackups }: {
    open?: boolean;
    /** Opens the list of backups made before writes. */
    onbackups?: () => void;
  } = $props();

  let view = $state<api.SourcesView | null>(null);
  let busy = $state<string | null>(null);
  let failed = $state<string | null>(null);
  let typed = $state("");

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

  async function forget(spec: api.SourceSpec) {
    await api.forgetSource(spec);
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
  title={view?.current ? `${view.current.kind}: ${view.current.location}` : "Open a catalog"}
  onclick={() => (open = !open)}
>
  <span class="brand">C1 Studio</span>
  <span class="current" class:none={!view?.current}>{view?.current?.name ?? "no catalog"}</span>
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
      {#if onbackups}
        <button role="menuitem" onclick={() => { open = false; onbackups(); }}>Backups…</button>
      {/if}
    {/if}
    {#if view.error}<p class="source-note error">{view.error}</p>{/if}
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
      <button role="menuitem" disabled={!!busy} onclick={() => browse(p)}>{p.name}…</button>
    {/each}
    <form class="source-path" onsubmit={openTyped}>
      <input placeholder="or type a path, then Enter" spellcheck="false" bind:value={typed} disabled={!!busy} />
    </form>
    {#if busy}<p class="source-note">Opening {busy}…</p>{/if}
    {#if failed}<p class="source-note error">{failed}</p>{/if}
  </div>
{/if}
