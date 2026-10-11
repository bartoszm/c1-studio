<script lang="ts">
  import * as api from "./lib/api";

  // Rename, join (replace) or group keywords across the catalog. The change is queued
  // like any edit: it shows at once and is written to Capture One on demand.
  let { mode, start, keywords, onclose, ondone }: {
    mode: "rename" | "join" | "group";
    /** The keyword the dialog was opened on. */
    start: string;
    /** Every keyword path, for completion. */
    keywords: string[];
    onclose: () => void;
    ondone: (request: api.RemapRequest) => Promise<void>;
  } = $props();

  const clean = (path: string) => path.split("|").map((l) => l.trim()).join("|");
  const last = (path: string) => path.slice(path.lastIndexOf("|") + 1);
  const parentOf = (path: string) => (path.includes("|") ? path.slice(0, path.lastIndexOf("|")) : "");

  // svelte-ignore state_referenced_locally
  let picked = $state<string[]>([start]);
  let adding = $state("");
  // svelte-ignore state_referenced_locally
  let name = $state(mode === "rename" ? last(start) : "");
  let target = $state("");
  let users = $state<number | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);

  $effect(() => {
    const paths = $state.snapshot(picked);
    users = null;
    api.keywordUsers(paths).then((n) => { if (JSON.stringify(paths) === JSON.stringify(picked)) users = n; });
  });

  function add(e: SubmitEvent) {
    e.preventDefault();
    const path = clean(adding);
    if (path && !picked.includes(path)) picked = [...picked, path];
    adding = "";
  }

  let request = $derived.by((): api.RemapRequest | string => {
    if (mode === "rename") {
      const n = name.trim();
      if (!n) return "Type the new name.";
      if (/[|,]/.test(n)) return "A name cannot contain | or a comma.";
      if (n === last(start)) return "That is its name already.";
      return { Rename: { path: start, name: n } };
    }
    const to = clean(target);
    if (to.split("|").some((l) => !l) && to) return "A keyword level is empty.";
    if (to.includes(",")) return "Keyword names cannot contain a comma.";
    if (mode === "join") {
      if (!to) return "Choose the keyword to join them into.";
      if (picked.every((p) => p === to)) return "Choose a different keyword to join into.";
      return { Join: { from: picked, into: to } };
    }
    if (to && picked.some((p) => to === p || to.startsWith(p + "|"))) return "A keyword cannot go under itself.";
    return { Group: { paths: picked, under: to || null } };
  });

  let newTarget = $derived(!!clean(target) && !keywords.includes(clean(target)));

  async function submit(e?: Event) {
    e?.preventDefault();
    if (typeof request === "string" || busy) return;
    busy = true;
    error = null;
    try {
      await ondone(request);
      onclose();
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  const TITLES = { rename: "Rename keyword", join: "Join keywords", group: "Group keywords" };
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && onclose()} />

<datalist id="remap-keywords">
  {#each keywords as k}<option value={k}></option>{/each}
</datalist>

<div class="backdrop" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="kw-title">
    <h2 id="kw-title">{TITLES[mode]}</h2>

    {#if mode === "rename"}
      <form onsubmit={submit}>
        <label>
          New name for <b>{start}</b>
          <!-- svelte-ignore a11y_autofocus -->
          <input bind:value={name} autofocus spellcheck="false" />
        </label>
        {#if parentOf(start) && name.trim()}<p class="muted">It becomes {parentOf(start)}|{name.trim()}.</p>{/if}
      </form>
    {:else}
      <p class="muted">
        {#if mode === "join"}
          Every photo with these keywords gets the one below instead, and they leave the keyword list. Keywords
          under them move along. With one keyword, this replaces it.
        {:else}
          These keywords, with everything under them, go under the parent below. Leave it empty for the top level.
        {/if}
      </p>
      <ul class="chips">
        {#each picked as p (p)}
          <li>
            {p}
            {#if picked.length > 1}
              <button aria-label="Leave out {p}" onclick={() => (picked = picked.filter((x) => x !== p))}>×</button>
            {/if}
          </li>
        {/each}
      </ul>
      <form onsubmit={add} class="row">
        <input list="remap-keywords" placeholder="Add another keyword" bind:value={adding} spellcheck="false" />
      </form>
      <form onsubmit={submit}>
        <label>
          {mode === "join" ? "Into" : "Under"}
          <!-- svelte-ignore a11y_autofocus -->
          <input
            list="remap-keywords"
            bind:value={target}
            autofocus
            spellcheck="false"
            placeholder={mode === "join" ? "an existing keyword or a new one" : "a parent keyword, existing or new"}
          />
        </label>
        {#if newTarget}<p class="muted">“{clean(target)}” is new: it will be created.</p>{/if}
      </form>
    {/if}

    <p class="muted">
      {#if users === null}Counting photos…{:else}{users.toLocaleString()} photo{users === 1 ? "" : "s"} carry {picked.length > 1 ? "these keywords" : "it"}.{/if}
      Queued until you write to Capture One; Ctrl+Z takes it back.
    </p>
    {#if typeof request === "string" && (name || target)}<p class="warn">{request}</p>{/if}
    {#if error}<p class="error">{error}</p>{/if}

    <div class="buttons">
      <button onclick={onclose}>Cancel</button>
      <button
        class="primary"
        disabled={typeof request === "string" || busy}
        onclick={() => submit()}
      >
        {mode === "rename" ? "Rename" : mode === "join" ? (picked.length > 1 ? "Join" : "Replace") : "Group"}
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
    width: min(480px, 100%);
    max-height: calc(100vh - 32px);
    overflow: auto;
    padding: 18px 20px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 6px;
    box-shadow: 0 12px 40px rgb(0 0 0 / 50%);
  }
  h2 { margin: 0 0 10px; font-size: 16px; font-weight: 600; }
  label { display: grid; gap: 4px; margin-top: 10px; }
  input {
    width: 100%;
    padding: 5px 7px;
    font: inherit;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 3px;
  }
  input:focus { outline: none; border-color: var(--queued); }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; margin: 8px 0; padding: 0; list-style: none; }
  .chips li { display: flex; align-items: center; gap: 4px; padding: 2px 8px; background: var(--on); border-radius: 10px; }
  .chips button { padding: 0 2px; color: var(--muted); }
  .chips button:hover { color: var(--error); }
  .row input { margin-top: 2px; }
  .buttons { display: flex; justify-content: flex-end; gap: 8px; margin-top: 16px; }
  .buttons button { padding: 4px 12px; border: 1px solid var(--line); border-radius: 4px; }
  .buttons button:hover:not(:disabled) { background: var(--hover); }
  .buttons .primary { border: 0; background: var(--queued); color: #fff; font-weight: 600; }
  .buttons button:disabled { opacity: 0.4; cursor: default; }
  .muted { color: var(--muted); }
  .warn { color: #e8b03a; }
  .error { padding: 0; }
  p { margin: 8px 0; }
</style>
