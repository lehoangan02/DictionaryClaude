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
  // 'waiting': Android sent the person to the "allow installs" page; the update carries on when they come back.
  let phase = $state<'idle' | 'checking' | 'downloading' | 'installing' | 'waiting'>('idle');
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
  const label = $derived(phase === 'downloading' ? 'Downloading…' : phase === 'installing' ? 'Installing…' : phase === 'waiting' ? 'Continue' : msg.startsWith('Update failed') ? 'Try again' : 'Update now');
  // The buttons stay usable while waiting, in case the page is not told that the person came back.
  const busy = $derived(phase !== 'idle' && phase !== 'waiting');

  // The person is on Android's "allow installs" page: remember it, so that coming back (even to a page the system
  // reloaded) carries on with the update by itself. The download is kept, so carrying on is instant.
  const RESUME = 'update.resume';
  const remember = (v: string | null) => { try { v ? localStorage.setItem(RESUME, v) : localStorage.removeItem(RESUME); } catch { /* private mode */ } };

  async function install(auto = false) {
    if (!info?.asset) { msg = 'There is no download for this device in that release.'; return; }
    phase = 'downloading'; msg = ''; progress = { done: 0, total: info.asset.size ?? 0, stage: 'downloading' };
    const timer = setInterval(async () => { try { progress = await call('update_progress'); } catch { /* busy */ } }, 400);
    try {
      const r = await call('update_download', { asset: info.asset });
      clearInterval(timer);
      phase = 'installing';
      const how = await native<string>('update_install', { path: r.path });
      if (how === 'permission' && !auto) {
        phase = 'waiting'; remember(info.latest);
        msg = 'Allow installs from LadyBird on the settings page that just opened, then come back: the update carries on by itself.';
      } else if (how === 'permission') {
        // Back from the settings page without having allowed it: ask once more, then leave it to the button.
        msg = 'Installs from LadyBird are still not allowed. Turn them on in the settings page, come back and press Update now.';
      } else if (how === 'installer') {
        remember(null);
        msg = 'The Android installer is open: press Install, then open LadyBird again.';
      }
    } catch (e) { msg = `Update failed: ${(e as Error).message}`; remember(null); }
    clearInterval(timer);
    if (phase === 'downloading' || phase === 'installing') phase = 'idle';
  }

  // Coming back to the app while it waits for the permission.
  function back() {
    if (phase === 'waiting' && document.visibilityState === 'visible') install(true);
  }

  onMount(async () => {
    if (!version) version = (await native<string>('app_version').catch(() => null)) ?? '';
    if (!version) return;
    await check();
    // The system may have rebuilt the page while the person was on the settings page.
    let pending: string | null = null;
    try { pending = localStorage.getItem(RESUME); } catch { /* private mode */ }
    if (pending) {
      remember(null); // whichever copy of this card gets here first carries on; the other must not start a second install
      if (isAndroid && info?.newer && info.latest === pending && info.asset) install(true);
    }
  });
</script>

<svelte:document onvisibilitychange={back} />
<svelte:window onfocus={back} />

{#if banner}
  {#if info?.newer && info.asset}
    <div class="ban card fadein">
      <div class="txt">
        <span><b>Version {info.latest}</b> is available (you have {version}).</span>
        {#if status}<span class="small" class:bad={msg.startsWith('Update failed')}>{status}</span>{/if}
        {#if phase === 'downloading' && progress.total}<div class="bar"><i style="width:{Math.min(100, (100 * progress.done) / progress.total)}%"></i></div>{/if}
      </div>
      <button class="btn small primary" disabled={busy} onclick={() => install()}>{label}</button>
    </div>
  {:else if msg.startsWith('Could not check')}
    <div class="ban card fadein"><span class="small bad">{msg}</span></div>
  {/if}
{:else if version}
  {#if info?.newer}
    <p><b>Version {info.latest}</b> is available (you have {version}).</p>
    {#if info.notes}<p class="muted small notes">{info.notes}</p>{/if}
    <button class="btn primary" disabled={busy || !info.asset} onclick={() => install()}>{label}</button>
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
