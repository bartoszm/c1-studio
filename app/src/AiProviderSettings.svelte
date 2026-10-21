<script lang="ts">
  import { onMount } from "svelte";
  import * as api from "./lib/api";
  import type { AiView } from "./lib/api";

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
  onMount(() => run(api.aiSettings));
  const save = (change: Partial<api.AiSettings>) => view && run(() => api.setAiSettings({ ...view!.settings, ...change }));
  const saveKey = (key: string) => run(() => api.setAiKey(key)).then(() => (keyInput = ""));
  const money = (usd: number) => (usd < 0.01 && usd > 0 ? `$${usd.toFixed(4)}` : `$${usd.toFixed(2)}`);
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
      </select>
    </div>

    {#if view.settings.access === "plan"}
      <span class="label">Claude Code</span>
      <div>
        {#if busy && !view.plan}
          <span class="muted">Asking Claude Code…</span>
        {:else if !view.plan?.installed}
          <span class="warn">Not installed. Install Claude Code and run <code>claude</code> once to sign in.</span>
        {:else if view.plan.error}
          <span class="warn">{view.plan.error}</span>
        {:else if view.plan.loggedIn}
          <span>Signed in{view.plan.subscriptionType ? ` with Claude ${view.plan.subscriptionType}` : ""}</span>
          {#if view.plan.authMethod !== "claude.ai"}
            <span class="muted">({view.plan.authMethod}: billed to that account, not a plan)</span>
          {/if}
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
      <select id="ai-model" value={view.settings.model} disabled={busy} onchange={(e) => save({ model: e.currentTarget.value })}>
        {#each view.models as m (m.id)}
          <option value={m.id}>{m.name}{view.settings.access === "api" ? ` · $${m.input} / $${m.output} per M tokens` : ""}</option>
        {/each}
      </select>
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
  </div>
  {#if error}<p class="warn">{error}</p>{/if}
  <p class="muted small">
    {view.calls.toLocaleString()} call{view.calls === 1 ? "" : "s"} in this library, {money(view.spent)} at API list prices. Photos are
    sent to Anthropic as previews of at most 1568 pixels. These settings belong to this catalog.
  </p>
{/if}

<style>
  h3 { margin: 0 0 8px; font-size: 13px; font-weight: 600; }
  p { margin: 8px 0; }
  .grid { display: grid; grid-template-columns: 110px 1fr; gap: 8px 12px; align-items: baseline; }
  .grid > label, .label { color: var(--muted); }
  .grid > div { display: flex; flex-wrap: wrap; align-items: center; gap: 6px 8px; }
  .grid small { flex-basis: 100%; font-size: 12px; }
  select, input { padding: 2px 6px; background: var(--panel); color: inherit; border: 1px solid var(--line, #444); border-radius: 3px; font: inherit; }
  select:focus, input:focus { outline: none; border-color: var(--queued); }
  input[type="password"] { width: 280px; }
  input[type="number"] { width: 56px; }
  .small-btn { padding: 2px 10px; border: 1px solid var(--line); border-radius: 4px; }
  .small-btn:hover:not(:disabled) { background: var(--hover); }
  .muted { color: var(--muted); }
  .small { font-size: 12px; }
  .warn { color: #e8b03a; }
</style>
