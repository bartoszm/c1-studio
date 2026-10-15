<script lang="ts">
  import { onMount } from "svelte";
  import * as api from "./lib/api";
  import {
    PRESETS, TOKENS, addRoot, excludesToText, gbToMb, mbToGb, renderPattern, storeMoved, textToExcludes, withPreset,
  } from "./lib/folders";

  // A photo folders source: its name, the folders it holds, what it leaves out, where its
  // thumbnails and previews are kept, and whether edits are also written as XMP. Nothing is
  // saved until Save. For the source that is open, saving reads it again and walks the folders.
  let { at = null, onclose, onsaved }: {
    /** The definition file to edit; none for a new source. */
    at?: string | null;
    onclose: () => void;
    /** Saved. `opened`: it was the open source, which reads the changes by itself. */
    onsaved: (opened: boolean) => void;
  } = $props();

  let def = $state<api.FoldersDefinition | null>(null);
  /** The image store as it was when the dialog opened, to tell whether it changed. */
  let before = $state<api.ImageStore | null>(null);
  let isOpen = $state(false);
  let excludes = $state("");
  let limit = $state("");
  let problem = $state<string | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  /** Asking what to do with the images already made, before saving. */
  let asking = $state(false);
  let asked = 0;

  onMount(async () => {
    try {
      const view = at ? await api.foldersLoad(at) : await api.foldersNew("", []);
      def = view.definition;
      before = structuredClone($state.snapshot(view.definition.images));
      isOpen = view.is_open;
      excludes = excludesToText(view.definition.excludes);
      limit = mbToGb(view.definition.images.central_limit_mb);
    } catch (e) {
      error = String(e);
    }
  });

  let limitMb = $derived(gbToMb(limit));
  let moved = $derived(isOpen && !!def && !!before && storeMoved(before, def.images));
  let canSave = $derived(
    !!def && !!def.name.trim() && def.roots.length > 0 && !problem && limitMb !== undefined && !busy,
  );
  let preset = $derived(PRESETS.find((p) => p.id === def?.images.preset));

  // The image store is checked a moment after it stops changing.
  $effect(() => {
    if (!def) return;
    const store = { ...$state.snapshot(def.images), central_limit_mb: limitMb ?? null };
    const mine = ++asked;
    const timer = setTimeout(() => {
      api.foldersCheckStore(store)
        .then((p) => { if (mine === asked) problem = p; })
        .catch((e) => { if (mine === asked) problem = String(e); });
    }, 350);
    return () => clearTimeout(timer);
  });

  function setPreset(id: api.Preset) {
    if (def) def.images = withPreset($state.snapshot(def.images), id);
  }

  async function add() {
    error = null;
    try {
      const path = await api.pickFolder();
      if (path && def) {
        const id = () => crypto.randomUUID().replaceAll("-", "").slice(0, 8);
        def.roots = addRoot($state.snapshot(def.roots), path, id);
        if (!def.name.trim() && def.roots.length === 1) def.name = def.roots[0].path.split(/[\\/]/).filter(Boolean).pop() ?? "";
      }
    } catch (e) {
      error = String(e);
    }
  }

  function remove(id: string) {
    if (def) def.roots = def.roots.filter((r) => r.id !== id);
  }

  async function save(migrate: api.Migrate | null) {
    if (!def || !canSave) return;
    busy = true;
    error = null;
    try {
      const out: api.FoldersDefinition = $state.snapshot(def);
      out.name = out.name.trim();
      out.excludes = textToExcludes(excludes);
      out.images.central_limit_mb = limitMb ?? null;
      out.roots = out.roots.map((r) => ({ ...r, label: r.label?.trim() || null }));
      if (!out.write_xmp) out.modify_originals = false;
      const path = await api.foldersSave(out, at, migrate);
      if (isOpen) {
        onsaved(true);
        onclose();
      } else {
        await api.openSource({ provider: "folders", location: path });
        location.reload();
      }
    } catch (e) {
      error = String(e);
      asking = false;
      busy = false;
    }
  }

  const submit = () => (moved ? (asking = true) : save(null));

  // Focus goes into the dialog, stays in it, and goes back where it was on closing.
  let dialog: HTMLDivElement;
  const FOCUSABLE = "input:not(:disabled), textarea, button:not(:disabled)";
  onMount(() => {
    const before = document.activeElement;
    dialog.querySelector<HTMLElement>(FOCUSABLE)?.focus();
    return () => { if (before instanceof HTMLElement) before.focus(); };
  });
  function onkey(e: KeyboardEvent) {
    if (e.key === "Escape") return onclose();
    if (e.key !== "Tab") return;
    const all = [...dialog.querySelectorAll<HTMLElement>(FOCUSABLE)];
    if (!all.length) return;
    const here = all.indexOf(document.activeElement as HTMLElement);
    const next = e.shiftKey ? (here <= 0 ? all.length - 1 : here - 1) : (here < 0 || here === all.length - 1 ? 0 : here + 1);
    all[next].focus();
    e.preventDefault();
  }
