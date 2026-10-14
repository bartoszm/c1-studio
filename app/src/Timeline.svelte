<script lang="ts">
  import { untrack } from "svelte";
  import * as api from "./lib/api";
  import type { Filter, TimeRange, Trip } from "./lib/api";
  import { tripName, tripDates } from "./lib/trips";

  // A strip above the grid: photos per day, week or month over the whole span (or the
  // picked trip's), with the trips as bands under the bars. Dragging across it picks a
  // range of days; a click picks one bar; a click on a band picks that trip.
  let { filter, version, trips, trip, onrange, ontrip, onerror }: {
    filter: Filter;
    version: number;
    trips: Trip[];
    /** The picked trip: the strip shows its days. */
    trip: Trip | null;
    onrange: (range: TimeRange | null) => void;
    ontrip: (trip: Trip | null) => void;
    onerror: (text: string) => void;
  } = $props();

  const DAY = 86_400_000;
  const BARS_H = 38;
  const BAND_Y = BARS_H + 4;
  const BAND_H = 6;
  const AXIS_Y = BAND_Y + BAND_H + 11;
  const HEIGHT = AXIS_Y + 3;

  let days = $state.raw<[number, number][]>([]);
  let width = $state(600);

  // The strip counts as if no date or trip were picked, so the rest stays in view.
  let counted = $derived({ ...filter, captured: undefined, year: undefined, month: undefined, week: undefined, trip: undefined });
  let key = $derived(JSON.stringify(counted));
  let asked = 0;
  $effect(() => {
    void key;
    void version;
    untrack(async () => {
      const mine = ++asked;
      try {
        const got = await api.timeline($state.snapshot(counted));
        if (mine === asked) days = got;
      } catch (e) {
        if (mine === asked) onerror(String(e));
      }
    });
  });

  type Unit = "day" | "week" | "month";
  /** The start of the bar holding `ms`, and of the next one. */
  function bar(ms: number, unit: Unit): [number, number] {
    const d = new Date(ms);
    if (unit === "month") return [new Date(d.getFullYear(), d.getMonth(), 1).getTime(), new Date(d.getFullYear(), d.getMonth() + 1, 1).getTime()];
    const start = new Date(d.getFullYear(), d.getMonth(), d.getDate() - (unit === "week" ? (d.getDay() + 6) % 7 : 0));
    const next = new Date(start.getFullYear(), start.getMonth(), start.getDate() + (unit === "week" ? 7 : 1));
    return [start.getTime(), next.getTime()];
  }

  /** What the strip spans: the trip's days with a little either side, else every day with photos. */
  let span = $derived.by((): [number, number] | null => {
    if (trip) {
      const pad = Math.max(DAY, (trip.end - trip.start) * 0.15);
      return [bar(trip.start - pad, "day")[0], bar(trip.end + pad, "day")[1]];
    }
    if (!days.length) return null;
    return [days[0][0], bar(days[days.length - 1][0], "day")[1]];
  });
  let unit = $derived.by((): Unit => {
    if (!span) return "day";
    const n = (span[1] - span[0]) / DAY;
    // About as many bars as there is room for, three pixels each at least.
    if (n <= width / 3) return "day";
    if (n / 7 <= width / 3) return "week";
    return "month";
  });
  let bars = $derived.by(() => {
    if (!span) return [];
    const by = new Map<number, { start: number; end: number; n: number }>();
    for (const [ms, n] of days) {
      if (ms < span[0] || ms >= span[1]) continue;
      const [start, end] = bar(ms, unit);
      const b = by.get(start);
      if (b) b.n += n;
      else by.set(start, { start, end, n });
    }
    return [...by.values()];
  });
  let most = $derived(Math.max(1, ...bars.map((b) => b.n)));
  const x = (ms: number) => (span ? ((ms - span[0]) / (span[1] - span[0])) * width : 0);
  const at = (px: number) => (span ? span[0] + (Math.min(Math.max(px, 0), width) / width) * (span[1] - span[0]) : 0);

  /** Year (or month, when the strip spans little) marks along the bottom. */
  let ticks = $derived.by(() => {
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
    // Thin out marks that would overlap.
    const room = months ? 44 : 34;
    let last = -Infinity;
    return out.filter((m) => {
      const px = x(m.ms);
      if (px - last < room) return false;
      last = px;
      return true;
    });
  });

  let bands = $derived(trips.filter((t) => t.kind !== "dismissed" && span && t.end >= span[0] && t.start < span[1]));

  // Dragging picks the days it crosses; a click (no drag) the bar under it.
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
        const [a, b] = [at(Math.min(from, to)), at(Math.max(from, to))];
        onrange({ from: bar(a, "day")[0], to: bar(b, "day")[1] - 1 });
      }
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  }

  /** The range picked, as drawn: the filter's days. */
  let picked = $derived(filter.captured && span ? [x(filter.captured.from ?? span[0]), x((filter.captured.to ?? span[1]) + 1)] : null);
  let hover = $state<{ px: number; text: string } | null>(null);
  function hoverAt(e: MouseEvent) {
    if (drag || !span) return;
    const p = px(e);
    const [start] = bar(at(p), unit);
    const b = bars.find((b) => b.start === start);
    const when =
      unit === "month"
        ? new Date(start).toLocaleDateString(undefined, { month: "long", year: "numeric" })
        : unit === "week"
          ? `week of ${new Date(start).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" })}`
          : new Date(start).toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short", year: "numeric" });
    hover = { px: p, text: `${when}: ${(b?.n ?? 0).toLocaleString()} photos` };
  }
