<script lang="ts">
  import { untrack } from "svelte";
  import * as api from "./lib/api";
  import { source } from "./lib/source.svelte";
  import type { Accept, AiView, DescribeState, SuggestionRow, SuggestionStatus } from "./lib/api";

  let {
    picked, describe, version, settingsOpen, onsettings, onselect, onaccepted, onerror,
  }: {
    /** The photos an edit would reach now: what a run describes. */
    picked: number[];
    describe: DescribeState | null;
    /** Bumped when photos were edited or described. */
    version: number;
    /** The Settings dialog is open: closed, the AI settings may have changed. */
    settingsOpen: boolean;
    /** Opens Settings on its AI tab. */
    onsettings: () => void;
    onselect: (row: number) => void;
    /** Suggestions were queued: the ops that undo it. */
    onaccepted: (undo: [number, api.Op][]) => void;
    onerror: (message: string) => void;
  } = $props();

  // ---- settings (edited in Settings > AI) ---------------------------------------------
  // Read when the tab opens, when Settings closes and when a run ends (what was spent):
  // on a plan, reading them asks Claude Code, which takes a moment.
  let view = $state<AiView | null>(null);
  async function loadView() {
    try {
      view = await api.aiSettings();
    } catch (e) {
      onerror(String(e));
    }
  }
  let running = $derived(!!describe?.running);
  $effect(() => {
    void running;
    if (!settingsOpen) untrack(loadView);
  });
  const money = (usd: number) => (usd < 0.01 && usd > 0 ? `$${usd.toFixed(4)}` : `$${usd.toFixed(2)}`);
  let model = $derived(view?.models.find((m) => m.id === view?.settings.model));

  // ---- a run --------------------------------------------------------------------------
  /** The estimate shown, with the photos and choice it was made for: Describe runs those. */
  let estimate = $state<{ of: api.AiEstimate; rows: number[]; again: boolean } | null>(null);
  let again = $state(false);
  let estimating = $state(false);
  let estimates = 0;
  async function ask() {
    if (!picked.length) return;
    const mine = ++estimates;
    const rows = [...picked];
    const twice = again;
    estimating = true;
    try {
      const of = await api.aiEstimate(rows, twice);
      if (mine === estimates) estimate = { of, rows, again: twice };
    } catch (e) {
      onerror(String(e));
    } finally {
      if (mine === estimates) estimating = false;
    }
  }
  $effect(() => {
    // Another pick or model: the estimate shown, or one under way, no longer holds.
    void picked;
    void view?.settings.model;
    void again;
    estimates++;
    estimate = null;
    estimating = false;
  });
  async function start() {
    if (!estimate) return;
    try {
      await api.startDescribe(estimate.rows, estimate.again);
      estimate = null;
    } catch (e) {
      onerror(String(e));
    }
  }

  // How long a run has taken and will take: a clock that ticks while it runs.
  let now = $state(Date.now());
  $effect(() => {
    if (!describe?.running) return;
    now = Date.now();
    const timer = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(timer);
  });
  let elapsed = $derived(describe?.running && describe.started ? Math.max(0, now - describe.started) : 0);
  /** From the pace so far, once a photo is back; the photos at work count as half done. */
  let left = $derived.by(() => {
    if (!describe?.running || describe.done < 1) return null;
    const done = describe.done + describe.working.length / 2;
    return (elapsed / done) * (describe.total - done);
  });
  function duration(ms: number): string {
    const s = Math.round(ms / 1000);
    if (s < 60) return `${s} s`;
    const m = Math.round(s / 60);
    return m < 60 ? `${m} min` : `${Math.floor(m / 60)} h ${m % 60} min`;
  }

  // ---- the review ---------------------------------------------------------------------
  type Show = SuggestionStatus | "All";
  let show = $state<Show>("Proposed");
  let rows = $state<SuggestionRow[]>([]);
  /** What each card takes, as the user edited it: by row. `at` and `seen` say which
   *  suggestion, and which state of the photo, it was made for. */
  type Draft = {
    at: number; seen: api.Seen;
    caption: string; headline: string; rating: number | null; keywords: Record<string, boolean>;
    takeCaption: boolean; takeHeadline: boolean; takeRating: boolean;
  };
  let drafts = $state<Record<number, Draft>>({});
  const seenOf = (r: SuggestionRow): api.Seen => ({
    caption: r.current?.caption ?? null,
    headline: r.current?.headline ?? null,
    rating: r.current?.rating ?? null,
  });
  const sameSeen = (a: api.Seen, b: api.Seen) => a.caption === b.caption && a.headline === b.headline && a.rating === b.rating;
  let asked = 0;
  async function loadRows() {
    const mine = ++asked;
    try {
      const list = await api.suggestions(show === "All" ? null : show);
      if (mine !== asked) return;
      rows = list;
      // A card keeps its edits while it shows the same suggestion of the same photo: a
      // new description, or a caption written meanwhile, starts it again.
      const next: Record<number, Draft> = {};
      for (const r of list) {
        const d = drafts[r.row];
        next[r.row] = d && d.at === r.at && sameSeen(d.seen, seenOf(r)) ? d : draftOf(r);
      }
      drafts = next;
    } catch (e) {
      onerror(String(e));
    }
  }
  $effect(() => {
    void version;
    void show;
    untrack(loadRows);
  });
  // While a run goes, new suggestions come in every few seconds at most, not once per
  // photo; its end bumps `version`, which reads them all.
  let pending: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    void describe?.done;
    if (pending || !describe?.running) return;
    pending = setTimeout(() => {
      pending = undefined;
      loadRows();
    }, 3000);
  });
  $effect(() => () => clearTimeout(pending));

  /** A card as first shown: known keywords ticked, new ones not (they would be added to the
   *  catalog), the rating not taken (ratings are the photographer's own), and a caption or
   *  headline taken only where the photo has none: what it has is replaced only on a tick. */
  function draftOf(r: SuggestionRow): Draft {
    const s = r.suggestion;
    const now = r.current;
    const keywords: Record<string, boolean> = {};
    for (const k of s.keywords) keywords[k] = true;
    for (const k of s.keywords_new) keywords[k] = false;
    const caption = s.caption_en ?? s.caption_pl ?? "";
    return {
      at: r.at,
      seen: seenOf(r),
      caption,
      headline: s.headline ?? "",
      rating: s.rating,
      keywords,
      takeCaption: !!caption && !now?.caption,
      takeHeadline: !!s.headline && !now?.headline,
      takeRating: false,
    };
  }

  /** The photo has this keyword already: taking it changes nothing. */
  const has = (r: SuggestionRow, path: string) =>
    !!r.current?.keywords.some((k) => k.toLowerCase() === path.toLowerCase());

  /** What accepting this card would overwrite that the photo has now. */
  function replaces(r: SuggestionRow): string[] {
    const d = drafts[r.row];
    const now = r.current;
    if (!d || !now) return [];
    return [
      d.takeCaption && now.caption && d.caption.trim() !== now.caption && "caption",
      d.takeHeadline && now.headline && d.headline.trim() !== now.headline && "headline",
      d.takeRating && now.rating && d.rating !== now.rating && `rating (${now.rating}★)`,
    ].filter((x): x is string => !!x);
  }

  function pickOf(row: number): Accept {
    const d = drafts[row];
    return {
      row,
      keywords: Object.entries(d.keywords).filter(([, on]) => on).map(([k]) => k),
      caption: d.takeCaption ? d.caption : undefined,
      headline: d.takeHeadline ? d.headline : undefined,
      rating: d.takeRating && d.rating ? d.rating : undefined,
      seen: d.seen,
    };
  }

  let working = $state(false);
  /** Said after accepting: photos changed since their card was shown. */
  let notice = $state("");
  async function accept(list: number[]) {
    if (!list.length) return;
    working = true;
    notice = "";
    try {
      const { undo, changed } = await api.acceptSuggestions(list.map(pickOf));
      onaccepted(undo);
      if (changed.length) {
        notice = `${changed.length} photo${changed.length === 1 ? "" : "s"} changed since shown: the caption, headline or rating written since was kept. The card shows it now; accept again to replace it.`;
      }
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
        <span>
          Describing <b>{describe.done.toLocaleString()}</b> of {describe.total.toLocaleString()}
          <span class="muted">
            · {describe.working.length} at work · {duration(elapsed)}
            {left === null ? "· the first answers take a few seconds" : `· about ${duration(left)} left`}
            {describe.failed ? `· ${describe.failed} failed` : ""}{describe.cost ? ` · ${money(describe.cost)}` : ""}
          </span>
        </span>
        <progress max={describe.total} value={describe.done}></progress>
        <button class="btn" onclick={() => api.stopDescribe().catch((e) => onerror(String(e)))}>Stop</button>
      {:else if estimate}
        {@const e = estimate.of}
        <span>
          {e.photos.toLocaleString()} photo{e.photos === 1 ? "" : "s"} with {e.model},
          {#if e.plan}on your Claude plan: counts toward its usage limits{:else}about <b>{money(e.cost)}</b>{/if}{e.skipped
            ? ` · ${e.skipped.toLocaleString()} described already, left out`
            : ""}
        </span>
        <button class="btn primary" disabled={!e.photos || !view?.ready} onclick={start}>Describe</button>
        <button class="btn" onclick={() => (estimate = null)}>Cancel</button>
      {:else}
        <button
          class="btn primary"
          disabled={!picked.length || estimating || !view?.ready}
          title={!view?.ready
            ? view?.settings.access === "plan" ? "Sign in to Claude Code first" : "Add an API key in Settings > AI first"
            : picked.length ? "" : "Pick photos in the strip below"}
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
    <button class="link" title="Settings > AI: access, model, language" onclick={onsettings}>
      {#if view && !view.ready}
        {view.settings.access === "plan" ? "Sign in to Claude Code…" : "Add an API key…"}
      {:else}
        {model?.name ?? "Settings"}{view?.settings.access === "plan" ? " · Claude plan" : view ? ` · ${money(view.spent)} spent` : ""}
      {/if}
    </button>
  </div>

  {#if notice}
    <p class="notice">{notice} <button class="link" onclick={() => (notice = "")}>OK</button></p>
  {/if}

  {#if describe?.running && describe.working.length}
    <div class="at-work" aria-label="Photos being described">
      <span class="muted">Now:</span>
      {#each describe.working as row (row)}
        <button class="busy" onclick={() => onselect(row)} title="Being described: show this photo">
          <img src={api.thumbUrl(row)} alt="" decoding="async" />
        </button>
      {/each}
    </div>
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
        <button
          class="btn"
          disabled={working || !sure.length}
          title="Takes what each card has ticked: a caption, headline or rating a photo already has is kept unless ticked on its card"
          onclick={() => accept(sure.map((r) => r.row))}
        >Accept {sure.length.toLocaleString()}</button>
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
        {@const now = r.current}
        {@const over = replaces(r)}
        <article class="card" class:done={r.status !== "Proposed"}>
          <button class="thumb" onclick={() => onselect(r.row)} title="Show this photo">
            <img src={api.thumbUrl(r.row)} alt="" loading="lazy" decoding="async" />
          </button>
          <div class="body">
            <div class="head">
              {#if s.headline}
                <input class="headline" bind:value={d.headline} placeholder="Headline" disabled={!d.takeHeadline} />
                <input type="checkbox" bind:checked={d.takeHeadline} title={now?.headline ? "Replace the headline the photo has" : "Take the headline"} />
              {:else}
                <span class="headline muted">{s.subject ?? ""}</span>
              {/if}
              <span class="conf" title="How sure the model is">{Math.round(s.confidence * 100)}%</span>
            </div>
            {#if s.headline && now?.headline && now.headline !== d.headline.trim()}
              <p class="now" class:replacing={d.takeHeadline}><span>Now</span>{now.headline}</p>
            {:else if s.headline && now?.headline}
              <p class="now"><span>Now</span>the same headline</p>
            {/if}
            {#if s.caption_en || s.caption_pl}
              {#if now?.caption && now.caption !== d.caption.trim()}
                <p class="now" class:replacing={d.takeCaption}><span>Now</span>{now.caption}</p>
              {/if}
              <div class="caption">
                <textarea rows="3" bind:value={d.caption} disabled={!d.takeCaption}></textarea>
                <div class="side">
                  <input type="checkbox" bind:checked={d.takeCaption} title={now?.caption ? "Replace the caption the photo has" : "Take the caption"} />
                  {#if s.caption_en && s.caption_pl}
                    <button class="tiny" onclick={() => (d.caption = s.caption_en ?? "")}>EN</button>
                    <button class="tiny" onclick={() => (d.caption = s.caption_pl ?? "")}>PL</button>
                  {/if}
                  {#if now?.caption}
                    <button class="tiny" title="Start from the caption the photo has" onclick={() => (d.caption = now.caption ?? "")}>Now</button>
                  {/if}
                </div>
              </div>
            {/if}
            {#if Object.keys(d.keywords).length}
              <div class="keywords">
                {#each Object.keys(d.keywords) as k (k)}
                  {@const already = has(r, k)}
                  <label
                    class="kw"
                    class:new={s.keywords_new.includes(k)}
                    class:has={already}
                    title={already ? "The photo has it already" : s.keywords_new.includes(k) ? `New: added to the ${source.noun}` : k}
                  >
                    <input type="checkbox" checked={already || d.keywords[k]} disabled={already} onchange={(e) => (d.keywords[k] = e.currentTarget.checked)} />{leaf(k)}
                  </label>
                {/each}
              </div>
            {/if}
            <div class="facts muted">
              {#if s.rating}
                <label title={now?.rating ? "Replace the photo's rating" : "Take the rating"}>
                  <input type="checkbox" bind:checked={d.takeRating} />
                  <span class="stars">{stars(d.rating)}</span>
                </label>
                {#if now?.rating && now.rating !== d.rating}<span title="The photo's rating now">now <span class="stars">{"★".repeat(now.rating)}</span></span>{/if}
              {/if}
              {#if s.quality}<span>quality {s.quality}/5</span>{/if}
              {#if s.composition}<span>composition {s.composition}/5</span>{/if}
              {#if s.subject && s.headline}<span>subject: {s.subject}</span>{/if}
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
                {#if over.length}<span class="replace-note">replaces the {over.join(", ")} it has now</span>{/if}
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
  .link:hover { text-decoration: underline; }
  .muted { color: var(--muted, #999); font-size: 12px; }
  .warn { color: #e8b03a; font-size: 12px; margin: 4px 0 0; }
  .empty { margin: 16px 0; }
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
  .actions { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
  .replace-note { color: #e8b03a; font-size: 12px; }
  .notice { margin: 0 0 10px; padding: 6px 10px; font-size: 12px; border: 1px solid #e8b03a; border-radius: 3px; }
  /* What the photo has now, above what would replace it. */
  .now { margin: 0; padding: 3px 6px; font-size: 12px; color: var(--muted, #999); border-left: 2px solid var(--line, #444); }
  .now > span { margin-right: 6px; font-size: 10px; text-transform: uppercase; letter-spacing: 0.5px; }
  .now.replacing { border-left-color: #e8b03a; text-decoration: line-through; text-decoration-color: rgb(232 176 58 / 60%); }
  .kw.has { opacity: 0.55; }
  .at-work { display: flex; align-items: center; gap: 6px; margin: 0 0 10px; flex-wrap: wrap; }
  .busy { padding: 0; border-radius: 3px; outline: 2px solid #5b8def; animation: pulse 1.4s ease-in-out infinite; }
  .busy img { display: block; width: 56px; height: 56px; object-fit: cover; border-radius: 3px; }
  @keyframes pulse { 50% { outline-color: rgb(91 141 239 / 25%); } }
  @media (prefers-reduced-motion: reduce) { .busy { animation: none; } }
</style>
