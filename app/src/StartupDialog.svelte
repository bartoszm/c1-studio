<script lang="ts">
  import { onDestroy } from "svelte";
  import { abortOpen, type Startup } from "./lib/api";
  import { progressText } from "./lib/folders";

  // Shown while a source opens: the window would otherwise sit empty for as long as the
  // index takes to open, or a first fill takes to read. It ends itself; Abort stops the
  // open, and the window then offers another source (or keeps the one open before).
  let { progress }: { progress: Startup } = $props();

  // A repair shows only once one is needed, and stays in the list after.
  let repaired = $state(false);
  $effect.pre(() => {
    if (progress.step === "repairing") repaired = true;
  });

  let noun = $derived(progress.noun ?? "library");
  let steps: { step: Startup["step"]; label: string }[] = $derived([
    { step: "opening", label: "Opening the library" },
    ...(repaired ? [{ step: "repairing" as const, label: "Repairing the library" }] : []),
    { step: "reading", label: `Reading the ${noun}` },
    { step: "indexing", label: "Indexing the photos" },
    { step: "saving", label: "Saving the index" },
  ]);
  let at = $derived(steps.findIndex((s) => s.step === progress.step));
  // Indexing counts photos, a repair thousandths; while reading, the source may count its
  // own work (a scan).
  let counted: [number, number] | null = $derived(
    (progress.step === "indexing" || progress.step === "repairing") && progress.total
      ? [progress.done, progress.total]
      : progress.step === "reading" && progress.source?.total
        ? [progress.source.done, progress.source.total]
        : null,
  );
  let fraction = $derived(counted ? counted[0] / counted[1] : 0);

  let stopping = $state(false);
  async function abort() {
    if (stopping) return;
    stopping = true;
    try {
      await abortOpen();
    } finally {
      stopping = false;
    }
  }

  const began = Date.now();
  let elapsed = $state(0);
  const tick = setInterval(() => (elapsed = Math.floor((Date.now() - began) / 1000)), 500);
  onDestroy(() => clearInterval(tick));
  /** Opening the library normally takes seconds: past this, the dialog says so. */
  const SLOW_S = 20;
  let slow = $derived((progress.step === "opening" || progress.step === "repairing") && elapsed >= SLOW_S);

  const time = (s: number) => (s < 60 ? `${s} s` : `${Math.floor(s / 60)} min ${String(s % 60).padStart(2, "0")} s`);
</script>

<!-- Esc only while stopping loses nothing: a stray key must not throw away a first read. -->
<svelte:window onkeydown={(e) => { if (e.key === "Escape" && (progress.step === "opening" || progress.step === "repairing")) abort(); }} />

<div class="backdrop" role="presentation">
  <div class="dialog" role="alertdialog" aria-modal="true" aria-labelledby="startup-title" aria-describedby="startup-note">
    <h2 id="startup-title">Getting the library ready</h2>
    <ol>
      {#each steps as s, i (s.step)}
        <li class:done={i < at} class:now={i === at}>
          <span class="mark" aria-hidden="true">{i < at ? "✓" : i === at ? "●" : "○"}</span>
          <span>{s.label}{#if i === at && progress.part}<span class="muted"> · {progress.part}</span>{/if}{#if i === at && s.step === "repairing" && progress.total}
              <span class="muted"> · {Math.round((progress.done / progress.total) * 100)}%</span>{/if}{#if i === at && s.step === "indexing" && progress.total}
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
      {#if progress.step === "repairing"}
        The app was not closed properly last time, so the library is checked before it is used.
      {:else if progress.step === "reading"}
        A {noun} is read in full the first time, and again when it changed outside the app. This can take a minute or two for
        a big one.
      {:else}
        This happens when the app opens a {noun} it has not indexed yet, or after an update.
      {/if}
      <span class="time">{time(elapsed)}</span>
    </p>
    {#if slow}
      <p class="muted">
        This is taking longer than usual: the disk may be busy (Capture One making previews, say). Abort to open another
        {noun} or try again later.
      </p>
    {/if}
    <div class="actions">
      <button onclick={abort} disabled={stopping}>{stopping ? "Stopping…" : "Abort"}</button>
    </div>
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
  .actions { display: flex; justify-content: flex-end; margin-top: 14px; }
  @media (prefers-reduced-motion: reduce) { .bar.wait span { animation: none; margin-left: 30%; } }
</style>
