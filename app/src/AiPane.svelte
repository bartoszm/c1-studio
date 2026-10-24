<script lang="ts">
  import { untrack } from "svelte";
  import * as api from "./lib/api";
  import { SvelteSet } from "svelte/reactivity";
  import {
    LEVELS, aboveOf, guessNames, levelOptions, percent, placeReplaces, placeTicks, takenPlace,
    type Level, type Names, type PlaceTicks,
  } from "./lib/aiPlace";
  import { catalogPlaces } from "./lib/placeHints";
  import { changes, chipText, digest, shows, type Shown as ReviewShown } from "./lib/reviews";
  import { source } from "./lib/source.svelte";
  import type { Accept, AiView, DescribeState, SuggestionRow, SuggestionStatus } from "./lib/api";

  let {
    picked, describe, version, settingsOpen, keywords, onsettings, onselect, onaccepted, onerror,
  }: {
    /** The photos an edit would reach now: what a run describes. */
    picked: number[];
    describe: DescribeState | null;
    /** Bumped when photos were edited or described. */
    version: number;
    /** The Settings dialog is open: closed, the AI settings may have changed. */
    settingsOpen: boolean;
    /** The catalog's keyword paths, most used first: what a card's keyword box offers. */
    keywords: string[];
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
  let modelId = $derived(view?.settings.access === "gemini" ? view.settings.gemini_model : view?.settings.model);
  let model = $derived(view?.models.find((m) => m.id === modelId) ?? (modelId ? { name: modelId } : undefined));
  /** What to do before a run can start. */
  const notReady = (v: AiView) =>
    v.settings.access === "plan" ? "Sign in to Claude Code first" : v.settings.access === "gemini" ? "Install agy first" : "Add an API key in Settings > AI first";

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
  /** Photos go as contact sheets: a call is a sheet of many. */
  let bySheet = $derived(!!describe?.sheets);
  /** From the pace so far, once a call is back; the calls at work count as half done. */
  let left = $derived.by(() => {
    if (!describe?.running || describe.calls_done < 1) return null;
    const done = describe.calls_done + describe.units.length / 2;
    return (elapsed / done) * (describe.calls_total - done);
  });
  /** The call at work longest: what the run line tells of. */
  let lead = $derived(describe?.running ? describe.units[0] : undefined);
  const PHASES: Record<api.DescribeUnit["phase"], string> = {
    preparing: "making its image",
    waiting: "waiting for the answer",
    retrying: "trying again shortly",
  };
  const phase = (u: api.DescribeUnit) =>
    u.phase === "waiting" && u.attempt > 1 ? `waiting for the answer, try ${u.attempt}` : PHASES[u.phase];
  const clock = (ms: number) => new Date(ms).toLocaleTimeString();

  // ---- what the run is doing, shown on demand -----------------------------------------
  let details = $state(readFlag("ai-details"));
  function readFlag(key: string): boolean {
    try {
      return localStorage.getItem(key) === "1";
    } catch {
      return false;
    }
  }
  $effect(() => {
    const open = details;
    try {
      localStorage.setItem("ai-details", open ? "1" : "0");
    } catch {
      // Not kept: it opens closed next time.
    }
  });
  /** The contact sheet shown large, by its call. */
  let sheetShown = $state<number | null>(null);
  let sheetGone = $state(false);
  function showSheet(call: number) {
    sheetGone = false;
    sheetShown = call;
  }
  function closeSheet(e: KeyboardEvent) {
    if (sheetShown === null || e.key !== "Escape") return;
    e.preventDefault();
    e.stopPropagation();
    sheetShown = null;
  }
  function duration(ms: number): string {
    const s = Math.round(ms / 1000);
    if (s < 60) return `${s} s`;
    const m = Math.round(s / 60);
    return m < 60 ? `${m} min` : `${Math.floor(m / 60)} h ${m % 60} min`;
  }

  // ---- the review ---------------------------------------------------------------------
  type Show = SuggestionStatus | "All" | "Reviewed";
  let show = $state<Show>("Proposed");
  let rows = $state<SuggestionRow[]>([]);
  /** What each card takes, as the user edited it: by row. `at` and `seen` say which
   *  suggestion, and which state of the photo, it was made for. */
  type Draft = {
    at: number; seen: api.Seen;
    caption: string; headline: string; rating: number | null; keywords: Record<string, boolean>;
    takeCaption: boolean; takeHeadline: boolean; takeRating: boolean;
    /** The place levels and the position taken. */
    place: PlaceTicks | null;
    /** Each level's name as the card has it: the model's until edited. */
    names: Names | null;
    /** Keywords the user added to the card. */
    added: string[];
  };
  let drafts = $state<Record<number, Draft>>({});
  const seenOf = (r: SuggestionRow): api.Seen => ({
    caption: r.current?.caption ?? null,
    headline: r.current?.headline ?? null,
    rating: r.current?.rating ?? null,
    place: r.current?.place ?? { country: null, country_code: null, state: null, city: null, location: null },
  });
  const samePlace = (a: api.PlaceNames | null, b: api.PlaceNames | null) =>
    LEVELS.every((l) => (a?.[l] ?? null) === (b?.[l] ?? null));
  const sameSeen = (a: api.Seen, b: api.Seen) =>
    a.caption === b.caption && a.headline === b.headline && a.rating === b.rating && samePlace(a.place, b.place);
  let asked = 0;
  async function loadRows() {
    const mine = ++asked;
    if (show === "Reviewed") return loadReviews();
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
      place: s.place && placeTicks(s.place, now?.place ?? null, !!now?.placed, (view?.settings.place_tick ?? 80) / 100),
      names: s.place && guessNames(s.place),
      added: [],
    };
  }

  // ---- correcting a card ---------------------------------------------------------------
  /** The catalog's places (`place_tree` keys with counts): what the place boxes offer. */
  let placeKeys = $state.raw<[string, number][]>([]);
  $effect(() => {
    catalogPlaces(version).then((k) => (placeKeys = k)).catch(() => {});
  });
  /** The place box being typed in: its level's list offers names under the card's levels above. */
  let placeFocus = $state<{ row: number; level: Level } | null>(null);
  let placeOptions = $derived.by(() => {
    const f = placeFocus;
    const names = f && drafts[f.row]?.names;
    return f && names ? levelOptions(placeKeys, f.level, names).slice(0, 300) : [];
  });
  function nameTyped(d: Draft, level: Level, value: string) {
    if (!d.names || !d.place) return;
    d.names[level] = value;
    d.place[level] = !!value.trim();
  }
  /** A name picked (or typed and left): the levels above it follow the catalog's place with it. */
  function nameChosen(d: Draft, r: SuggestionRow, level: Level, value: string) {
    if (!d.names || !d.place || !value.trim()) return;
    const up = aboveOf(placeKeys, level, value) ?? {};
    for (const l of LEVELS) {
      const name = up[l];
      if (!name || d.names[l] === name) continue;
      d.names[l] = name;
      d.place[l] = (r.current?.place[l] ?? "") !== name;
    }
  }
  function addKeyword(d: Draft, input: HTMLInputElement) {
    const path = input.value.split("|").map((l) => l.trim()).join("|");
    if (!path) return;
    if (path.split("|").some((l) => !l)) return onerror("A keyword level is empty.");
    if (path.includes(",")) return onerror("Keyword names cannot contain a comma.");
    const known = keywords.find((k) => k.toLowerCase() === path.toLowerCase()) ?? path;
    if (!(known in d.keywords)) d.added = [...d.added, known];
    d.keywords[known] = true;
    input.value = "";
  }

  // ---- what was reviewed, and lessons from it -----------------------------------------
  let reviews = $state.raw<api.Review[]>([]);
  let reviewShown = $state<ReviewShown>("all");
  const ticked = new SvelteSet<number>();
  let listed = $derived(reviews.filter((r) => shows(r, reviewShown)));
  /** What refining is asked from: the ticked reviews, or all listed. */
  let basis = $derived(ticked.size ? listed.filter((r) => ticked.has(r.seq)) : listed);
  let basisLines = $derived(basis.map(digest).filter((l): l is string => !!l));
  async function loadReviews() {
    const mine = asked;
    try {
      const list = await api.aiReviews();
      if (mine !== asked) return;
      reviews = list;
      const seqs = new Set(list.map((r) => r.seq));
      for (const s of [...ticked]) if (!seqs.has(s)) ticked.delete(s);
    } catch (e) {
      onerror(String(e));
    }
  }
  async function removeReviews(seqs: number[]) {
    if (!seqs.length) return;
    try {
      await api.deleteAiReviews(seqs);
      await loadReviews();
    } catch (e) {
      onerror(String(e));
    }
  }
  let refined = $state<api.Refined | null>(null);
  let refining = $state(false);
  async function refine() {
    refining = true;
    try {
      const unchanged = basis.filter((r) => r.status === "Accepted" && !changes(r).length).length;
      refined = await api.refineLessons(basisLines, unchanged);
    } catch (e) {
      onerror(String(e));
    } finally {
      refining = false;
    }
  }
  async function saveLessons() {
    if (!view || !refined) return;
    try {
      view = await api.setAiSettings({ ...view.settings, lessons: refined.lessons.trim() || null });
      refined = null;
    } catch (e) {
      onerror(String(e));
    }
  }
  const day = (ms: number) => new Date(ms).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });

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
      ...(r.suggestion.place && d.place ? placeReplaces(r.suggestion.place, d.place, now.place, d.names ?? undefined) : []),
    ].filter((x): x is string => !!x);
  }

  function pickOf(row: number): Accept {
    const d = drafts[row];
    const guess = rows.find((r) => r.row === row)?.suggestion.place;
    const point = guess?.point && d.place?.point ? ([guess.point.lat, guess.point.lon] as [number, number]) : undefined;
    return {
      place: guess && d.place ? takenPlace(guess, d.place, d.names ?? undefined) : undefined,
      point,
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
        notice = `${changed.length} photo${changed.length === 1 ? "" : "s"} changed since shown: the caption, headline, rating or place written since was kept. The card shows it now; accept again to replace it.`;
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
</script>

<svelte:window onkeydowncapture={closeSheet} />

<div class="ai">
  <div class="bar">
    <div class="run">
      {#if describe?.running}
        <span>
          Describing <b>{describe.done.toLocaleString()}</b> of {describe.total.toLocaleString()}
          <span class="muted">
            {#if bySheet && lead}
              · sheet {lead.call} of {describe.calls_total.toLocaleString()} ({lead.rows.length} photos): {phase(lead)}, {duration(now - lead.since)}{describe.units.length > 1
                ? ` · ${describe.units.length - 1} more at work`
                : ""}
            {:else}
              · {describe.working.length} at work
            {/if}
            · {duration(elapsed)}
            {#if left !== null}
              · about {duration(left)} left
            {:else if !lead || now - lead.since < 10_000}
              {bySheet ? "· a sheet's answer takes a minute or two" : "· the first answers take a few seconds"}
            {/if}
            {describe.failed ? `· ${describe.failed} failed` : ""}{describe.cost ? ` · ${money(describe.cost)}` : ""}
          </span>
        </span>
        {#if bySheet}
          <progress max={describe.calls_total} value={describe.calls_done}></progress>
        {:else}
          <progress max={describe.total} value={describe.done}></progress>
        {/if}
        <button class="btn" onclick={() => api.stopDescribe().catch((e) => onerror(String(e)))}>Stop</button>
      {:else if estimate}
        {@const e = estimate.of}
        <span>
          {e.photos.toLocaleString()} photo{e.photos === 1 ? "" : "s"} with {e.model},
          {#if e.via === "Gemini"}{e.calls.toLocaleString()} call{e.calls === 1 ? "" : "s"} to Gemini: counts toward the account's quota{:else if e.plan}on your Claude plan: counts toward its usage limits{:else}about <b>{money(e.cost)}</b>{/if}{e.skipped
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
            ? view ? notReady(view) : ""
            : picked.length ? "" : "Pick photos in the strip below"}
          onclick={ask}
        >Describe {picked.length ? picked.length.toLocaleString() : ""} photo{picked.length === 1 ? "" : "s"}…</button>
        <label class="muted" title="Describe photos again even when they were described with this model and prompt">
          <input type="checkbox" bind:checked={again} /> again
        </label>
      {/if}
      {#if describe && !describe.running && describe.failed}
        <button class="warn" title="What happened, call by call" onclick={() => (details = true)}>
          ⚠ {describe.failed} failed{describe.error ? `: ${describe.error}` : ""}
        </button>
      {/if}
      {#if describe?.running || describe?.log.length}
        <button class="link" aria-expanded={details} onclick={() => (details = !details)}>Details {details ? "▾" : "▸"}</button>
      {/if}
    </div>
    <button class="link" title="Settings > AI: access, model, language" onclick={onsettings}>
      {#if view && !view.ready}
        {notReady(view).replace(/ first$| in Settings > AI first$/, "…")}
      {:else}
        {model?.name ?? "Settings"}{view?.settings.access === "plan"
          ? " · Claude plan"
          : view?.settings.access === "gemini"
            ? view.settings.sheets ? " · contact sheets" : ""
            : view ? ` · ${money(view.spent)} spent` : ""}
      {/if}
    </button>
  </div>

  {#if notice}
    <p class="notice">{notice} <button class="link" onclick={() => (notice = "")}>OK</button></p>
  {/if}

  {#if details && describe && (describe.running || describe.log.length)}
    <section class="details" aria-label="What the run is doing">
      {#each describe.units as u (u.call)}
        <div class="unit">
          <div class="unit-head">
            <b>{u.sheet ? "Sheet" : "Photo"} {u.call}</b>
            {#if u.sheet}<span class="muted">{u.rows.length} photos</span>{/if}
            <span class:warn={u.phase === "retrying"}>{phase(u)} · {duration(now - u.since)}</span>
            <span class="muted">· started {duration(now - u.began)} ago</span>
            {#if u.sheet && u.phase !== "preparing"}
              <button class="link" onclick={() => showSheet(u.call)}>Show sheet</button>
            {/if}
          </div>
          {#if u.note}<p class="muted note">{u.note}</p>{/if}
          <div class="strip">
            {#each u.rows as row (row)}
              <button class="busy" onclick={() => onselect(row)} title="Being described: show this photo">
                <img src={api.thumbUrl(row)} alt="" decoding="async" loading="lazy" />
              </button>
            {/each}
          </div>
        </div>
      {/each}
      {#if describe.log.length}
        <ol class="log" aria-label="What the run did, newest first">
          {#each [...describe.log].reverse() as line, i (describe.log.length - i)}
            <li class:fail={!line.ok}>
              <time class="muted">{clock(line.at)}</time>
              {line.text}
              {#if bySheet && line.call && !describe.units.some((u) => u.call === line.call) && /sent$/.test(line.text)}
                <button class="link" onclick={() => showSheet(line.call)}>sheet</button>
              {/if}
            </li>
          {/each}
        </ol>
      {/if}
    </section>
  {/if}

  {#if sheetShown !== null && describe}
    <div class="overlay" role="dialog" aria-modal="true" aria-label="Contact sheet {sheetShown}">
      <button class="overlay-close" aria-label="Close" onclick={() => (sheetShown = null)}></button>
      <figure>
        {#if sheetGone}
          <p>This sheet is no longer kept: only the last few are.</p>
        {:else}
          <img src={api.sheetUrl(describe.started, sheetShown)} alt="Contact sheet {sheetShown}" onerror={() => (sheetGone = true)} />
        {/if}
        <figcaption class="muted">Sheet {sheetShown}, as sent · Esc or a click closes it</figcaption>
      </figure>
    </div>
  {/if}

  {#if describe?.running && describe.working.length && !details}
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
      <button
        class:on={show === "Reviewed"}
        title="What you did with each suggestion: corrections to draw lessons for the prompt from"
        onclick={() => (show = "Reviewed")}
      >Reviewed</button>
    </nav>
    {#if show === "Reviewed"}
      <div class="bulk">
        <button class="btn" disabled={!ticked.size} onclick={() => removeReviews([...ticked])}>Remove {ticked.size || ""}</button>
        <button
          class="btn primary"
          disabled={refining || !basisLines.length}
          title="Asks Claude to turn these corrections into lessons for the prompt; you read them before they are kept"
          onclick={refine}
        >{refining ? "Asking Claude…" : `Refine the prompt from ${basisLines.length.toLocaleString()} correction${basisLines.length === 1 ? "" : "s"}…`}</button>
      </div>
    {:else if proposed.length}
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

  {#if show === "Reviewed"}
    {#if refined}
      <section class="refine" aria-label="Lessons proposed">
        <h4>Lessons proposed</h4>
        {#if refined.rationale}<p class="muted">{refined.rationale}</p>{/if}
        <div class="lessons">
          <div>
            <span class="muted">In use now</span>
            <pre>{view?.settings.lessons ?? "(none)"}</pre>
          </div>
          <div>
            <span class="muted">Proposed: edit before saving</span>
            <textarea rows="10" bind:value={refined.lessons} spellcheck="false"></textarea>
          </div>
        </div>
        <div class="actions">
          <button class="btn primary" onclick={saveLessons}>Use these lessons</button>
          <button class="btn" onclick={() => (refined = null)}>Discard</button>
          <span class="muted">They follow the instructions in every run from now on (Settings › AI › Prompt). Photos described before are described again only with "again".</span>
        </div>
      </section>
    {/if}
    <div class="bar">
      <nav class="parts small">
        {#each [["all", "All"], ["edited", "Corrected"], ["rejected", "Rejected"], ["unchanged", "Accepted as suggested"]] as const as [k, label] (k)}
          <button class:on={reviewShown === k} onclick={() => { reviewShown = k; ticked.clear(); }}>
            {label} <span class="muted">{reviews.filter((r) => shows(r, k)).length.toLocaleString()}</span>
          </button>
        {/each}
      </nav>
      {#if listed.length}
        <label class="muted">
          <input
            type="checkbox"
            checked={ticked.size === listed.length}
            onchange={(e) => { ticked.clear(); if (e.currentTarget.checked) for (const r of listed) ticked.add(r.seq); }}
          /> all
        </label>
      {/if}
    </div>
    {#if !listed.length}
      <p class="muted empty">Nothing yet: suggestions you accept or reject are listed here, with what you changed.</p>
    {/if}
    <ul class="reviews">
      {#each listed as r (r.seq)}
        {@const list = changes(r)}
        <li>
          <input type="checkbox" checked={ticked.has(r.seq)} onchange={(e) => (e.currentTarget.checked ? ticked.add(r.seq) : ticked.delete(r.seq))} />
          {#if r.row !== null}
            <button class="mini" onclick={() => onselect(r.row!)} title="Show this photo"><img src={api.thumbUrl(r.row)} alt="" loading="lazy" decoding="async" /></button>
          {:else}
            <span class="mini gone" title="The photo is no longer in the {source.noun}"></span>
          {/if}
          <div class="what">
            <div class="muted">{day(r.at)} · {r.suggestion.headline || r.suggestion.subject || "—"} · {r.model}</div>
            <div class="changes">
              {#each list as c, i (i)}
                <span class="change" class:rejected={c.kind === "rejected"}>{chipText(c)}</span>
              {:else}
                <span class="muted">accepted as suggested</span>
              {/each}
            </div>
          </div>
        </li>
      {/each}
    </ul>
  {:else if !rows.length}
    <p class="muted empty">
      {show === "Proposed" ? "Nothing to review. Pick photos in the strip below and describe them." : "None."}
    </p>
  {/if}
  <datalist id="ai-keyword-list">
    {#each keywords as k (k)}<option value={k}></option>{/each}
  </datalist>
  {#each LEVELS as level (level)}
    <datalist id="ai-place-{level}">
      {#if placeFocus?.level === level}{#each placeOptions as name (name)}<option value={name}></option>{/each}{/if}
    </datalist>
  {/each}
  <div class="cards" hidden={show === "Reviewed"}>
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
            <div class="keywords">
              {#each Object.keys(d.keywords) as k (k)}
                {@const already = has(r, k)}
                {@const mine = d.added.includes(k)}
                <label
                  class="kw"
                  class:new={s.keywords_new.includes(k) || (mine && !keywords.includes(k))}
                  class:added={mine}
                  class:has={already}
                  title={already ? "The photo has it already" : mine ? `Added by you${keywords.includes(k) ? "" : `: new, added to the ${source.noun}`}` : s.keywords_new.includes(k) ? `New: added to the ${source.noun}` : k}
                >
                  <input type="checkbox" checked={already || d.keywords[k]} disabled={already} onchange={(e) => (d.keywords[k] = e.currentTarget.checked)} />{leaf(k)}
                </label>
              {/each}
              {#if r.status === "Proposed"}
                <input
                  class="kw-add"
                  list="ai-keyword-list"
                  placeholder="+ keyword"
                  title="Add a keyword the model missed: Enter adds it"
                  onkeydown={(e) => { if (e.key === "Enter") { e.preventDefault(); addKeyword(d, e.currentTarget); } }}
                />
              {/if}
            </div>
            {#if s.place && d.place && d.names}
              {@const g = s.place}
              {@const ticks = d.place}
              {@const names = d.names}
              <div class="place" title="Where the model thinks it was taken, with how likely each is right. Write another name to correct it: the {source.noun}'s names are offered.">
                {#each LEVELS as level (level)}
                  {@const guess = g[level]}
                  {@const has = now?.place[level]}
                  {@const name = names[level].trim()}
                  <span
                    class="level"
                    class:over={ticks[level] && !!has && has !== name}
                    title={has ? `${level}: the photo has ${has}` : `${level}: the photo has none`}
                  >
                    <input type="checkbox" bind:checked={ticks[level]} disabled={!name} aria-label="Take the {level}" />
                    <input
                      class="name"
                      class:edited={name !== (guess?.name ?? "")}
                      list="ai-place-{level}"
                      placeholder={level}
                      value={names[level]}
                      size={Math.max(6, Math.min(22, names[level].length + 1))}
                      onfocus={() => (placeFocus = { row: r.row, level })}
                      oninput={(e) => nameTyped(d, level, e.currentTarget.value)}
                      onchange={(e) => nameChosen(d, r, level, e.currentTarget.value)}
                    />
                    {#if guess && name === guess.name}<span class="muted">{percent(guess.p)}</span>{/if}
                  </span>
                {/each}
                {#if g.point}
                  <label
                    class="level"
                    title={now?.placed ? "The photo has a position: a guessed one is never given" : `About here, within ${g.point.radius_km} km: shown as ≈`}
                  >
                    <input type="checkbox" disabled={!!now?.placed} bind:checked={ticks.point} />≈ {g.point.lat.toFixed(3)}, {g.point.lon.toFixed(3)}
                    <span class="muted">{percent(g.point.p)}</span>
                  </label>
                {/if}
              </div>
            {/if}
            <div class="facts muted">
              {#if s.rating}
                <label title={now?.rating ? "Replace the photo's rating" : "Take the rating"}>
                  <input type="checkbox" bind:checked={d.takeRating} />
                </label>
                <span class="stars" role="group" aria-label="Rating to take" title="Click a star to give another rating">
                  {#each [1, 2, 3, 4, 5] as n (n)}
                    <button class="star" onclick={() => { d.rating = n; d.takeRating = true; }}>{(d.rating ?? 0) >= n ? "★" : "☆"}</button>
                  {/each}
                </span>
                {#if now?.rating && now.rating !== d.rating}<span title="The photo's rating now">now <span class="stars">{"★".repeat(now.rating)}</span></span>{/if}
              {/if}
              {#if s.quality}<span>quality {s.quality}/5</span>{/if}
              {#if s.composition}<span>composition {s.composition}/5</span>{/if}
              {#if s.subject && s.headline}<span>subject: {s.subject}</span>{/if}
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
  .cards[hidden] { display: none; }
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
  .kw.added { border-color: #5b8def; }
  .kw-add { width: 120px; font-size: 12px; padding: 1px 6px; border-radius: 10px; }
  .place .name { font: inherit; font-size: 12px; padding: 0 4px; border: 1px solid transparent; background: transparent; color: inherit; }
  .place .name:hover, .place .name:focus { border-color: var(--line, #444); }
  .place .name.edited { color: #8fb3ff; }
  .star { padding: 0; color: inherit; font-size: 13px; line-height: 1; }
  .parts.small button { padding: 3px 10px; font-size: 12px; }
  .reviews { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .reviews li { display: flex; align-items: center; gap: 8px; padding: 4px 6px; border: 1px solid var(--line, #333); border-radius: 4px; }
  .mini { padding: 0; flex: none; }
  .mini img, .mini.gone { display: block; width: 48px; height: 48px; object-fit: cover; border-radius: 3px; background: #111; }
  .what { min-width: 0; display: flex; flex-direction: column; gap: 3px; }
  .changes { display: flex; flex-wrap: wrap; gap: 4px; }
  .change { font-size: 12px; padding: 0 6px; border: 1px solid var(--line, #444); border-radius: 10px; }
  .change.rejected { border-color: #c66; color: #e99; }
  .refine { margin: 0 0 12px; padding: 8px 10px; border: 1px solid #5b8def; border-radius: 4px; }
  .refine h4 { margin: 0 0 4px; }
  .lessons { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; margin: 6px 0; }
  .lessons pre { white-space: pre-wrap; font-size: 12px; margin: 4px 0 0; }
  .lessons textarea { width: 100%; font: inherit; font-size: 12px; margin-top: 4px; }
  .facts { display: flex; flex-wrap: wrap; gap: 10px; align-items: center; }
  .facts label { display: inline-flex; align-items: center; gap: 4px; }
  .stars { color: #e8b03a; letter-spacing: 1px; }
  .place { display: flex; flex-wrap: wrap; gap: 4px 10px; font-size: 12px; }
  .place .level { display: inline-flex; align-items: center; gap: 4px; }
  .place .over { text-decoration: line-through; text-decoration-color: #e8b03a; }
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
  .details { margin: 0 0 12px; padding: 8px 10px; border: 1px solid var(--line, #333); border-radius: 4px; background: rgba(255, 255, 255, 0.03); font-size: 12px; }
  .unit { padding: 4px 0 8px; border-bottom: 1px solid var(--line, #333); margin-bottom: 6px; }
  .unit-head { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
  .unit .note { margin: 2px 0 0; }
  .strip { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 6px; }
  .strip .busy img { width: 36px; height: 36px; }
  .log { list-style: none; margin: 0; padding: 0; max-height: 220px; overflow-y: auto; }
  .log li { padding: 1px 0; }
  .log li.fail { color: #e8b03a; }
  .log time { margin-right: 6px; font-variant-numeric: tabular-nums; }
  .overlay { position: fixed; inset: 0; z-index: 50; display: grid; place-items: center; background: rgb(0 0 0 / 80%); }
  .overlay-close { position: absolute; inset: 0; cursor: zoom-out; }
  .overlay figure { position: relative; margin: 0; pointer-events: none; text-align: center; }
  .overlay img { max-width: 92vw; max-height: 86vh; display: block; }
  .overlay p { color: #ddd; }
</style>
