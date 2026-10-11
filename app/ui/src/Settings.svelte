<script lang="ts">
  import { call } from './lib/api';
  import { app, setTheme, bump, orderedSources, saveOrder } from './lib/store.svelte';
  import Icon from './Icon.svelte';
  import PackManager from './PackManager.svelte';
  import AiCard from './AiCard.svelte';
  import { flip } from 'svelte/animate';
  import { flipDur } from './lib/motion';
  import { when } from './lib/format';

  let status = $state<any>({ paired: false });
  let code = $state('');
  let ssids = $state('Anan, Anan 5g');
  let msg = $state('');
  let busy = $state(false);

  async function load() { status = await call('server_status', { probe: false }); }
  $effect(() => { void app.version; load().catch(() => {}); });

  async function pair() {
    busy = true; msg = '';
    try { await call('sync_pair', { code }); code = ''; await call('sync_set_home_ssids', { ssids }); await load(); bump(); msg = 'Paired. Syncing…'; await syncNow(); }
    catch (e) { msg = (e as Error).message; }
    busy = false;
  }
  // Not paired yet: the app finds the server on the home Wi-Fi by itself; this button asks it to look right now.
  async function autoPair() {
    busy = true; msg = '';
    try { await call('sync_autopair'); await load(); bump(); msg = 'Paired. Syncing…'; await syncNow(); }
    catch (e) { msg = (e as Error).message; }
    busy = false;
  }
  $effect(() => {
    if (status.paired) return;
    const t = setInterval(() => { load().then(() => { if (status.paired) bump(); }).catch(() => {}); }, 5000);
    return () => clearInterval(t);
  });
  async function syncNow() {
    busy = true;
    try { const r = await call('sync_now'); msg = `Synced via ${r.route === 'lan' ? 'home network' : 'remote port'}: sent ${r.pushed}, received ${r.pulled}.`; bump(); }
    catch (e) { msg = (e as Error).message; }
    busy = false; await load();
  }
  // Pronunciation downloads: a few recordings at a time, waiting when the host asks us to slow down.
  let dl = $state({ running: false, done: 0, total: 0, note: '' });
  let stopDl = false;
  async function downloadAudio() {
    dl = { running: true, done: dl.done, total: dl.total, note: '' };
    stopDl = false;
    try {
      while (!stopDl) {
        const r = await call('audio_prefetch', { limit: 4 });
        dl = { running: true, done: r.done, total: r.total, note: r.rate_limited ? 'Waiting politely for the server…' : '' };
        if (r.done >= r.total) { dl.note = 'All recordings are on this device.'; break; }
        if (r.failed && !r.fetched && !r.rate_limited) { dl.note = r.error ?? 'Could not download (offline?).'; break; }
        if (r.rate_limited) await new Promise((res) => setTimeout(res, ((r.retry_after ?? 5) + 2) * 1000));
      }
    } catch (e) { dl.note = (e as Error).message; }
    dl.running = false;
  }

  // Merriam-Webster: keys stay on this device and are never shown again after saving.
  let mw = $state<any>({ configured: false });
  let mwKey = $state('');
  let mwThes = $state('');
  let mwMsg = $state('');
  async function loadMw() { mw = await call('mw_status'); }
  $effect(() => { void app.version; loadMw().catch(() => {}); });
  async function saveMw(extra: Record<string, unknown> = {}) {
    try {
      mw = await call('mw_config', { ...(mwKey ? { collegiate: mwKey } : {}), ...(mwThes ? { thesaurus: mwThes } : {}), ...extra });
      mwKey = ''; mwThes = '';
      app.sources = await call('sources');
      mwMsg = 'Saved.';
    } catch (e) { mwMsg = (e as Error).message; }
  }

  // Dictionary order: the first one shown is the default and the first tab.
  type Row = { id: string; name: string; entries: number | null; shown: boolean };
  let rows = $state<Row[]>([]);
  function loadRows() {
    const o = orderedSources();
    rows = o.all.map((s) => ({ id: s.id, name: s.name, entries: s.entries, shown: !o.hidden.includes(s.id) }));
  }
  $effect(() => { void app.sources; loadRows(); });
  function commit() { saveOrder(rows.map((r) => r.id), rows.filter((r) => !r.shown).map((r) => r.id)); }
  function move(i: number, d: number) {
    const j = i + d;
    if (j < 0 || j >= rows.length) return;
    [rows[i], rows[j]] = [rows[j], rows[i]];
    commit();
  }
  function toggle(i: number) {
    // At least one dictionary must stay visible.
    if (rows[i].shown && rows.filter((r) => r.shown).length === 1) return;
    rows[i].shown = !rows[i].shown;
    commit();
  }

  async function unpair() { await call('sync_unpair'); await load(); bump(); msg = 'Unpaired. Your data stays on this device.'; }
  async function saveSsids() { await call('sync_set_home_ssids', { ssids }); msg = 'Saved.'; }
