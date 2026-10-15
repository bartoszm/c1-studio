<script lang="ts">
  import { looseKey } from "./lib/countries";
  import { catalogPlaces, nearbyPlaces, placeAt } from "./lib/placeHints";
  import { fillFor, suggest, type Level, type Place, type PlaceFields, type PlacePath, type Suggestion } from "./lib/places";

  // One level of a place (country, state, city or location) with suggestions as it is
  // typed: the place the photo's GPS points at, places near it, the selection's and the
  // catalog's, those inside the photo's own country and state first. Picking one sets
  // the levels above it too; Enter or leaving the field sets what was typed.
  let {
    id, level, shown, placeholder, title, readonly = false, context, selection, picked, geo, version, onfields, ontext,
  }: {
    id: string;
    level: Level;
    /** What the field shows: the value, or "" when there is none or several. */
    shown: string;
    placeholder: string;
    title?: string;
    /** Not to be typed in yet (the selection is being read). */
    readonly?: boolean;
    /** The levels already filled: the photo's, or those the selection shares. */
    context: Partial<Place>;
    /** With several photos selected: their places, counted among them. */
    selection?: [string, number][];
    picked: number;
    geo: { lat: number; lon: number } | null;
    version: number;
    /** A suggestion picked: the fields it sets. */
    onfields: (fields: PlaceFields) => void;
    /** Text typed and committed; "" clears the field. */
    ontext: (value: string) => void;
  } = $props();

  let input: HTMLInputElement;
  let open = $state(false);
  /** Typed since the field was entered; nothing typed shows every suggestion. */
  let typed = $state("");
  let active = $state(-1);
  let nearby = $state<[string, number][]>([]);
  let here = $state<PlacePath | null>(null);
  let catalog = $state<[string, number][]>([]);
  /** Bumped on entering: replies asked for before (another photo's) are dropped. */
  let entered = 0;
  /** Picked or put back: leaving the field then sets nothing. */
  let settled = false;

  let list: Suggestion[] = $derived(open ? suggest(level, typed, context, { catalog, nearby, selection, here }) : []);

  function enter() {
    open = true;
    typed = "";
    active = -1;
    settled = false;
    nearby = [];
    here = null;
    const mine = ++entered;
    const now = () => open && mine === entered;
    catalogPlaces(version).then((c) => { if (now()) catalog = c; });
    if (geo) {
      const { lat, lon } = geo;
      nearbyPlaces(lat, lon, version).then((n) => { if (now()) nearby = n; });
      placeAt(lat, lon).then((h) => { if (now()) here = h; });
    }
  }

  function leave() {
    open = false;
    entered++;
    if (!settled && input.value.trim() !== shown.trim()) ontext(input.value);
  }

  function typing() {
    typed = input.value;
    open = true;
    // What was typed is a place there already: Enter takes it, with the levels above.
    const k = looseKey(typed);
    active = k ? list.findIndex((s) => looseKey(s.value) === k) : -1;
  }

  function pick(s: Suggestion) {
    settled = true;
    input.value = s.value;
    onfields(fillFor(s.path, level, context));
    input.blur();
  }

  function keys(e: KeyboardEvent) {
    if (e.key === "ArrowDown" && list.length) {
      e.preventDefault();
      open = true;
      active = Math.min(active + 1, list.length - 1);
    } else if (e.key === "ArrowUp" && list.length) {
      e.preventDefault();
      active = Math.max(active - 1, -1);
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (open && list[active]) pick(list[active]);
      else input.blur();
    } else if (e.key === "Escape") {
      e.stopPropagation();
      if (open && list.length && typed) {
        open = false;
        return;
      }
      settled = true;
      input.value = shown;
      input.blur();
    }
  }

  let listEl = $state<HTMLUListElement>();
  $effect(() => {
    if (active >= 0) listEl?.children[active]?.scrollIntoView({ block: "nearest" });
  });

  const SOURCE: Partial<Record<Suggestion["source"], [string, string]>> = {
    here: ["GPS", "Where the photo's GPS points, from OpenStreetMap"],
    nearby: ["nearby", "Used on photos taken within 3 km"],
  };
</script>

<div class="place-input">
  <input
    bind:this={input}
    {id}
    class="field"
    value={shown}
    {placeholder}
    {title}
    {readonly}
    autocomplete="off"
    spellcheck="false"
    role="combobox"
    aria-expanded={open && list.length > 0}
    aria-controls="{id}-list"
    aria-autocomplete="list"
    onfocus={enter}
    onblur={leave}
    oninput={typing}
    onkeydown={keys}
  />
  {#if open && list.length}
    <ul class="suggestions" id="{id}-list" role="listbox" bind:this={listEl}>
      {#each list as s, i (s.key)}
        <li role="option" aria-selected={i === active}>
          <button
            type="button"
            tabindex="-1"
            class:on={i === active}
            onmousedown={(e) => e.preventDefault()}
            onclick={() => pick(s)}
            onmouseenter={() => (active = i)}
          >
            <span class="top">
              <span class="value">{s.value}</span>
              {#if SOURCE[s.source]}<span class="source" title={SOURCE[s.source]![1]}>{SOURCE[s.source]![0]}</span>{/if}
              <span class="n">
                {#if s.source === "selection"}
                  <span title="On {s.n.toLocaleString()} of the {picked.toLocaleString()} selected photos">{s.n.toLocaleString()}/{picked.toLocaleString()}</span>
                {:else if s.n}
                  <span title="{s.n.toLocaleString()} photos in the catalog">{s.n.toLocaleString()}</span>
                {:else}
                  <span title="Not in the catalog yet">new</span>
                {/if}
              </span>
            </span>
            {#if s.parents}<span class="parents">in {s.parents}</span>{/if}
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .place-input { position: relative; }
  .suggestions {
    position: absolute;
    z-index: 10;
    top: calc(100% + 2px);
    right: 0;
    width: max(100%, 230px);
    max-height: 260px;
    margin: 0;
    padding: 2px 0;
    overflow-y: auto;
    list-style: none;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 4px;
    box-shadow: 0 8px 24px rgb(0 0 0 / 45%);
  }
  .suggestions button { display: block; width: 100%; padding: 3px 8px; text-align: left; }
  .suggestions button.on { background: var(--hover); }
  .top { display: flex; align-items: baseline; gap: 6px; }
  .value { overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
  .source { flex: none; padding: 0 4px; font-size: 10px; color: var(--queued); border: 1px solid currentColor; border-radius: 3px; }
  .n { flex: none; margin-left: auto; font-size: 11px; color: var(--muted); font-variant-numeric: tabular-nums; }
  .parents { display: block; overflow: hidden; font-size: 11px; color: var(--muted); white-space: nowrap; text-overflow: ellipsis; }
</style>
