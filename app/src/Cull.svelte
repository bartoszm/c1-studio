<script lang="ts">
  import { onDestroy, onMount, untrack } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { listen } from "@tauri-apps/api/event";
  import * as api from "./lib/api";
  import type { Filter } from "./lib/api";
  import Compare from "./Compare.svelte";
  import { REST_CHOICES, SUGGEST_CHOICES, askCost, askPlanKey, askRecipient, cullKey, keepers, leftOutNote, lookedAt, restLabel, restOf, runToggle, shownRows, stepGroup, stepPhoto, suggestLabel, suggestTitle, suggestedKeep, unseenNote, type RestChoice } from "./lib/cull";
  import { settings } from "./lib/settings.svelte";

  // Choosing the best of alike photos: the duplicate groups or series under the filter, one
  // at a time, or the photos picked by hand as one group. The group's photos are compared
  // side by side (four at a time of a big one, all of them in the strip below), the ones to
  // keep are suggested (the best, and as Settings › Culling say a few more), and Enter keeps
  // the active photo and the marked ones and deals with the rest as the settings say.
  let { groups, at, series, selection = false, canSelect = false, filter, version, blocked, busy, done, removes, message, failed, onclose, onat, onstale, onseries, onselection, onactive, onkeep, onsettings }: {
    /** Open Settings › AI, where the local model is downloaded. */
    onsettings: () => void;
    /** The groups' ids, in the order they are gone through. */
    groups: string[];
    /** Which of them is shown. */
    at: number;
    /** Series (bursts), not duplicate groups. */
    series: boolean;
    /** The photos picked by hand, as one group, not the groups the analysis found. */
    selection?: boolean;
    /** Photos were picked by hand: they can be chosen among (`onselection`). */
    canSelect?: boolean;
    onselection?: () => void;
    /** What the grid shows: only a group's photos under it are chosen among. */
    filter: Filter;
    /** Bumped after edits, to read the group again. */
    version: number;
    /** A dialog or menu is open over this: the keys are its own. */
    blocked: boolean;
    /** A pick is being made: the next one waits. */
    busy: boolean;
    /** Groups already picked from, this time. */
    done: string[];
    /** Whether the source has a trash to move the rest to. */
    removes?: api.Removal;
    /** The footer's word (work under way, an error, a note), which this covers. */
    message?: string | null;
    failed?: boolean;
    onclose: () => void;
    onat: (at: number) => void;
    /** The group shown has no photos left under the filter: the groups may be named
     *  otherwise by now, and are to be listed again. `row`: a photo that was in it. */
    onstale: (row: number | undefined) => void;
    onseries: (series: boolean) => void;
    /** The photo edits land on, by row. */
    onactive: (row: number | undefined) => void;
    /** Keep `keep` of the group and deal with `rest`. */
    onkeep: (group: string, keep: number[], rest: number[]) => void;
  } = $props();

  let id = $derived<string | undefined>(groups[at]);
  let view = $state<api.CullGroup | null>(null);
  let error = $state<string | null>(null);
  /** The photo edits land on and Enter keeps, by row. */
  let active = $state<number | undefined>(undefined);
  /** Photos marked to be kept too (`k`). */
  const marked = new SvelteSet<number>();
  /** The photos side by side: all of a small group, four of a big one. */
  let shown = $state<number[]>([]);

  /** `view` is the group shown, not still the one before it while the next is read. */
  let current = $derived(view !== null && view.id === id);
  let rows = $derived(view?.photos.map((p) => p.row) ?? []);
  let byRow = $derived(new Map(view?.photos.map((p) => [p.row, p]) ?? []));
  let best = $derived(view?.order[0]);
  /** The photos suggested to keep, the best first. */
  let suggestion = $derived(view ? suggestedKeep(view, settings.cull.suggest) : []);
  /** Each photo's place in the order of which to keep, from 1. */
  let rank = $derived(new Map(view?.order.map((row, i) => [row, i + 1]) ?? []));

  let asked = 0;
  /** The next group, read while this one is looked at: good until an edit (`version`). */
  const ahead = new Map<string, api.CullGroup>();
  /** Groups found empty and reported, so that one is reported once. */
  const reported = new Set<string>();
  /** Shows a group as read. `same`: the one shown already, after an edit, so the active
   *  photo and the marks stay. */
  function show(next: api.CullGroup, same: boolean) {
    const now = next.photos.map((p) => p.row);
    if (!same) {
      // A group shown afresh: the ones suggested are marked to keep, the best active.
      marked.clear();
      const keep = suggestedKeep(next, settings.cull.suggest);
      for (const row of keep.slice(1)) marked.add(row);
      active = keep[0] ?? next.order[0];
    }
    for (const row of [...marked]) if (!now.includes(row)) marked.delete(row);
    if (active === undefined || !now.includes(active)) active = next.order[0];
    shown = shownRows(now, active, same ? shown : []);
    view = next;
    error = null;
  }
  /** Reads the group. `same`: after an edit. The group before stays on screen until this
   *  one is there (no empty screen between two groups), but takes no pick and no edit. */
  async function read(same: boolean) {
    const mine = ++asked;
    const group = id;
    if (group === undefined) {
      view = null;
      return;
    }
    const ready = same ? undefined : ahead.get(group);
    if (ready) {
      ahead.delete(group);
      return show(ready, false);
    }
    try {
      const next = await api.cullGroup($state.snapshot(filter) as Filter, group);
      if (mine !== asked) return;
      const was = view?.id === group;
      if (!next.photos.length && !reported.has(group)) {
        reported.add(group);
        onstale(was ? active : undefined);
      }
      show(next, same && was);
    } catch (e) {
      if (mine !== asked) return;
      if (view?.id !== group) view = null;
      error = String(e);
    }
  }
  let seen = untrack(() => version);
  $effect(() => {
    void id;
    untrack(() => {
      // An edit that came with the change of group: what was read ahead is of before it.
      if (version !== seen) ahead.clear();
      seen = version;
      read(false);
    });
  });
  $effect(() => {
    const now = version;
    untrack(() => {
      if (now === seen) return;
      seen = now;
      ahead.clear();
      read(true);
    });
  });
  // While the next group is read, the photo of the one before takes no edit.
  $effect(() => onactive(current ? active : undefined));

  /** The local model is looking at this group. */
  let looking = $state(false);
  /** It is looking at every group, in the background: how far it has got. */
  let ranking = $state<api.RankState | null>(null);
  /** Why the last look, or the last question to a model, could not be made (the model
   *  would not load, no key, another run goes on): said in the bar, since the group stays
   *  on screen. */
  let lookError = $state<string | null>(null);
  /** The group it was last asked to look at by hand. */
  let lookedGroup = $state<string | undefined>(undefined);
  /** The photos of the group shown it could not look at, in words. */
  let unseen = $derived(view && current ? unseenNote(view.unseen, view.photos.length, lookedGroup === view.id) : "");
  async function rankThis() {
    const group = id;
    if (looking || !view || group === undefined) return;
    if (!view.model) return onsettings();
    looking = true;
    lookError = null;
    try {
      const next = await api.rankGroup($state.snapshot(filter) as Filter, group);
      // The answer is of the group asked about: another may be shown by now.
      ahead.delete(group);
      lookedGroup = group;
      if (id === group) {
        const same = view?.id === group;
        const follows = active === best;
        show(next, same);
        // The active photo follows the suggestion while it was on it.
        if (same && follows) setActive(next.order[0]);
      }
    } catch (e) {
      lookError = String(e);
    } finally {
      looking = false;
    }
  }
  /** Said when a run's button or key is used while the other kind of run goes on. */
  const OTHER_RUN = {
    cloud: "AI is being asked about every group: stop that first (Shift+A)",
    local: "The local model is looking at every group: stop that first (Shift+R)",
  };
  async function rankAll() {
    if (view && !view.model) return onsettings();
    const what = runToggle(ranking, false);
    // Not the run asking a model: this key does not stop what is paid for.
    lookError = what === "other" ? OTHER_RUN.cloud : null;
    try {
      if (what === "stop") await api.stopRanking();
      else if (what === "start") await api.startRanking($state.snapshot(filter) as Filter, series);
    } catch (e) {
      lookError = String(e);
    }
  }

  /** A model (Claude or Gemini) is comparing this group's frames. */
  let asking = $state(false);
  /** Every group is being asked about: this one is not asked about by hand meanwhile. */
  let askingAll = $derived(runToggle(ranking, true) === "stop");
  async function askThis() {
    const group = id;
    if (asking || !view || !current || group === undefined || view.photos.length < 2) return;
    if (askingAll) {
      lookError = OTHER_RUN.cloud;
      return;
    }
    asking = true;
    lookError = null;
    try {
      const next = await api.askGroup($state.snapshot(filter) as Filter, group);
      // The answer is of the group asked about: another may be shown by now.
      ahead.delete(group);
      if (id === group) {
        const same = view?.id === group;
        const follows = active === best;
        show(next, same);
        if (same && follows) setActive(next.order[0]);
      }
    } catch (e) {
      lookError = String(e);
    } finally {
      asking = false;
    }
  }
  /** What asking about every group would take, shown before it starts. */
  let askPlan = $state<api.AskEstimate | null>(null);
  /** The groups not asked about yet are being counted, for `askPlan`. */
  let counting = $state(false);
  async function askAll() {
    if (counting || askPlan) return;
    const what = runToggle(ranking, true);
    // Not the local model's run: this key does not stop it.
    lookError = what === "other" ? OTHER_RUN.local : null;
    try {
      if (what === "stop") await api.stopRanking();
      if (what !== "start") return;
      counting = true;
      askPlan = await api.askEstimate($state.snapshot(filter) as Filter, series);
    } catch (e) {
      lookError = String(e);
    } finally {
      counting = false;
    }
  }
  /** The question can be answered with a yes: there are groups to ask about, and a model to ask. */
  let askReady = $derived(!!askPlan && askPlan.groups > 0 && !askPlan.unable);
  async function askAllGo() {
    if (!askReady) return;
    askPlan = null;
    lookError = null;
    try {
      await api.startRanking($state.snapshot(filter) as Filter, series, true);
    } catch (e) {
      lookError = String(e);
    }
  }
  // The question about asking every group takes every key until it is answered, before
  // anyone else hears it: the window's other listeners would rate, tag or trash the photo
  // behind it. A dialog or menu over this screen (`blocked`) has the keys instead.
  $effect(() => {
    if (!askPlan) return;
    const taken = (e: KeyboardEvent) => {
      if (blocked) return;
      const onButton = !!(e.target as HTMLElement | null)?.closest?.(".ask button");
      const what = askPlanKey(e, onButton);
      e.stopPropagation();
      if (what === "own") return;
      // Stopped above, it reaches no one here; a key of the window's own (F12) still works,
      // but none presses a button of the bar left focused.
      if (what !== "none" || e.key === "Enter" || e.key === " ") e.preventDefault();
      if (what === "go") askAllGo();
      else if (what === "cancel") askPlan = null;
    };
    window.addEventListener("keydown", taken, true);
    return () => window.removeEventListener("keydown", taken, true);
  });
  const PROVIDERS: Record<string, string> = { claude: "Claude", "claude-code": "Claude", gemini: "Gemini" };
  onMount(() => {
    api.rankingState().then((s) => (ranking = s)).catch(() => {});
    const heard = [
      // The event names the group just looked at: only when that is the one shown is it
      // read again, not for every group of a long run. One read ahead is of before the look.
      listen<api.RankState>("ranking", (e) => {
        const before = ranking;
        ranking = e.payload;
        if (e.payload.group !== null) ahead.delete(e.payload.group);
        if (lookedAt(before, e.payload, id)) read(true);
      }),
      // A run stopped in the middle of the group shown has looked at a part of it.
      listen("ranking-done", () => { if (view?.unseen) read(true); }),
    ];
    return () => heard.forEach((h) => h.then((off) => off()));
  });
  // The model is let go with the screen (a run in the background keeps it until it ends).
  onDestroy(() => { api.cullDone().catch(() => {}); });
  // Back from Settings, where the model is downloaded: it may be there now.
  let covered = untrack(() => blocked);
  $effect(() => {
    const now = blocked;
    untrack(() => {
      if (covered && !now && view && !view.model) read(true);
      covered = now;
    });
  });

  function setActive(row: number | undefined) {
    if (row === undefined) return;
    active = row;
    shown = shownRows(rows, row, shown);
  }
  /** Back to the suggestion: the best active, the others suggested marked. */
  function suggest() {
    if (!view || !current) return;
    marked.clear();
    for (const row of suggestion.slice(1)) marked.add(row);
    setActive(suggestion[0] ?? best);
  }
  function chooseSuggest(choice: (typeof SUGGEST_CHOICES)[number]) {
    settings.cull.suggest = choice;
    suggest();
  }
  function toggleMark(row = active) {
    if (row === undefined) return;
    if (marked.has(row)) marked.delete(row);
    else marked.add(row);
  }
  function keep() {
    if (busy || !view || !current || id === undefined || active === undefined) return;
    const kept = keepers(rows, active, marked);
    if (!kept.length) return;
    onkeep(id, kept, restOf(rows, kept));
  }
  const go = (delta: 1 | -1) => onat(stepGroup(at, groups.length, delta));

  /** Our keys come before the comparison's: the arrows move through the whole group, not
   *  only the photos side by side. */
  function onkey(e: KeyboardEvent): boolean {
    const action = cullKey(e, blocked);
    if (!action) return false;
    e.preventDefault();
    switch (action.kind) {
      case "keep": keep(); break;
      case "mark": toggleMark(); break;
      case "group": go(action.delta); break;
      case "photo": setActive(stepPhoto(rows, active, action.delta)); break;
      case "best": suggest(); break;
      // Photos picked by hand are one group: all of them is this one.
      case "rank": if (action.all && !selection) rankAll(); else rankThis(); break;
      case "ask": if (action.all && !selection) askAll(); else askThis(); break;
    }
    return true;
  }

  // The next group is read, and the photos it will show side by side fetched, while this one
  // is looked at: once the group shown has stayed a moment, so that going through many with
  // a key held down reads none of them ahead.
  $effect(() => {
    const next = groups[at + 1];
    const edit = version;
    if (next === undefined || ahead.has(next)) return;
    const f = $state.snapshot(filter) as Filter;
    // Gone on from, or edited since: what comes back then is not kept (a group read before a
    // pick from it would show the photos the pick sent away).
    let left = false;
    const timer = setTimeout(() => {
      api.cullGroup(f, next).then((g) => {
        if (left || edit !== seen || !g.photos.length) return;
        ahead.clear();
        ahead.set(next, g);
        for (const row of shownRows(g.photos.map((p) => p.row), g.order[0])) new Image().src = api.largeUrl(row);
      }).catch(() => {});
    }, AHEAD_MS);
    return () => {
      left = true;
      clearTimeout(timer);
    };
  });

  let strip: HTMLElement | undefined = $state();
  $effect(() => {
    const row = active;
    void view;
    if (row === undefined) return;
    strip?.querySelector(`[data-row="${row}"]`)?.scrollIntoView({ block: "nearest", inline: "nearest" });
  });

  /** How long a group is shown before the next one is read ahead. */
  const AHEAD_MS = 150;
  const kind = $derived(selection ? "selection" : series ? "series" : "duplicate group");
  const judged = $derived.by(() => {
    if (!view || best === undefined) return "";
    const p = byRow.get(best);
    if (!p) return "";
    const why = p.reasons.length ? `: ${p.reasons.join(", ")}` : "";
    if (view.verdict) {
      const v = view.verdict;
      const of = v.sent < view.photos.length ? ` (${v.sent} of the ${view.photos.length} frames)` : "";
      return `${PROVIDERS[v.provider] ?? v.provider} suggests ${p.name}${why}${of}${v.note ? ` · ${v.note}` : ""}`;
    }
    const more = suggestion.length > 1 ? ` (and ${suggestion.length - 1} more worth keeping)` : "";
    return view.close ? `Too close to call; ${p.name} by a hair${why}${more}` : `Suggested: ${p.name}${why}${more}`;
  });
  const JUDGES: Record<api.Judge, string> = {
    cloud: "A model was shown these frames together and put them in order; frames it was not shown follow. Your own pick is still yours: Enter keeps the active photo.",
    composition: "By composition: where the subject sits (on a third, in the middle), whether the edge cuts it, busy edges, a level horizon, with the local model's view of composition once it has looked. Your rating comes first and blurry frames last; sharpness and edits decide between frames composed alike.",
    local: "By the local model's view of each photo, with sharpness and your own marks",
    sharpness: "By sharpness, clipped tones and your own marks (rating, edits)",
  };
  const pct = (v: number) => Math.round(v * 100);
  /** The parts of a photo's framing, for the title of its composition score. */
  function framingTitle(f: api.Framing | null): string {
    const parts = ["How well composed, 0 to 100, next to the others of the group"];
    if (f) {
      parts.push(`subject placed ${pct(f.placement)}${f.subject ? "" : " (nothing stands out)"}`);
      if (f.cut > 0) parts.push(`cut by the edge ${pct(f.cut)}`);
      parts.push(`busy edges ${pct(f.clutter)}`);
      if (f.tilt !== null) parts.push(`horizon ${Math.abs(f.tilt) < 0.25 ? "level" : `${Math.abs(f.tilt).toFixed(1)}° off level`}`);
    }
    return parts.join(" · ");
  }
  const restTitle ="What Enter does with the photos of the group you do not keep. More in Settings › Culling.";
