<script lang="ts">
  // One icon from icons.ts. The SVG is built once per icon and size as a
  // string: the ribbon holds many icons and they should cost nothing to show.
  import { iconSvg } from "./icons";

  let { name, size = 20 }: { name: string; size?: number } = $props();
</script>

<!-- eslint-disable-next-line svelte/no-at-html-tags -- markup built from icons.ts only -->
<span class="icon" aria-hidden="true">{@html iconSvg(name, size)}</span>

<style>
  .icon {
    flex: none;
    display: block;
    line-height: 0;
  }
  .icon :global(svg) {
    stroke-width: 1.4;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
</style>
