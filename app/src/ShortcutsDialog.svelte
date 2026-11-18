<script lang="ts">
  import { backdrop } from "./lib/backdrop";
  import { PLACES, SHORTCUTS } from "./lib/shortcuts";

  // `?` or F1: every key the app answers to, by where it works.
  let { onclose }: { onclose: () => void } = $props();
  const shade = backdrop(() => onclose());
</script>

<svelte:window onkeydown={(e) => (e.key === "Escape" || e.key === "?" || e.key === "F1") && (e.preventDefault(), onclose())} />

<div class="backdrop" role="presentation" {...shade}>
  <div class="dialog shortcuts" role="dialog" aria-modal="true" aria-labelledby="keys-title">
    <h2 id="keys-title">Keys</h2>
    <div class="columns">
      {#each PLACES as place}
        <section>
          <h3>{place === "Anywhere" ? "Photos and viewer" : place}</h3>
          <table>
            <tbody>
              {#each SHORTCUTS.filter((s) => s.where === place) as s}
                <tr>
                  <td class="keys">
                    {#each s.keys.split(" · ") as k, i}{#if i}<span class="or"> or </span>{/if}<kbd>{k}</kbd>{/each}
                  </td>
                  <td>{s.what}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </section>
      {/each}
    </div>
    <p class="muted">Keys do not act while you type in a field.</p>
    <div class="buttons"><button onclick={onclose}>Close</button></div>
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
    padding: 18px 20px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 6px;
    box-shadow: 0 12px 40px rgb(0 0 0 / 50%);
  }
  h2 { margin: 0 0 4px; font-size: 16px; font-weight: 600; }
  .muted { color: var(--muted); margin: 12px 0 0; }
  .buttons { display: flex; justify-content: flex-end; margin-top: 12px; }
  .buttons button { padding: 3px 10px; border: 1px solid var(--line); border-radius: 4px; }
  .buttons button:hover { background: var(--hover); }
  .shortcuts { width: min(920px, 100%); max-height: calc(100vh - 32px); overflow: auto; }
  .columns { display: grid; grid-template-columns: repeat(auto-fit, minmax(380px, 1fr)); gap: 8px 24px; }
  h3 { margin: 12px 0 4px; font-size: 12px; color: var(--muted); text-transform: uppercase; letter-spacing: 0.05em; }
  table { border-collapse: collapse; width: 100%; }
  td { padding: 3px 0; vertical-align: top; font-size: 13px; }
  td.keys { width: 45%; padding-right: 12px; }
  kbd {
    display: inline-block;
    padding: 0 5px;
    font: 12px/18px inherit;
    border: 1px solid var(--line);
    border-bottom-width: 2px;
    border-radius: 3px;
    white-space: nowrap;
  }
  .or { color: var(--muted); font-size: 11px; }
</style>
