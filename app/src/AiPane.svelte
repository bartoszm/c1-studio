<script lang="ts">
  import * as api from "./lib/api";
  import type { Accept, AiView, DescribeState, SuggestionRow, SuggestionStatus } from "./lib/api";

  let {
    picked, describe, version, onselect, onaccepted, onerror,
  }: {
    /** The photos an edit would reach now: what a run describes. */
    picked: number[];
    describe: DescribeState | null;
    /** Bumped when photos were edited or described. */
    version: number;
    onselect: (row: number) => void;
    /** Suggestions were queued: the ops that undo it. */
    onaccepted: (undo: [number, api.Op][]) => void;
    onerror: (message: string) => void;
  } = $props();

  // ---- settings and key -------------------------------------------------------------
  let view = $state<AiView | null>(null);
  let showSettings = $state(false);
  let keyInput = $state("");
  async function loadView() {
    try {
      view = await api.aiSettings();
      if (!view.key) showSettings = true;
    } catch (e) {
      onerror(String(e));
    }
  }
  $effect(() => {
    void version;
    loadView();
  });
  async function saveSettings(change: Partial<api.AiSettings>) {
    if (!view) return;
    try {
      view = await api.setAiSettings({ ...view.settings, ...change });
    } catch (e) {
      onerror(String(e));
    }
  }
  async function saveKey(key: string) {
    try {
      view = await api.setAiKey(key);
      keyInput = "";
    } catch (e) {
      onerror(String(e));
    }
  }
  const money = (usd: number) => (usd < 0.01 && usd > 0 ? `$${usd.toFixed(4)}` : `$${usd.toFixed(2)}`);
  let model = $derived(view?.models.find((m) => m.id === view?.settings.model));

  // ---- a run --------------------------------------------------------------------------
  let estimate = $state<api.AiEstimate | null>(null);
  let again = $state(false);
  let estimating = $state(false);
  async function ask() {
    if (!picked.length) return;
    estimating = true;
    try {
      estimate = await api.aiEstimate(picked, again);
    } catch (e) {
      onerror(String(e));
    } finally {
      estimating = false;
    }
  }
  $effect(() => {
    // Another pick or model: the estimate shown no longer holds.
    void picked;
    void view?.settings.model;
    void again;
    estimate = null;
  });
  async function start() {
    try {
      await api.startDescribe(picked, again);
      estimate = null;
    } catch (e) {
      onerror(String(e));
    }
  }

  // ---- the review ---------------------------------------------------------------------
  type Show = SuggestionStatus | "All";
  let show = $state<Show>("Proposed");
  let rows = $state<SuggestionRow[]>([]);
  /** What each card takes, as the user edited it: by row. */
  type Draft = { caption: string; headline: string; rating: number | null; keywords: Record<string, boolean>; takeCaption: boolean; takeHeadline: boolean; takeRating: boolean };
  let drafts = $state<Record<number, Draft>>({});
  let asked = 0;
  async function loadRows() {
    const mine = ++asked;
    try {
      const list = await api.suggestions(show === "All" ? null : show);
      if (mine !== asked) return;
      rows = list;
      const next: Record<number, Draft> = {};
      for (const r of list) next[r.row] = drafts[r.row] ?? draftOf(r);
      drafts = next;
    } catch (e) {
      onerror(String(e));
    }
  }
  $effect(() => {
    void version;
    void show;
    void describe?.done;
    loadRows();
  });

  /** A card as first shown: known keywords ticked, new ones not (they would be added to the
   *  catalog), the rating not taken (ratings are the photographer's own). */
  function draftOf(r: SuggestionRow): Draft {
    const s = r.suggestion;
    const keywords: Record<string, boolean> = {};
    for (const k of s.keywords) keywords[k] = true;
    for (const k of s.keywords_new) keywords[k] = false;
    return {
      caption: s.caption_en ?? s.caption_pl ?? "",
      headline: s.headline ?? "",
      rating: s.rating,
      keywords,
      takeCaption: !!(s.caption_en ?? s.caption_pl),
      takeHeadline: !!s.headline,
      takeRating: false,
    };
  }

  function pickOf(row: number): Accept {
    const d = drafts[row];
    return {
      row,
      keywords: Object.entries(d.keywords).filter(([, on]) => on).map(([k]) => k),
      caption: d.takeCaption ? d.caption : undefined,
      headline: d.takeHeadline ? d.headline : undefined,
      rating: d.takeRating && d.rating ? d.rating : undefined,
    };
  }

  let working = $state(false);
  async function accept(list: number[]) {
    if (!list.length) return;
    working = true;
    try {
      const undo = await api.acceptSuggestions(list.map(pickOf));
      onaccepted(undo);
      await loadRows();
    } catch (e) {
      onerror(String(e));
    } finally {
      working = false;
    }
  }
  async function setStatus(list: number[], status: SuggestionStatus) {
    if (!list.length) return;
    working = true;
    try {
      await api.setSuggestionStatus(list, status);
      await loadRows();
    } catch (e) {
      onerror(String(e));
    } finally {
      working = false;
    }
  }

  let minConfidence = $state(0.7);
  let proposed = $derived(rows.filter((r) => r.status === "Proposed"));
  let sure = $derived(proposed.filter((r) => r.suggestion.confidence >= minConfidence));
  const leaf = (path: string) => path.split("|").at(-1) ?? path;
  const stars = (n: number | null) => (n ? "★".repeat(n) + "☆".repeat(5 - n) : "—");
