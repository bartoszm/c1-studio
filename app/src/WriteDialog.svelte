<script lang="ts">
  import * as api from "./lib/api";
  import { opText } from "./lib/format";
  import { source } from "./lib/source.svelte";

  // The queue of edits, checked against the source's latest state, and the one button
  // that writes it. A Capture One catalog is written only while Capture One is closed;
  // everything else works while it runs.
  let { onclose, onchanged }: {
    onclose: () => void;
    /** The queue or the library changed: the grid and counts should refresh. */
    onchanged: () => void;
  } = $props();

  const SHOWN = 200;

  let plan = $state<api.WritePlan | null>(null);
  let error = $state<string | null>(null);
  let busy = $state<string | null>(null);
  let done = $state<api.WriteReport | null>(null);
  let confirmDiscard = $state(false);

  async function load() {
    busy = `Checking ${source.word}…`;
    error = null;
    try {
      plan = await api.writePlan();
    } catch (e) {
      error = String(e);
    } finally {
      busy = null;
    }
  }
  load();

  /** How the source's own side of a conflict reads. */
  let theirs_ = $derived(source.owner ? `${source.owner}'s` : "the source's");
  let locked = $derived(plan && typeof plan.status === "object" ? plan.status.Locked.by : null);
  let writable = $derived(plan?.status === "Ready");
  let toWrite = $derived(plan ? plan.remaps.length + plan.collections.length + plan.apply.length + plan.unchanged.length : 0);
  /** Album names, for edits that put photos into albums. */
  let albumPaths = $state(new Map<string, string>());
  api.collections().then((all) => (albumPaths = new Map(all.map((c) => [c.id, c.path])))).catch(() => {});
  const albumName = (id: string) => `“${albumPaths.get(id) ?? id}”`;
  let structural = $derived(
    plan ? plan.remaps.length + plan.blocked_remaps.length + plan.collections.length + plan.blocked_collections.length : 0,
  );
  let canWrite = $derived(!!plan && writable && plan.conflicts.length === 0 && toWrite > 0 && !busy);
  let everything = $derived(
    plan ? [...plan.apply, ...plan.unchanged, ...plan.conflicts, ...plan.blocked.map(([p]) => p)] : [],
  );

  async function write() {
    busy = `Backing up the ${source.noun} and writing…`;
    error = null;
    try {
      done = await api.writeApply();
      onchanged();
    } catch (e) {
      error = String(e);
    } finally {
      busy = null;
    }
    if (!done) await load();
  }

  async function act(what: () => Promise<void>) {
    busy = "Updating…";
    error = null;
    try {
      await what();
      onchanged();
    } catch (e) {
      error = String(e);
    } finally {
      busy = null;
    }
    await load();
  }

  const resolve = (keys: string[], keepMine: boolean) => act(() => api.resolve(keys, keepMine));
  const discard = (keys: string[]) => act(() => api.discard(keys));
  const dropRemaps = (seqs: number[]) =>
    act(async () => {
      for (const seq of seqs) await api.undoRemap(seq);
    });
  const dropCollections = (seqs: number[]) =>
    act(async () => {
      for (const seq of seqs.sort((a, b) => b - a)) await api.undoCollection(seq);
    });
  /** Undo everything queued: album and keyword tree changes newest first, then the edits. */
  const undoAll = () =>
    act(async () => {
      const albums = [...(plan?.collections ?? []).map(([s]) => s), ...(plan?.blocked_collections ?? []).map(([s]) => s)];
      for (const seq of albums.sort((a, b) => b - a)) await api.undoCollection(seq);
      const seqs = [...(plan?.remaps ?? []).map(([s]) => s), ...(plan?.blocked_remaps ?? []).map(([s]) => s)];
      for (const seq of seqs.sort((a, b) => b - a)) await api.undoRemap(seq);
      // Taking back a new album takes the photos put into it: discard only what is left.
      const left = new Set((await api.pending()).map((p) => p.key));
      await api.discard(everything.map((p) => p.key).filter((k) => left.has(k)));
    });

  function onkey(e: KeyboardEvent) {
    if (e.key === "Escape") onclose();
  }
</script>

<svelte:window onkeydown={onkey} />

