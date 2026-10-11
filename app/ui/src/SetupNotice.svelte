<script lang="ts">
  import { onMount } from 'svelte';
  import { call } from './lib/api';

  let paired = $state<boolean | null>(null);
  let prog = $state<Record<string, { done: number; total: number }>>({});
  const mb = (n: number) => Math.max(1, Math.round(n / 1e6));

  onMount(() => {
    const tick = async () => {
      try {
        const st = await call('server_status', { probe: false });
        paired = !!st.paired;
        prog = await call('pack_progress');
      } catch { /* starting up */ }
    };
    tick();
    const t = setInterval(tick, 2000);
    return () => clearInterval(t);
  });
  const busy = $derived(Object.values(prog));
  const done = $derived(busy.reduce((a, p) => a + p.done, 0));
  const total = $derived(busy.reduce((a, p) => a + p.total, 0));
</script>

<div class="card setup">
  {#if busy.length}
    <b>Setting up your dictionaries…</b>
    <div class="bar"><i style="transform: scaleX({total ? done / total : 0})"></i></div>
    <span class="muted small">{mb(done)} of {total ? mb(total) : '…'} MB. You can keep using Merriam-Webster meanwhile.</span>
  {:else if paired}
    <b>Waiting for the home network</b>
    <span class="muted small">Your dictionaries download by themselves when this device is on your home Wi-Fi. Nothing to press.</span>
  {:else}
    <b>Connecting to your server</b>
    <span class="muted small">On your home Wi-Fi this device pairs with your server and downloads its dictionaries by itself. Nothing to type. Away from home, Merriam-Webster still works online.</span>
  {/if}
</div>

<style>
  .setup { display: grid; gap: 8px; margin: 12px 0; }
  .bar { height: 6px; border-radius: 3px; background: var(--panel-2); overflow: hidden; }
  .bar i { display: block; height: 100%; background: var(--blue); transform-origin: left; transition: transform var(--t) linear; }
</style>