</script>

<svelte:window onkeydown={onkey} />

<div class="backdrop" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="folders-title" bind:this={dialog}>
    <h2 id="folders-title">{at ? "Photo folders" : "New photo folders"}</h2>

    {#if def}
      <label class="field">
        <span>Name</span>
        <input type="text" placeholder="Family archive" spellcheck="false" bind:value={def.name} />
      </label>

      <h3>Folders</h3>
      <p class="muted small">
        Every photo in these folders and the ones below them is listed. Removing a folder keeps its photos' edits here;
        they come back if it is added again.
      </p>
      <ul class="roots">
        {#each def.roots as r (r.id)}
          <li>
            <span class="path" title={r.path}>{r.path}</span>
            <input type="text" class="label" placeholder="label (optional)" aria-label="Label for {r.path}" bind:value={r.label} />
            <button onclick={() => remove(r.id)} aria-label="Remove {r.path}">Remove</button>
          </li>
        {/each}
        {#if !def.roots.length}<li class="muted">No folders yet.</li>{/if}
      </ul>
      <button class="add" onclick={add}>Add folder…</button>

      <h3>Leave out</h3>
      <textarea rows="3" spellcheck="false" aria-label="Excluded paths" placeholder={"**/tmp/**\n*.tmp"} bind:value={excludes}></textarea>
      <p class="muted small">One pattern per line, matched against the path below its folder. A folder that matches is not entered.</p>

      <h3>Thumbnails and previews</h3>
      <fieldset>
        <legend class="sr">Where they are kept</legend>
        {#each PRESETS as p (p.id)}
          <label class="radio">
            <input type="radio" name="preset" checked={def.images.preset === p.id} onchange={() => setPreset(p.id)} />
            <span><b>{p.label}</b> <small class="muted">{p.about}</small></span>
          </label>
        {/each}
      </fieldset>
      {#if def.images.preset === "Custom"}
        <label class="field">
          <span>Thumbnails</span>
          <input type="text" spellcheck="false" bind:value={def.images.thumb} />
        </label>
        <label class="field">
          <span>Previews</span>
          <input type="text" spellcheck="false" bind:value={def.images.preview} />
        </label>
        <p class="muted small tokens">
          {#each TOKENS as [token, what]}<code>{token}</code> {what}; {/each}
        </p>
      {/if}
      <p class="muted small example">
        For <code>D:/Photos/Family/2024/Trip/DSCF0001.RAF</code>:<br />
        <code>{renderPattern(def.images.thumb, "thumb")}</code><br />
        <code>{renderPattern(def.images.preview, "preview")}</code>
      </p>
      {#if problem}<p class="error">{problem}</p>{/if}
      {#if def.images.preset === "Central"}
        <label class="field inline">
          <span>Size limit</span>
          <input class="short" type="text" inputmode="decimal" placeholder="none" aria-invalid={limitMb === undefined} bind:value={limit} />
          <span>GB <small class="muted">Least recently used images go first. Empty: no limit.</small></span>
        </label>
      {:else if preset}
        <p class="muted small">These images are yours: they are never trimmed.</p>
      {/if}

      <h3>XMP</h3>
      <fieldset>
        <legend class="small muted">Sidecar file name</legend>
        <label class="radio">
          <input type="radio" name="sidecar" checked={def.sidecar === "Stem"} onchange={() => (def!.sidecar = "Stem")} />
          <span><code>{"{stem}"}.xmp</code> <small class="muted">Lightroom, Capture One</small></span>
        </label>
        <label class="radio">
          <input type="radio" name="sidecar" checked={def.sidecar === "Name"} onchange={() => (def!.sidecar = "Name")} />
          <span><code>{"{name}"}.xmp</code> <small class="muted">darktable</small></span>
        </label>
      </fieldset>
      <label class="option">
        <input type="checkbox" bind:checked={def.write_xmp} />
        <span>
          Write XMP
          <small class="muted">
            export: Lightroom reads it only on import or Read Metadata from Files. Edits are kept here either way.
          </small>
        </span>
      </label>
      <label class="option" class:off={!def.write_xmp}>
        <input type="checkbox" disabled={!def.write_xmp} bind:checked={def.modify_originals} />
        <span>
          Allow changing original JPEG, TIFF and DNG files
          <small class="muted">The XMP is then written into those files; for other files it goes to a sidecar. Off: originals are never touched.</small>
        </span>
      </label>
    {:else if !error}
      <p class="muted">Loading…</p>
    {/if}

    {#if asking}
      <p class="ask">
        The place for thumbnails and previews changed. Move the ones already made, or make them again as photos are shown?
      </p>
      <div class="buttons">
        <button onclick={() => (asking = false)} disabled={busy}>Back</button>
        <button onclick={() => save("Regenerate")} disabled={busy}>Make them again</button>
        <button class="primary" onclick={() => save("Move")} disabled={busy}>Move existing images</button>
      </div>
    {:else}
      <div class="buttons">
        <button onclick={onclose}>Cancel</button>
        <button class="primary" disabled={!canSave} onclick={submit}>{busy ? "Saving…" : isOpen ? "Save" : "Save and open"}</button>
      </div>
    {/if}
    {#if error}<p class="error">{error}</p>{/if}
  </div>
</div>

<style>
  .backdrop { position: fixed; inset: 0; z-index: 20; display: grid; place-items: center; padding: 16px; background: rgb(0 0 0 / 55%); }
  .dialog { width: min(560px, 100%); max-height: calc(100vh - 32px); overflow: auto; padding: 18px 20px; background: var(--panel); border: 1px solid var(--line); border-radius: 6px; box-shadow: 0 12px 40px rgb(0 0 0 / 50%); }
  h2 { margin: 0 0 10px; font-size: 16px; font-weight: 600; }
  h3 { margin: 16px 0 4px; font-size: 13px; font-weight: 600; }
  p { margin: 4px 0 8px; }
  .muted { color: var(--muted); }
  .small { font-size: 12px; }
  code { font-size: 12px; overflow-wrap: anywhere; }
  .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }
  fieldset { margin: 0; padding: 0; border: 0; }
  input[type="text"], textarea {
    padding: 3px 6px; font: inherit; color: var(--text); background: var(--bg);
    border: 1px solid var(--line); border-radius: 3px;
  }
  input[type="text"]:focus, textarea:focus { outline: none; border-color: var(--queued); }
  textarea { width: 100%; box-sizing: border-box; resize: vertical; font-family: inherit; }
  [aria-invalid="true"] { border-color: var(--error) !important; }
  .field { display: grid; grid-template-columns: 6em 1fr; align-items: center; gap: 8px; margin: 6px 0; }
  .field.inline { grid-template-columns: 6em auto 1fr; }
  .short { width: 5em; text-align: right; }
  .roots { margin: 0 0 8px; padding: 0; list-style: none; }
  .roots li { display: flex; align-items: center; gap: 8px; padding: 3px 0; border-top: 1px solid var(--line); }
  .roots .path { flex: 1; min-width: 0; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; direction: rtl; text-align: left; }
  .roots .label { width: 9em; }
  .roots button, .add { padding: 2px 10px; border: 1px solid var(--line); border-radius: 4px; }
  .roots button:hover, .add:hover { background: var(--hover); }
  .radio, .option { display: flex; align-items: flex-start; gap: 8px; margin: 3px 0; cursor: pointer; }
  .radio input, .option input { margin-top: 3px; accent-color: var(--queued); }
  .option { margin-top: 8px; }
  .option.off { opacity: 0.5; }
  small { font-size: 12px; }
  .option small { display: block; margin-top: 2px; }
  .tokens { line-height: 1.6; }
  .example { margin-top: 8px; }
  .ask { margin-top: 16px; }
  .error { padding: 0; }
  .buttons { display: flex; justify-content: flex-end; gap: 8px; margin-top: 16px; }
  .buttons button { padding: 4px 12px; border: 1px solid var(--line); border-radius: 4px; }
  .buttons button:hover:not(:disabled) { background: var(--hover); }
  .buttons .primary { border: 0; background: var(--queued); color: #fff; font-weight: 600; }
  .buttons button:disabled { opacity: 0.4; cursor: default; }
</style>