{#snippet item(p: api.PendingEdit, note?: string)}
  {@const t = opText(p.change.op, albumName)}
  <li>
    {#if p.row !== null}<img src={api.thumbUrl(p.row)} alt="" />{:else}<span class="gone"></span>{/if}
    <span class="what">
      <span class="name">{p.name ?? p.change.photo.ext}</span>
      {#if t}{t.field}: {#if t.from}<span class="from">{t.from}</span> → {/if}<b>{t.to}</b>{/if}
      {#if note}<span class="note">{note}</span>{/if}
    </span>
  </li>
{/snippet}

<div class="backdrop" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="write-title">
    <h2 id="write-title">Write to {source.word}</h2>

    {#if done}
      <p class="ok">Written: {done.written.toLocaleString()} edit{done.written === 1 ? "" : "s"}.</p>
      {#if done.backup}<p class="muted">The {source.noun} as it was is backed up in <code>{done.backup}</code>.</p>{/if}
      {#if done.warnings.length}
        <p class="warn">The edits are written, but:</p>
        <ul class="labels">
          {#each done.warnings as w}<li>{w}</li>{/each}
        </ul>
      {/if}
      <p class="muted">
        {#if source.owner}Open {source.owner} to see the changes there. {/if}To have edits written by themselves, turn on Autosave at the bottom right.
      </p>
      <div class="buttons"><button class="primary" onclick={onclose}>Done</button></div>
    {:else if plan}
      <p class:warn={!writable}>
        {#if writable}
          {#if source.owner}{source.owner} is closed. {/if}The {source.noun} is backed up before anything is written.
        {:else if locked}
          Close {source.word} to write: it keeps the {source.noun} in memory and would overwrite these changes.
          <span class="muted">(Held by {locked}.)</span> Your edits stay queued meanwhile.
        {:else}
          This {source.noun} cannot be written. <span class="muted">{plan.compatibility.format}.</span>
          {plan.compatibility.note ?? ""} Your edits stay queued here.
        {/if}
      </p>

      {#if plan.conflicts.length}
        <section>
          <h3>
            Changed in {source.word} too ({plan.conflicts.length})
            <span class="bulk">
              <button onclick={() => resolve(plan!.conflicts.map((p) => p.key), true)}>Keep all mine</button>
              <button onclick={() => resolve(plan!.conflicts.map((p) => p.key), false)}>Take all from {source.word}</button>
            </span>
          </h3>
          <ul>
            {#each plan.conflicts.slice(0, SHOWN) as p (p.key)}
              {@const mine = opText(p.change.op)}
              {@const theirs = p.theirs && opText(p.theirs)}
              <li>
                {#if p.row !== null}<img src={api.thumbUrl(p.row)} alt="" />{/if}
                <span class="what">
                  <span class="name">{p.name ?? p.change.photo.ext}</span>
                  {mine?.field}: yours <b>{mine?.to}</b>, {theirs_} <b>{theirs?.to}</b>
                  <span class="note">(both changed from {mine?.from})</span>
                </span>
                <span class="choose">
                  <button onclick={() => resolve([p.key], true)}>Keep mine</button>
                  <button onclick={() => resolve([p.key], false)}>Take {theirs_}</button>
                </span>
              </li>
            {/each}
          </ul>
        </section>
      {/if}

      {#if plan.remaps.length}
        <section>
          <h3>Keyword changes ({plan.remaps.length}), written first</h3>
          <ul class="labels">
            {#each plan.remaps as [seq, label] (seq)}<li>{label}</li>{/each}
          </ul>
        </section>
      {/if}

      {#if plan.blocked_remaps.length}
        <section>
          <h3>Keyword changes that cannot be written ({plan.blocked_remaps.length})</h3>
          <ul class="labels">
            {#each plan.blocked_remaps as [seq, label, why] (seq)}<li>{label} <span class="note">{why}</span></li>{/each}
          </ul>
          <button onclick={() => dropRemaps(plan!.blocked_remaps.map(([s]) => s))}>Drop these</button>
        </section>
      {/if}

      {#if plan.collections.length}
        <section>
          <h3>Album changes ({plan.collections.length}), written before photo edits</h3>
          <ul class="labels">
            {#each plan.collections as [seq, label] (seq)}<li>{label}</li>{/each}
          </ul>
        </section>
      {/if}

      {#if plan.blocked_collections.length}
        <section>
          <h3>Album changes that cannot be written ({plan.blocked_collections.length})</h3>
          <ul class="labels">
            {#each plan.blocked_collections as [seq, label, why] (seq)}<li>{label} <span class="note">{why}</span></li>{/each}
          </ul>
          <button onclick={() => dropCollections(plan!.blocked_collections.map(([s]) => s))}>Drop these</button>
        </section>
      {/if}

      {#if plan.apply.length}
        <section>
          <h3>To write ({plan.apply.length.toLocaleString()})</h3>
          <ul>
            {#each plan.apply.slice(0, SHOWN) as p (p.key)}{@render item(p)}{/each}
          </ul>
          {#if plan.apply.length > SHOWN}<p class="muted">and {plan.apply.length - SHOWN} more</p>{/if}
        </section>
      {/if}

      {#if plan.blocked.length}
        <section>
          <h3>Cannot be written ({plan.blocked.length})</h3>
          <ul>
            {#each plan.blocked.slice(0, SHOWN) as [p, why] (p.key)}{@render item(p, why)}{/each}
          </ul>
          <button onclick={() => discard(plan!.blocked.map(([p]) => p.key))}>Drop these</button>
        </section>
      {/if}

      {#if plan.unchanged.length}
        <p class="muted">
          {plan.unchanged.length} edit{plan.unchanged.length === 1 ? " matches" : "s match"} {source.word} already and
          will leave the queue.
        </p>
      {/if}

      {#if !everything.length && !structural}
        <p class="muted">Nothing is waiting to be written.</p>
      {/if}

      <div class="buttons">
        {#if everything.length || structural}
          {#if confirmDiscard}
            <span class="warn">Undo everything queued here?</span>
            <button onclick={() => undoAll().then(onclose)}>Yes, undo it</button>
            <button onclick={() => (confirmDiscard = false)}>No</button>
          {:else}
            <button class="left" onclick={() => (confirmDiscard = true)}>Undo all…</button>
          {/if}
        {/if}
        {#if !writable}<button onclick={load} disabled={!!busy}>Check again</button>{/if}
        <button onclick={onclose}>Close</button>
        <button class="primary" disabled={!canWrite} onclick={write}>
          Write {toWrite.toLocaleString()} edit{toWrite === 1 ? "" : "s"}
        </button>
      </div>
    {/if}

    {#if busy}<p class="muted">{busy}</p>{/if}
    {#if error}<p class="error">{error}</p>{/if}
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
    width: min(640px, 100%);
    max-height: calc(100vh - 32px);
    overflow: auto;
    padding: 18px 20px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 6px;
    box-shadow: 0 12px 40px rgb(0 0 0 / 50%);
  }
  h2 { margin: 0 0 10px; font-size: 16px; font-weight: 600; }
  h3 { display: flex; align-items: center; gap: 8px; margin: 16px 0 6px; font-size: 12px; font-weight: 600; }
  .bulk { margin-left: auto; display: flex; gap: 6px; }
  ul { margin: 0; padding: 0; list-style: none; }
  li { display: flex; align-items: center; gap: 10px; padding: 3px 0; }
  li img, .gone { width: 48px; height: 32px; object-fit: contain; background: var(--bg); border-radius: 2px; flex: none; }
  .what { flex: 1; min-width: 0; }
  .from { color: var(--muted); }
  .labels li { display: block; padding: 2px 0; }
  .name { display: inline-block; min-width: 110px; margin-right: 8px; color: var(--muted); }
  .note { margin-left: 6px; color: var(--muted); }
  .choose { display: flex; gap: 6px; flex: none; }
  .buttons { display: flex; align-items: center; justify-content: flex-end; gap: 8px; margin-top: 18px; }
  .buttons .left { margin-right: auto; }
  button:not(.primary) { padding: 4px 10px; border: 1px solid var(--line); border-radius: 4px; }
  button:not(.primary):hover:not(:disabled) { background: var(--hover); }
  .primary { padding: 5px 14px; border-radius: 4px; background: var(--queued); color: #fff; font-weight: 600; }
  .primary:disabled, button:disabled { opacity: 0.4; cursor: default; }
  .muted { color: var(--muted); }
  .warn { color: #e8b03a; }
  .ok { color: #7cc47c; }
  .error { padding: 0; }
  code { overflow-wrap: anywhere; font-size: 11px; }
  p { margin: 8px 0; }
</style>