</script>

<!-- With nothing to compare, the comparison is not there to pass the keys on: they still
     close this and move to another group. -->
<svelte:window
  onkeydown={(e) => {
    if (view && shown.length) return;
    if (!onkey(e) && e.key === "Escape" && !blocked && !e.defaultPrevented) {
      e.preventDefault();
      onclose();
    }
  }}
/>

{#snippet badge(row: number)}
  {@const p = byRow.get(row)}
  {#if row === best}<span class="badge best" title={p?.reasons.join(", ")}>suggested</span>
  {:else if suggestion.includes(row)}<span class="badge best also" title={p?.reasons.join(", ")}>suggested too</span>{/if}
  {#if marked.has(row)}<span class="badge keep">keep</span>{:else if p?.kept}<span class="badge keep was" title="Picked before as the one to keep">kept</span>{/if}
  {#if p && row !== best && p.reasons.length}<span class="why">{p.reasons.join(", ")}</span>{/if}
  {#if p && p.composition !== null}
    <span class="why" title={framingTitle(p.framing)}>composition {pct(p.composition)}</span>
  {/if}
  {#if p?.aesthetic}
    {@const a = p.aesthetic}
    <span class="why" title="How the local model saw this photo, 0 to 100, above 50 the better side. It tells the frames of a group apart; it is no rating.">
      model: composition {pct(a.composition)} · moment {pct(a.moment)} · mood {pct(a.mood)} · technical {pct(a.technical)}
    </span>
  {/if}
{/snippet}

<div class="cull">
  <div class="bar">
    <div class="kinds" role="group" aria-label="Kind of group">
      <button class="toggle" class:on={!selection && series} aria-pressed={!selection && series} title="Bursts: photos from one camera, seconds apart, looking alike" onclick={() => onseries(true)}>Series</button>
      <button class="toggle" class:on={!selection && !series} aria-pressed={!selection && !series} title="The same frame twice: different images taken at the same instant" onclick={() => onseries(false)}>Duplicates</button>
      {#if canSelect || selection}
        <button class="toggle" class:on={selection} aria-pressed={selection} title="The photos you picked in the grid, as one group" onclick={() => onselection?.()}>Picked</button>
      {/if}
    </div>
    <span class="where">
      {#if selection}
        <strong>Best of {view && current ? view.photos.length.toLocaleString() : ""} picked photo{view && current && view.photos.length === 1 ? "" : "s"}</strong>
        {#if id !== undefined && done.includes(id)}<span class="done">✓ picked</span>{/if}
        {#if view && current && view.left_out}<span class="unseen" title="These photos have no thumbnail or preview that could be read: they could not be measured, and are not shown">{leftOutNote(view.left_out)}</span>{/if}
      {:else}
      <button title="Previous group (↑)" aria-label="Previous group" disabled={at <= 0} onclick={() => go(-1)}>‹</button>
      {#if groups.length}
        <strong>{kind} {(at + 1).toLocaleString()} of {groups.length.toLocaleString()}</strong>
        {#if view && current}<span class="muted">· {view.photos.length} photo{view.photos.length === 1 ? "" : "s"}</span>{/if}
        {#if id !== undefined && done.includes(id)}<span class="done">✓ picked</span>{/if}
      {:else}
        <strong>No {series ? "series" : "duplicates"} among these photos</strong>
      {/if}
      <button title="Next group, leaving this one as it is (↓)" aria-label="Next group" disabled={at >= groups.length - 1} onclick={() => go(1)}>›</button>
      {/if}
    </span>
    {#if view && current}<span class="judge" title={JUDGES[view.judge]}>{judged}</span>{/if}
    <span class="end">
      <span class="model">
        {#if view && !view.model}
          <button class="link" title="The local model (SigLIP 2, about 1.5 GB, run on this computer) can weigh composition, moment and mood too: get it in Settings › AI" onclick={onsettings}>Get the local model…</button>
        {:else}
          <button
            class="toggle"
            disabled={looking || !view}
            title="Have the local model look at this group's photos: composition, moment and mood count beside sharpness, where they tell the frames apart (r)"
            onclick={rankThis}
          >{looking ? "Looking…" : "Look"}</button>
          {#if !selection}
          <button
            class="toggle"
            class:on={ranking?.running && !ranking.cloud}
            disabled={ranking?.running && ranking.cloud}
            title={ranking?.running && !ranking.cloud
              ? "The local model is looking at every group, in the background: click to stop (Shift+R). What it saw so far is kept."
              : "Have the local model look at every group here, in the background, oldest first (Shift+R)"}
            onclick={rankAll}
          >{ranking?.running && !ranking.cloud ? `${ranking.done.toLocaleString()} of ${ranking.total.toLocaleString()} · Stop` : "Look at all"}</button>
          {/if}
          {#if ranking?.failed && !ranking.cloud}
            <span class="unseen" title="Photos Look at all could not look at: they have no thumbnail or preview yet (it makes none), or could not be read. Their groups are still decided by sharpness; Look on a group makes what is missing. events.log in the catalog's folder says why of each.">{ranking.failed.toLocaleString()} failed</span>
          {/if}
          {#if unseen}
            <span class="unseen" title="The local model could not look at every photo of this group (no picture of it could be read): its view of the others does not count until it has seen them all. events.log in the catalog's folder says why.">{unseen}</span>
          {/if}
        {/if}
        <button
          class="toggle"
          disabled={asking || askingAll || !view || !current || view.photos.length < 2}
          title="Ask AI (Claude or Gemini, as Settings › AI say) to compare this group's frames: it puts them in order and says in a few words what sets each apart: an expression, a gesture, the framing (a). One call, at most 16 frames, which are sent to Anthropic or Google and leave this computer.{view?.verdict ? ' This group has an answer: asking again is another call.' : ''}"
          onclick={askThis}
        >{asking ? "Asking…" : view && current && view.verdict ? "Ask again" : "Ask AI"}</button>
        {#if !selection}
        <button
          class="toggle"
          class:on={ranking?.running && ranking.cloud}
          disabled={counting || (ranking?.running && !ranking.cloud)}
          title={ranking?.running && ranking.cloud
            ? "AI is being asked about every group, in the background: click to stop (Shift+A). The answers so far are kept."
            : "Ask AI about every group here not asked about yet, in the background: says first how many calls that is, and what it costs (Shift+A)"}
          onclick={askAll}
        >{ranking?.running && ranking.cloud ? `${ranking.done.toLocaleString()} of ${ranking.total.toLocaleString()} · Stop` : counting ? "Counting…" : "Ask about all…"}</button>
        {/if}
        {#if ranking?.failed && ranking.cloud}
          <span class="unseen" title="Groups that got no answer: a photo with no image to send, an answer cut off or declined. They are not asked about again in this run; events.log in the catalog's folder says why of each.">{ranking.failed.toLocaleString()} failed</span>
        {/if}
        {#if ranking && !ranking.running && ranking.error}<span class="failed" title={ranking.error}>stopped: {ranking.error}</span>{/if}
        {#if lookError}<span class="failed" title={lookError}>{lookError}</span>{/if}
      </span>
      <span class="suggest" role="group" aria-label="How many to suggest keeping" title="How many of each group to suggest keeping: they are marked when the group is shown (g goes back to them). More in Settings › Culling.">
        Keep:
        {#each SUGGEST_CHOICES as choice (choice)}
          <button class="toggle" class:on={settings.cull.suggest === choice} aria-pressed={settings.cull.suggest === choice} title={suggestTitle(choice)} onclick={(e) => { chooseSuggest(choice); e.currentTarget.blur(); }}>{suggestLabel(choice)}</button>
        {/each}
      </span>
      <label class="rest" title={restTitle}>
        The rest:
        <select
          value={settings.cull.rest}
          onchange={(e) => { settings.cull.rest = e.currentTarget.value as RestChoice; e.currentTarget.blur(); }}
        >
          {#each REST_CHOICES as choice (choice)}
            <option value={choice}>{restLabel(choice, settings.cull, removes)}</option>
          {/each}
        </select>
      </label>
      <button class="primary" disabled={busy || !view || !current || active === undefined} title="Keep the active photo and the ones marked, deal with the rest, go to the next group (Enter)" onclick={keep}>
        Keep {marked.size + (active !== undefined && !marked.has(active) ? 1 : 0) || ""}
      </button>
      <button class="close" title="Close (Esc)" aria-label="Close" onclick={onclose}>×</button>
    </span>
  </div>

  {#if askPlan}
    {@const p = askPlan}
    <div class="ask" role="alertdialog" aria-label="Ask AI about every group">
      {#if p.unable}
        <span>AI cannot be asked as Settings › AI are: {p.unable}</span>
        <button onclick={() => { askPlan = null; onsettings(); }}>Settings › AI…</button>
      {:else if !p.groups}
        <span>Every group here has been asked about already{p.skipped ? ` (${p.skipped.toLocaleString()})` : ""}.</span>
      {:else}
        <span>
          Ask {p.via} ({p.model}) about {p.groups.toLocaleString()} group{p.groups === 1 ? "" : "s"}: {p.groups.toLocaleString()} call{p.groups === 1 ? "" : "s"}{p.workers > 1 && p.groups > 1 ? `, ${Math.min(p.workers, p.groups)} at a time` : ""},
          {askCost(p)}.
          <strong>{p.photos.toLocaleString()} photos are sent to {askRecipient(p.via)} and leave this computer</strong>
          ({p.via === "Gemini" ? "on numbered sheets, one a group" : "previews, up to 1024 px"}).
          {#if p.skipped}{p.skipped.toLocaleString()} asked about already are left out.{/if}
          It runs in the background and can be stopped; answers are kept as they come.
        </span>
        <button class="primary" onclick={askAllGo}>Ask (Enter)</button>
      {/if}
      <button onclick={() => (askPlan = null)}>{askReady ? "Cancel (Esc)" : "Close (Esc)"}</button>
    </div>
  {/if}

  {#if view && shown.length}
    {#key view.id}
      <Compare
        rows={shown}
        active={Math.max(0, active === undefined ? 0 : shown.indexOf(active))}
        {version}
        {blocked}
        {message}
        {failed}
        embedded
        hint="Enter keep the marked · k keep too · ← → photo · ↑ ↓ group · g back to the suggestion · r look · a ask AI · 0–5 - + * Delete edit · z zoom · Esc"
        onkeyfirst={onkey}
        {badge}
        {onclose}
        onactive={(i) => setActive(shown[i])}
        onrows={(next, i) => { shown = next; active = next[i]; }}
      />
    {/key}
  {:else}
    <p class="empty" class:failed={!!error}>
      {#if error}{error}
      {:else if !groups.length}Nothing to choose from here. Try the other kind, or a wider filter.
      {:else if view && selection}None of the picked photos are left under the filter{view.left_out ? ` (${leftOutNote(view.left_out)})` : ""}.
      {:else if view}None of this group's photos are left under the filter.
      {:else}reading the group…{/if}
    </p>
  {/if}

  {#if view && view.photos.length}
    <div class="strip" bind:this={strip}>
      {#each view.photos as p (p.row)}
        <button
          class="frame"
          class:active={p.row === active}
          class:shown={shown.includes(p.row)}
          data-row={p.row}
          title="{p.name}{p.reasons.length ? ` · ${p.reasons.join(', ')}` : ''}"
          onclick={(e) => { setActive(p.row); e.currentTarget.blur(); }}
          ondblclick={() => toggleMark(p.row)}
        >
          <img src={api.thumbUrl(p.row)} alt="" loading="lazy" decoding="async" draggable="false" />
          <span class="rank" class:best={p.row === best}>{rank.get(p.row) ?? ""}</span>
          {#if marked.has(p.row) || p.kept}<span class="tick" class:was={!marked.has(p.row)}>✓</span>{/if}
          {#if p.composition !== null}<span class="bar-comp" title="composition {pct(p.composition)}" style="width: {pct(p.composition)}%"></span>{/if}
          <span class="bar-score" style="width: {Math.round(p.score * 100)}%"></span>
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .cull {
    position: fixed;
    inset: 0;
    z-index: 10;
    display: grid;
    grid-template-rows: auto minmax(0, 1fr) auto;
    background: #111;
  }
  /* Over the photos, under the bar: the rows stay the bar, the photos, the strip. */
  .ask {
    position: absolute;
    top: 44px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 2;
    display: flex;
    align-items: center;
    gap: 12px;
    max-width: min(900px, 90vw);
    padding: 10px 14px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 6px;
    box-shadow: 0 6px 24px rgb(0 0 0 / 50%);
  }
  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 14px;
    padding: 6px 12px;
    background: var(--panel);
    border-bottom: 1px solid var(--line);
  }
  .kinds { display: flex; gap: 2px; }
  .where { display: flex; align-items: center; gap: 8px; }
  .where button { padding: 0 8px; font-size: 18px; line-height: 1.2; }
  .where button:disabled { opacity: 0.35; cursor: default; }
  .muted { color: var(--muted); }
  .done { color: var(--queued); }
  .judge { color: var(--muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; flex: 1 1 200px; }
  .end { display: flex; align-items: center; gap: 10px; margin-left: auto; }
  .rest { display: flex; align-items: center; gap: 6px; color: var(--muted); }
  .suggest { display: flex; align-items: center; gap: 2px; color: var(--muted); }
  .suggest .toggle { min-width: 26px; }
  .where .unseen { color: var(--muted); white-space: nowrap; }
  .model { display: flex; align-items: center; gap: 4px; }
  .model .failed { color: var(--error); max-width: 220px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .model .unseen { color: var(--muted); white-space: nowrap; }
  .close { padding: 0 6px; font-size: 20px; line-height: 1; color: var(--muted); }
  .close:hover { color: var(--text); }
  .empty { align-self: center; justify-self: center; color: var(--muted); }
  .empty.failed { color: var(--error); }

  .strip {
    display: flex;
    gap: 4px;
    padding: 6px 8px;
    overflow-x: auto;
    background: var(--panel);
    border-top: 1px solid var(--line);
  }
  .frame {
    position: relative;
    flex: 0 0 auto;
    width: 96px;
    height: 68px;
    padding: 0;
    overflow: hidden;
    background: #111;
    border: 2px solid transparent;
    border-radius: 3px;
    opacity: 0.6;
  }
  .frame.shown { opacity: 1; }
  .frame.active { border-color: var(--queued); }
  .frame img { width: 100%; height: 100%; object-fit: cover; }
  .rank {
    position: absolute;
    top: 2px;
    left: 2px;
    min-width: 16px;
    padding: 0 4px;
    font-size: 11px;
    color: #fff;
    background: rgb(0 0 0 / 65%);
    border-radius: 3px;
  }
  .rank.best { color: #111; background: var(--star); }
  .tick {
    position: absolute;
    top: 2px;
    right: 2px;
    padding: 0 4px;
    font-size: 11px;
    color: #111;
    background: var(--queued);
    border-radius: 3px;
  }
  .tick.was { opacity: 0.6; }
  .bar-score { position: absolute; left: 0; bottom: 0; height: 3px; background: var(--star); }
  .bar-comp { position: absolute; left: 0; bottom: 3px; height: 2px; background: var(--queued); opacity: 0.85; }

  /* What this screen says of a photo in the comparison's caption: its own, styled here. */
  .badge { padding: 0 6px; font-size: 11px; color: #111; border-radius: 3px; }
  .badge.best { background: var(--star); }
  .badge.best.also { opacity: 0.75; }
  .badge.keep { background: var(--queued); }
  .badge.was { opacity: 0.6; }
  .why { color: var(--muted); font-size: 12px; }
</style>