</script>

<div class="ai">
  <div class="bar">
    <div class="run">
      {#if describe?.running}
        <span class="muted">Describing {describe.done.toLocaleString()} / {describe.total.toLocaleString()} · {money(describe.cost)}</span>
        <progress max={describe.total} value={describe.done}></progress>
        <button class="btn" onclick={() => api.stopDescribe().catch((e) => onerror(String(e)))}>Stop</button>
      {:else if estimate}
        <span>
          {estimate.photos.toLocaleString()} photo{estimate.photos === 1 ? "" : "s"} with {estimate.model}, about <b>{money(estimate.cost)}</b>{estimate.skipped
            ? ` · ${estimate.skipped.toLocaleString()} described already, left out`
            : ""}
        </span>
        <button class="btn primary" disabled={!estimate.photos || !view?.key} onclick={start}>Describe</button>
        <button class="btn" onclick={() => (estimate = null)}>Cancel</button>
      {:else}
        <button
          class="btn primary"
          disabled={!picked.length || estimating || !view?.key}
          title={!view?.key ? "Add an API key in the settings first" : picked.length ? "" : "Pick photos in the strip below"}
          onclick={ask}
        >Describe {picked.length ? picked.length.toLocaleString() : ""} photo{picked.length === 1 ? "" : "s"}…</button>
        <label class="muted" title="Describe photos again even when they were described with this model and prompt">
          <input type="checkbox" bind:checked={again} /> again
        </label>
      {/if}
      {#if describe && !describe.running && describe.failed}
        <span class="warn" title={describe.error ?? ""}>⚠ {describe.failed} failed{describe.error ? `: ${describe.error}` : ""}</span>
      {/if}
    </div>
    <button class="link" class:on={showSettings} onclick={() => (showSettings = !showSettings)}>
      {model?.name ?? "Settings"}{view ? ` · ${money(view.spent)} spent` : ""}
    </button>
  </div>

  {#if showSettings && view}
    <section class="settings">
      <label>
        <span>Anthropic API key</span>
        {#if view.key}
          <code>{view.key[0]}</code>
          <span class="muted">{view.key[1] === "environment" ? "from ANTHROPIC_API_KEY" : "in the Windows Credential Manager"}</span>
          {#if view.key[1] === "stored"}<button class="btn" onclick={() => saveKey("")}>Remove</button>{/if}
        {:else}
          <input type="password" placeholder="sk-ant-…" bind:value={keyInput} autocomplete="off" />
          <button class="btn" disabled={!keyInput.trim()} onclick={() => saveKey(keyInput)}>Save</button>
          <span class="muted">Kept in the Windows Credential Manager, never in a file.</span>
        {/if}
      </label>
      <label>
        <span>Model</span>
        <select value={view.settings.model} onchange={(e) => saveSettings({ model: e.currentTarget.value })}>
          {#each view.models as m (m.id)}
            <option value={m.id}>{m.name} · ${m.input} / ${m.output} per M tokens</option>
          {/each}
        </select>
      </label>
      <label>
        <span>Captions</span>
        <select value={view.settings.language} onchange={(e) => saveSettings({ language: e.currentTarget.value as api.AiLanguage })}>
          <option value="en">English</option>
          <option value="pl">Polish</option>
          <option value="both">English and Polish</option>
        </select>
      </label>
      <label>
        <span>Effort</span>
        <select value={view.settings.effort} onchange={(e) => saveSettings({ effort: e.currentTarget.value as api.AiSettings["effort"] })}>
          <option value="low">Low: quick and cheap</option>
          <option value="medium">Medium</option>
          <option value="high">High: thinks longer, costs more</option>
        </select>
      </label>
      <label>
        <span>At once</span>
        <input type="number" min="1" max="8" value={view.settings.parallel} onchange={(e) => saveSettings({ parallel: Number(e.currentTarget.value) })} />
        <span class="muted">photos described in parallel</span>
      </label>
      <p class="muted">{view.calls.toLocaleString()} call{view.calls === 1 ? "" : "s"} in this library, {money(view.spent)} at list prices. Photos are sent to Anthropic as previews of at most 1568 pixels.</p>
    </section>
  {/if}

  <div class="bar">
    <nav class="parts">
      {#each ["Proposed", "Accepted", "Rejected", "All"] as const as s (s)}
        <button class:on={show === s} onclick={() => (show = s)}>{s === "Proposed" ? "To review" : s}</button>
      {/each}
    </nav>
    {#if proposed.length}
      <div class="bulk">
        <label class="muted">confidence ≥ <input type="number" min="0" max="1" step="0.05" bind:value={minConfidence} /></label>
        <button class="btn" disabled={working || !sure.length} onclick={() => accept(sure.map((r) => r.row))}>Accept {sure.length.toLocaleString()}</button>
        <button class="btn" disabled={working} onclick={() => setStatus(proposed.map((r) => r.row), "Rejected")}>Reject all {proposed.length.toLocaleString()}</button>
      </div>
    {/if}
  </div>

  {#if !rows.length}
    <p class="muted empty">
      {show === "Proposed" ? "Nothing to review. Pick photos in the strip below and describe them." : "None."}
    </p>
  {/if}
  <div class="cards">
    {#each rows as r (r.row)}
      {@const d = drafts[r.row]}
      {@const s = r.suggestion}
      {#if d}
        <article class="card" class:done={r.status !== "Proposed"}>
          <button class="thumb" onclick={() => onselect(r.row)} title="Show this photo">
            <img src={api.thumbUrl(r.row)} alt="" loading="lazy" decoding="async" />
          </button>
          <div class="body">
            <div class="head">
              <input class="headline" bind:value={d.headline} placeholder="Headline" disabled={!d.takeHeadline} />
              <input type="checkbox" bind:checked={d.takeHeadline} title="Take the headline" />
              <span class="conf" title="How sure the model is">{Math.round(s.confidence * 100)}%</span>
            </div>
            <div class="caption">
              <textarea rows="3" bind:value={d.caption} disabled={!d.takeCaption}></textarea>
              <div class="side">
                <input type="checkbox" bind:checked={d.takeCaption} title="Take the caption" />
                {#if s.caption_en && s.caption_pl}
                  <button class="tiny" onclick={() => (d.caption = s.caption_en ?? "")}>EN</button>
                  <button class="tiny" onclick={() => (d.caption = s.caption_pl ?? "")}>PL</button>
                {/if}
              </div>
            </div>
            <div class="keywords">
              {#each Object.keys(d.keywords) as k (k)}
                <label class="kw" class:new={s.keywords_new.includes(k)} title={s.keywords_new.includes(k) ? "New: added to the catalog" : k}>
                  <input type="checkbox" bind:checked={d.keywords[k]} />{leaf(k)}
                </label>
              {/each}
            </div>
            <div class="facts muted">
              <label title="Take the rating">
                <input type="checkbox" bind:checked={d.takeRating} />
                <span class="stars">{stars(d.rating)}</span>
              </label>
              {#if s.quality}<span>quality {s.quality}/5</span>{/if}
              {#if s.composition}<span>composition {s.composition}/5</span>{/if}
              {#if s.subject}<span>subject: {s.subject}</span>{/if}
              {#if s.place}<span title="A guess, never applied">place? {[s.place.city, s.place.country].filter(Boolean).join(", ")} ({Math.round(s.place.confidence * 100)}%)</span>{/if}
              <span title={r.model}>{money(r.cost)}</span>
            </div>
            {#if r.warnings.length}
              <p class="warn">{r.warnings.join(" · ")}</p>
            {/if}
            <div class="actions">
              {#if r.status === "Proposed"}
                <button class="btn primary" disabled={working} onclick={() => accept([r.row])}>Accept</button>
                <button class="btn" disabled={working} onclick={() => setStatus([r.row], "Rejected")}>Reject</button>
              {:else}
                <span class="muted">{r.status === "Accepted" ? "Accepted: in the edit queue" : "Rejected"}</span>
                <button class="btn" disabled={working} onclick={() => setStatus([r.row], "Proposed")}>Review again</button>
              {/if}
            </div>
          </div>
        </article>
      {/if}
    {/each}
  </div>
</div>

<style>
  .ai { flex: 1; min-height: 0; overflow-y: auto; padding: 12px 16px 24px; }
  .bar { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 8px; flex-wrap: wrap; }
  .run, .bulk { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  progress { width: 160px; }
  .btn { padding: 3px 12px; border: 1px solid var(--line, #444); border-radius: 3px; }
  .btn:hover:not(:disabled) { background: var(--hover, #333); }
  .btn.primary { background: var(--on, #2a3a55); border-color: #5b8def; }
  .btn:disabled { opacity: 0.5; }
  .link { color: #e8b03a; font-size: 12px; }
  .link:hover, .link.on { text-decoration: underline; }
  .muted { color: var(--muted, #999); font-size: 12px; }
  .warn { color: #e8b03a; font-size: 12px; margin: 4px 0 0; }
  .empty { margin: 16px 0; }
  .settings { display: grid; gap: 8px; padding: 10px 12px; margin-bottom: 12px; border: 1px solid var(--line, #333); border-radius: 4px; background: rgba(255, 255, 255, 0.03); }
  .settings label { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .settings label > span:first-child { width: 130px; color: #bbb; }
  .settings input[type="password"] { width: 280px; }
  .settings input[type="number"] { width: 56px; }
  .settings p { margin: 0; }
  .parts { display: flex; gap: 2px; border-bottom: 1px solid var(--line, #333); }
  .parts button { padding: 5px 14px; border-bottom: 2px solid transparent; margin-bottom: -1px; }
  .parts button:hover { background: var(--hover, #333); }
  .parts button.on { border-bottom-color: #5b8def; font-weight: 600; }
  .bulk input { width: 52px; }
  .cards { display: grid; grid-template-columns: repeat(auto-fill, minmax(460px, 1fr)); gap: 12px; }
  .card { display: grid; grid-template-columns: 140px 1fr; gap: 10px; padding: 8px; border: 1px solid var(--line, #333); border-radius: 4px; background: rgba(255, 255, 255, 0.03); }
  .card.done { opacity: 0.7; }
  .thumb { align-self: start; padding: 0; }
  .thumb img { width: 140px; height: 140px; object-fit: contain; background: #111; display: block; }
  .body { display: flex; flex-direction: column; gap: 6px; min-width: 0; }
  .head { display: flex; align-items: center; gap: 6px; }
  .headline { flex: 1; font-weight: 600; }
  .conf { font-size: 12px; color: #9bd; font-variant-numeric: tabular-nums; }
  .caption { display: flex; gap: 6px; }
  .caption textarea { flex: 1; resize: vertical; font: inherit; font-size: 12px; }
  .side { display: flex; flex-direction: column; gap: 4px; align-items: center; }
  .tiny { font-size: 10px; padding: 0 4px; border: 1px solid var(--line, #444); border-radius: 2px; }
  .keywords { display: flex; flex-wrap: wrap; gap: 4px; }
  .kw { display: inline-flex; align-items: center; gap: 3px; padding: 1px 6px; border: 1px solid var(--line, #444); border-radius: 10px; font-size: 12px; }
  .kw.new { border-style: dashed; border-color: #e8b03a; }
  .facts { display: flex; flex-wrap: wrap; gap: 10px; align-items: center; }
  .facts label { display: inline-flex; align-items: center; gap: 4px; }
  .stars { color: #e8b03a; letter-spacing: 1px; }
  .actions { display: flex; gap: 8px; align-items: center; }
</style>
