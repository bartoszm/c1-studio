<script lang="ts">
  import { untrack } from "svelte";
  import * as api from "./lib/api";
  import type { Filter, LogDays } from "./lib/api";
  import { bar, unitFor, type Unit } from "./lib/dates";
  import { dayRange } from "./lib/history";

  // The library over time: photos imported per day (square-root scale, so a big import
  // does not flatten the rest), the running total as a line, and under them when the log
  // saw changes. Dragging across it picks the days to look at; a click picks one bar.
  let { filter, version, range, onrange, onerror }: {
    filter: Filter;
    version: number;
    /** The days picked, inclusive milliseconds. */
    range: { from: number; to: number } | null;
    onrange: (range: { from: number; to: number } | null) => void;
    onerror: (text: string) => void;
  } = $props();

  const DAY = 86_400_000;
  const BARS_H = 56;
  const LOG_Y = BARS_H + 4;
  const LOG_H = 5;
  const AXIS_Y = LOG_Y + LOG_H + 12;
  const HEIGHT = AXIS_Y + 3;

  let imports = $state.raw<[number, number][]>([]);
  let log = $state.raw<LogDays>({ start: null, days: [] });
  let width = $state(600);

  // Counted as if no pick from the history were made, so the chart stays whole.
  let counted = $derived({ ...filter, history: undefined });
  let key = $derived(JSON.stringify(counted));
  let asked = 0;
  $effect(() => {
    void key;
    void version;
    untrack(async () => {
      const mine = ++asked;
      try {
        const [got, days] = await Promise.all([api.growth($state.snapshot(counted)), api.logDays()]);
        if (mine !== asked) return;
        imports = got;
        log = days;
      } catch (e) {
        if (mine === asked) onerror(String(e));
      }
    });
  });

  /** From the first import (or the log's start) to the end of today. */
  let span = $derived.by((): [number, number] | null => {
    const starts = [imports[0]?.[0], log.days[0]?.[0], log.start ?? undefined].filter((v): v is number => v !== undefined);
    if (!starts.length) return null;
    const ends = [imports.at(-1)?.[0] ?? 0, log.days.at(-1)?.[0] ?? 0, Date.now()];
    return [Math.min(...starts), bar(Math.max(...ends), "day")[1]];
  });
  let unit = $derived<Unit>(span ? unitFor(span[1] - span[0], width) : "day");

  /** Counts of `days` in the bars of the chart's unit. */
  function binned(days: [number, number][]) {
    const by = new Map<number, { start: number; end: number; n: number }>();
    if (!span) return [];
    for (const [ms, n] of days) {
      const [start, end] = bar(ms, unit);
      const b = by.get(start);
      if (b) b.n += n;
      else by.set(start, { start, end, n });
    }
    return [...by.values()];
  }
  let bars = $derived(binned(imports));
  let ticks = $derived(binned(log.days));
  let most = $derived(Math.max(1, ...bars.map((b) => b.n)));
  let busiest = $derived(Math.max(1, ...ticks.map((b) => b.n)));
  let total = $derived(imports.reduce((sum, [, n]) => sum + n, 0));
  const x = (ms: number) => (span ? ((ms - span[0]) / (span[1] - span[0])) * width : 0);
  const at = (px: number) => (span ? span[0] + (Math.min(Math.max(px, 0), width) / width) * (span[1] - span[0]) : 0);

  /** The total so far, drawn at the end of each bar. */
  let line = $derived.by(() => {
    if (!total) return "";
    let sum = 0;
    const points = bars.map((b) => {
      sum += b.n;
      return `${x(b.end).toFixed(1)},${(BARS_H - (sum / total) * (BARS_H - 2) - 1).toFixed(1)}`;
    });
    return points.length ? `M${x(bars[0].start).toFixed(1)},${BARS_H} L${points.join(" L")}` : "";
  });

  /** Year (or month, when the chart spans little) marks along the bottom. */
  let marks = $derived.by(() => {
    if (!span) return [];
    const out: { ms: number; text: string }[] = [];
    const months = (span[1] - span[0]) / DAY < 400;
    const d = new Date(span[0]);
    let t = months ? new Date(d.getFullYear(), d.getMonth() + 1, 1) : new Date(d.getFullYear() + 1, 0, 1);
    while (t.getTime() < span[1]) {
      out.push({
        ms: t.getTime(),
        text: months ? t.toLocaleDateString(undefined, { month: "short", ...(t.getMonth() === 0 ? { year: "numeric" } : {}) }) : String(t.getFullYear()),
      });
      t = months ? new Date(t.getFullYear(), t.getMonth() + 1, 1) : new Date(t.getFullYear() + 1, 0, 1);
    }
    const room = months ? 44 : 34;
    let last = -Infinity;
    return out.filter((m) => {
      const px = x(m.ms);
      if (px - last < room) return false;
      last = px;
      return true;
    });
  });

  let svg = $state<SVGSVGElement>();
  let drag = $state<{ from: number; to: number } | null>(null);
  function px(e: MouseEvent) {
    return e.clientX - svg!.getBoundingClientRect().left;
  }
  function down(e: MouseEvent) {
    if (e.button !== 0 || !span) return;
    const from = px(e);
    drag = { from, to: from };
    const move = (m: MouseEvent) => (drag = { from, to: px(m) });
    const up = (m: MouseEvent) => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      const to = px(m);
      drag = null;
      if (Math.abs(to - from) < 3) {
        const [start, end] = bar(at(from), unit);
        onrange({ from: start, to: end - 1 });
      } else {
        onrange(dayRange(at(Math.min(from, to)), at(Math.max(from, to))));
      }
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  }

  let picked = $derived(range && span ? [x(range.from), x(range.to + 1)] : null);
  let hover = $state<{ px: number; text: string } | null>(null);
  function hoverAt(e: MouseEvent) {
    if (drag || !span) return;
    const p = px(e);
    const [start] = bar(at(p), unit);
    const day = (ms: number) => new Date(ms).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
    const when = unit === "month" ? new Date(start).toLocaleDateString(undefined, { month: "long", year: "numeric" }) : unit === "week" ? `week of ${day(start)}` : day(start);
    const imported = bars.find((b) => b.start === start)?.n ?? 0;
    const logged = ticks.find((b) => b.start === start)?.n ?? 0;
    hover = { px: p, text: `${when}: ${imported.toLocaleString()} imported, ${logged.toLocaleString()} log entries` };
  }
