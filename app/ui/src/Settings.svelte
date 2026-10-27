<script lang="ts">
  import { call, native } from './lib/api';
  import { app, setTheme, bump, orderedSources, saveOrder, audioDl, startAudioDownload, stopAudioDownload } from './lib/store.svelte';
  import Icon from './Icon.svelte';
  import PackManager from './PackManager.svelte';
  import Update from './Update.svelte';
  import AiCard from './AiCard.svelte';
  import { flip } from 'svelte/animate';
  import { flipDur } from './lib/motion';
  import { when } from './lib/format';

  let status = $state<any>({ paired: false });
  let version = $state('');
  $effect(() => { native<string>('app_version').then((v) => (version = v ?? 'web')).catch(() => (version = '')); });
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

  // Devices paired with the server: a stale or lost one can be removed from here.
  let devs = $state<{ me: string; devices: { name: string; created: number; seen: number; role?: string }[] } | null>(null);
  let devErr = $state('');
  async function loadDevices() {
    devErr = '';
    try { devs = await call('devices_list'); } catch { devs = null; devErr = 'Connect to the server to see the paired devices.'; }
  }
  async function removeDevice(name: string) {
    if (!confirm(`Remove ${name}? It will stop syncing until it is paired again. If it is a guest, its chats and Claude sign-in on the server are deleted.`)) return;
    try { await call('device_revoke', { name }); await loadDevices(); } catch (e) { devErr = (e as Error).message; }
  }
  // Who this device is on the server: the owner's (after the owner password, entered once) or a guest, which sees none of
  // the owner's data and chats on its own Claude account.
  let who = $state<{ role: 'owner' | 'guest'; password_set: boolean } | null>(null);
  let pw = $state('');
  let pwOld = $state('');
  let pwMsg = $state('');
  let pwBusy = $state(false);
  async function loadWho() { who = status.paired ? await call('owner_status') : null; }
  $effect(() => { void app.version; void status.paired; loadWho().catch(() => { who = null; }); });
  async function enterPassword() {
    pwBusy = true; pwMsg = '';
    try { who = await call('owner_login', { password: pw }); pw = ''; pwMsg = 'Welcome back. This device is now yours; your notes and history will sync.'; bump(); call('sync_now').catch(() => {}); }
    catch (e) { pwMsg = (e as Error).message; }
    pwBusy = false;
  }
  async function savePassword() {
    pwBusy = true; pwMsg = '';
    try {
      const changed = who?.password_set;
      who = await call('owner_set', { password: pw, old: pwOld }); pw = ''; pwOld = '';
      pwMsg = changed ? 'Password changed. Your other devices ask for the new one once.' : 'Password set. A new device asks for it once, in Settings.';
    } catch (e) { pwMsg = (e as Error).message; }
    pwBusy = false;
  }

  // Claude account (guests): sign in with a link and the code Claude shows, or paste a token. Kept on the server, never shown again.
  let cl = $state<{ signed_in: boolean; since: number | null } | null>(null);
  let clLink = $state('');
  let clCode = $state('');
  let clToken = $state('');
  let clMsg = $state('');
  let clBusy = $state(false);
  async function loadCl() { cl = status.paired && who?.role === 'guest' ? await call('claude_login_status') : null; }
  $effect(() => { void app.version; void who?.role; loadCl().catch(() => { cl = null; }); });
  async function startLink() {
    clBusy = true; clMsg = 'Asking Claude for a sign-in link…';
    try { clLink = (await call('claude_link_start')).url; clCode = ''; clMsg = ''; }
    catch (e) { clMsg = (e as Error).message; }
    clBusy = false;
  }
  async function copyLink() { try { await navigator.clipboard.writeText(clLink); clMsg = 'Link copied. Open it in your browser.'; } catch { clMsg = 'Select the link and copy it.'; } }
  async function finishLink() {
    clBusy = true; clMsg = 'Checking the code with Claude…';
    try { cl = await call('claude_link_finish', { code: clCode }); clLink = ''; clCode = ''; clMsg = 'Signed in. Your chats now use your own Claude subscription.'; }
    catch (e) { clMsg = (e as Error).message; }
    clBusy = false;
  }
  async function saveCl() {
    clBusy = true; clMsg = 'Checking the token with Claude…';
    try { cl = await call('claude_login_set', { token: clToken }); clToken = ''; clMsg = 'Signed in. Your chats now use your own Claude subscription.'; }
    catch (e) { clMsg = (e as Error).message; }
    clBusy = false;
  }
  async function clearCl() {
    if (!confirm('Sign out of Claude on the server? Your chats stop until you sign in again.')) return;
    try { cl = await call('claude_login_clear'); clLink = ''; clMsg = 'Signed out.'; } catch (e) { clMsg = (e as Error).message; }
  }
  async function unpair() { await call('sync_unpair'); await load(); bump(); msg = 'Unpaired. Your data stays on this device.'; }
  async function saveSsids() { await call('sync_set_home_ssids', { ssids }); msg = 'Saved.'; }
