<script lang="ts">
  import type { Snippet } from 'svelte';
  import Collapse from './Collapse.svelte';
  import Icon from './Icon.svelte';

  // A trigger plus content that expands smoothly; the plus turns into a minus as it opens.
  let { label, boxed = false, children }: { label: string; boxed?: boolean; children: Snippet } = $props();
  let open = $state(false);
</script>

<div class="disc" class:boxed class:open>
  <button class="trig" aria-expanded={open} onclick={() => (open = !open)}>
    <span class="pm"><span class="h"><Icon name="minus" size={18} /></span><span class="v"><Icon name="minus" size={18} /></span></span>
    <span class="lbl">{label}</span>
  </button>
  <Collapse {open}>{@render children()}</Collapse>
</div>

<style>
  .trig { display: flex; align-items: center; gap: 10px; background: none; border: 0; width: 100%; text-align: left; color: inherit; padding: 10px 4px; }
  .pm { position: relative; width: 18px; height: 18px; flex: none; color: var(--blue); }
  .pm > span { position: absolute; inset: 0; display: flex; }
  .pm .v { transition: transform var(--t) var(--ease); transform: rotate(90deg); }
  .open .pm .v { transform: rotate(0deg); }
  .lbl { font-size: 15px; color: var(--blue); }
  .boxed { background: var(--salmon); border-left: 5px solid var(--salmon-line); }
  .boxed .trig { padding: 14px 16px; }
  .boxed .lbl { font-size: 20px; color: var(--text); }
  .boxed .pm { color: var(--salmon-line); }
</style>
