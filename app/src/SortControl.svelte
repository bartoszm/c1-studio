<script lang="ts">
  // "Sort by" a field, and a direction button that says in words what it does for that field.
  // The direction is always visible and flips on every click.
  type Option = { key: string; label: string; /** Not offered now; the hint says why. */ disabled?: boolean; hint?: string };
  let {
    fields, field, words, label = "Sort by", onfield, onflip,
  }: {
    fields: Option[];
    field: string;
    /** What the direction button says: "Newest first". */
    words: string;
    label?: string;
    onfield: (key: string) => void;
    onflip: () => void;
  } = $props();
  let current = $derived(fields.find((f) => f.key === field));
</script>

<span class="sort">
  <select aria-label={label} value={field} title={current?.hint} onchange={(e) => onfield(e.currentTarget.value)}>
    {#each fields as f (f.key)}
      <option value={f.key} disabled={f.disabled} title={f.hint}>{f.label}{f.disabled && f.hint ? ` (${f.hint})` : ""}</option>
    {/each}
  </select>
  <button class="dir" aria-label="{label} {current?.label ?? ''}: {words}. Click to reverse." title="{words}. Click to reverse." onclick={onflip}>
    <span aria-hidden="true">⇅</span> {words}
  </button>
</span>

<style>
  .sort { display: inline-flex; align-items: center; gap: 4px; }
  select, .dir { padding: 2px 6px; background: var(--panel); color: inherit; border: 1px solid var(--line, #444); border-radius: 3px; font: inherit; }
  select { max-width: 150px; }
  .dir { white-space: nowrap; }
  .dir:hover { background: var(--hover, #333); }
  @media (max-width: 1700px) {
    select { max-width: 112px; }
    .dir span { display: none; }
  }
</style>
