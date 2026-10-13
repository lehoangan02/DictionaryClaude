<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { call } from './lib/api';
  import { app, bump, openDetail, closeDetail } from './lib/store.svelte';

  type Idea = { title: string; detail: string; status: 'proposed' | 'approved' | 'dismissed' };
  type Msg = { role: 'user' | 'claude' | 'error'; text: string; at: number; idea?: Idea };
  type Chat = { id: string; title: string; state: string; updated: number; last: string };
  let chats = $state<Chat[]>([]);
  let cur = $state<string | null>(null);
  let msgs = $state<Msg[]>([]);
  let phase = $state('');
  let msg = $state('');
  let error = $state('');
  let sending = $state(false);
  let end = $state<HTMLElement | null>(null);
  const waiting = $derived(phase === 'asked' || phase === 'running');
  const approved = $derived(msgs.filter((m) => m.idea?.status === 'approved'));

  async function loadList() { chats = await call('talk_list'); }
  async function load() {
    await loadList();
    if (!cur) return;
    const t = await call('talk_get', { id: cur });
    if (!chats.some((c) => c.id === cur)) { cur = null; return; }
    const grew = t.msgs.length !== msgs.length;
    msgs = t.msgs; phase = t.state;
    if (grew) { await tick(); end?.scrollIntoView({ block: 'end' }); }
  }
  $effect(() => { void app.version; void cur; load().catch((e) => (error = e.message)); });
  onMount(() => {
    // The server answers; asking it for news brings the reply without waiting for the next background sync.
    const t = setInterval(async () => {
      if (!waiting && !chats.some((c) => c.state)) return;
      await call('sync_now').catch(() => {});
      load().catch(() => {});
    }, 3000);
    return () => clearInterval(t);
  });

  async function send() {
    const m = msg.trim();
    if (!m || waiting || sending) return;
    sending = true; error = '';
    try { const t = await call('talk_send', { id: cur, message: m }); msgs = t.msgs; phase = t.state; msg = ''; call('sync_now').catch(() => {}); bump(); await tick(); end?.scrollIntoView({ block: 'end' }); }
    catch (e) { error = (e as Error).message; }
    sending = false;
  }
  function sendKey(e: KeyboardEvent) { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); send(); } }
  async function decide(m: Msg, decision: string) {
    try { const t = await call('talk_idea', { id: cur, at: m.at, decision }); msgs = t.msgs; call('sync_now').catch(() => {}); } catch (e) { error = (e as Error).message; }
  }
  // Back from a chat returns to the list.
  function openChat(id: string) { msgs = []; phase = ''; cur = id; openDetail(() => (cur = null)); }
  async function newChat() { try { const t = await call('talk_new'); await loadList(); openChat(t.id); } catch (e) { error = (e as Error).message; } }
  async function removeChat() {
    if (!cur) return;
    try { await call('talk_delete', { id: cur }); call('sync_now').catch(() => {}); closeDetail(); await loadList(); } catch (e) { error = (e as Error).message; }
  }
  const ago = (t: number) => new Date(t).toLocaleDateString();
</script>

{#if cur === null}
<h1>Claude</h1>
<p class="muted">Ask about the app, report a problem or suggest a change. Claude reads the app's code to answer but changes nothing; what it suggests appears as an idea you approve or dismiss. Each message uses one agent call.</p>
<p><button class="btn primary" onclick={newChat}>New chat</button></p>
{#if error}<p class="err">{error}</p>{/if}
{#each chats as c (c.id)}
  <button class="card row" onclick={() => openChat(c.id)}>
    <b>{c.title || 'New chat'}</b>
    <span class="muted small">{c.state ? 'Claude is answering…' : c.last}</span>
    <span class="muted small">{ago(c.updated)}</span>
  </button>
{:else}
  <p class="empty">No chats yet.</p>
{/each}
{:else}
<button class="btn small ghost" onclick={closeDetail}>← Chats</button>

{#if approved.length}
  <details class="card ok">
    <summary>Approved ideas ({approved.length})</summary>
    {#each approved as m (m.at)}
      <div class="idea"><b>{m.idea?.title}</b><p>{m.idea?.detail}</p>
        <button class="btn small ghost" onclick={() => decide(m, 'dismissed')}>Remove</button></div>
    {/each}
  </details>
{/if}

<div class="msgs" aria-live="polite">
  {#each msgs as m (m.at + m.role)}
    {#if m.role === 'user'}<div class="m me">{m.text}</div>
    {:else if m.role === 'error'}<div class="m bad">{m.text}</div>
    {:else}
      <div class="m ai">{m.text}
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
    <p class="empty">Nothing yet. Try “the History tab feels slow” or “how does the quiz pick words?”.</p>
  {/each}
  {#if waiting}<div class="m ai wait">Claude is thinking…</div>{/if}
  <span bind:this={end}></span>
</div>

{#if error}<p class="err">{error}</p>{/if}
<div class="send">
  <textarea rows="2" bind:value={msg} onkeydown={sendKey} placeholder="Message Claude"></textarea>
  <button class="btn primary" disabled={!msg.trim() || waiting || sending} onclick={send}>Send</button>
</div>
<p><button class="btn small ghost" disabled={waiting} onclick={removeChat}>Delete this chat</button></p>

{/if}

<style>
  .row { display: grid; gap: 2px; width: 100%; text-align: left; margin: 8px 0; cursor: pointer; }
  h1 { margin: 0 0 4px; font-size: 28px; }
  .msgs { display: grid; gap: 8px; margin: 14px 0; }
  .m { max-width: 92%; padding: 8px 12px; border-radius: 14px; white-space: pre-wrap; overflow-wrap: anywhere; }
  .m.me { justify-self: end; background: var(--blue); color: #fff; border-bottom-right-radius: 4px; }
  .m.ai { justify-self: start; background: var(--panel); border-bottom-left-radius: 4px; }
  .m.bad { justify-self: start; background: var(--bad-bg); color: var(--bad); }
  .m.wait { color: var(--muted); font-style: italic; }
  .idea { margin: 8px 0 2px; padding: 8px 10px; background: var(--bg); border: 1px solid var(--line); border-radius: 8px; }
  .idea p { margin: 4px 0 8px; }
  .ok { margin: 10px 0; }
  .send { display: flex; gap: 8px; align-items: end; }
  .send textarea { flex: 1; font-size: 16px; }
  .small { font-size: 13px; }
  button:disabled { opacity: 0.5; }
</style>
