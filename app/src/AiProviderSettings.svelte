<script lang="ts">
  import { onMount, untrack } from "svelte";
  import * as api from "./lib/api";
  import { source } from "./lib/source.svelte";
  import type { AiImage, AiView } from "./lib/api";

  // How photos are described (the AI tab): Claude through the API with a key or through
  // Claude Code on a Claude plan, the model, the captions' language, effort, photos at
  // once. Kept per catalog; applied as changed.
  let view = $state<AiView | null>(null);
  let keyInput = $state("");
  let error = $state("");
  let busy = $state(false);

  async function run(call: () => Promise<AiView>) {
    busy = true;
    error = "";
    try {
      view = await call();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
  onMount(() => run(api.aiSettings).then(() => (draft = view?.settings.instructions ?? view?.default_instructions ?? "")));
  const save = (change: Partial<api.AiSettings>) => view && run(() => api.setAiSettings({ ...view!.settings, ...change }));
  const saveKey = (key: string) => run(() => api.setAiKey(key)).then(() => (keyInput = ""));
  const money = (usd: number) => (usd < 0.01 && usd > 0 ? `$${usd.toFixed(4)}` : `$${usd.toFixed(2)}`);

  // ---- what a description holds -------------------------------------------------------
  const FIELDS: { key: keyof api.AiFields; label: string; hint: string }[] = [
    { key: "caption", label: "Caption", hint: "two or three sentences, in the language below" },
    { key: "headline", label: "Headline", hint: "at most five words" },
    { key: "subject", label: "Subject", hint: "the main subject, shown on the card" },
    { key: "keywords", label: "Keywords", hint: `chosen from the ${source.noun}'s own` },
    { key: "keywords_new", label: "New keywords", hint: `at most three the ${source.noun} lacks` },
    { key: "rating", label: "Rating", hint: "1 to 5 stars, never taken unless ticked" },
    { key: "scores", label: "Quality and composition", hint: "1 to 5 each, shown on the card" },
    { key: "place", label: "Place", hint: "country, region, city, location and a position, each with how likely it is" },
  ];
  function toggleField(key: keyof api.AiFields, on: boolean) {
    if (!view) return;
    save({ fields: { ...view.settings.fields, [key]: on } });
  }

  // ---- the prompt -------------------------------------------------------------------
  let showPrompt = $state(false);
  let draft = $state("");
  let own = $derived(!!view?.settings.instructions);
  let edited = $derived(!!view && draft.trim() !== (view.settings.instructions ?? view.default_instructions).trim());
  let preview = $state<api.AiPrompt | null>(null);
  async function saveInstructions(text: string | null) {
    await run(() => api.setAiSettings({ ...view!.settings, instructions: text }));
    draft = view?.settings.instructions ?? view?.default_instructions ?? "";
  }
  async function togglePreview() {
    if (preview) return (preview = null);
    try {
      preview = await api.aiPrompt();
    } catch (e) {
      error = String(e);
    }
  }
  // The preview follows the settings it shows (read untracked: it is what this sets).
  $effect(() => {
    void view?.settings;
    if (untrack(() => preview)) api.aiPrompt().then((p) => (preview = p)).catch(() => {});
  });
</script>

<h3>Describing photos</h3>
{#if !view}
  <p class="muted">{error || "Loading…"}</p>
{:else}
  <div class="grid">
    <label for="ai-access">Access</label>
    <div>
      <select id="ai-access" value={view.settings.access} disabled={busy} onchange={(e) => save({ access: e.currentTarget.value as api.AiAccess })}>
        <option value="api">Anthropic API key: billed per token</option>
        <option value="plan">Claude plan, through Claude Code</option>
        <option value="gemini">Gemini, through agy (Antigravity CLI)</option>
      </select>
    </div>

    {#if view.settings.access === "gemini"}
      <span class="label">agy</span>
      <div>
        {#if view.gemini && view.gemini_error}
          <span class="warn">Installed, but {view.gemini_error}</span>
        {:else if view.gemini}
          <span>Installed{view.models.length ? `: ${view.models.length} Gemini models` : ""}</span>
        {:else}
          <span class="warn">Not found. Install the Antigravity CLI and run <code>agy</code> once to sign in.</span>
        {/if}
        <button class="small-btn" disabled={busy} onclick={() => run(api.aiSettings)}>Check again</button>
        <small class="muted">Runs count toward the Google account's Gemini quota.</small>
      </div>

      <label for="ai-sheets">Send</label>
      <div>
        <select id="ai-sheets" value={view.settings.sheets ? "sheets" : "photo"} disabled={busy} onchange={(e) => save({ sheets: e.currentTarget.value === "sheets" })}>
          <option value="photo">One photo per call</option>
          <option value="sheets">Contact sheets: 36 photos per call, numbered</option>
        </select>
        <small class="muted">Sheets take far fewer calls for a first pass over many photos; each photo is seen smaller.</small>
      </div>
    {/if}

    {#if view.settings.access === "gemini"}
      <!-- agy needs no key -->
    {:else if view.settings.access === "plan"}
      <span class="label">Claude Code</span>
      <div>
        {#if busy && !view.plan}
          <span class="muted">Asking Claude Code…</span>
        {:else if !view.plan?.installed}
          <span class="warn">Not installed. Install Claude Code and run <code>claude</code> once to sign in.</span>
        {:else if view.plan.error}
          <span class="warn">{view.plan.error}</span>
        {:else if view.plan.loggedIn && view.plan.authMethod !== "claude.ai"}
          <span class="warn">
            Signed in with {view.plan.authMethod ?? "something other than a plan"}, which is billed per token, not a Claude plan.
            Run <code>claude auth login</code> with your plan, or choose API key access.
          </span>
        {:else if view.plan.loggedIn}
          <span>Signed in{view.plan.subscriptionType ? ` with Claude ${view.plan.subscriptionType}` : ""}</span>
        {:else}
          <span class="warn">Not signed in. Run <code>claude auth login</code> in a terminal.</span>
        {/if}
        <button class="small-btn" disabled={busy} onclick={() => run(api.aiSettings)}>Check again</button>
        <small class="muted">Runs count toward the plan's usage limits; each photo is one Claude Code call.</small>
      </div>
    {:else}
      <label for="ai-key">API key</label>
      <div>
        {#if view.key}
          <code>{view.key[0]}</code>
          <span class="muted">{view.key[1] === "environment" ? "from ANTHROPIC_API_KEY" : "in the Windows Credential Manager"}</span>
          {#if view.key[1] === "stored"}<button class="small-btn" disabled={busy} onclick={() => saveKey("")}>Remove</button>{/if}
        {:else}
          <input id="ai-key" type="password" placeholder="sk-ant-…" bind:value={keyInput} autocomplete="off" />
          <button class="small-btn" disabled={busy || !keyInput.trim()} onclick={() => saveKey(keyInput)}>Save</button>
          <small class="muted">Kept in the Windows Credential Manager, never in a file.</small>
        {/if}
      </div>
    {/if}

    <label for="ai-model">Model</label>
    <div>
      <select
        id="ai-model"
        value={view.settings.access === "gemini" ? view.settings.gemini_model : view.settings.model}
        disabled={busy}
        onchange={(e) => save(view?.settings.access === "gemini" ? { gemini_model: e.currentTarget.value } : { model: e.currentTarget.value })}
      >
        {#if view.settings.access === "gemini" && !view.models.some((m) => m.id === view?.settings.gemini_model)}
          <option value={view.settings.gemini_model}>{view.settings.gemini_model}</option>
        {/if}
        {#each view.models as m (m.id)}
          <option value={m.id}>{m.name}{view.settings.access === "api" ? ` · $${m.input} / $${m.output} per M tokens` : ""}</option>
        {/each}
      </select>
    </div>

    <label for="ai-image">Photo sent</label>
    <div>
      <select id="ai-image" value={view.settings.image} disabled={busy} onchange={(e) => save({ image: e.currentTarget.value as AiImage })}>
        <option value="thumb">Thumbnail: the {source.noun}'s own render of the edits, about 450 pixels</option>
        <option value="preview">Preview: up to 1568 pixels, more detail</option>
      </select>
      <small class="muted">
        {view.settings.image === "thumb"
          ? "Where a photo has none, its preview goes."
          : "A Capture One preview gets its colours matched here, and they can be off. Where a photo has none, its thumbnail goes."}
      </small>
    </div>

    <label for="ai-language">Captions</label>
    <div>
      <select id="ai-language" value={view.settings.language} disabled={busy} onchange={(e) => save({ language: e.currentTarget.value as api.AiLanguage })}>
        <option value="en">English</option>
        <option value="pl">Polish</option>
        <option value="both">English and Polish</option>
      </select>
    </div>

    <label for="ai-effort">Effort</label>
    <div>
      <select id="ai-effort" value={view.settings.effort} disabled={busy} onchange={(e) => save({ effort: e.currentTarget.value as api.AiSettings["effort"] })}>
        <option value="low">Low: quick and cheap</option>
        <option value="medium">Medium</option>
        <option value="high">High: thinks longer, costs more</option>
      </select>
    </div>

    <label for="ai-parallel">At once</label>
    <div>
      <input id="ai-parallel" type="number" min="1" max="8" value={view.settings.parallel} disabled={busy} onchange={(e) => save({ parallel: Number(e.currentTarget.value) })} />
      <span class="muted">photos described in parallel</span>
    </div>

    <label for="ai-place-tick">Place</label>
    <div>
      <input id="ai-place-tick" type="number" min="1" max="100" value={view.settings.place_tick} disabled={busy} onchange={(e) => save({ place_tick: Number(e.currentTarget.value) })} />
      <span class="muted">% likely or more: ticked in the review where the photo has none</span>
    </div>
  </div>
  {#if error}<p class="warn">{error}</p>{/if}

  <h3 class="next">What a description holds</h3>
  <div class="fields">
    {#each FIELDS as f (f.key)}
      <label class="field">
        <input type="checkbox" checked={view.settings.fields[f.key]} disabled={busy} onchange={(e) => toggleField(f.key, e.currentTarget.checked)} />
        <span>{f.label} <small class="muted">{f.hint}</small></span>
      </label>
    {/each}
  </div>
  <p class="muted small">
    What the photo already has (caption, headline, keywords, rating) is sent along as context, and the review shows it beside
    each suggestion. Changing these settings, or the prompt, lets photos described before be described again.
  </p>

  <h3 class="next">
    <button class="disclosure" aria-expanded={showPrompt} onclick={() => (showPrompt = !showPrompt)}>
      {showPrompt ? "▾" : "▸"} Prompt {own ? "(your own)" : ""}
    </button>
  </h3>
  {#if showPrompt}
    <p class="muted small">
      The instructions the model gets first. The list of fields ticked above, the caption language and the {source.noun}'s keyword
      vocabulary are added after them, so the answer always fits the review.
    </p>
    <textarea class="prompt" rows="10" bind:value={draft} spellcheck="false"></textarea>
    <div class="row-buttons">
      <button class="small-btn" disabled={busy || !edited} onclick={() => saveInstructions(draft)}>Save prompt</button>
      <button class="small-btn" disabled={busy || !edited} onclick={() => (draft = view?.settings.instructions ?? view?.default_instructions ?? "")}>Undo changes</button>
      <button class="small-btn" disabled={busy || !own} onclick={() => saveInstructions(null)}>Back to the default</button>
      <button class="small-btn" onclick={togglePreview}>{preview ? "Hide" : "Show"} the full prompt</button>
    </div>
    {#if preview}
      <pre class="preview">{preview.system}</pre>
      <details>
        <summary class="muted small">The answer's JSON Schema</summary>
        <pre class="preview">{JSON.stringify(preview.schema, null, 2)}</pre>
      </details>
    {/if}
  {/if}

  <p class="muted small">
    {view.calls.toLocaleString()} call{view.calls === 1 ? "" : "s"} in this library, {money(view.spent)} at API list prices. Photos are
    sent to {view.settings.access === "gemini" ? "Google" : "Anthropic"} as {view.settings.access === "gemini" && view.settings.sheets
      ? `contact sheets of their ${view.settings.image === "thumb" ? "thumbnails" : "previews"}`
      : view.settings.image === "thumb"
        ? "thumbnails"
        : "previews of at most 1568 pixels"}. These settings belong to this {source.noun}.
  </p>
{/if}

<style>
  h3 { margin: 0 0 8px; font-size: 13px; font-weight: 600; }
  p { margin: 8px 0; }
  .grid { display: grid; grid-template-columns: 110px 1fr; gap: 8px 12px; align-items: baseline; }
  .grid > label, .label { color: var(--muted); }
  .grid > div { display: flex; flex-wrap: wrap; align-items: center; gap: 6px 8px; }
  .grid small { flex-basis: 100%; font-size: 12px; }
  select, input, textarea { padding: 2px 6px; background: var(--panel); color: inherit; border: 1px solid var(--line, #444); border-radius: 3px; font: inherit; }
  select:focus, input:focus, textarea:focus { outline: none; border-color: var(--queued); }
  input[type="password"] { width: 280px; }
  input[type="number"] { width: 56px; }
  .small-btn { padding: 2px 10px; border: 1px solid var(--line); border-radius: 4px; }
  .small-btn:hover:not(:disabled) { background: var(--hover); }
  .muted { color: var(--muted); }
  .small { font-size: 12px; }
  .warn { color: #e8b03a; }
  h3.next { margin-top: 18px; }
  .fields { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 4px 16px; }
  .field { display: flex; align-items: baseline; gap: 6px; cursor: pointer; }
  .field small { font-size: 11px; }
  .disclosure { padding: 0; font: inherit; font-weight: 600; }
  .disclosure:hover { text-decoration: underline; }
  .prompt { width: 100%; box-sizing: border-box; resize: vertical; font: 12px/1.4 ui-monospace, Consolas, monospace; }
  .row-buttons { display: flex; flex-wrap: wrap; gap: 6px; margin: 6px 0; }
  .preview { max-height: 260px; overflow: auto; margin: 6px 0; padding: 8px; white-space: pre-wrap; font: 11px/1.4 ui-monospace, Consolas, monospace; background: rgb(0 0 0 / 25%); border: 1px solid var(--line); border-radius: 3px; }
</style>
