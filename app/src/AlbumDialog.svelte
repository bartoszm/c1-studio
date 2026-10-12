<script lang="ts" module>
  /** One condition as the rule editor shows it. */
  export type Row = { what: "keyword" | "rating" | "at_least" | "year"; value: string };

  /** The rows a rule shows as, or null when some condition cannot be edited here. */
  export function rows(rule: { conditions: import("./lib/api").Condition[] }): Row[] | null {
    const out: Row[] = [];
    for (const c of rule.conditions) {
      if ("KeywordContains" in c) out.push({ what: "keyword", value: c.KeywordContains });
      else if ("Year" in c) out.push({ what: "year", value: String(c.Year) });
      else if ("Rating" in c && c.Rating.min === c.Rating.max) out.push({ what: "rating", value: String(c.Rating.min) });
      else if ("Rating" in c && c.Rating.max === 5) out.push({ what: "at_least", value: String(c.Rating.min) });
      else return null;
    }
    return out;
  }
</script>

<script lang="ts">
  import * as api from "./lib/api";

  // Make an album, smart album or group; rename one; change a smart album's rule; or
  // delete one. Queued like any edit: it shows at once and is written on demand.
  let { mode, kind = "Album", collection, collections, keywords, picked = 0, onclose, ondone }: {
    mode: "create" | "rename" | "rule" | "delete" | "move";
    /** What `create` makes. */
    kind?: "Album" | "Smart" | "Group";
    /** The collection it was opened on: the one to change, or for `create`, where to
     *  put the new one (in it if it is a group or project, else beside it). */
    collection?: api.Collection;
    collections: api.Collection[];
    /** Keyword paths, for completing a smart album's keyword conditions. */
    keywords: string[];
    /** Photos picked in the grid; a new album can take them. */
    picked?: number;
    onclose: () => void;
    /** `fill`: put the picked photos into the new album. */
    ondone: (change: api.CollectionChange, fill: boolean) => Promise<void>;
  } = $props();

  const holds = (c: api.Collection | undefined) => !!c && (c.kind === "Group" || c.kind === "Project") && !c.system;
  /** Where a collection can go: the top level, or a group or project (when moving, not
   *  into itself or what is under it). */
  let inside = $derived.by(() => {
    if (mode !== "move" || !collection) return new Set<string>();
    const out = new Set([collection.id]);
    let grew = true;
    while (grew) {
      grew = false;
      for (const c of collections) if (c.parent && out.has(c.parent) && !out.has(c.id)) out.add(c.id), (grew = true);
    }
    return out;
  });
  let places = $derived(
    collections.filter((c) => holds(c) && !inside.has(c.id)).sort((a, b) => a.path.localeCompare(b.path)),
  );

  // svelte-ignore state_referenced_locally
  let name = $state(mode === "rename" ? (collection?.name ?? "") : "");
  // svelte-ignore state_referenced_locally
  let parent = $state<string>(
    mode === "move" ? (collection?.parent ?? "")
    : holds(collection) ? collection!.id
    : collection?.parent && holds(collections.find((c) => c.id === collection!.parent)) ? collection.parent : "",
  );
  // svelte-ignore state_referenced_locally
  let matchAll = $state(collection?.rule?.match_all ?? true);
  // svelte-ignore state_referenced_locally
  let conditions = $state<Row[]>((collection?.rule && rows(collection.rule)) || [{ what: "keyword", value: "" }]);
  // svelte-ignore state_referenced_locally
  let fill = $state(mode === "create" && kind === "Album" && picked > 0);
  let error = $state<string | null>(null);
  let busy = $state(false);

  let smart = $derived(mode === "rule" || (mode === "create" && kind === "Smart"));

  function condition(row: Row): api.Condition | string {
    const v = row.value.trim();
    if (row.what === "keyword") return v ? { KeywordContains: v } : "A keyword condition needs some text.";
    const n = Number(v);
    if (row.what === "year") return Number.isInteger(n) && n >= 1800 && n <= 9999 ? { Year: n } : "Type a year, like 2025.";
    if (!v || !Number.isInteger(n) || n < 0 || n > 5) return "A rating is 0 to 5 stars.";
    return row.what === "rating" ? { Rating: { min: n, max: n } } : { Rating: { min: n, max: 5 } };
  }

  let rule = $derived.by((): api.SmartRule | string => {
    if (!conditions.length) return "Add at least one condition.";
    const out: api.Condition[] = [];
    for (const row of conditions) {
      const c = condition(row);
      if (typeof c === "string") return c;
      out.push(c);
    }
    return { match_all: matchAll, conditions: out };
  });

  /** Everything a delete takes: the collection and what is under it. */
  let doomed = $derived.by(() => {
    if (mode !== "delete" || !collection) return [];
    const out = [collection];
    for (let i = 0; i < out.length; i++) out.push(...collections.filter((c) => c.parent === out[i].id));
    return out;
  });

  let change = $derived.by((): api.CollectionChange | string => {
    if (mode === "delete") return collection ? { Delete: { id: collection.id } } : "Nothing to delete.";
    if (mode === "move") {
      if (!collection) return "Nothing to move.";
      const to = parent || null;
      if (to === (collection.parent ?? null)) return "It is there already.";
      const same = collections.find((c) => (c.parent ?? null) === to && c.id !== collection.id && c.name.toLowerCase() === collection.name.toLowerCase());
      if (same) return `“${same.path}” is there already.`;
      return { Move: { id: collection.id, parent: to } };
    }
    if (mode === "rule") {
      if (typeof rule === "string") return rule;
      return { SetRule: { id: collection!.id, rule } };
    }
    const n = name.trim();
    if (!n) return "Type a name.";
    const where = mode === "rename" ? collection?.parent ?? null : parent || null;
    const same = collections.find(
      (c) => c.id !== collection?.id && (c.parent ?? null) === where && c.name.toLowerCase() === n.toLowerCase(),
    );
    if (same) return `“${same.path}” is there already.`;
    if (mode === "rename") return n === collection?.name ? "That is its name already." : { Rename: { id: collection!.id, name: n } };
    if (kind === "Smart" && typeof rule === "string") return rule;
    return {
      Create: { id: "", name: n, kind, parent: parent || null, rule: kind === "Smart" ? (rule as api.SmartRule) : null },
    };
  });

  async function submit(e?: Event) {
    e?.preventDefault();
    if (typeof change === "string" || busy) return;
    busy = true;
    error = null;
    try {
      await ondone(change, fill);
      onclose();
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  const NOUN = { Album: "album", Smart: "smart album", Group: "group", Project: "project", Folder: "folder", Other: "collection" };
  let title = $derived(
    mode === "create" ? `New ${NOUN[kind]}`
    : mode === "rename" ? `Rename ${NOUN[collection?.kind ?? "Other"]}`
    : mode === "rule" ? `Rule of “${collection?.name}”`
    : mode === "move" ? `Move “${collection?.name}”`
    : `Delete ${NOUN[collection?.kind ?? "Other"]}`,
  );
  const ACTION = { create: "Create", rename: "Rename", rule: "Save rule", delete: "Delete", move: "Move" };
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && onclose()} />

<datalist id="smart-keywords">
  {#each keywords as k}<option value={k.slice(k.lastIndexOf("|") + 1)}></option>{/each}
</datalist>

<div class="backdrop" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="album-title">
    <h2 id="album-title">{title}</h2>

    {#if mode === "delete" && collection}
      <p>
        <b>{collection.path}</b>
        {#if doomed.length > 1}and the {doomed.length - 1} album{doomed.length === 2 ? "" : "s"} and group{doomed.length === 2 ? "" : "s"} in it{/if}
        will be deleted from the catalog.
      </p>
      <p class="muted">
        {#if collection.kind === "Smart"}
          A smart album holds no photos of its own: only its rule goes.
        {:else}
          The photos stay in the catalog; only their place in {doomed.length > 1 ? "these albums" : "the album"} goes.
        {/if}
        Queued until you write to Capture One; Ctrl+Z takes it back.
      </p>
      {#if doomed.length > 1}
        <ul class="doomed">
          {#each doomed.slice(1) as c (c.id)}<li>{c.path}</li>{/each}
        </ul>
      {/if}
    {:else}
      <form onsubmit={submit}>
        {#if mode !== "rule" && mode !== "move"}
          <label>
            Name
            <!-- svelte-ignore a11y_autofocus -->
            <input bind:value={name} autofocus spellcheck="false" />
          </label>
        {/if}
        {#if mode === "create" || mode === "move"}
          <label>
            {mode === "move" ? "Into" : "In"}
            <select bind:value={parent}>
              <option value="">the top level</option>
              {#each places as p (p.id)}<option value={p.id}>{p.path}</option>{/each}
            </select>
          </label>
        {/if}
      </form>

      {#if smart}
        <p class="muted">
          A smart album holds the photos its rule selects, now and later; photos are not put into it by hand.
        </p>
        <div class="match" role="radiogroup" aria-label="Match">
          <label><input type="radio" name="match" checked={matchAll} onchange={() => (matchAll = true)} /> all of these</label>
          <label><input type="radio" name="match" checked={!matchAll} onchange={() => (matchAll = false)} /> any of these</label>
        </div>
        {#each conditions as row, i}
          <div class="condition">
            <select bind:value={row.what} aria-label="Condition">
              <option value="keyword">Keyword contains</option>
              <option value="rating">Rating is</option>
              <option value="at_least">Rating is at least</option>
              <option value="year">Captured in year</option>
            </select>
            {#if row.what === "keyword"}
              <input bind:value={row.value} list="smart-keywords" spellcheck="false" placeholder="street" aria-label="Text" />
            {:else if row.what === "year"}
              <input bind:value={row.value} inputmode="numeric" placeholder="2025" aria-label="Year" />
            {:else}
              <select bind:value={row.value} aria-label="Stars">
                {#each [0, 1, 2, 3, 4, 5] as n}<option value={String(n)}>{n ? "★".repeat(n) : "unrated"}</option>{/each}
              </select>
            {/if}
            <button
              class="drop"
              aria-label="Remove this condition"
              disabled={conditions.length === 1}
              onclick={() => (conditions = conditions.filter((_, k) => k !== i))}>×</button
            >
          </div>
        {/each}
        <button class="add" onclick={() => (conditions = [...conditions, { what: "keyword", value: "" }])}>+ Condition</button>
        <p class="muted small">A keyword condition matches any level of a keyword's name, ignoring case: “street” finds street and streets.</p>
      {/if}

      {#if mode === "create" && kind === "Album" && picked > 0}
        <label class="check">
          <input type="checkbox" bind:checked={fill} />
          Put the {picked.toLocaleString()} selected photo{picked === 1 ? "" : "s"} in it
        </label>
      {/if}
      {#if mode !== "delete"}<p class="muted">Queued until you write to Capture One; Ctrl+Z takes it back.</p>{/if}
    {/if}

    {#if typeof change === "string" && (name || mode === "rule" || mode === "move")}<p class="warn">{change}</p>{/if}
    {#if error}<p class="error">{error}</p>{/if}

    <div class="buttons">
      <button onclick={onclose}>Cancel</button>
      <button class="primary" class:danger={mode === "delete"} disabled={typeof change === "string" || busy} onclick={() => submit()}>
        {ACTION[mode]}
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
  input:not([type]), select {
    width: 100%;
    padding: 5px 7px;
    font: inherit;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 3px;
  }
  input:focus, select:focus { outline: none; border-color: var(--queued); }
  input[type="radio"], input[type="checkbox"] { margin: 0; accent-color: var(--queued); }
  .match { display: flex; gap: 16px; margin: 6px 0 4px; }
  .match label, .check { display: flex; align-items: center; gap: 6px; margin: 0; }
  .check { margin-top: 12px; }
  .condition { display: grid; grid-template-columns: 11em 1fr auto; gap: 6px; align-items: center; margin-top: 6px; }
  .condition .drop { padding: 0 6px; color: var(--muted); }
  .condition .drop:hover:not(:disabled) { color: var(--error); }
  .condition .drop:disabled { visibility: hidden; }
  .add { margin-top: 8px; padding: 2px 8px; border: 1px solid var(--line); border-radius: 3px; }
  .add:hover { background: var(--hover); }
  .doomed { max-height: 10em; overflow: auto; margin: 4px 0; padding-left: 18px; color: var(--muted); }
  .buttons { display: flex; justify-content: flex-end; gap: 8px; margin-top: 16px; }
  .buttons button { padding: 4px 12px; border: 1px solid var(--line); border-radius: 4px; }
  .buttons button:hover:not(:disabled) { background: var(--hover); }
  .buttons .primary { border: 0; background: var(--queued); color: #fff; font-weight: 600; }
  .buttons .primary.danger { background: var(--error); }
  .buttons button:disabled { opacity: 0.4; cursor: default; }
  .muted { color: var(--muted); }
  .small { font-size: 12px; }
  .warn { color: #e8b03a; }
  .error { padding: 0; }
  p { margin: 8px 0; }
</style>