</script>

<div class="growth" bind:clientWidth={width}>
  {#if span}
    <svg
      bind:this={svg}
      {width}
      height={HEIGHT}
      role="slider"
      aria-label="Photos imported over time: drag to pick days"
      aria-valuenow={range?.from ?? 0}
      tabindex="-1"
      onmousedown={down}
      onmousemove={hoverAt}
      onmouseleave={() => (hover = null)}
    >
      {#if picked}<rect class="picked" x={picked[0]} width={Math.max(2, picked[1] - picked[0])} y="0" height={LOG_Y + LOG_H} />{/if}
      {#each bars as b (b.start)}
        {@const h = Math.max(1, Math.sqrt(b.n / most) * BARS_H)}
        <rect class="bar" x={x(b.start)} width={Math.max(1, x(b.end) - x(b.start) - (unit === "day" && width / bars.length > 4 ? 1 : 0))} y={BARS_H - h} height={h} />
      {/each}
      {#if line}<path class="total" d={line} />{/if}
      {#each ticks as b (b.start)}
        <rect class="logged" x={x(b.start)} width={Math.max(1, x(b.end) - x(b.start))} y={LOG_Y} height={LOG_H} opacity={0.35 + 0.65 * Math.sqrt(b.n / busiest)} />
      {/each}
      {#if log.start !== null}
        <line class="starts" x1={x(log.start)} x2={x(log.start)} y1="0" y2={LOG_Y + LOG_H} />
        <text class="starts-text" x={x(log.start) + 3} y="9">log starts</text>
      {/if}
      {#each marks as m (m.ms)}
        <line class="tick" x1={x(m.ms)} x2={x(m.ms)} y1={LOG_Y + LOG_H + 1} y2={AXIS_Y - 8} />
        <text x={x(m.ms) + 3} y={AXIS_Y}>{m.text}</text>
      {/each}
      {#if drag}<rect class="dragging" x={Math.min(drag.from, drag.to)} width={Math.abs(drag.to - drag.from)} y="0" height={LOG_Y + LOG_H} />{/if}
      {#if hover}<line class="cursor" x1={hover.px} x2={hover.px} y1="0" y2={LOG_Y + LOG_H} />{/if}
    </svg>
    {#if hover}<span class="tip" style:left="{Math.min(hover.px + 8, Math.max(0, width - 260))}px">{hover.text}</span>{/if}
    {#if range}<button class="out" title="Let go of the days" onclick={() => onrange(null)}>⤢ all</button>{/if}
  {:else}
    <p class="empty">Nothing imported or logged yet.</p>
  {/if}
</div>

<style>
  .growth { position: relative; flex: none; padding: 4px 0 2px; border-bottom: 1px solid var(--line); background: var(--panel); user-select: none; }
  svg { display: block; cursor: crosshair; }
  .bar { fill: #6f7780; }
  .total { fill: none; stroke: #4f8f6a; stroke-width: 1.5; pointer-events: none; }
  .logged { fill: var(--queued); }
  .starts { stroke: var(--muted); stroke-dasharray: 3 3; }
  .starts-text { fill: var(--muted); font-size: 10px; }
  .picked { fill: rgba(79, 163, 224, 0.18); }
  .dragging { fill: rgba(79, 163, 224, 0.28); }
  .tick { stroke: #555; }
  text { fill: var(--muted); font-size: 10px; }
  .cursor { stroke: #aaa; stroke-dasharray: 2 2; pointer-events: none; }
  .tip { position: absolute; top: 4px; padding: 1px 6px; font-size: 11px; color: var(--text); background: rgba(30, 30, 30, 0.9); border: 1px solid var(--line); border-radius: 3px; pointer-events: none; white-space: nowrap; }
  .out { position: absolute; top: 4px; right: 6px; padding: 0 6px; font-size: 11px; color: var(--muted); border: 1px solid var(--line); border-radius: 3px; background: var(--panel); }
  .out:hover { color: var(--text); }
  .empty { margin: 8px 12px; color: var(--muted); font-size: 12px; }
</style>