</script>

<h1>Settings</h1>

<section class="card rise" style="--i:0">
  <h3>Dictionaries</h3>
  <p class="muted small">Choose the order of the tabs. The first dictionary that is switched on is the one every word opens in.</p>
  <ol class="order">
    {#each rows as r, i (r.id)}
      <li class:off={!r.shown} animate:flip={{ duration: flipDur() }}>
        <span class="n">{i + 1}</span>
        <span class="nm"><b>{r.name}</b>
          <span class="muted small">{r.id === 'mw' ? 'looked up online when you open a word' : r.entries != null ? `${r.entries.toLocaleString()} entries` : ''}</span></span>
        <button class="mv" disabled={i === 0} onclick={() => move(i, -1)} aria-label="Move {r.name} up"><Icon name="up" size={22} /></button>
        <button class="mv" disabled={i === rows.length - 1} onclick={() => move(i, 1)} aria-label="Move {r.name} down"><Icon name="down" size={22} /></button>
        <label class="sw" title="Show this dictionary"><input type="checkbox" checked={r.shown} onchange={() => toggle(i)} /><span></span></label>
      </li>
    {/each}
  </ol>
  <p class="muted small">Wiktionary is CC BY-SA 4.0, WordNet is CC BY 4.0; Webster 1913 and the Moby thesaurus are public domain. Barron's material stays private on your devices.</p>
</section>


<section class="card rise" style="--i:2">
  <h3>Sync server</h3>
  {#if status.paired}
    <p>Paired with <b>{status.name}</b>. {status.pending ?? 0} change{status.pending === 1 ? '' : 's'} waiting.
      {#if status.last_sync}<span class="muted">Last sync {when(status.last_sync.at)}.</span>{/if}</p>
    <div class="row">
      <button class="btn primary" disabled={busy} onclick={syncNow}>{busy ? 'Syncing…' : 'Sync now'}</button>
      <button class="btn ghost" onclick={unpair}>Unpair</button>
    </div>
    <label class="f">Home Wi-Fi names (use the fast local port on these)
      <div class="row"><input type="text" bind:value={ssids} /><button class="btn small" onclick={saveSsids}>Save</button></div>
    </label>
  {:else}
    <p class="muted">{status.auto === false
      ? 'Automatic pairing is off because you unpaired this device.'
      : 'This device pairs itself with your server as soon as it is on the home Wi-Fi.'}</p>
    <button class="btn primary" disabled={busy} onclick={autoPair}>{busy ? 'Looking…' : 'Pair now'}</button>
    <details class="alt">
      <summary>Pair with a code instead</summary>
      <p class="muted">On the server run <code>gredict-syncd pair {'<device name>'}</code> and paste the code it prints.</p>
      <label class="f">Pairing code
        <textarea rows="3" bind:value={code} placeholder="lexis1:…" spellcheck="false"></textarea></label>
      <label class="f">Home Wi-Fi names
        <input type="text" bind:value={ssids} /></label>
      <button class="btn" disabled={busy || !code.trim()} onclick={pair}>Pair with code</button>
    </details>
  {/if}
  {#if msg}<p class="msg">{msg}</p>{/if}
</section>

<PackManager />
<AiCard />

<section class="card rise" style="--i:4">
  <h3>Merriam-Webster</h3>
  <p class="muted small">
    Optional. Uses Merriam-Webster's free API for personal, non-commercial use (1,000 lookups a day per reference). Get free keys for
    <i>Collegiate® Dictionary</i> and <i>Thesaurus</i> at dictionaryapi.com. A word is looked up only when you open it in the Merriam-Webster tab;
    nothing is fetched in bulk.
  </p>
  <label class="f">Collegiate® Dictionary key
    <input type="password" bind:value={mwKey} placeholder={mw.configured ? '•••••••• (saved)' : 'paste key'} autocomplete="off" /></label>
  <label class="f">Thesaurus key (optional)
    <input type="password" bind:value={mwThes} placeholder={mw.thesaurus ? '•••••••• (saved)' : 'paste key'} autocomplete="off" /></label>
  <div class="row">
    <button class="btn primary" disabled={!mwKey && !mwThes} onclick={() => saveMw()}>Save keys</button>
    {#if mw.configured}<button class="btn ghost" onclick={() => saveMw({ collegiate: '', thesaurus: '' })}>Remove keys</button>{/if}
  </div>
  {#if mw.configured}
    <p class="muted small">Used today: {mw.used_today} of {mw.limit}. {mw.cache_entries} entr{mw.cache_entries === 1 ? 'y' : 'ies'} saved on this device.</p>
    <label class="chk"><input type="checkbox" checked={mw.keep_offline} onchange={(e) => saveMw({ keep_offline: (e.target as HTMLInputElement).checked })} />
      Keep words I look up for offline use</label>
    {#if mw.cache_entries}<button class="btn small ghost" onclick={() => saveMw({ clear_cache: true })}>Clear saved entries</button>{/if}
  {/if}
  {#if mwMsg}<p class="msg small">{mwMsg}</p>{/if}
</section>

<section class="card rise" style="--i:6">
  <h3>Pronunciation</h3>
  <p class="muted small">Recordings play from this device once downloaded; until then the app speaks the word. Download the Barron's words so they work without internet.</p>
  <div class="row">
    {#if dl.running}
      <button class="btn" onclick={() => (stopDl = true)}>Stop</button>
    {:else}
      <button class="btn primary" onclick={downloadAudio}>Download Barron's pronunciations</button>
    {/if}
    {#if dl.total}<span class="muted">{dl.done} / {dl.total}</span>{/if}
  </div>
  {#if dl.total}<progress max={dl.total} value={dl.done} style="width:100%;margin-top:8px"></progress>{/if}
  {#if dl.note}<p class="muted small">{dl.note}</p>{/if}
</section>

<section class="card rise" style="--i:7">
  <h3>Appearance</h3>
  <div class="seg" role="radiogroup" aria-label="Theme">
    {#each ['system', 'light', 'dark'] as t}
      <button role="radio" aria-checked={app.theme === t} class:on={app.theme === t} onclick={() => setTheme(t as 'system')}>{t}</button>
    {/each}
  </div>
</section>

<style>
  h1 { margin: 0 0 12px; font-size: 28px; }
  h3 { margin: 0 0 8px; font-size: 14px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); }
  section { margin: 14px 0; }
  .seg { display: inline-flex; border: 1px solid var(--line); border-radius: 999px; overflow: hidden; }
  .seg button { background: var(--bg); border: 0; padding: 8px 18px; text-transform: capitalize; }
  .seg button.on { background: var(--blue); color: #fff; }
  .small { font-size: 13px; }
  .row { display: flex; gap: 10px; align-items: center; flex-wrap: wrap; }
  .f { display: flex; flex-direction: column; gap: 4px; margin: 12px 0; font-size: 13px; color: var(--muted); }
  .f input, .f textarea { font-size: 15px; }
  code { background: var(--panel-2); border-radius: 4px; padding: 1px 6px; font-size: 0.85em; overflow-wrap: anywhere; }
  .msg { margin-top: 10px; }
  .alt { margin-top: 14px; }
  .alt summary { color: var(--blue); padding: 6px 0; }
  .chk { display: flex; gap: 8px; align-items: center; margin: 8px 0; }
  button:disabled { opacity: 0.35; }
  .order { list-style: none; margin: 10px 0; padding: 0; }
  .order li { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 10px; padding: 8px 4px; border-top: 1px solid var(--line); }
  .order li:first-child { border-top: 0; }
  .order li.off .nm { opacity: 0.45; }
  .n { width: 26px; height: 26px; border-radius: 50%; background: var(--blue-soft); color: var(--blue); display: grid; place-items: center; font-weight: 700; font-size: 13px; flex: none; }
  .nm { flex: 1 1 150px; display: flex; flex-direction: column; min-width: 0; }
  .mv { width: 40px; height: 40px; border-radius: 50%; border: 1px solid var(--line); background: var(--surface); color: var(--blue); display: grid; place-items: center; box-shadow: 0 1px 3px rgba(16, 40, 80, 0.08); }
  .nm + .mv { margin-left: auto; }
  .mv:hover:not(:disabled) { background: var(--blue-soft); }
  .sw { position: relative; width: 44px; height: 26px; flex: none; }
  .sw input { position: absolute; inset: 0; opacity: 0; margin: 0; cursor: pointer; z-index: 1; }
  .sw span { position: absolute; inset: 0; border-radius: 999px; background: var(--panel-2); transition: background 0.15s; }
  .sw span::after { content: ''; position: absolute; top: 3px; left: 3px; width: 20px; height: 20px; border-radius: 50%; background: #fff; box-shadow: 0 1px 3px rgba(0,0,0,0.3); transition: transform 0.15s; }
  .sw input:checked + span { background: var(--blue); }
  .sw input:checked + span::after { transform: translateX(18px); }
</style>
