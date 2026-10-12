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
  let progress = $state({ done: 0, total: 0, stage: 'idle' });
  const mb = (n: number) => (n / 1048576).toFixed(1);
  const isAndroid = /Android/i.test(navigator.userAgent);

  async function check(force = false) {
    if (!version) return;
    phase = 'checking'; msg = '';
    try { info = await checkUpdate(version, force); if (force && !info.newer) msg = 'You have the newest version.'; }
    catch (e) { msg = `Could not check for updates: ${(e as Error).message}`; }
    phase = 'idle';
  }

  // What the button and the line under it say while the update works.
  const status = $derived.by(() => {
    if (phase === 'downloading') {
      if (progress.stage === 'verifying') return 'Checking the download…';
      const t = progress.total || info?.asset?.size || 0;
      return t ? `Downloading ${mb(progress.done)} of ${mb(t)} MB (${Math.min(100, Math.round((100 * progress.done) / t))}%)` : `Downloading ${mb(progress.done)} MB…`;
    }
    if (phase === 'installing') return isAndroid ? 'Opening the Android installer…' : 'Installing… LadyBird restarts by itself.';
    return msg;
  });
  const label = $derived(phase === 'downloading' ? 'Downloading…' : phase === 'installing' ? 'Installing…' : msg.startsWith('Update failed') || msg.startsWith('Allow') ? 'Try again' : 'Update now');

  async function install() {
    if (!info?.asset) { msg = 'There is no download for this device in that release.'; return; }
    phase = 'downloading'; msg = ''; progress = { done: 0, total: info.asset.size ?? 0, stage: 'downloading' };
    const timer = setInterval(async () => { try { progress = await call('update_progress'); } catch { /* busy */ } }, 400);
    try {
      const r = await call('update_download', { asset: info.asset });
      clearInterval(timer);
      phase = 'installing';
      const how = await native<string>('update_install', { path: r.path });
      if (how === 'permission') {
        msg = 'Allow installs from LadyBird in the settings page that just opened, come back here and press Update now again.';
      } else if (how === 'installer') {
        msg = 'The Android installer is open: press Install, then open LadyBird again.';
      }
    } catch (e) { msg = `Update failed: ${(e as Error).message}`; }
    clearInterval(timer);
    if (phase === 'downloading' || phase === 'installing') phase = 'idle';
  }

  onMount(async () => {
    if (!version) version = (await native<string>('app_version').catch(() => null)) ?? '';
    if (version) check();
  });
</script>

{#if banner}
  {#if info?.newer && info.asset}
    <div class="ban card fadein">
      <div class="txt">
        <span><b>Version {info.latest}</b> is available (you have {version}).</span>
        {#if status}<span class="small" class:bad={msg.startsWith('Update failed')}>{status}</span>{/if}
        {#if phase === 'downloading' && progress.total}<div class="bar"><i style="width:{Math.min(100, (100 * progress.done) / progress.total)}%"></i></div>{/if}
      </div>
      <button class="btn small primary" disabled={phase !== 'idle'} onclick={install}>{label}</button>
    </div>
  {:else if msg.startsWith('Could not check')}
    <div class="ban card fadein"><span class="small bad">{msg}</span></div>
  {/if}
{:else if version}
  {#if info?.newer}
    <p><b>Version {info.latest}</b> is available (you have {version}).</p>
    {#if info.notes}<p class="muted small notes">{info.notes}</p>{/if}
    <button class="btn primary" disabled={phase !== 'idle' || !info.asset} onclick={install}>{label}</button>
    {#if phase === 'downloading' && progress.total}<div class="bar"><i style="width:{Math.min(100, (100 * progress.done) / progress.total)}%"></i></div>{/if}
  {:else}
    <button class="btn small" disabled={phase !== 'idle'} onclick={() => check(true)}>{phase === 'checking' ? 'Checking…' : 'Check for updates'}</button>
  {/if}
  {#if status}<p class="small" class:bad={msg.startsWith('Update failed') || msg.startsWith('Could not')} class:muted={!msg.startsWith('Update failed') && !msg.startsWith('Could not')}>{status}</p>{/if}
{/if}

<style>
  .ban { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin: 0 0 14px; padding: 10px 14px; background: var(--blue-soft); }
  .notes { white-space: pre-line; max-height: 9em; overflow: auto; }
  .small { font-size: 13px; }
  .txt { display: flex; flex-direction: column; gap: 4px; min-width: 0; }
  .bad { color: var(--red, #b3261e); }
  .bar { height: 5px; border-radius: 3px; background: var(--line); overflow: hidden; margin-top: 4px; }
  .bar i { display: block; height: 100%; background: var(--blue); transition: width 0.3s; }
</style>
