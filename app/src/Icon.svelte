<script lang="ts">
  import type { Icon } from "./lib/tree";

  let { name }: { name: Icon } = $props();
</script>

<!-- 16×16 outlines in the text colour. -->
<svg class="icon" viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
  {#if name === "album"}
    <rect x="2.5" y="3.5" width="11" height="9" rx="1" /><path d="M2.5 10.5l3-3 3 3 2-2 3 3" />
  {:else if name === "smart"}
    <rect x="2.5" y="3.5" width="11" height="9" rx="1" /><path d="M8 5.5v5M5.5 8h5" />
  {:else if name === "group"}
    <path d="M1.5 4.5v8h13v-7h-7l-1.5-1.5h-4z" /><path d="M4 2.5h8" />
  {:else if name === "project"}
    <rect x="1.5" y="5.5" width="13" height="8" rx="1" /><path d="M5.5 5.5v-2h5v2" />
  {:else if name === "folder"}
    <path d="M1.5 3.5v9h13v-7.5h-7l-1.5-1.5z" />
  {:else}
    <path d="M2.5 2.5h5l6 6-5 5-6-6z" /><circle cx="5.5" cy="5.5" r="1" />
  {/if}
</svg>

<style>
  .icon {
    flex: none;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.1;
    stroke-linejoin: round;
    color: var(--muted);
  }
</style>
