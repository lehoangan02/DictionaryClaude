<script lang="ts">
  import { onMount } from 'svelte';
  import { call } from './lib/api';

  let st = $state<any>(null);
  let prog = $state<Record<string, { done: number; total: number }>>({});
  let pressed = $state(false);
  const mb = (n: number) => Math.max(1, Math.round(n / 1e6));

  onMount(() => {
    const tick = async () => {
      try {
        st = await call('packs_auto_status');
        prog = await call('pack_progress');
        if (Object.keys(prog).length) pressed = false;
      } catch { /* starting up */ }
    };
    tick();
    const t = setInterval(tick, 2000);
    return () => clearInterval(t);
  });
  async function now() { pressed = true; await call('packs_auto_now').catch(() => {}); }
  const busy = $derived(Object.values(prog));
  const done = $derived(busy.reduce((a, p) => a + p.done, 0));
  const total = $derived(busy.reduce((a, p) => a + p.total, 0));
  const mins = (s: number) => (s >= 90 ? `${Math.round(s / 60)} min` : `${Math.max(1, Math.round(s))} s`);
</script>

<div class="card setup">
  {#if busy.length}
    <b>Setting up your dictionaries…</b>
    <div class="bar"><i style="transform: scaleX({total ? done / total : 0})"></i></div>
    <span class="muted small">{mb(done)} of {total ? mb(total) : '…'} MB. You can keep using Merriam-Webster meanwhile.</span>
  {:else if !st}
    <b>Checking your server…</b>
  {:else if !st.paired}
    <b>Connecting to your server</b>
    <span class="muted small">On your home Wi-Fi this device pairs with your server and downloads its dictionaries by itself. Nothing to type. Away from home, Merriam-Webster still works online.</span>
  {:else if st.error}
    <b>The dictionaries could not be downloaded</b>
    <span class="muted small">{st.error}{st.retry_in_s ? ` Trying again in ${mins(st.retry_in_s)}.` : ''}</span>
    <div><button class="btn small primary" disabled={pressed} onclick={now}>{pressed ? 'Starting…' : 'Try now'}</button></div>
  {:else if st.route === 'home'}
    <b>Starting the download…</b>
    <span class="muted small">Connected to your server at home; the dictionaries begin downloading within a few seconds.</span>
    <div><button class="btn small" disabled={pressed} onclick={now}>{pressed ? 'Starting…' : 'Download now'}</button></div>
  {:else if st.route === 'remote'}
    <b>Connected to your server from outside</b>
    <span class="muted small">The dictionaries are several hundred MB, so they wait for your home Wi-Fi unless you start them yourself (this uses mobile data if you are on it).</span>
    <div><button class="btn small primary" disabled={pressed} onclick={now}>{pressed ? 'Starting…' : 'Download now'}</button></div>
  {:else}
    <b>Your server is not reachable</b>
    <span class="muted small">No connection to your sync server right now; the download starts as soon as there is one.</span>
  {/if}
</div>

<style>
  .setup { display: grid; gap: 8px; margin: 12px 0; }
  .bar { height: 6px; border-radius: 3px; background: var(--panel-2); overflow: hidden; }
  .bar i { display: block; height: 100%; background: var(--blue); transform-origin: left; transition: transform var(--t) linear; }
</style>
