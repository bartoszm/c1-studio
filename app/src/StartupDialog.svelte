<script lang="ts">
  import { onDestroy } from "svelte";
  import type { Startup } from "./lib/api";
  import { progressText } from "./lib/folders";

  // Shown while a source opens: the window would otherwise sit empty for as long as the
  // index takes to open, or a first fill takes to read. Cannot be closed: it ends itself.
  let { progress }: { progress: Startup } = $props();

  let noun = $derived(progress.noun ?? "library");
  let steps: { step: Startup["step"]; label: string }[] = $derived([
    { step: "opening", label: "Opening the library" },
    { step: "reading", label: `Reading the ${noun}` },
    { step: "indexing", label: "Indexing the photos" },
    { step: "saving", label: "Saving the index" },
  ]);
  let at = $derived(steps.findIndex((s) => s.step === progress.step));
  // Indexing counts photos; while reading, the source may count its own work (a scan).
  let counted: [number, number] | null = $derived(
    progress.step === "indexing" && progress.total
      ? [progress.done, progress.total]
      : progress.step === "reading" && progress.source?.total
        ? [progress.source.done, progress.source.total]
        : null,
  );
  let fraction = $derived(counted ? counted[0] / counted[1] : 0);

  const began = Date.now();
  let elapsed = $state(0);
  const tick = setInterval(() => (elapsed = Math.floor((Date.now() - began) / 1000)), 500);
  onDestroy(() => clearInterval(tick));

  const time = (s: number) => (s < 60 ? `${s} s` : `${Math.floor(s / 60)} min ${String(s % 60).padStart(2, "0")} s`);
</script>

<div class="backdrop" role="presentation">
  <div class="dialog" role="alertdialog" aria-modal="true" aria-labelledby="startup-title" aria-describedby="startup-note">
    <h2 id="startup-title">Getting the library ready</h2>
    <ol>
      {#each steps as s, i (s.step)}
        <li class:done={i < at} class:now={i === at}>
          <span class="mark" aria-hidden="true">{i < at ? "✓" : i === at ? "●" : "○"}</span>
          <span>{s.label}{#if i === at && s.step === "indexing" && progress.total}
              <span class="muted"> · {progress.done.toLocaleString()} of {progress.total.toLocaleString()}</span>{/if}{#if i === at && s.step === "reading" && progress.source}
              <span class="muted"> · {progressText(progress.source)}</span>{/if}</span>
        </li>
      {/each}
    </ol>
    <div
      class="bar"
      class:wait={!counted}
      role="progressbar"
      aria-valuemin="0"
      aria-valuemax="100"
      aria-valuenow={counted ? Math.round(fraction * 100) : undefined}
    >
      <span style:width="{Math.round(fraction * 100)}%"></span>
    </div>
    <p id="startup-note" class="muted">
      {#if progress.step === "reading"}
        A {noun} is read in full the first time, and again when it changed outside the app. This can take a minute or two for
        a big one.
      {:else}
        This happens when the app opens a {noun} it has not indexed yet, or after an update.
      {/if}
      <span class="time">{time(elapsed)}</span>
    </p>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 40;
    display: grid;
    place-items: center;
    padding: 16px;
    background: rgb(0 0 0 / 65%);
  }
  .dialog {
    width: min(460px, 100%);
    padding: 18px 20px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 6px;
    box-shadow: 0 12px 40px rgb(0 0 0 / 50%);
  }
  h2 { margin: 0 0 12px; font-size: 16px; font-weight: 600; }
  ol { margin: 0 0 14px; padding: 0; list-style: none; display: grid; gap: 5px; }
  li { display: flex; gap: 8px; color: var(--muted); }
  li.done { color: #7cc47c; }
  li.now { color: inherit; font-weight: 600; }
  .mark { width: 1em; text-align: center; }
  .muted { color: var(--muted); font-weight: 400; }
  .bar { height: 6px; overflow: hidden; border-radius: 3px; background: var(--line); }
  .bar span { display: block; height: 100%; background: var(--queued); transition: width 0.12s linear; }
  .bar.wait span { width: 35% !important; animation: slide 1.3s ease-in-out infinite alternate; }
  @keyframes slide { from { margin-left: 0; } to { margin-left: 65%; } }
  p { margin: 12px 0 0; font-size: 12px; }
  .time { float: right; margin-left: 12px; font-variant-numeric: tabular-nums; }
  @media (prefers-reduced-motion: reduce) { .bar.wait span { animation: none; margin-left: 30%; } }
</style>
