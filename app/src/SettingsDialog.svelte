<script lang="ts">
  import { onMount } from "svelte";
  import { source } from "./lib/source.svelte";
  import { defaultSections, move } from "./lib/sections";
  import { settings } from "./lib/settings.svelte";
  import ClassSettings from "./ClassSettings.svelte";
  import AiProviderSettings from "./AiProviderSettings.svelte";

  // The app's preferences, applied as they are changed: the sidebar's sections (which,
  // in what order), whether the map may use the internet, how photos are described. The
  // analysis lines are the exception: they are applied on request, and `onclasses` is
  // told when they are. The History tab is the open catalog's own setting, kept with it:
  // `onhistory` sets it.
  let {
    titles,
    initial = "sidebar",
    history,
    onhistory,
    onclose,
    onclasses,
  }: {
    titles: Record<string, string>;
    initial?: Tab;
    history: boolean;
    onhistory: (on: boolean) => void;
    onclose: () => void;
    onclasses: () => void;
  } = $props();

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

  const TABS = [
    { key: "sidebar", label: "Sidebar" },
    { key: "map", label: "Map & places" },
    { key: "analysis", label: "Analysis" },
    { key: "history", label: "History" },
    { key: "ai", label: "AI" },
  ] as const;
  type Tab = (typeof TABS)[number]["key"];
  // svelte-ignore state_referenced_locally
  let tab = $state<Tab>(initial);
  /** Arrow keys move between the categories, as in any tab list. */
  function tabKey(e: KeyboardEvent, key: Tab) {
    const step = e.key === "ArrowDown" || e.key === "ArrowRight" ? 1 : e.key === "ArrowUp" || e.key === "ArrowLeft" ? -1 : 0;
    if (!step) return;
    const i = TABS.findIndex((t) => t.key === key);
    tab = TABS[(i + step + TABS.length) % TABS.length].key;
    e.preventDefault();
    queueMicrotask(() => dialog.querySelector<HTMLElement>(`#tab-${tab}`)?.focus());
  }
  /** What can take focus now: not in a category that is not shown. */
  const focusable = () =>
    [...dialog.querySelectorAll<HTMLElement>("input, select, button:not(:disabled)")].filter((el) => !el.closest("[hidden]"));

  // Focus goes into the dialog, stays in it, and goes back where it was on closing.
  let dialog: HTMLDivElement;
  onMount(() => {
    const before = document.activeElement;
    focusable()[0]?.focus();
    return () => { if (before instanceof HTMLElement) before.focus(); };
  });
  function onkey(e: KeyboardEvent) {
    if (e.key === "Escape") return onclose();
    if (e.key !== "Tab") return;
    const all = focusable();
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

    <div class="body">
    <div class="tabs" role="tablist" aria-orientation="vertical" aria-label="Settings categories">
      {#each TABS as t (t.key)}
        <button
          role="tab"
          id="tab-{t.key}"
          aria-selected={tab === t.key}
          aria-controls="panel-{t.key}"
          tabindex={tab === t.key ? 0 : -1}
          class:active={tab === t.key}
          onclick={() => (tab = t.key)}
          onkeydown={(e) => tabKey(e, t.key)}
        >{t.label}</button>
      {/each}
    </div>
    <div class="pane">

    <div role="tabpanel" id="panel-sidebar" aria-labelledby="tab-sidebar" hidden={tab !== "sidebar"}>
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

    <div class="reset"><button disabled={isDefault} onclick={() => (settings.sections = defaultSections())}>Reset sidebar</button></div>
    </div>

    <div role="tabpanel" id="panel-map" aria-labelledby="tab-map" hidden={tab !== "map"}>
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
    <label class="option">
      <input type="checkbox" bind:checked={settings.approxPlaces} />
      <span>
        Show photos that have only a place
        <small class="muted">
          Photos with a city, region or country but no GPS show at that place, marked ≈, while the map is zoomed out about as far
          as the name reaches. Places come from GeoNames (built in); with online services on, the rest are looked up once.
        </small>
      </span>
    </label>
    <label class="option">
      <input type="checkbox" bind:checked={settings.fillPlaces} />
      <span>
        Fill in the place of photos I put on the map
        <small class="muted">
          Empty city, region and country fields are filled from the new position without asking. Off, the app offers to.
        </small>
      </span>
    </label>
    <h3>Photos around a photo</h3>
    <label class="option">
      <input
        type="number"
        min="1"
        max="1440"
        value={settings.sessionGap}
        onchange={(e) => { const n = Math.round(+e.currentTarget.value); if (n >= 1) settings.sessionGap = n; }}
      />
      <span>
        Minutes between two photos that end a shoot
        <small class="muted">
          "shoot" under Captured in the details panel shows the photos taken one after another around the photo, with no gap
          longer than this between two of them.
        </small>
      </span>
    </label>
    <p class="muted small">
      Place names and positions: <a href="https://www.geonames.org/" target="_blank" rel="noreferrer">GeoNames</a>, CC BY 4.0.
    </p>

    </div>

    <div role="tabpanel" id="panel-analysis" aria-labelledby="tab-analysis" hidden={tab !== "analysis"}>
      <ClassSettings onapplied={onclasses} />
    </div>

    <div role="tabpanel" id="panel-history" aria-labelledby="tab-history" hidden={tab !== "history"}>
    <h3>History</h3>
    <label class="option">
      <input type="checkbox" checked={history} onchange={(e) => onhistory(e.currentTarget.checked)} />
      <span>
        Show the History tab
        <small class="muted">
          The {source.noun} over time: two dates compared, the change log in groups, photos imported per day. For it, the change log
          also keeps what a change replaced (a removed photo, an album as it was, the keywords a keyword tree change rewrote).
          Off, the tab is hidden and the log keeps only the changes themselves, as before; a photo's own history in the details
          panel stays. What was logged so far is kept either way. Changes made while it is off are not fully counted when
          compared later (a keyword tree change or an album delete then counts as a change, not by the photos it touched);
          the History tab says when the dates it compares include such a time.
        </small>
      </span>
    </label>
    <p class="muted small">Kept with this {source.noun}'s index, so the command line (c1cat) follows it too; another {source.noun} has its own.</p>
    </div>

    <div role="tabpanel" id="panel-ai" aria-labelledby="tab-ai" hidden={tab !== "ai"}>
      <!-- Mounted when shown: on a plan it asks Claude Code, which takes a moment. -->
      {#if tab === "ai"}<AiProviderSettings />{/if}
    </div>
    </div>
    </div>

    <div class="buttons">
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
    width: min(900px, 100%);
    height: min(640px, calc(100vh - 32px));
    display: flex;
    flex-direction: column;
    padding: 18px 20px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 6px;
    box-shadow: 0 12px 40px rgb(0 0 0 / 50%);
  }
  h2 { margin: 0 0 10px; font-size: 16px; font-weight: 600; }
  h3 { margin: 16px 0 4px; font-size: 13px; font-weight: 600; }
  [role="tabpanel"] > h3:first-child { margin-top: 0; }
  .body { flex: 1; min-height: 0; display: flex; gap: 18px; }
  .tabs { flex: none; width: 150px; display: flex; flex-direction: column; gap: 2px; padding-right: 12px; border-right: 1px solid var(--line); }
  .tabs button { padding: 6px 10px; text-align: left; border-radius: 4px; }
  .tabs button:hover { background: var(--hover); }
  .tabs button.active { background: var(--hover); font-weight: 600; box-shadow: inset 2px 0 0 var(--queued); }
  .pane { flex: 1; min-width: 0; overflow: auto; padding-right: 4px; }
  .reset { margin-top: 8px; }
  .reset button { padding: 3px 10px; border: 1px solid var(--line); border-radius: 4px; }
  .reset button:hover:not(:disabled) { background: var(--hover); }
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
