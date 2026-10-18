<script lang="ts">
  import * as api from "./lib/api";
  import type { AnalysisState, Dim, Filter } from "./lib/api";
  import { HUES } from "./lib/format";
  import { emptyNote } from "./lib/analysis";
  import { COMBINE_NOTE, barRule, cardHint, type Card } from "./lib/explain";
  import { LIGHT_PROBLEMS, type AllOf } from "./lib/criteria";
  import AlikePanel from "./AlikePanel.svelte";

  type Key = "shape" | "hues" | "light" | "focus";
  let {
    filter, facets, total, analysis, version, stack, allOf, picked, ontoggle, onflag, ongroup, onshow, onstack, onclasses, onallof,
  }: {
    /** Cards asking for every pick rather than any. */
    allOf: AllOf;
    /** The picks as made (`filter` is as asked, with an "all" card's picks combined). */
    picked: Filter;
    onallof: (card: keyof AllOf) => void;
    filter: Filter;
    /** Counts over the photos the filter selects. */
    facets: Partial<Record<Dim, [string, number][]>>;
    total: number;
    analysis: AnalysisState | null;
    /** Bumped when photos were edited or measured again. */
    version: number;
    /** One photo per burst or duplicate group is shown (the counts follow). */
    stack: boolean;
    ontoggle: (key: Key, value: string) => void;
    onflag: (flag: "duplicates" | "series" | "analysed") => void;
    /** Show one group's photos in the grid. */
    ongroup: (id: string) => void;
    onshow: () => void;
    onstack: () => void;
    /** The lines between classes moved. */
    onclasses: () => void;
  } = $props();

  /** Each chart: what it explains, dimension, title, filter field, its values in order (empty:
   *  every value, by count), and the "all of" switch it has. Light is two cards: brightness
   *  (one per photo) and problems (a photo can have several). */
  type Chart = { card: Card; dim: Dim; title: string; key: Key; order: string[]; all?: keyof AllOf };
  const CHARTS: Chart[] = [
    { card: "shape", dim: "shape", title: "Shape", key: "shape", order: ["landscape", "portrait", "square", "panorama"] },
    { card: "hue", dim: "hue", title: "Main colour", key: "hues", order: [], all: "hues" },
    { card: "brightness", dim: "light", title: "Brightness", key: "light", order: ["dark", "medium", "bright"] },
    { card: "problems", dim: "light", title: "Light problems", key: "light", order: LIGHT_PROBLEMS, all: "problems" },
    { card: "focus", dim: "focus", title: "Focus", key: "focus", order: ["sharp", "soft", "blurry"] },
  ];

  function values(dim: Dim, order: string[]): [string, number][] {
    const all = facets[dim] ?? [];
    if (!order.length) return all;
    return order.map((v) => all.find(([x]) => x === v)).filter((e): e is [string, number] => !!e);
  }

  // The lines the classes are drawn with, for the hints: read again when they may have moved.
  let lines = $state<api.ClassSettings | null>(null);
  // Only the newest answer counts: an older one arriving late must not put back old lines.
  let linesAsked = 0;
  $effect(() => {
    void version;
    const mine = ++linesAsked;
    api.classSettings()
      .then((l) => { if (mine === linesAsked) lines = l; })
      .catch((e) => console.warn("analysis lines", e));
  });
  const flag = (name: string) => facets.flag?.find(([v]) => v === name)?.[1] ?? 0;
  let measured = $derived(flag("analysed"));

  type Part = "look" | "alike";
  function rememberedPart(): Part {
    try {
      return localStorage.getItem("analysisPart") === "alike" ? "alike" : "look";
    } catch {
      return "look";
    }
  }
  let part = $state<Part>(rememberedPart());
  const PARTS: [Part, string][] = [["look", "Look"], ["alike", "Alike"]];
  function show(next: Part) {
    part = next;
    try { localStorage.setItem("analysisPart", next); } catch { /* not remembered */ }
  }
  /** Arrow keys move between the tabs, and focus goes with them. */
  function partKey(e: KeyboardEvent, key: Part) {
    const step = e.key === "ArrowRight" || e.key === "ArrowDown" ? 1 : e.key === "ArrowLeft" || e.key === "ArrowUp" ? -1 : 0;
    if (!step) return;
    const next = PARTS[(PARTS.findIndex(([k]) => k === key) + step + PARTS.length) % PARTS.length][0];
    show(next);
    e.preventDefault();
    queueMicrotask(() => document.getElementById(`analysis-tab-${next}`)?.focus());
  }
