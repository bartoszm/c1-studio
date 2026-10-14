<script lang="ts">
  import { onMount } from "svelte";
  import { defaultSections, move } from "./lib/sections";
  import { settings } from "./lib/settings.svelte";

  // The app's preferences, applied as they are changed: the sidebar's sections (which,
  // in what order) and whether the map may use the internet.
  let { titles, onclose }: { titles: Record<string, string>; onclose: () => void } = $props();

  /** The row being dragged, and the row it would land before. */
  let from = $state<number | null>(null);
  let over = $state<number | null>(null);

  function moveTo(i: number, to: number) {
    settings.sections = move(settings.sections, i, to);
  }
  function drop(to: number) {
    if (from !== null) moveTo(from, from < to ? to - 1 : to);
    from = over = null;
  }
  let isDefault = $derived(
    JSON.stringify(settings.sections) === JSON.stringify(defaultSections()),
  );
  const title = (key: string) => titles[key] ?? key;

  // Focus goes into the dialog, stays in it, and goes back where it was on closing.
  let dialog: HTMLDivElement;
  onMount(() => {
    const before = document.activeElement;
    dialog.querySelector<HTMLElement>("input, button:not(:disabled)")?.focus();
    return () => { if (before instanceof HTMLElement) before.focus(); };
  });
  function onkey(e: KeyboardEvent) {
    if (e.key === "Escape") return onclose();
    if (e.key !== "Tab") return;
    const all = [...dialog.querySelectorAll<HTMLElement>("input, button:not(:disabled)")];
    if (!all.length) return;
    const at = all.indexOf(document.activeElement as HTMLElement);
    const next = e.shiftKey ? (at <= 0 ? all.length - 1 : at - 1) : (at < 0 || at === all.length - 1 ? 0 : at + 1);
    all[next].focus();
    e.preventDefault();
  }
</script>

<svelte:window onkeydown={onkey} />

<div class="backdrop" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="settings-title" bind:this={dialog}>
    <h2 id="settings-title">Settings</h2>

    <h3>Sidebar sections</h3>
    <p class="muted">Untick a section to hide it; drag one, or use the arrows, to change the order.</p>
    <ul ondragleave={(e) => { if (e.currentTarget === e.target) over = null; }}>
      {#each settings.sections as s, i (s.key)}
        <li
          draggable="true"
          class:dragging={from === i}
          class:over={over === i && from !== null && from !== i && from !== i - 1}
          ondragstart={(e) => { from = i; e.dataTransfer?.setData("text/plain", s.key); }}
          ondragend={() => (from = over = null)}
          ondragover={(e) => { if (from === null) return; e.preventDefault(); const r = e.currentTarget.getBoundingClientRect(); over = e.clientY > r.top + r.height / 2 ? i + 1 : i; }}
          ondrop={(e) => { e.preventDefault(); if (over !== null) drop(over); }}
        >
          <span class="grip" aria-hidden="true">⋮⋮</span>
          <label>
            <input type="checkbox" checked={!s.hidden} onchange={(e) => (s.hidden = !e.currentTarget.checked)} />
            <span class:hidden={s.hidden}>{title(s.key)}</span>
          </label>
          <button aria-label="Move {title(s.key)} up" disabled={i === 0} onclick={() => moveTo(i, i - 1)}>↑</button>
          <button aria-label="Move {title(s.key)} down" disabled={i === settings.sections.length - 1} onclick={() => moveTo(i, i + 1)}>↓</button>
        </li>
      {/each}
      <li
        class="end"
        class:over={over === settings.sections.length && from !== null && from !== settings.sections.length - 1}
        ondragover={(e) => { if (from === null) return; e.preventDefault(); over = settings.sections.length; }}
        ondrop={(e) => { e.preventDefault(); drop(settings.sections.length); }}
      ></li>
    </ul>
    <p class="muted small">Main colour lists the colours the analysis found; it shows once photos are measured.</p>

    <h3>Map</h3>
    <label class="option">
      <input type="checkbox" bind:checked={settings.onlineMap} />
      <span>
        Use online map services
        <small class="muted">
          Map tiles come from OpenFreeMap; place search and place hints from OpenStreetMap's Nominatim. Off, the map shows your
          photos on a plain background and nothing is asked over the internet.
        </small>
      </span>
    </label>

    <div class="buttons">
      <button disabled={isDefault} onclick={() => (settings.sections = defaultSections())}>Reset sidebar</button>
      <button onclick={onclose}>Close</button>
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 20;
    display: grid;
    place-items: center;
    padding: 16px;
    background: rgb(0 0 0 / 55%);
  }
  .dialog {
    width: min(460px, 100%);
    max-height: calc(100vh - 32px);
    overflow: auto;
    padding: 18px 20px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 6px;
    box-shadow: 0 12px 40px rgb(0 0 0 / 50%);
  }
  h2 { margin: 0 0 10px; font-size: 16px; font-weight: 600; }
  h3 { margin: 16px 0 4px; font-size: 13px; font-weight: 600; }
  p { margin: 4px 0 8px; }
  .muted { color: var(--muted); }
  .small { font-size: 12px; }
  ul { margin: 0; padding: 0; list-style: none; }
  li {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 6px;
    border-top: 2px solid transparent;
    border-radius: 3px;
    cursor: grab;
  }
  li:hover { background: var(--hover); }
  li.dragging { opacity: 0.4; }
  li.over { border-top-color: var(--queued); }
  li.end { height: 6px; padding: 0; cursor: default; }
  li.end:hover { background: none; }
  .grip { color: var(--muted); font-size: 11px; letter-spacing: -2px; }
  li label { flex: 1; display: flex; align-items: center; gap: 8px; cursor: pointer; }
  li .hidden { color: var(--muted); text-decoration: line-through; }
  li button { width: 24px; padding: 1px 0; border: 1px solid var(--line); border-radius: 3px; }
  li button:hover:not(:disabled) { background: var(--panel); }
  button:disabled { opacity: 0.35; cursor: default; }
  .option { display: flex; align-items: flex-start; gap: 8px; cursor: pointer; }
  .option input { margin-top: 3px; }
  .option small { display: block; margin-top: 2px; font-size: 12px; }
  .buttons { display: flex; justify-content: flex-end; gap: 8px; margin-top: 16px; }
  .buttons button { padding: 3px 10px; border: 1px solid var(--line); border-radius: 4px; }
  .buttons button:hover:not(:disabled) { background: var(--hover); }
</style>
