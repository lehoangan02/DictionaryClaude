<script lang="ts">
  import { call } from './lib/api';
  import { app, bump } from './lib/store.svelte';
  import { when } from './lib/format';

  let list = $state<any>({ packs: [], paired: false });
  let prog = $state<Record<string, { done: number; total: number; route?: string; note?: string }>>({});
  let err = $state('');
  let checking = $state(false);
  let timer: ReturnType<typeof setInterval> | undefined;
  let wasBusy = false;

  const mb = (n: number) => (n >= 1e9 ? (n / 1e9).toFixed(2) + ' GB' : Math.max(1, Math.round(n / 1e6)) + ' MB');
  async function load() { list = await call('packs_list'); }
  async function refreshApp() { app.sources = await call('sources'); bump(); }

  // The app itself does the downloading, whatever screen is open, so this list only watches it:
  // coming back to Settings (or the page reloading) picks the running downloads up again.
  // Speed: bytes since the last look, smoothed, so a stalled or slow link is visible instead of guessed at.
  let last: Record<string, { done: number; at: number; bps: number }> = {};
  let speed = $state<Record<string, number>>({});
  function measure() {
    const now = Date.now();
    for (const [id, p] of Object.entries(prog) as [string, any][]) {
      const l = last[id];
      if (l && now > l.at) {
        const inst = Math.max(0, (p.done - l.done) / ((now - l.at) / 1000));
        last[id] = { done: p.done, at: now, bps: l.bps ? l.bps * 0.8 + inst * 0.2 : inst };
      } else if (!l) last[id] = { done: p.done, at: now, bps: 0 };
      speed[id] = last[id].bps;
    }
    for (const id of Object.keys(last)) if (!(id in prog)) { delete last[id]; delete speed[id]; }
  }
  const rate = (bps: number) => (bps >= 1e6 ? (bps / 1e6).toFixed(1) + ' MB/s' : Math.round(bps / 1e3) + ' kB/s');
  const eta = (p: any, bps: number) => {
    if (!p.total || bps < 1000) return '';
    const s = (p.total - p.done) / bps;
    return s > 90 ? `about ${Math.round(s / 60)} min left` : `about ${Math.round(s)} s left`;
  };
  async function tick() {
    try { prog = await call('pack_progress'); measure(); } catch { return; /* busy */ }
    const busy = Object.keys(prog).length > 0;
    if (busy) { wasBusy = true; return; }
    clearInterval(timer); timer = undefined;
    if (wasBusy) { wasBusy = false; await load().catch(() => {}); await refreshApp().catch(() => {}); }
  }
  function watch() { if (!timer) timer = setInterval(tick, 500); }
  $effect(() => { void app.version; load().catch(() => {}); tick().then(() => { if (Object.keys(prog).length) watch(); }); return () => clearInterval(timer); });

  async function check() {
    checking = true; err = '';
    try { list = await call('packs_catalog'); } catch (e) { err = (e as Error).message; }
    checking = false;
  }
  async function install(id: string) {
    err = '';
    prog = { ...prog, [id]: { done: 0, total: 0 } };
    wasBusy = true; watch();
    try { await call('pack_install', { id }); }
    catch (e) { err = (e as Error).message; }
    await tick();
  }
  async function remove(id: string) {
    err = '';
    try { await call('pack_remove', { id }); await load(); await refreshApp(); } catch (e) { err = (e as Error).message; }
  }
</script>

<section class="card rise" style="--i:3">
  <h3>Downloads</h3>
  {#if !list.paired}
    <p class="muted small">Pair with your server above, then the dictionaries it holds can be downloaded here, for example on a phone that came without them.</p>
  {:else}
    <p class="muted small">Dictionaries come from your own server. {#if list.checked}Checked {when(list.checked)}.{/if}</p>
  {/if}
  <ul class="packs">
    {#each list.packs as p (p.id)}
      <li>
        <div class="info">
          <b>{p.title}</b>
          <span class="muted small">{p.about}</span>
          <span class="muted small">{p.licence}{p.size ? ' · ' + mb(p.size) : ''}{p.built_in ? ' · built in' : ''}</span>
          {#if p.error && !prog[p.id]}<span class="err small">Stopped: {p.error}{#if p.want} It starts again by itself when the connection is back.{/if}</span>{/if}
          {#if prog[p.id]}
            <div class="bar" role="progressbar" aria-valuenow={prog[p.id].done} aria-valuemax={prog[p.id].total}>
              <i style="transform: scaleX({prog[p.id].total ? prog[p.id].done / prog[p.id].total : 0})"></i></div>
            <span class="muted small">{mb(prog[p.id].done)} of {prog[p.id].total ? mb(prog[p.id].total) : '…'}{#if speed[p.id] > 0} · {rate(speed[p.id])}{/if}{#if prog[p.id].route} · via {prog[p.id].route}{/if}{#if eta(prog[p.id], speed[p.id])} · {eta(prog[p.id], speed[p.id])}{/if}</span>
            {#if prog[p.id].note}<span class="err small">{prog[p.id].note}</span>
            {:else if speed[p.id] < 20000 && prog[p.id].done > 0}<span class="muted small">Very slow right now. It keeps trying and continues when the connection improves.</span>{/if}
          {/if}
        </div>
        {#if prog[p.id]}
          <span class="muted small">Downloading…</span>
        {:else if p.downloaded}
          {#if p.update}<button class="btn small primary" onclick={() => install(p.id)}>Update</button>{/if}
          <button class="btn small ghost" onclick={() => remove(p.id)}>Remove</button>
        {:else if p.built_in}
          <span class="muted small">Installed</span>
        {:else}
          <button class="btn small primary" disabled={!list.paired || (list.checked && !p.on_server)} onclick={() => install(p.id)}>{p.error ? 'Resume' : 'Download'}</button>
        {/if}
      </li>
    {/each}
  </ul>
  {#if list.paired && Object.keys(prog).length === 0}<button class="btn small ghost" disabled={checking} onclick={check}>{checking ? 'Checking…' : 'Refresh the list from the server'}</button>{/if}
  {#if err}<p class="msg">{err}</p>{/if}
</section>

<style>
  .packs { list-style: none; margin: 8px 0 12px; padding: 0; display: grid; gap: 4px; }
  .packs li { display: flex; flex-wrap: wrap; gap: 8px 12px; align-items: center; padding: 10px 0; border-top: 1px solid var(--line); }
  .packs li:first-child { border-top: 0; }
  .info { flex: 1 1 220px; display: grid; gap: 2px; min-width: 0; }
  .bar { height: 6px; border-radius: 3px; background: var(--panel-2); overflow: hidden; margin-top: 4px; }
  .bar i { display: block; height: 100%; background: var(--blue); transform-origin: left; transition: transform var(--t) linear; }
</style>
