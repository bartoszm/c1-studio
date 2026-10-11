<script lang="ts">
  import * as api from "./lib/api";
  import { geoText } from "./lib/format";

  // Choose what to copy from one photo; pasting then puts those fields on others.
  let { photo, fields, oncopy, onclose }: {
    photo: api.Photo;
    /** What was copied last time, ticked to start with. */
    fields: api.CopyField[];
    oncopy: (fields: api.CopyField[]) => void;
    onclose: () => void;
  } = $props();

  type Item = [api.CopyField, string, string | null];
  const text = (v: string | null | undefined) => v || null;
  let groups: [string, Item[]][] = $derived([
    ["Marks", [
      ["rating", "Rating", photo.rating ? "★".repeat(photo.rating) : null],
      ["color", "Colour", photo.color?.toLowerCase() ?? null],
    ]],
    ["Keywords", [["keywords", "Keywords", text(photo.keywords.map((k) => k.join("|")).join(", "))]]],
    ["Location", [
      ["country", "Country", text(photo.place.country)],
      ["country_code", "Country code", text(photo.place.country_code)],
      ["state", "State", text(photo.place.state)],
      ["city", "City", text(photo.place.city)],
      ["location", "Location", text(photo.place.location)],
      ["geo", "GPS", photo.geo ? geoText(photo.geo) : null],
    ]],
    ["IPTC – Content", [
      ["headline", "Headline", text(photo.iptc.headline)],
      ["description", "Description", text(photo.iptc.description)],
    ]],
    ["IPTC – Image", [["title", "Title (object name)", text(photo.iptc.title)]]],
    ["IPTC – Status", [
      ["job_id", "Job identifier", text(photo.iptc.job_id)],
      ["instructions", "Instructions", text(photo.iptc.instructions)],
      ["source", "Source", text(photo.iptc.source)],
      ["copyright", "Copyright", text(photo.iptc.copyright)],
    ]],
    ["IPTC – Contact", [["creator", "Creator", text(photo.iptc.creator)]]],
  ]);

  // svelte-ignore state_referenced_locally
  let chosen = $state(new Set<api.CopyField>(fields.map((f) => (f === "keywords_exact" ? "keywords" : f))));
  // svelte-ignore state_referenced_locally
  let exact = $state(fields.includes("keywords_exact"));

  function toggle(field: api.CopyField, on: boolean) {
    const next = new Set(chosen);
    if (on) next.add(field);
    else next.delete(field);
    chosen = next;
  }
  function toggleGroup(items: Item[], on: boolean) {
    const next = new Set(chosen);
    for (const [f] of items) {
      if (on) next.add(f);
      else next.delete(f);
    }
    chosen = next;
  }
  /** Every field that has a value on this photo (and nothing empty). */
  function filled() {
    chosen = new Set(groups.flatMap(([, items]) => items.filter(([, , v]) => v !== null).map(([f]) => f)));
  }

  let picked = $derived(
    groups.flatMap(([, items]) => items.map(([f]) => f)).filter((f) => chosen.has(f))
      .map((f) => (f === "keywords" && exact ? "keywords_exact" : f)) as api.CopyField[],
  );
  let clears = $derived(groups.flatMap(([, items]) => items).filter(([f, , v]) => chosen.has(f) && v === null && !(f === "keywords" && !exact)));

  function submit(e?: Event) {
    e?.preventDefault();
    if (picked.length) {
      oncopy(picked);
      onclose();
    }
  }
</script>

<svelte:window onkeydown={(e) => { if (e.key === "Escape") onclose(); else if (e.key === "Enter" && !(e.target instanceof HTMLButtonElement)) submit(e); }} />

<div class="backdrop" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="copy-title">
    <h2 id="copy-title">Copy from {photo.file.name}</h2>
    <p class="muted">Tick what to copy. Paste puts it on the selected photos (Ctrl+Shift+V); it is queued for Capture One like any edit.</p>

    {#each groups as [title, items] (title)}
      {@const all = items.every(([f]) => chosen.has(f))}
      {@const some = items.some(([f]) => chosen.has(f))}
      <fieldset>
        <legend>
          <label>
            <input type="checkbox" checked={all} indeterminate={some && !all} onchange={(e) => toggleGroup(items, e.currentTarget.checked)} />
            {title}
          </label>
        </legend>
        {#each items as [field, label, value] (field)}
          <label class="item">
            <input type="checkbox" checked={chosen.has(field)} onchange={(e) => toggle(field, e.currentTarget.checked)} />
            <span class="name">{label}</span>
            <span class="value" class:none={value === null} title={value ?? ""}>{value ?? "none"}</span>
          </label>
        {/each}
        {#if title === "Keywords" && chosen.has("keywords")}
          <div class="mode">
            <label><input type="radio" name="kw-mode" checked={!exact} onchange={() => (exact = false)} /> add to theirs</label>
            <label><input type="radio" name="kw-mode" checked={exact} onchange={() => (exact = true)} /> replace theirs</label>
          </div>
        {/if}
      </fieldset>
    {/each}

    {#if clears.length}
      <p class="warn">Empty here, so pasting clears it: {clears.map(([, l]) => l.toLowerCase()).join(", ")}.</p>
    {/if}

    <div class="buttons">
      <button class="plain" onclick={filled}>Everything it has</button>
      <span class="spacer"></span>
      <button onclick={onclose}>Cancel</button>
      <button class="primary" disabled={!picked.length} onclick={() => submit()}>Copy</button>
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
    width: min(500px, 100%);
    max-height: calc(100vh - 32px);
    overflow: auto;
    padding: 18px 20px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 6px;
    box-shadow: 0 12px 40px rgb(0 0 0 / 50%);
  }
  h2 { margin: 0 0 6px; font-size: 16px; font-weight: 600; overflow-wrap: anywhere; }
  p { margin: 6px 0; }
  fieldset { margin: 10px 0 0; padding: 4px 0 6px; border: 0; border-top: 1px solid var(--line); }
  legend { padding: 0 6px 0 0; font-weight: 600; }
  label { display: flex; align-items: center; gap: 6px; cursor: pointer; }
  .item { display: grid; grid-template-columns: auto 100px 1fr; gap: 6px; padding: 2px 0 2px 18px; }
  .item .name { color: var(--muted); }
  .value { overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
  .value.none { color: #5a5a5a; font-style: italic; }
  .mode { display: flex; gap: 16px; padding: 4px 0 0 18px; color: var(--muted); }
  input { margin: 0; accent-color: var(--queued); }
  .buttons { display: flex; align-items: center; gap: 8px; margin-top: 16px; }
  .spacer { flex: 1; }
  .buttons button { padding: 4px 12px; border: 1px solid var(--line); border-radius: 4px; }
  .buttons button:hover:not(:disabled) { background: var(--hover); }
  .buttons .plain { border-color: transparent; color: var(--muted); }
  .buttons .primary { border: 0; background: var(--queued); color: #fff; font-weight: 600; }
  .buttons button:disabled { opacity: 0.4; cursor: default; }
  .muted { color: var(--muted); }
  .warn { color: #e8b03a; }
</style>
