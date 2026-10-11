<script lang="ts" module>
  // One check per few minutes, shared by the banner and the Settings card.
  let cache: { at: number; p: Promise<any> } | null = null;
  export function checkUpdate(current: string, force = false) {
    if (!force && cache && Date.now() - cache.at < 10 * 60_000) return cache.p;
    const p = import('./lib/api').then(({ call }) => call('update_check', { current }));
    cache = { at: Date.now(), p };
    p.catch(() => { cache = null; });
    return p;
  }
</script>

<script lang="ts">
  import { onMount } from 'svelte';
  import { call, native } from './lib/api';

  // `banner`: a slim notice shown only when something newer exists; otherwise the full card body.
  let { banner = false, version = $bindable('') }: { banner?: boolean; version?: string } = $props();
  let info = $state<any>(null);
  let phase = $state<'idle' | 'checking' | 'downloading' | 'installing'>('idle');
  let msg = $state('');

  async function check(force = false) {
    if (!version) return;
    phase = 'checking'; msg = '';
    try { info = await checkUpdate(version, force); if (force && !info.newer) msg = 'You have the newest version.'; }
    catch (e) { if (force) msg = (e as Error).message; }
    phase = 'idle';
  }

  async function install() {
    if (!info?.asset) { msg = 'There is no download for this device in that release.'; return; }
    phase = 'downloading'; msg = '';
    try {
      const r = await call('update_download', { asset: info.asset });
      phase = 'installing';
      await native('update_install', { path: r.path });
      msg = 'Installing… the app restarts by itself.';
    } catch (e) { msg = (e as Error).message; phase = 'idle'; }
  }

  onMount(async () => {
    if (!version) version = (await native<string>('app_version').catch(() => null)) ?? '';
    if (version) check();
  });
</script>

{#if banner}
  {#if info?.newer && info.asset}
    <div class="ban card fadein">
      <span><b>Version {info.latest}</b> is available.</span>
      <button class="btn small primary" disabled={phase !== 'idle'} onclick={install}>
        {phase === 'downloading' ? 'Downloading…' : phase === 'installing' ? 'Installing…' : 'Update now'}
      </button>
    </div>
  {/if}
{:else if version}
  {#if info?.newer}
    <p><b>Version {info.latest}</b> is available (you have {version}).</p>
    {#if info.notes}<p class="muted small notes">{info.notes}</p>{/if}
    <button class="btn primary" disabled={phase !== 'idle' || !info.asset} onclick={install}>
      {phase === 'downloading' ? 'Downloading…' : phase === 'installing' ? 'Installing…' : 'Update now'}
    </button>
  {:else}
    <button class="btn small" disabled={phase !== 'idle'} onclick={() => check(true)}>{phase === 'checking' ? 'Checking…' : 'Check for updates'}</button>
  {/if}
  {#if msg}<p class="muted small">{msg}</p>{/if}
{/if}

<style>
  .ban { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin: 0 0 14px; padding: 10px 14px; background: var(--blue-soft); }
  .notes { white-space: pre-line; max-height: 9em; overflow: auto; }
  .small { font-size: 13px; }
</style>