</script>


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
    <details class="alt" ontoggle={(e) => { if ((e.currentTarget as HTMLDetailsElement).open) loadDevices(); }}>
      <summary>Paired devices</summary>
      {#if devs}
        <ul class="devs">
          {#each devs.devices as d (d.name)}
            <li>
              <span class="nm"><b>{d.name}</b>{#if d.name === devs.me} <span class="muted small">(this device)</span>{/if}{#if d.role === 'guest'} <span class="muted small">(guest)</span>{/if}
                <span class="muted small">{d.seen ? `last seen ${when(d.seen)}` : 'not seen since this list began'}</span></span>
              {#if d.name !== devs.me}<button class="btn small ghost" onclick={() => removeDevice(d.name)}>Remove</button>{/if}
            </li>
          {/each}
        </ul>
      {:else}
        <p class="muted small">{devErr || 'Loading…'}</p>
      {/if}
      {#if devErr && devs}<p class="muted small">{devErr}</p>{/if}
    </details>
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

{#if status.paired && who}
  <section class="card rise" style="--i:4">
    <h3>Owner password</h3>
    {#if who.role === 'owner'}
      <p class="muted small">This is the owner's device: it syncs your notes, history and chats, and chats use the server's own Claude. Other people who pair a device are guests: they see none of this.</p>
      <details class="alt">
        <summary>{who.password_set ? 'Change the owner password' : 'Set the owner password'}</summary>
        <p class="muted small">{who.password_set
          ? 'Your other devices will ask for the new password once.'
          : 'A new device of yours asks for this password once, in Settings, to be recognised as yours. Until you set one, a new device stays a guest.'}</p>
        {#if who.password_set}<label class="f">Current password
          <input type="password" bind:value={pwOld} autocomplete="current-password" /></label>{/if}
        <label class="f">{who.password_set ? 'New password' : 'Password'} (at least 8 characters)
          <input type="password" bind:value={pw} autocomplete="new-password" /></label>
        <button class="btn" disabled={pwBusy || pw.length < 8 || (who.password_set && !pwOld)} onclick={savePassword}>{who.password_set ? 'Change password' : 'Set password'}</button>
      </details>
    {:else}
      <p class="muted small">This device is a <b>guest</b>: the owner's notes, history and Barron's never reach it, and your own notes stay on this device. If it is the owner's, enter the owner password once.</p>
      {#if who.password_set}
        <label class="f">Owner password
          <input type="password" bind:value={pw} autocomplete="current-password" onkeydown={(e) => { if (e.key === 'Enter' && pw) enterPassword(); }} /></label>
        <button class="btn primary" disabled={pwBusy || !pw} onclick={enterPassword}>{pwBusy ? 'Checking…' : 'This is my device'}</button>
      {:else}
        <p class="muted small">The owner has not set a password yet (they do it from their own device).</p>
      {/if}
    {/if}
    {#if pwMsg}<p class="msg small">{pwMsg}</p>{/if}
  </section>
{/if}

{#if status.paired && who?.role === 'guest'}
  <section class="card rise" style="--i:5">
    <h3>Claude account</h3>
    <p class="muted small">
      Chats are answered by <b>your own</b> Claude subscription, so you must sign in to Claude here. The sign-in is kept private on the
      server and never shown again. Your chats cannot read any file on the server or change the app.
    </p>
    {#if cl?.signed_in}
      <p>Signed in to Claude{cl.since ? ` since ${when(cl.since)}` : ''}.</p>
    {:else}
      <p class="muted small">You are not signed in, so Claude will not answer yet.</p>
    {/if}
    {#if clLink}
      <ol class="steps small">
        <li>Open <a href={clLink} target="_blank" rel="noopener">Claude's sign-in page</a> (or <button class="btn small ghost" onclick={copyLink}>copy the link</button>) and sign in to your Claude account.</li>
        <li>Claude then shows a code. Paste it here.</li>
      </ol>
      <label class="f">Code from Claude
        <input type="text" bind:value={clCode} autocomplete="off" spellcheck="false" placeholder="paste the code" /></label>
      <div class="row">
        <button class="btn primary" disabled={clBusy || !clCode.trim()} onclick={finishLink}>{clBusy ? 'Checking…' : 'Finish sign-in'}</button>
        <button class="btn ghost" disabled={clBusy} onclick={() => { clLink = ''; clMsg = ''; }}>Cancel</button>
      </div>
    {:else}
      <div class="row">
        <button class="btn primary" disabled={clBusy} onclick={startLink}>{clBusy ? 'Please wait…' : cl?.signed_in ? 'Sign in again' : 'Sign in with Claude'}</button>
        {#if cl?.signed_in}<button class="btn ghost" disabled={clBusy} onclick={clearCl}>Sign out</button>{/if}
      </div>
    {/if}
    <details class="alt">
      <summary>Or paste a token instead</summary>
      <p class="muted small">On a computer where you are signed in to Claude, run <code>claude setup-token</code> and paste the token it prints (it starts with <code>sk-ant-</code>).</p>
      <label class="f">Claude token
        <input type="password" bind:value={clToken} placeholder="sk-ant-…" autocomplete="off" spellcheck="false" /></label>
      <button class="btn" disabled={clBusy || !clToken.trim()} onclick={saveCl}>{clBusy ? 'Checking…' : 'Sign in with the token'}</button>
    </details>
    {#if clMsg}<p class="msg small">{clMsg}</p>{/if}
  </section>
{/if}

<section class="card rise" style="--i:6">
  <h3>About</h3>
  <p>LadyBird {version ? `version ${version}` : ''}</p>
  <p class="muted small">Offline GRE dictionary, thesaurus and quizzes.</p>
  <Update bind:version />
</section>

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
    {#if audioDl.running}
      <button class="btn" onclick={stopAudioDownload}>Stop</button>
    {:else}
      <button class="btn primary" onclick={startAudioDownload}>Download Barron's pronunciations</button>
    {/if}
    {#if audioDl.total}<span class="muted">{audioDl.done} / {audioDl.total}</span>{/if}
  </div>
  {#if audioDl.total}<progress max={audioDl.total} value={audioDl.done} style="width:100%;margin-top:8px"></progress>{/if}
  {#if audioDl.note}<p class="muted small">{audioDl.note}</p>{/if}
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
  h3 { margin: 0 0 8px; font-size: 14px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); }
  section { margin: 14px 0; }
  .seg { display: inline-flex; border: 1px solid var(--line); border-radius: 999px; overflow: hidden; }
  .seg button { background: var(--bg); border: 0; padding: 8px 18px; text-transform: capitalize; }
  .seg button.on { background: var(--blue); color: #fff; }
  .small { font-size: 13px; }
  .devs { list-style: none; margin: 8px 0 0; padding: 0; }
  .devs li { display: flex; align-items: center; gap: 10px; padding: 8px 0; border-top: 1px solid var(--line); }
  .devs .nm { flex: 1; min-width: 0; display: flex; flex-direction: column; overflow-wrap: anywhere; }
  .row { display: flex; gap: 10px; align-items: center; flex-wrap: wrap; }
  .f { display: flex; flex-direction: column; gap: 4px; margin: 12px 0; font-size: 13px; color: var(--muted); }
  .f input, .f textarea { font-size: 15px; }
  code { background: var(--panel-2); border-radius: 4px; padding: 1px 6px; font-size: 0.85em; overflow-wrap: anywhere; }
  .msg { margin-top: 10px; }
  .steps { margin: 8px 0; padding-left: 20px; }
  .steps li { margin: 6px 0; }
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