</script>

<div class="analysis">
  <header class="bar">
    <div class="progress">
      {#if analysis?.running && analysis.total}
        Measuring photos: {analysis.done.toLocaleString()} / {analysis.total.toLocaleString()}
        <progress max={analysis.total} value={analysis.done}></progress>
      {:else if total > 0}
        {measured.toLocaleString()} of {total.toLocaleString()} photos measured
      {/if}
      {#if total > measured}
        <button class="link" class:on={filter.analysed === false} onclick={() => onflag("analysed")}>
          {(total - measured).toLocaleString()} not measured yet
        </button>
      {/if}
    </div>
    <button class="show" onclick={onshow}>Show {total.toLocaleString()} photos</button>
  </header>

  <div class="parts" role="tablist" aria-label="Analysis">
    {#each PARTS as [key, label] (key)}
      <button
        role="tab"
        id="analysis-tab-{key}"
        aria-selected={part === key}
        aria-controls="analysis-panel-{key}"
        tabindex={part === key ? 0 : -1}
        class:on={part === key}
        onclick={() => show(key)}
        onkeydown={(e) => partKey(e, key)}
      >{label}</button>
    {/each}
  </div>

  <div role="tabpanel" id="analysis-panel-{part}" aria-labelledby="analysis-tab-{part}">
  {#if part === "look"}
    <div class="stack">
      <button class="toggle" class:on={stack} aria-pressed={stack} onclick={onstack}>Stack bursts &amp; duplicates</button>
      <span class="muted">
        {stack
          ? "One photo per group, its sharpest: the counts below leave the others out."
          : "Every photo counts. Stack to show one photo per burst or duplicate group."}
      </span>
    </div>
    <div class="charts">
      {#each CHARTS as { card, dim, title, key, order, all } (card)}
        {@const list = values(dim, order)}
        {@const max = Math.max(1, ...list.map(([, n]) => n))}
        <section class="card">
          <!-- The heading is the title alone; the any/all switch sits beside it, not in it. -->
          <div class="head">
            <h3 id="card-{card}">{title}</h3>
            {#if all}
              <!-- How this card's picks combine: any of them, or every one. -->
              <span class="combine" role="group" aria-label="{title}: picks combine">
                <button class:on={!allOf[all]} aria-pressed={!allOf[all]} title="Photos with any of the picked values" onclick={() => allOf[all] && onallof(all)}>any</button>
                <button class:on={!!allOf[all]} aria-pressed={!!allOf[all]} title="Photos with every picked value" onclick={() => !allOf[all] && onallof(all)}>all</button>
              </span>
            {/if}
          </div>
          {#if lines}<p class="hint" id="hint-{card}">{cardHint(card, lines)}</p>{/if}
          {#each list as [value, n] (value)}
            {@const rule = lines ? barRule(card, value, lines) : ""}
            <!-- The rule is in the accessible name too: a tooltip alone never reaches the keyboard. -->
            <button
              class="bar-row"
              class:on={picked[key]?.includes(value)}
              title={rule ? `${value}: ${rule}` : value}
              aria-label="{value}, {n.toLocaleString()} photos{rule ? `: ${rule}` : ''}"
              aria-pressed={!!picked[key]?.includes(value)}
              onclick={() => ontoggle(key, value)}
            >
              <span class="name">
                {#if dim === "hue"}<i class="dot" style:background={HUES[value] ?? "#888"}></i>{/if}{value}
              </span>
              <span class="track"><span class="fill" style:width="{(100 * n) / max}%" style:background={dim === "hue" ? (HUES[value] ?? "#888") : null}></span></span>
              <span class="n">{n.toLocaleString()}</span>
            </button>
          {:else}
            <p class="none">{emptyNote(total, measured)}</p>
          {/each}
        </section>
      {/each}
    </div>
    <p class="muted note">{COMBINE_NOTE}</p>
  {:else}
    {#if stack}
      <p class="muted stacknote">Stack is on: the photo counts below leave out the stacked frames; the groups are counted whole.</p>
    {/if}
    <AlikePanel
      {filter}
      {version}
      {total}
      counts={{ duplicates: flag("duplicate"), series: flag("series") }}
      onflag={onflag}
      {ongroup}
      {onclasses}
    />
  {/if}
  </div>
</div>

<style>
  .analysis { flex: 1; min-height: 0; overflow-y: auto; padding: 12px 16px 24px; }
  .bar { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 8px; }
  .progress { display: flex; align-items: center; gap: 10px; color: var(--muted, #999); }
  progress { width: 160px; }
  .link { color: #e8b03a; font-size: 12px; }
  .link:hover, .link.on { text-decoration: underline; }
  .link.on { font-weight: 600; }
  .show { padding: 3px 12px; border: 1px solid var(--line, #444); border-radius: 3px; }
  .show:hover { background: var(--hover, #333); }
  .parts { display: flex; gap: 2px; margin-bottom: 10px; border-bottom: 1px solid var(--line, #333); }
  .parts button { padding: 5px 14px; border-bottom: 2px solid transparent; margin-bottom: -1px; }
  .parts button:hover { background: var(--hover, #333); }
  .parts button.on { border-bottom-color: #5b8def; font-weight: 600; }
  .stacknote { margin: 0 0 8px; }
  .stack { display: flex; align-items: center; gap: 10px; margin-bottom: 10px; }
  .toggle { padding: 3px 12px; border: 1px solid var(--line, #444); border-radius: 3px; }
  .toggle:hover { background: var(--hover, #333); }
  .toggle.on { background: var(--on, #2a3a55); border-color: #5b8def; }
  .muted { color: var(--muted, #999); font-size: 12px; }
  .charts { display: grid; grid-template-columns: repeat(auto-fill, minmax(240px, 1fr)); gap: 12px; }
  .card { background: rgba(255, 255, 255, 0.03); border: 1px solid var(--line, #333); border-radius: 4px; padding: 8px 10px; }
  .head { display: flex; justify-content: space-between; align-items: center; gap: 8px; margin: 0 0 4px; }
  h3 { margin: 0; font-size: 12px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em; color: #999; }
  .hint { margin: 0 0 6px; font-size: 11px; line-height: 1.35; color: var(--muted, #999); }
  .note { margin: 10px 0 0; }
  .combine { display: inline-flex; border: 1px solid var(--line, #444); border-radius: 3px; overflow: hidden; text-transform: none; letter-spacing: 0; font-weight: 400; }
  .combine button { padding: 0 7px; font-size: 11px; line-height: 18px; }
  .combine button.on { background: var(--on, #2a3a55); color: var(--text, #ddd); }
  .bar-row { display: grid; grid-template-columns: 110px 1fr 48px; gap: 8px; align-items: center; width: 100%; padding: 2px 4px; border-radius: 3px; text-align: left; }
  .bar-row:hover { background: var(--hover, #333); }
  .bar-row.on { background: var(--on, #2a3a55); }
  .name { overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
  .dot { display: inline-block; width: 9px; height: 9px; margin-right: 6px; border-radius: 50%; border: 1px solid #555; }
  .track { height: 8px; background: rgba(255, 255, 255, 0.07); border-radius: 4px; overflow: hidden; }
  .fill { display: block; height: 100%; background: #5b8def; border-radius: 4px; }
  .n { text-align: right; color: var(--muted, #999); font-variant-numeric: tabular-nums; }
  .none { margin: 0 0 8px; font-size: 12px; color: #888; }
</style>
