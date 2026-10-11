<script lang="ts">
  import { call } from './lib/api';
  import { app, bump } from './lib/store.svelte';
  import { when } from './lib/format';

  let list = $state<any>({ packs: [], paired: false });
  let prog = $state<Record<string, { done: number; total: number }>>({});
  let err = $state('');
  let checking = $state(false);
  let timer: ReturnType<typeof setInterval> | undefined;

  const mb = (n: number) => (n >= 1e9 ? (n / 1e9).toFixed(2) + ' GB' : Math.max(1, Math.round(n / 1e6)) + ' MB');
  async function load() { list = await call('packs_list'); }
  $effect(() => { void app.version; load().catch(() => {}); return () => clearInterval(timer); });

  async function check() {
    checking = true; err = '';
    try { list = await call('packs_catalog'); } catch (e) { err = (e as Error).message; }
    checking = false;
  }
  async function refreshApp() { app.sources = await call('sources'); bump(); }
  async function install(id: string) {
    err = '';
    prog = { ...prog, [id]: { done: 0, total: 0 } };
    timer = setInterval(async () => { try { prog = { ...prog, ...(await call('pack_progress')) }; } catch { /* busy */ } }, 400);
    try { await call('pack_install', { id }); await load(); await refreshApp(); }
    catch (e) { err = (e as Error).message; }
    clearInterval(timer);
    const { [id]: _gone, ...rest } = prog; prog = rest;
  }
  async function remove(id: string) {
    err = '';
    try { await call('pack_remove', { id }); await load(); await refreshApp(); } catch (e) { err = (e as Error).message; }
  }
</script>

<section class="card rise" style="--i:1">
  <h3>Downloads</h3>
  {#if !list.paired}
    <p class="muted small">Pair with your server below, then the dictionaries it holds can be downloaded here, for example on a phone that came without them.</p>
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
          {#if prog[p.id]}
            <div class="bar" role="progressbar" aria-valuenow={prog[p.id].done} aria-valuemax={prog[p.id].total}>
              <i style="transform: scaleX({prog[p.id].total ? prog[p.id].done / prog[p.id].total : 0})"></i></div>
            <span class="muted small">{mb(prog[p.id].done)} of {prog[p.id].total ? mb(prog[p.id].total) : '…'}</span>
          {/if}
        </div>
        {#if prog[p.id]}
          <button class="btn small" disabled>Downloading…</button>
        {:else if p.downloaded}
          {#if p.update}<button class="btn small primary" onclick={() => install(p.id)}>Update</button>{/if}
          <button class="btn small ghost" onclick={() => remove(p.id)}>Remove</button>
        {:else if p.built_in}
          <span class="muted small">Installed</span>
        {:else}
          <button class="btn small primary" disabled={!list.paired || (list.checked && !p.on_server)} onclick={() => install(p.id)}>Download</button>
        {/if}
      </li>
    {/each}
  </ul>
  {#if list.paired}<button class="btn small" disabled={checking} onclick={check}>{checking ? 'Checking…' : 'Check the server'}</button>{/if}
  {#if err}<p class="msg">{err}</p>{/if}
</section>

<style>
  .packs { list-style: none; margin: 8px 0 12px; padding: 0; display: grid; gap: 4px; }
  .packs li { display: flex; gap: 12px; align-items: center; padding: 10px 0; border-top: 1px solid var(--line); }
  .packs li:first-child { border-top: 0; }
  .info { flex: 1; display: grid; gap: 2px; min-width: 0; }
  .bar { height: 6px; border-radius: 3px; background: var(--panel-2); overflow: hidden; margin-top: 4px; }
  .bar i { display: block; height: 100%; background: var(--blue); transform-origin: left; transition: transform var(--t) linear; }
</style>