</script>

<div class="timeline" bind:clientWidth={width}>
  {#if span}
    <svg
      bind:this={svg}
      {width}
      height={HEIGHT}
      role="slider"
      aria-label="Photos over time: drag to pick days"
      aria-valuenow={filter.captured?.from ?? 0}
      tabindex="-1"
      onmousedown={down}
      onmousemove={hoverAt}
      onmouseleave={() => (hover = null)}
    >
      {#if picked}<rect class="picked" x={picked[0]} width={Math.max(2, picked[1] - picked[0])} y="0" height={BARS_H} />{/if}
      {#each bars as b (b.start)}
        {@const h = Math.max(1, Math.sqrt(b.n / most) * BARS_H)}
        <rect class="bar" x={x(b.start)} width={Math.max(1, x(b.end) - x(b.start) - (unit === "day" && width / bars.length > 4 ? 1 : 0))} y={BARS_H - h} height={h} />
      {/each}
      {#each bands as t (t.id)}
        <rect
          class="band {t.kind}"
          class:on={trip?.id === t.id}
          x={x(t.start)}
          width={Math.max(3, x(t.end) - x(t.start))}
          y={BAND_Y}
          height={BAND_H}
          rx="2"
          role="button"
          tabindex="-1"
          aria-label={tripName(t)}
          onmousedown={(e) => e.stopPropagation()}
          onclick={() => ontrip(trip?.id === t.id ? null : t)}
          onkeydown={() => {}}
        ><title>{tripName(t)} · {tripDates(t, true)}</title></rect>
      {/each}
      {#each ticks as m (m.ms)}
        <line class="tick" x1={x(m.ms)} x2={x(m.ms)} y1={BAND_Y + BAND_H + 1} y2={AXIS_Y - 8} />
        <text x={x(m.ms) + 3} y={AXIS_Y}>{m.text}</text>
      {/each}
      {#if drag}
        <rect class="dragging" x={Math.min(drag.from, drag.to)} width={Math.abs(drag.to - drag.from)} y="0" height={BARS_H} />
      {/if}
      {#if hover}<line class="cursor" x1={hover.px} x2={hover.px} y1="0" y2={BARS_H} />{/if}
    </svg>
    {#if hover}<span class="tip" style:left="{Math.min(hover.px + 8, width - 190)}px">{hover.text}</span>{/if}
    {#if trip}
      <button class="out" title="Back to all days" onclick={() => ontrip(null)}>⤢ all</button>
    {/if}
  {/if}
</div>

<style>
  .timeline { position: relative; flex: none; height: 62px; padding: 0 0 2px; border-bottom: 1px solid var(--line); background: var(--panel); user-select: none; }
  svg { display: block; cursor: crosshair; }
  .bar { fill: #6f7780; }
  .picked { fill: rgba(79, 163, 224, 0.18); }
  .dragging { fill: rgba(79, 163, 224, 0.28); }
  .band { fill: #4f8f6a; cursor: pointer; }
  .band.suggested { fill: #5d6670; }
  .band.kept { fill: #3f9e72; }
  .band.on { fill: var(--queued); }
  .band:hover { fill: #8fd0aa; }
  .tick { stroke: #555; }
  text { fill: var(--muted); font-size: 10px; }
  .cursor { stroke: #aaa; stroke-dasharray: 2 2; pointer-events: none; }
  .tip {
    position: absolute;
    top: 2px;
    padding: 1px 6px;
    font-size: 11px;
    color: var(--text);
    background: rgba(30, 30, 30, 0.9);
    border: 1px solid var(--line);
    border-radius: 3px;
    pointer-events: none;
    white-space: nowrap;
  }
  .out { position: absolute; top: 2px; right: 6px; padding: 0 6px; font-size: 11px; color: var(--muted); border: 1px solid var(--line); border-radius: 3px; background: var(--panel); }
  .out:hover { color: var(--text); }
</style>
