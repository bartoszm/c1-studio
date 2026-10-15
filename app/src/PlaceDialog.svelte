<script lang="ts">
  import { onMount } from "svelte";
  import * as api from "./lib/api";
  import { source } from "./lib/source.svelte";
  import { codeFor, looseKey } from "./lib/countries";
  import { placeText } from "./lib/tree";

  // Rename a place, or join it into another at the same level: every photo at or under
  // it takes the new country / state / city / location, keeping the levels below, as
  // one edit queued for Capture One.
  let { mode, path, onclose, ondone }: {
    mode: "rename" | "join";
    /** The place, as the tree's path: `Poland|—|Krakow`. */
    path: string;
    onclose: () => void;
    ondone: (undo: [number, api.Op][]) => Promise<void>;
  } = $props();

  const LEVELS = ["country", "state", "city", "location"];
  // svelte-ignore state_referenced_locally
  const parts = path.split("|");
  const depth = parts.length;
  const parent = parts.slice(0, -1).join("|");
  const own = parts[depth - 1] === "—" ? "" : parts[depth - 1];
  const level = LEVELS[depth - 1];

  // svelte-ignore state_referenced_locally
  let name = $state(mode === "rename" ? own : "");
  let target = $state<string | null>(null);
  let filterText = $state("");
  let photos = $state<number | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);

  /** Every place in the catalog with its count, whatever the view's filters. */
  let places = $state<[string, number][]>([]);
  onMount(async () => {
    const [here, all] = await Promise.all([
      api.query({ place_under: path }, { limit: 0 }),
      api.query({}, { limit: 0, facets: ["place_tree"] }),
    ]);
    photos = here.total;
    places = all.facets.place_tree ?? [];
  });

  // Places at the same level, similar names first (Krakow ≈ Kraków, jordan ≈ Jordan),
  // then those under the same parent, then the most used.
  const last = (p: string) => p.slice(p.lastIndexOf("|") + 1);
  let candidates = $derived(
    places
      .filter(([p]) => p !== path && p.split("|").length === depth && last(p) !== "—")
      .map(([p, n]) => ({
        p,
        n,
        similar: !!own && looseKey(last(p)) === looseKey(own),
        sibling: p.slice(0, p.lastIndexOf("|")) === parent,
      }))
      .sort((a, b) => +b.similar - +a.similar || +b.sibling - +a.sibling || b.n - a.n),
  );
  let shown = $derived(
    candidates.filter((c) => !filterText.trim() || looseKey(placeText(c.p)).includes(looseKey(filterText))).slice(0, 200),
  );
  $effect(() => {
    if (mode === "join" && target === null && candidates[0]?.similar) target = candidates[0].p;
  });

  /** Where the photos go: a new name under the same parent, or the place picked. */
  let to = $derived(
    mode === "rename" ? (name.trim() ? (parent ? `${parent}|${name.trim()}` : name.trim()) : null) : target,
  );
  let existing = $derived(to && to !== path ? places.find(([p]) => p === to) : undefined);
  // A new country brings its ISO code.
  let code = $derived(depth === 1 && to ? codeFor(to) : null);

  let problem = $derived.by(() => {
    if (!to) return mode === "rename" ? `Type the new ${level}.` : "Pick the place to join into.";
    if (to === path) return "That is its name already.";
    if (mode === "rename" && name.includes("|")) return "A name cannot contain |.";
    return null;
  });

  async function submit(e?: Event) {
    e?.preventDefault();
    if (problem || !to || busy) return;
    busy = true;
    error = null;
    try {
      const undo = await api.movePlace(path, to, code);
      await ondone(undo);
      onclose();
    } catch (err) {
      error = String(err);
      busy = false;
    }
  }
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && onclose()} />

<div class="backdrop" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="place-title">
    <h2 id="place-title">{mode === "rename" ? `Rename ${level}` : `Join ${level}`}</h2>
    <p class="muted">
      <b>{placeText(path)}</b>{#if photos !== null}{" · "}{photos.toLocaleString()} photo{photos === 1 ? "" : "s"}{/if}.
      {mode === "rename" ? "Every photo there gets the new name;" : "Every photo there moves into the place you pick;"}
      what is below the {level} stays. Queued for {source.word}; Ctrl+Z takes it back.
    </p>

    {#if mode === "rename"}
      <form onsubmit={submit}>
        <!-- svelte-ignore a11y_autofocus -->
        <input class="name" bind:value={name} autofocus spellcheck="false" placeholder="new {level}" />
      </form>
    {:else}
      <input class="name" bind:value={filterText} placeholder="find a {level}" spellcheck="false" />
      <ul class="targets" role="listbox">
        {#each shown as c (c.p)}
          <li>
            <button class:on={target === c.p} role="option" aria-selected={target === c.p} onclick={() => (target = c.p)}>
              <span>{placeText(c.p)}</span>
              {#if c.similar}<span class="tag">similar</span>{/if}
              <span class="n">{c.n.toLocaleString()}</span>
            </button>
          </li>
        {/each}
        {#if !shown.length}<li class="muted">No {level} matches.</li>{/if}
      </ul>
    {/if}

    {#if to && to !== path}
      <p class="muted">
        → <b>{placeText(to)}</b>
        {#if existing}(exists, {existing[1].toLocaleString()} photos: they join){/if}
        {#if depth === 1}· ISO code {code ? code : "left as each photo has it (not a country name the app knows)"}{/if}
      </p>
    {/if}
    {#if problem && (name || target)}<p class="warn">{problem}</p>{/if}
    {#if error}<p class="error">{error}</p>{/if}

    <div class="buttons">
      <button onclick={onclose}>Cancel</button>
      <button class="primary" disabled={!!problem || busy} onclick={() => submit()}>
        {busy ? "Working…" : mode === "rename" ? (existing ? "Rename and join" : "Rename") : "Join"}
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
  h2 { margin: 0 0 6px; font-size: 16px; font-weight: 600; }
  p { margin: 8px 0; }
  .muted { color: var(--muted); }
  .muted b { color: var(--text); font-weight: 600; }
  .warn { color: #e8b03a; }
  .error { padding: 0; }
  .name {
    width: 100%;
    margin-top: 6px;
    padding: 5px 7px;
    font: inherit;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 3px;
  }
  .name:focus { outline: none; border-color: var(--queued); }
  .targets { max-height: 260px; overflow-y: auto; margin: 6px 0 0; padding: 0; list-style: none; border: 1px solid var(--line); border-radius: 3px; }
  .targets li.muted { padding: 6px 8px; }
  .targets button { display: flex; align-items: center; gap: 8px; width: 100%; padding: 4px 8px; text-align: left; }
  .targets button:hover { background: var(--hover); }
  .targets button.on { background: var(--on); }
  .targets span:first-child { flex: 1; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
  .targets .n { color: var(--muted); font-variant-numeric: tabular-nums; }
  .tag { padding: 0 5px; font-size: 11px; color: #e8b03a; border: 1px solid #6b5a2a; border-radius: 3px; }
  .buttons { display: flex; justify-content: flex-end; gap: 8px; margin-top: 16px; }
  .buttons button { padding: 4px 12px; border: 1px solid var(--line); border-radius: 4px; }
  .buttons button:hover:not(:disabled) { background: var(--hover); }
  .buttons .primary { border: 0; background: var(--queued); color: #fff; font-weight: 600; }
  .buttons button:disabled { opacity: 0.4; cursor: default; }
</style>
