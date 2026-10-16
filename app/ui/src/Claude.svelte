<script lang="ts">
  import { onMount, tick } from 'svelte';
  import Icon from './Icon.svelte';
  import { call } from './lib/api';
  import { shrink } from './lib/image';
  import { markdown, diffRows } from './lib/markdown';
  import { app, bump, openDetail, closeDetail } from './lib/store.svelte';

  type Idea = { title: string; detail: string; status: 'proposed' | 'approved' | 'dismissed' };
  type Step = { kind: string; text: string; at?: number };
  type Msg = { role: 'user' | 'claude' | 'error' | 'activity'; steps?: Step[]; text: string; at: number; idea?: Idea; photos?: string[]; offer?: string; queued?: boolean };
  type Chat = { id: string; title: string; state: string; updated: number; last: string; mode: string };
  let chats = $state<Chat[]>([]);
  let cur = $state<string | null>(null);
  let msgs = $state<Msg[]>([]);
  let phase = $state('');
  let mode = $state('ask');
  let branch = $state('');
  let changes = $state('');
  let since = $state(0);
  let started = $state(0);
  let now = $state(Date.now());
  let msg = $state('');
  let error = $state('');
  let sending = $state(false);
  // Photos attached to the next message (shrunk data URLs) and the thumbnails of sent ones, loaded by name.
  let attached = $state<string[]>([]);
  let thumbs = $state<Record<string, string>>({});
  async function attach(e: Event) {
    const input = e.target as HTMLInputElement;
    try { for (const f of Array.from(input.files ?? [])) if (attached.length < 4) attached.push(await shrink(f)); }
    catch { error = 'That photo could not be read. Try a JPEG or PNG.'; }
    input.value = '';
  }
  $effect(() => {
    for (const m of msgs) for (const n of m.photos ?? []) {
      if (n in thumbs) continue;
      thumbs[n] = '';
      call('photo_data', { name: n }).then((r) => (thumbs[n] = r.data_url)).catch(() => delete thumbs[n]);
    }
  });
  // The model and effort for the next message ('' = the CLI's own default), remembered on this device.
  const MODELS = [['', 'Default model'], ['fable', 'Fable'], ['opus', 'Opus'], ['sonnet', 'Sonnet'], ['haiku', 'Haiku']];
  const EFFORTS = [['', 'Default effort'], ['low', 'Low'], ['medium', 'Medium'], ['high', 'High'], ['xhigh', 'Extra high'], ['max', 'Max']];
  const saved = (k: string, d = '') => { try { return localStorage.getItem(k) ?? d; } catch { return d; } };
  // Sonnet unless this device chose otherwise (key v2: earlier devices stored '' without choosing).
  let model = $state(saved('claude.model2', 'sonnet'));
  let effort = $state(saved('claude.effort'));
  $effect(() => { try { localStorage.setItem('claude.model2', model); localStorage.setItem('claude.effort', effort); } catch { /* private mode */ } });
  let end = $state<HTMLElement | null>(null);
  let scroller = $state<HTMLElement | null>(null);
  // The chat fills what is left of the screen below the top bar, and follows the keyboard.
  let vh = $state(window.visualViewport?.height ?? window.innerHeight);
  let top = $state(120);
  function measure() {
    vh = window.visualViewport?.height ?? window.innerHeight;
    top = Math.round(document.querySelector('.top')?.getBoundingClientRect().bottom ?? 120);
  }
  $effect(() => { app.focus = cur !== null; measure(); return () => { app.focus = false; }; });
  const waiting = $derived(['asked', 'running', 'releasing', 'merge', 'discard', 'release_patch', 'release_minor'].includes(phase));
  // Nobody answered for a minute (the server away or out of date): offer to ask again.
  // Merging and releasing are slow by nature; a question or a change should show progress within minutes.
  const staleAfter = (state: string) => (state === 'asked' || state === 'running' ? 90_000 : 30 * 60_000);
  const stuck = $derived(waiting && now - since > staleAfter(phase));
  // While Claude answers, new messages are queued (and can be edited or removed) instead of blocked.
  const answering = $derived(phase === 'asked' || phase === 'running');
  const queued = $derived(msgs.filter((m) => m.queued));
  let editing = $state<number | null>(null);
  let editText = $state('');
  async function saveEdit(m: Msg) {
    try { const t = await call('talk_queue_edit', { id: cur, at: m.at, message: editText }); msgs = t.msgs; editing = null; call('sync_now').catch(() => {}); } catch (e) { error = (e as Error).message; }
  }
  async function unqueue(m: Msg) {
    try { const t = await call('talk_queue_remove', { id: cur, at: m.at }); msgs = t.msgs; call('sync_now').catch(() => {}); } catch (e) { error = (e as Error).message; }
  }
  async function sendNow() {
    try { const t = await call('talk_next', { id: cur }); msgs = t.msgs; phase = t.state; started = t.started; since = t.updated; call('sync_now').catch(() => {}); } catch (e) { error = (e as Error).message; }
  }
  // How long a step took (or has been running): each step starts when the previous one was reported.
  const took = (steps: Step[], i: number) => { const a = steps[i]?.at; return a ? clock((steps[i + 1]?.at ?? now) - a) : ''; };
  const ICON: Record<string, string> = { read: 'file', edit: 'notes', search: 'search', run: 'terminal', say: 'chat' };
  // What Claude is doing right now, as the server reports it.
  const live = $derived([...msgs].reverse().find((m: Msg) => m.role === 'activity')?.steps ?? []);
  const LABEL: Record<string, [string, string]> = { read: ['read', 'reads'], edit: ['edit', 'edits'], search: ['search', 'searches'], run: ['command', 'commands'], say: ['note', 'notes'] };
  // "2 reads, 1 edit, 3 commands": what a finished run did, at a glance.
  function tally(steps: Step[]) {
    const n: Record<string, number> = {};
    for (const s of steps) n[s.kind] = (n[s.kind] ?? 0) + 1;
    return Object.entries(n).filter(([k]) => k !== 'say').map(([k, c]) => `${c} ${(LABEL[k] ?? [k, k])[c === 1 ? 0 : 1]}`).join(', ');
  }
  const clock = (ms: number) => { const s = Math.max(0, Math.floor(ms / 1000)); return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, '0')}s`; };
  const hhmm = (t: number) => new Date(t).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  let copied = $state(0);
  async function copy(m: Msg) { try { await navigator.clipboard.writeText(m.text); copied = m.at; setTimeout(() => (copied = 0), 1500); } catch { /* no clipboard */ } }
  let allSteps = $state(false);
  const SUGGEST = ['How does the quiz pick words?', 'The History tab feels slow', 'Add a dark-mode toggle to Settings'];
  const approved = $derived(msgs.filter((m) => m.idea?.status === 'approved'));

  async function loadList() { chats = await call('talk_list'); }
  async function load() {
    await loadList();
    if (!cur) return;
    const t = await call('talk_get', { id: cur });
    if (!chats.some((c) => c.id === cur)) { cur = null; return; }
    const grew = t.msgs.length !== msgs.length;
    msgs = t.msgs; phase = t.state; mode = t.mode; branch = t.branch; changes = t.changes; since = t.updated; started = t.started;
    if (grew) { await tick(); end?.scrollIntoView({ block: 'end' }); }
  }
  $effect(() => { void app.version; void cur; load().catch((e) => (error = e.message)); });
  onMount(() => {
    window.visualViewport?.addEventListener('resize', measure);
    window.addEventListener('resize', measure);
    // The server answers; asking it for news brings the reply without waiting for the next background sync.
    const tick1 = setInterval(() => { now = Date.now(); }, 1000);
    const t = setInterval(async () => {
      if (!waiting && !chats.some((c) => c.state)) return;
      await call('sync_now').catch(() => {});
      load().catch(() => {});
    }, 3000);
    return () => { clearInterval(t); clearInterval(tick1); window.visualViewport?.removeEventListener('resize', measure); window.removeEventListener('resize', measure); };
  });

  async function send() {
    const m = msg.trim();
    if ((!m && !attached.length) || (waiting && !answering) || sending) return;
    sending = true; error = '';
    try { const t = await call('talk_send', { id: cur, message: m, photos: attached, ...(model && { model }), ...(effort && { effort }) }); msgs = t.msgs; phase = t.state; started = t.started; since = t.updated; msg = ''; attached = []; call('sync_now').catch(() => {}); bump(); await tick(); end?.scrollIntoView({ block: 'end' }); }
    catch (e) { error = (e as Error).message; }
    sending = false;
  }
  function sendKey(e: KeyboardEvent) { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); send(); } }
  async function decide(m: Msg, decision: string) {
    try { const t = await call('talk_idea', { id: cur, at: m.at, decision }); msgs = t.msgs; call('sync_now').catch(() => {}); } catch (e) { error = (e as Error).message; }
  }
  // Back from a chat returns to the list.
  function openChat(id: string) { msgs = []; phase = ''; cur = id; openDetail(() => (cur = null)); }
  async function newChat(m = 'work') { try { const t = await call('talk_new', { mode: m }); await loadList(); openChat(t.id); } catch (e) { error = (e as Error).message; } }
  async function removeChat() {
    if (!cur) return;
    try { await call('talk_delete', { id: cur }); call('sync_now').catch(() => {}); closeDetail(); await loadList(); } catch (e) { error = (e as Error).message; }
  }
  async function retry() {
    try { const t = await call('talk_retry', { id: cur }); phase = t.state; since = t.updated; call('sync_now').catch(() => {}); } catch (e) { error = (e as Error).message; }
  }
  async function stop(id: string | null = cur) {
    try { const t = await call('talk_stop', { id }); if (id === cur) { msgs = t.msgs; phase = t.state; } call('sync_now').catch(() => {}); await loadList(); } catch (e) { error = (e as Error).message; }
  }
  let sure = $state(false);
  async function finish(how: 'merge' | 'discard' | 'release_patch' | 'release_minor') {
    if (how === 'discard' && !sure) { sure = true; return; }
    sure = false;
    try { const t = await call('talk_finish', { id: cur, how }); phase = t.state; call('sync_now').catch(() => {}); } catch (e) { error = (e as Error).message; }
  }
  let askDelete = $state('');
  // Two taps in the list: the first asks, the second deletes.
  async function removeFromList(id: string) {
    if (askDelete !== id) { askDelete = id; return; }
    askDelete = '';
    try { await call('talk_delete', { id }); call('sync_now').catch(() => {}); await loadList(); } catch (e) { error = (e as Error).message; }
  }
  const STATE: Record<string, string> = { asked: 'Waiting for the server…', running: 'Claude is answering…', merge: 'Merging…', discard: 'Discarding…', releasing: 'Releasing…', release_patch: 'Release queued…', release_minor: 'Release queued…' };
  const ago = (t: number) => {
    const m = Math.round((now - t) / 60000);
    if (m < 1) return 'just now';
    if (m < 60) return `${m} min ago`;
    if (m < 60 * 24) return `${Math.round(m / 60)} h ago`;
    return new Date(t).toLocaleDateString();
  };
</script>

{#if cur === null}
<h1>Claude</h1>
<p class="muted">Ask about the app or ask for a change. When you ask for a change Claude makes it on its own copy of the app; you read what changed and press Merge, then Release to build and publish the update. Nothing reaches the app without your buttons. Each message uses one agent call. Older read-only chats stay read-only: start a new chat to change the app.</p>
<p class="row2"><button class="btn primary" onclick={() => newChat('work')}>New chat</button></p>
{#if error}<p class="err">{error}</p>{/if}
{#each chats as c (c.id)}
  <div class="card item">
    <button class="row" onclick={() => openChat(c.id)}>
      <b>{c.title || 'New chat'}{#if c.mode !== 'work'} <span class="tag">Read-only</span>{/if}</b>
      <span class="muted small">{c.state ? STATE[c.state] ?? 'Claude is working…' : c.last}</span>
      <span class="muted small">{c.state ? `${clock(now - c.updated)} since the last sign of life` : ago(c.updated)}</span>
    </button>
    {#if c.state}
      <button class="btn small ghost" disabled={now - c.updated <= staleAfter(c.state)} title="Available once the chat stalls" onclick={() => stop(c.id)}>Stop</button>
    {:else}
      <button class="btn small ghost" onclick={() => removeFromList(c.id)}>{askDelete === c.id ? 'Sure?' : 'Delete'}</button>
    {/if}
  </div>
{:else}
  <p class="empty">No chats yet.</p>
{/each}
{:else}
<!-- A real chat screen: its own header, the messages scroll, the message box stays at the bottom above the keyboard. -->
<div class="pane" style="top:{top}px;height:{Math.max(220, vh - top)}px">
  <div class="chead">
    <button class="gbtn" onclick={closeDetail} aria-label="Back to chats"><Icon name="back" size={20} /></button>
    <b class="ct">{chats.find((c) => c.id === cur)?.title || 'New chat'}</b>
    {#if waiting}<button class="btn small ghost" onclick={() => (stuck ? stop() : (error = 'Claude is still working; you can stop waiting once it stalls.'))}>{stuck ? 'Stop' : 'Working…'}</button>{:else}<button class="btn small ghost" onclick={removeChat}>Delete</button>{/if}
  </div>
  <div class="msgs" aria-live="polite" bind:this={scroller}>
    {#if approved.length}
      <details class="card ok">
        <summary>Approved ideas ({approved.length})</summary>
        {#each approved as m (m.at)}
          <div class="idea"><b>{m.idea?.title}</b><p>{m.idea?.detail}</p>
            <button class="btn small ghost" onclick={() => decide(m, 'dismissed')}>Remove</button></div>
        {/each}
      </details>
    {/if}
    {#each msgs as m (m.at + m.role)}
      {#if m.role === 'activity'}
      {:else if m.role === 'user'}<div class="m me" class:queued={m.queued}>{#each m.photos ?? [] as n}{#if thumbs[n]}<img class="th" src={thumbs[n]} alt="Attached" />{:else}<span class="th ph">…</span>{/if}{/each}
        {#if editing === m.at}<textarea class="qedit" rows="2" bind:value={editText} onkeydown={(e) => { if (e.key === 'Escape') editing = null; }}></textarea>
          <span class="qrow"><button class="lnk" onclick={() => saveEdit(m)}>Save</button><button class="lnk" onclick={() => (editing = null)}>Cancel</button></span>
        {:else}{m.text}<time>{m.queued ? 'Queued · goes out after this answer' : hhmm(m.at)}</time>
          {#if m.queued}<span class="qrow"><button class="lnk" onclick={() => { editing = m.at; editText = m.text; }}>Edit</button><button class="lnk" onclick={() => unqueue(m)}>Remove</button></span>{/if}{/if}</div>
      {:else if m.role === 'error'}<div class="m bad">{m.text}</div>
      {:else}
        <div class="m ai">{#if m.steps?.length}<details class="steps"><summary>Worked through {m.steps.length} step{m.steps.length === 1 ? '' : 's'}{#if tally(m.steps)} <span class="muted">· {tally(m.steps)}</span>{/if}</summary>
            {#each m.steps as st}<div class="step {st.kind}"><span class="ic"><Icon name={ICON[st.kind] ?? 'minus'} size={16} /></span>{st.text}</div>{/each}</details>{/if}<div class="md">{@html markdown(m.text)}</div>
          <div class="meta"><time>{hhmm(m.at)}</time><button class="lnk" onclick={() => copy(m)}>{copied === m.at ? 'Copied' : 'Copy'}</button></div>
          {#if m.idea}
            <div class="idea card">
              <b>{m.idea.title}</b>
              <p>{m.idea.detail}</p>
              {#if m.idea.status === 'proposed'}
                <button class="btn small primary" onclick={() => decide(m, 'approved')}>Approve</button>
                <button class="btn small ghost" onclick={() => decide(m, 'dismissed')}>Dismiss</button>
              {:else}<span class="muted small">{m.idea.status === 'approved' ? 'Approved' : 'Dismissed'}</span>{/if}
            </div>
          {/if}
        </div>
      {/if}
    {:else}
      <div class="empty"><p>Ask a question or describe a change.</p>{#each SUGGEST as q}<button class="chip" onclick={() => (msg = q)}>{q}</button>{/each}</div>
    {/each}
    {#if waiting}<div class="m ai wait" role="status">
      <div class="status"><span class="spin" aria-hidden="true"></span><span>{phase === 'merge' ? 'Merging…' : phase === 'discard' ? 'Discarding…' : phase.startsWith('release') ? 'Building and publishing the release' : mode === 'work' ? 'Claude is working' : 'Claude is thinking'}</span>
        <span class="muted small">{clock(now - (started || now))}{#if live.length} · {live.length} step{live.length === 1 ? '' : 's'}{/if}</span></div>
      {#if live.length}<div class="small now">Now: {live.at(-1)?.text}{#if live.at(-1)?.at} <span class="muted">({took(live, live.length - 1)})</span>{/if}</div>
        {#if tally(live)}<div class="small muted">So far: {tally(live)}</div>{/if}{/if}
      {#if phase.startsWith('release')}<p class="small">This takes many minutes; it is safe to leave this screen.</p>{/if}
      {#if live.length}<div class="steps live">{#each (allSteps ? live : live.slice(-6)) as st, i}{@const shown = allSteps ? live : live.slice(-6)}<div class="step {st.kind}" class:cur={i === shown.length - 1}><span class="ic"><Icon name={ICON[st.kind] ?? 'minus'} size={16} /></span><span class="tx">{st.text}</span>{#if st.at}<span class="dur">{took(live, live.length - shown.length + i)}</span>{/if}</div>{/each}
        {#if live.length > 6}<button class="lnk" onclick={() => (allSteps = !allSteps)}>{allSteps ? 'Show fewer' : `Show all ${live.length}`}</button>{/if}</div>{/if}
      {#if stuck}<div class="small stuckrow">{phase === 'asked' || phase === 'running' ? 'No progress for a while. ' : 'Taking very long. '}{#if phase === 'asked' || phase === 'running'}<button class="btn small" onclick={retry}>Try again</button>{/if}<button class="btn small ghost" onclick={() => stop()}>Stop waiting</button></div>{:else if phase === 'asked'}<span class="muted small">Waiting for the server to pick it up…</span>{/if}</div>{/if}
    {#if queued.length && !waiting}
      <div class="card work">
        <b>{queued.length} message{queued.length === 1 ? '' : 's'} waiting</b>
        <p class="muted small">Nothing is running (the last run was stopped or ended early). Send the first one now, or edit or remove them above.</p>
        <button class="btn small primary" onclick={sendNow}>Send now</button>
      </div>
    {/if}
    {#if branch && !waiting}
      <div class="card work">
        <b>Changes ready</b>
        {#each diffRows(changes) as r}<div class="file"><code>{r.file}</code><span class="bar"><i class="add" style="flex:{r.add}"></i><i class="del" style="flex:{r.del}"></i></span><span class="muted small">{r.n}</span></div>{:else}<pre>{changes || 'No files changed.'}</pre>{/each}
        <button class="btn small primary" disabled={!changes} onclick={() => finish('merge')}>Merge into the app</button>
        <button class="btn small ghost" onclick={() => finish('discard')}>{sure ? 'Really throw the changes away?' : 'Discard'}</button>
      </div>
    {/if}
    {#if !waiting && msgs.at(-1)?.offer === 'release'}
      <div class="card work">
        <b>Publish this update?</b>
        <p class="muted small">Runs the tests, builds the apps and publishes a GitHub release. Fix = small change; New feature = visible change.</p>
        <button class="btn small primary" onclick={() => finish('release_patch')}>Release as a fix</button>
        <button class="btn small" onclick={() => finish('release_minor')}>Release as a new feature</button>
      </div>
    {/if}
    <span bind:this={end}></span>
  </div>
  {#if error}<p class="err small">{error}</p>{/if}
  {#if attached.length}
    <div class="pick">
      {#each attached as a, i}<span class="at"><img class="th" src={a} alt="To send" /><button class="x" aria-label="Remove photo" onclick={() => attached.splice(i, 1)}><Icon name="close" size={14} /></button></span>{/each}
    </div>
  {/if}
  <div class="pick">
    <select bind:value={model} aria-label="Model">{#each MODELS as [v, l]}<option value={v}>{l}</option>{/each}</select>
    <select bind:value={effort} aria-label="Effort">{#each EFFORTS as [v, l]}<option value={v}>{l}</option>{/each}</select>
  </div>
  <div class="send">
    <label class="btn ghost" aria-label="Attach photos"><Icon name="attach" size={22} /><input type="file" accept="image/*" multiple onchange={attach} hidden /></label>
    <textarea rows="1" bind:value={msg} onkeydown={sendKey} onfocus={() => setTimeout(() => end?.scrollIntoView({ block: 'end' }), 250)} placeholder={answering ? 'Queue a message for after this answer' : mode === 'work' ? 'Ask a question or describe a change' : 'Message Claude'}></textarea>
    <button class="btn primary" disabled={(!msg.trim() && !attached.length) || (waiting && !answering) || sending} onclick={send}>{answering ? 'Queue' : 'Send'}</button>
  </div>
</div>
{/if}

<style>
  .row2 { display: flex; gap: 10px; flex-wrap: wrap; }
  .tag { background: var(--blue-soft); color: var(--blue); border-radius: 999px; padding: 0 8px; font-size: 12px; font-weight: 600; }
  .work { margin: 6px 0; }
  .work pre { white-space: pre-wrap; overflow-wrap: anywhere; font-size: 13px; margin: 6px 0 10px; }
  .item { display: flex; align-items: center; gap: 8px; margin: 8px 0; }
  .row { display: grid; gap: 2px; flex: 1; min-width: 0; text-align: left; background: none; border: 0; padding: 0; color: inherit; font: inherit; cursor: pointer; }
  h1 { margin: 0 0 4px; font-size: 28px; }
  .pane { position: fixed; left: 0; right: 0; z-index: 15; display: flex; flex-direction: column; background: var(--bg); }
  .chead { display: flex; align-items: center; gap: 8px; padding: 6px 12px; border-bottom: 1px solid var(--line); }
  .ct { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .gbtn { background: none; border: 0; font-size: 24px; width: 40px; height: 40px; color: var(--text); border-radius: 50%; }
  .msgs { flex: 1; overflow-y: auto; display: flex; flex-direction: column; gap: 8px; padding: 12px 14px; -webkit-overflow-scrolling: touch; }
  .m { max-width: 92%; padding: 8px 12px; border-radius: 14px; white-space: pre-wrap; overflow-wrap: anywhere; }
  .m.me { align-self: flex-end; background: var(--blue); color: #fff; border-bottom-right-radius: 4px; }
  .m.ai { align-self: flex-start; background: var(--panel); border-bottom-left-radius: 4px; }
  .m.bad { align-self: flex-start; background: var(--bad-bg); color: var(--bad); }
  .m.wait { color: var(--muted); font-style: italic; }
  .steps { font-style: normal; font-size: 0.82rem; color: var(--muted); margin: 0 0 6px; }
  .steps.live { margin: 8px 0 0; }
  .steps summary { cursor: pointer; }
  .step { display: flex; gap: 6px; padding: 2px 0; word-break: break-word; font-family: ui-monospace, monospace; }
  .step.say { font-family: inherit; font-style: italic; }
  .step .ic { flex: none; }
  .step .tx { flex: 1; min-width: 0; }
  .step .dur { flex: none; opacity: 0.7; font-family: inherit; }
  .now { color: var(--text); font-style: normal; margin-top: 4px; overflow-wrap: anywhere; }
  .m.me.queued { background: var(--blue-soft); color: var(--text); border: 1px dashed var(--blue); }
  .qrow { display: flex; gap: 12px; margin-top: 4px; }
  .m.me .qrow { justify-content: flex-end; }
  .qedit { width: 100%; min-width: 220px; font-size: 16px; resize: none; }
  .idea { margin: 8px 0 2px; padding: 8px 10px; background: var(--bg); border: 1px solid var(--line); border-radius: 8px; }
  .idea p { margin: 4px 0 8px; }
  .ok { margin: 10px 0; }
  .pick { display: flex; gap: 8px; padding: 8px 12px 6px; background: var(--surface); border-top: 1px solid var(--line); }
  .pick select { flex: 1; min-width: 0; height: 36px; font-size: 14px; padding: 0 8px; }
  .th { display: block; max-width: 160px; max-height: 120px; border-radius: 8px; margin-bottom: 4px; }
  .th.ph { width: 60px; height: 40px; background: var(--panel); }
  .at { position: relative; }
  .at .th { max-width: 64px; max-height: 64px; }
  .x { position: absolute; top: -6px; right: -6px; width: 20px; height: 20px; border-radius: 50%; border: 0; background: var(--text); color: var(--bg); line-height: 1; }
  .send { display: flex; gap: 8px; align-items: end; padding: 4px 12px calc(8px + env(safe-area-inset-bottom)); background: var(--surface); }
  .send textarea { flex: 1; min-width: 0; font-size: 16px; line-height: 1.4; min-height: 44px; max-height: 120px; resize: none; field-sizing: content; }
  .send .btn { height: 44px; flex: none; display: grid; place-items: center; }
  .send label.btn { width: 44px; padding: 0; font-size: 20px; cursor: pointer; }
  .small { font-size: 13px; }
  .stuckrow { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; margin-top: 6px; font-style: normal; }
  .m time, .meta time { font-size: 11px; opacity: 0.7; }
  .m.me time { display: block; text-align: right; margin-top: 2px; }
  .m.ai { white-space: normal; }
  .meta { display: flex; gap: 10px; align-items: center; margin-top: 4px; }
  .lnk { background: none; border: 0; padding: 0; font-size: 12px; color: var(--blue); cursor: pointer; }
  .md :global(p) { margin: 0 0 8px; } .md :global(p:last-child) { margin-bottom: 0; }
  .md :global(ul), .md :global(ol) { margin: 0 0 8px; padding-left: 22px; }
  .md :global(code) { font-family: ui-monospace, monospace; font-size: 0.88em; background: var(--bg); border: 1px solid var(--line); border-radius: 4px; padding: 0 4px; }
  .md :global(pre.code) { margin: 6px 0 8px; padding: 8px 10px; background: var(--bg); border: 1px solid var(--line); border-radius: 8px; overflow-x: auto; white-space: pre; }
  .md :global(pre.code code) { border: 0; padding: 0; background: none; }
  .status { display: flex; align-items: center; gap: 8px; font-style: normal; color: var(--text); }
  .spin { width: 14px; height: 14px; border-radius: 50%; border: 2px solid var(--line); border-top-color: var(--blue); animation: turn 0.9s linear infinite; flex: none; }
  @keyframes turn { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) { .spin { animation-duration: 3s; } }
  .step.cur { color: var(--text); font-weight: 600; }
  .file { display: flex; align-items: center; gap: 8px; margin: 4px 0; }
  .file code { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12.5px; }
  .bar { display: flex; width: 70px; height: 6px; border-radius: 3px; overflow: hidden; background: var(--line); }
  .bar .add { background: #2e9e5b; } .bar .del { background: #d14b4b; }
  .empty .chip { display: block; margin: 6px 0; background: var(--panel); border: 1px solid var(--line); border-radius: 999px; padding: 6px 12px; color: inherit; cursor: pointer; font: inherit; font-size: 14px; }
  button:disabled { opacity: 0.5; }
</style>
