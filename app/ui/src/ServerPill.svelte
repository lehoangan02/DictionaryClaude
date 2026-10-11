<script lang="ts">
  import { onMount } from 'svelte';
  import { call } from './lib/api';
  import { app, bump } from './lib/store.svelte';
  import { when } from './lib/format';
  import { pop, grow } from './lib/motion';

  type Status = {
    paired: boolean; online?: boolean; route?: 'lan' | 'wan' | null; latency_ms?: number | null; vpn?: boolean;
    ssid?: string | null; pending?: number; name?: string; last_sync?: { at: number; pushed: number; pulled: number; route: string } | null;
    probes?: { route: string; ok: boolean; ms: number; error?: string | null }[];
  };

  let st = $state<Status>({ paired: false });
  let open = $state(false);
  let wrap = $state<HTMLElement | null>(null);
  let busy = $state(false);
  let msg = $state('');
  let checking = $state(true);

  async function refresh() {
    try { st = await call('server_status', { probe: true }); } catch { /* keep the last known state */ }
    checking = false;
  }
  async function syncNow() {
    busy = true; msg = '';
    try {
      const r = await call('sync_now');
      msg = `Synced: sent ${r.pushed}, received ${r.pulled}.`;
      bump();
    } catch (e) { msg = (e as Error).message; }
    busy = false;
    await refresh();
  }

  onMount(() => {
    refresh();
    const t = setInterval(refresh, 30_000);
    const wake = () => refresh();
    window.addEventListener('online', wake);
    window.addEventListener('offline', wake);
    document.addEventListener('visibilitychange', () => { if (!document.hidden) refresh(); });
    return () => { clearInterval(t); window.removeEventListener('online', wake); window.removeEventListener('offline', wake); };
  });
  // After pairing or a sync elsewhere in the app, look again.
  $effect(() => { void app.version; refresh(); });

  const label = $derived(
    !st.paired ? 'Device only'
    : checking ? 'Checking…'
    : st.online ? (st.route === 'lan' ? 'Home' : 'Remote')
    : 'Offline',
  );
  const tone = $derived(!st.paired || checking ? 'grey' : st.online ? (st.vpn ? 'amber' : st.route === 'lan' ? 'green' : 'blue') : 'red');
</script>

<!-- Close on any press outside (captured so nothing can swallow it), on Escape, and when the window loses focus. -->
<svelte:window
  onpointerdowncapture={(e) => { if (open && wrap && !wrap.contains(e.target as Node)) open = false; }}
  onkeydown={(e) => { if (e.key === 'Escape') open = false; }}
  onblur={() => (open = false)}
/>

<div class="wrapper" bind:this={wrap}>
  <button class="pill" onclick={() => { open = !open; if (open) refresh(); }} aria-expanded={open} aria-label="Sync server: {label}">
    <span class="dot {tone}"></span><span class="lbl">{label}</span>
    {#if st.paired && (st.pending ?? 0) > 0}<span class="pend" in:grow title="Changes not yet sent">{st.pending}</span>{/if}
    {#if st.vpn && st.paired}<span class="vpn">VPN</span>{/if}
  </button>
  {#if open}
    <div class="pop" role="dialog" aria-label="Sync server" transition:pop>
      <b>Sync server</b>
      {#if !st.paired}
        <p>Not paired. Everything works offline on this device; pair a server in Settings to keep your notes, history and quizzes in step across devices.</p>
        <button class="btn small primary" onclick={() => { app.tab = 'settings'; open = false; }}>Set up sync</button>
      {:else}
        <dl>
          <dt>Status</dt>
          <dd class={tone}>{st.online ? `Online via ${st.route === 'lan' ? 'home network' : 'remote port'}` : 'Not reachable'}
            {#if st.latency_ms != null}<span class="muted lat"> · {st.latency_ms}&nbsp;ms</span>{/if}</dd>
          <dt>Server</dt><dd>{st.name}</dd>
          <dt>To send</dt><dd>{st.pending ?? 0} change{st.pending === 1 ? '' : 's'}</dd>
          <dt>Last sync</dt><dd>{st.last_sync ? when(st.last_sync.at) : 'never'}</dd>
          {#if st.ssid}<dt>Wi-Fi</dt><dd>{st.ssid}</dd>{/if}
        </dl>
        {#if st.vpn}<p class="warn">A VPN is active. If the server can't be reached, allow local network access or split tunneling for this app.</p>{/if}
        {#if !st.online && st.probes?.length}
          <ul class="why">{#each st.probes as p}<li><b>{p.route === 'lan' ? 'Home' : 'Remote'}</b>: {p.error ?? 'failed'}</li>{/each}</ul>
        {/if}
        {#if msg}<p class="msg">{msg}</p>{/if}
        <div class="row">
          <button class="btn small primary" disabled={busy || !st.online} onclick={syncNow}>{busy ? 'Syncing…' : 'Sync now'}</button>
          <button class="btn small" disabled={busy} onclick={refresh}>Test connection</button>
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .wrapper { position: relative; }
  .pill {
    display: inline-flex; align-items: center; gap: 7px; background: var(--panel); color: var(--text);
    border: 1px solid var(--line); border-radius: 999px; padding: 6px 13px; font-size: 13px; white-space: nowrap;
  }
  .dot { width: 9px; height: 9px; border-radius: 50%; background: #9aa7b8; flex: none; transition: background-color var(--t-slow) var(--ease), box-shadow var(--t-slow) var(--ease); }
  .dot.green { background: #3ddc84; } .dot.blue { background: #5aa9ff; } .dot.amber { background: #ffb020; } .dot.red { background: #ff6b5e; }
  .pend { background: var(--accent); border-radius: 999px; padding: 0 7px; font-size: 11px; font-weight: 700; }
  .vpn { font-size: 10px; font-weight: 700; border: 1px solid #ffb020; color: #ffb020; border-radius: 4px; padding: 0 4px; }
  .pop {
    position: absolute; right: 0; top: calc(100% + 24px); width: min(390px, 92vw); background: var(--bg); color: var(--text);
    border: 1px solid var(--line); border-radius: 16px; box-shadow: var(--shadow); padding: 14px 16px; z-index: 40; font-size: 14px;
  }
  .pop p { margin: 6px 0; color: var(--muted); }
  dl { display: grid; grid-template-columns: auto 1fr; gap: 3px 12px; margin: 8px 0; }
  dt { color: var(--muted); } dd { margin: 0; }
  .lat { white-space: nowrap; }
  dd.green { color: var(--good); } dd.red { color: var(--bad); }
  .warn { background: #fff4d6; color: #5d4300 !important; border-radius: 8px; padding: 6px 10px; }
  :global([data-theme='dark']) .warn { background: #3d3210; color: #ffd98a !important; }
  .why { margin: 6px 0; padding-left: 18px; color: var(--muted); font-size: 13px; }
  .msg { color: var(--text) !important; }
  .row { display: flex; gap: 8px; margin-top: 8px; }
  button:disabled { opacity: 0.5; }
  @media (max-width: 720px) { .lbl { display: none; } .pill { padding: 8px; } }
</style>
