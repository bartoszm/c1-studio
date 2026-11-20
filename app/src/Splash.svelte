<script lang="ts">
  import { fade } from "svelte/transition";
  import art from "./assets/splash.webp";

  // The start screen: the artwork fills the window from launch until the first photos (or
  // an error, or the source chooser) take its place. The startup dialog sits over it.
</script>

<div class="splash" aria-hidden="true" out:fade={{ duration: 350 }}>
  <img src={art} alt="" draggable="false" />
</div>

<style>
  .splash {
    position: fixed;
    inset: 0;
    z-index: 35;
    background: #1b1c20;
    pointer-events: none;
  }
  img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    user-select: none;
  }
</style>
