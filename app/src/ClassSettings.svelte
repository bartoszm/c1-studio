<script lang="ts">
  import { onMount } from "svelte";
  import * as api from "./lib/api";
  import { GROUPS, sameSettings, shown, summary, withField } from "./lib/classes";

  // Where the analysis draws the lines between classes (what counts as red, as soft, as a
  // burst). Moving a value previews how many photos each class would get; Apply classes the
  // measured photos again. Nothing is measured again.
  let { onapplied }: { onapplied: () => void } = $props();

  let saved = $state<api.ClassSettings | null>(null);
  let draft = $state<api.ClassSettings | null>(null);
  let preview = $state<api.ClassPreview | null>(null);
  let busy = $state(false);
  let error = $state("");
  let timer: ReturnType<typeof setTimeout> | undefined;

  let changed = $derived(!!saved && !!draft && !sameSettings(saved, draft));

  onMount(() => {
    (async () => {
      try {
        saved = await api.classSettings();
        draft = { ...saved };
        preview = await api.previewClasses(saved);
      } catch (e) {
        error = String(e);
      }
    })();
    return () => clearTimeout(timer);
  });

  // The preview follows the draft a moment after the last keystroke.
  function edit(next: api.ClassSettings) {
    draft = next;
    clearTimeout(timer);
    timer = setTimeout(async () => {
      try {
        const asked = next;
        const got = await api.previewClasses(asked);
        if (draft === asked) preview = got;
      } catch (e) {
        error = String(e);
      }
    }, 150);
  }

  async function apply() {
    if (!draft || busy) return;
    busy = true;
    error = "";
    try {
      saved = await api.setClassSettings(draft);
      draft = { ...saved };
      preview = await api.previewClasses(saved);
      onapplied();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function reset() {
    try {
      edit(await api.defaultClassSettings());
    } catch (e) {
      error = String(e);
    }
  }
</script>

<p class="muted intro">
  Where a photo counts as a colour, as dark, as soft or as part of a burst. The numbers show how many of the
  {preview?.measured ?? 0} measured photos each setting would give; Apply sorts them again without measuring.
</p>

{#if draft}
  <div class="grid">
  {#each GROUPS.filter((g) => g.title !== "Alike") as group (group.title)}
    <fieldset>
      <legend>{group.title}</legend>
      {#each group.fields as f (f.key)}
        <label class="row">
          <span class="name">{f.label}</span>
          <input
            type="number"
            min={f.min}
            max={f.max}
            step={f.step}
            value={shown(f, draft)}
            oninput={(e) => {
              const v = e.currentTarget.value;
              edit(withField(draft!, f, v === "" ? null : Number(v)));
            }}
          />
          <span class="unit">{f.unit}</span>
          {#if f.hint}<small class="muted">{f.hint}</small>{/if}
        </label>
      {/each}
      <p class="preview" aria-live="polite">{summary(group, preview)}</p>
    </fieldset>
  {/each}
  </div>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  <div class="actions">
    <button disabled={busy} onclick={reset}>Defaults</button>
    <button disabled={!changed || busy} onclick={apply}>{busy ? "Applying…" : "Apply"}</button>
  </div>
{:else if error}
  <p class="error" role="alert">{error}</p>
{:else}
  <p class="muted">Open a library to set these.</p>
{/if}

<style>
  p { margin: 4px 0 8px; }
  .muted { color: var(--muted); font-size: 12px; }
  .intro { margin-top: 0; }
  .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(330px, 1fr)); gap: 10px; align-items: start; }
  fieldset { margin: 0; min-width: 0; padding: 6px 10px 8px; border: 1px solid var(--line); border-radius: 4px; }
  legend { padding: 0 4px; font-size: 12px; font-weight: 600; }
  .row { display: grid; grid-template-columns: 1fr 78px 40px; align-items: center; gap: 2px 6px; padding: 3px 0; }
  .row small { grid-column: 1 / -1; }
  .name { font-size: 13px; }
  .unit { color: var(--muted); font-size: 12px; }
  input[type="number"] { width: 100%; box-sizing: border-box; text-align: right; }
  .preview { margin: 4px 0 0; color: var(--muted); font-size: 12px; min-height: 1.2em; }
  .error { color: var(--danger, #e5484d); font-size: 12px; }
  .actions { position: sticky; bottom: 0; display: flex; justify-content: flex-end; gap: 8px; padding: 8px 0 2px; background: var(--panel); }
  .actions button { padding: 3px 10px; border: 1px solid var(--line); border-radius: 4px; }
  .actions button:hover:not(:disabled) { background: var(--hover); }
  button:disabled { opacity: 0.35; cursor: default; }
</style>
