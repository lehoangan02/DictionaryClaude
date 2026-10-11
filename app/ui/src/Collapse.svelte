<script lang="ts">
  import type { Snippet } from 'svelte';
  // Animated show/hide for a block of content; the trigger lives outside.
  let { open = false, children }: { open?: boolean; children: Snippet } = $props();
</script>

<div class="collapse" class:open aria-hidden={!open} inert={!open}>
  <div class="inner">{@render children()}</div>
</div>
