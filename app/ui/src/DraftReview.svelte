<script lang="ts">
  import { onMount } from 'svelte';
  import { call } from './lib/api';
  import { bump } from './lib/store.svelte';
  import { REL } from './lib/format';

  let { id, onclose }: { id: string; onclose: () => void } = $props();

  let d = $state<any>(null);
  let text = $state('');
  let edited = $state(false);
  let imgs = $state<string[]>([]);
  let edges = $state<{ a: string; b: string; rel: keyof typeof REL }[]>([]);
  let error = $state('');
  let busy = $state(false);
  let timer: ReturnType<typeof setTimeout>;

  async function load() {
    d = await call('draft_get', { id });
    // Keep the person's edits; only adopt server text while they have not typed.
    if (!edited) text = d.text;
    for (const name of d.photos) {
      if (!imgs[d.photos.indexOf(name)]) {
        call('photo_data', { name }).then((r) => (imgs[d.photos.indexOf(name)] = r.data_url)).catch(() => {});
      }
    }
    parse();
  }
  onMount(() => {
    load().catch((e) => (error = e.message));
    const t = setInterval(async () => {
      if (!d) return;
      // The server answers; asking it for news keeps the reply coming without waiting for the next background sync.
      if (['chat', 'chat_running'].includes(d.agent_status)) await call('sync_now').catch(() => {});
      if (['queued', 'processing'].includes(d.status) || ['requested', 'running', 'chat', 'chat_running'].includes(d.agent_status)) load().catch(() => {});
    }, 3000);
    return () => clearInterval(t);
  });

  function parse() {
    clearTimeout(timer);
    timer = setTimeout(async () => { edges = await call('note_parse', { body: text }); }, 150);
  }
  function typed() {
    edited = true;
    for (const [a, b] of [['~=', '≈'], ['!=', '⍯']]) text = text.replaceAll(a, b);
    parse();
  }

  async function save() {
    busy = true; error = '';
    try {
      await call('draft_update', { id, text });
      await call('draft_accept', { id, text });
      bump(); onclose();
    } catch (e) { error = (e as Error).message; }
    busy = false;
  }
  async function discard() { await call('draft_reject', { id }); bump(); onclose(); }

  // Gemini's strongest model reads the photos again; the text below is replaced, so ask first.
  let confirmReread = $state(false);
  async function reread() {
    busy = true; error = ''; confirmReread = false;
    try { await call('draft_reread', { id }); call('sync_now').catch(() => {}); bump(); onclose(); }
    catch (e) { error = (e as Error).message; }
    busy = false;
  }

  // The chat with Claude about this reading. Each message costs one agent call, so nothing is sent unasked.
  let msg = $state('');
  let sending = $state(false);
  const chatting = $derived(['chat', 'chat_running'].includes(d?.agent_status));
  const chat = $derived<any[]>(d?.chat ?? []);
  async function send() {
    const m = msg.trim();
    if (!m || chatting || sending) return;
    sending = true; error = '';
    try { d = await call('draft_chat', { id, message: m }); msg = ''; call('sync_now').catch(() => {}); bump(); }
    catch (e) { error = (e as Error).message; }
    sending = false;
  }
  function useProposal(t: string) { text = t; edited = true; parse(); }
  function sendKey(e: KeyboardEvent) { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); send(); } }
</script>

<button class="btn small ghost" onclick={onclose}>← Back</button>
{#if !d}
  <p class="empty">Loading…</p>
{:else}
  <h1>Check the reading</h1>
  <p class="muted">{d.session_date || 'Undated'} · {d.photos.length} photo{d.photos.length > 1 ? 's' : ''}
    {#if d.ocr_engine}· read by {d.ocr_engine}{/if}</p>
  {#if ['queued', 'processing'].includes(d.status)}<p class="note">Waiting for the server to read the photos… this updates by itself.</p>{/if}
  {#if d.error}<p class="note warn">{d.error}</p>{/if}

  <div class="cols rise">
    <div class="photos">
      {#each d.photos as name, i (name)}
        {#if imgs[i]}<a href={imgs[i]} target="_blank" rel="noopener"><img src={imgs[i]} alt="Photo {i + 1} of your notes" /></a>
        {:else}<div class="ph">Photo {i + 1} is on its way…</div>{/if}
      {/each}
    </div>
    <div>
      <label class="f">Text (fix anything the machine misread)
        <textarea rows="10" bind:value={text} oninput={typed} spellcheck="false" autocapitalize="off"></textarea></label>
      {#if edges.length}
        <div class="pv">
          {#each edges as e (e.a + e.rel + e.b)}
            <span class="pill"><span>{e.a}</span><b>{REL[e.rel].sym}</b><span>{e.b}</span></span>
          {/each}
        </div>
      {:else if text.trim()}<p class="muted small">No relations found yet. Use = ≈ &gt;&lt; ⍯ between words.</p>{/if}
    </div>
  </div>

  <section class="card agents rise" style="--i:1">
    <h3>Hard to read?</h3>
    <p class="muted small">Not happy with the reading? A stronger model can read the photos again (slower; it replaces the text above).</p>
    <div class="row">
      {#if confirmReread}
        <button class="btn primary" disabled={busy} onclick={reread}>Yes, replace the text</button>
        <button class="btn ghost" onclick={() => (confirmReread = false)}>Cancel</button>
      {:else}
        <button class="btn" disabled={busy || d.status === 'queued'} onclick={() => (confirmReread = true)}>Read again carefully</button>
      {/if}
    </div>
  </section>

  <section class="card chat rise" style="--i:2" aria-label="Chat with Claude">
    <h3>Chat with Claude</h3>
    <p class="muted small">Ask Claude to look at the photo again, or tell it what a word should be. It never changes your text by itself: when it suggests a new version you choose to use it. Each message uses one agent call.</p>
    <div class="msgs" aria-live="polite">
      {#each chat as m}
        {#if m.role === 'user'}<div class="m me">{m.text}</div>
        {:else if m.role === 'error'}<div class="m bad">{m.text}</div>
        {:else}
          <div class="m ai">{m.text}
            {#if m.proposal}
              <pre class="prop">{m.proposal}</pre>
              <button class="btn small primary" onclick={() => useProposal(m.proposal)}>Use this text</button>
            {/if}
          </div>
        {/if}
      {/each}
      {#if chatting}<div class="m ai wait">Claude is looking at the photo…</div>{/if}
    </div>
    <div class="send">
      <textarea rows="2" bind:value={msg} onkeydown={sendKey} placeholder="Line 3 should be “imbibition”, can you check?" disabled={d.status === 'queued'}></textarea>
      <button class="btn primary" disabled={!msg.trim() || chatting || sending || d.status === 'queued'} onclick={send}>Send</button>
    </div>
  </section>

  {#if error}<p class="err">{error}</p>{/if}
  <div class="row act">
    <button class="btn primary" disabled={busy || !text.trim() || d.status === 'queued'} onclick={save}>Save as note</button>
    <button class="btn ghost" onclick={discard}>Discard</button>
  </div>
{/if}

<style>
  h1 { margin: 8px 0 2px; font-size: 26px; }
  h3 { margin: 12px 0 6px; font-size: 14px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); }
  .note { background: var(--panel); border-left: 4px solid var(--blue); padding: 8px 12px; border-radius: 4px; }
  .note.warn { border-left-color: var(--accent); }
  .cols { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; margin-top: 12px; }
  @media (max-width: 720px) { .cols { grid-template-columns: 1fr; } }
  .photos img { width: 100%; border-radius: 8px; border: 1px solid var(--line); margin-bottom: 8px; display: block; }
  .ph { background: var(--panel); border-radius: 8px; padding: 30px; text-align: center; color: var(--muted); }
  .f { display: flex; flex-direction: column; gap: 3px; font-size: 13px; color: var(--muted); }
  textarea { font-family: 'Lexis Symbols', ui-monospace, Menlo, Consolas, monospace; font-size: 16px; line-height: 1.6; }
  .pv { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 8px; }
  .pill { display: inline-flex; align-items: center; background: var(--bg); border: 1px solid var(--line); border-radius: 999px; overflow: hidden; }
  .pill span { padding: 2px 10px; } .pill b { background: var(--navy); color: #fff; padding: 2px 9px; font-family: 'Lexis Symbols', ui-monospace, 'DejaVu Sans Mono', monospace; }
  .agents, .chat { margin-top: 16px; }
  .msgs { display: grid; gap: 8px; margin: 10px 0; }
  .m { max-width: 92%; padding: 8px 12px; border-radius: 14px; white-space: pre-wrap; overflow-wrap: anywhere; }
  .m.me { justify-self: end; background: var(--blue); color: #fff; border-bottom-right-radius: 4px; }
  .m.ai { justify-self: start; background: var(--panel); border-bottom-left-radius: 4px; }
  .m.bad { justify-self: start; background: var(--bad-bg); color: var(--bad); }
  .m.wait { color: var(--muted); font-style: italic; }
  .prop { margin: 8px 0; padding: 8px 10px; background: var(--bg); border: 1px solid var(--line); border-radius: 8px; white-space: pre-wrap; font-family: 'Lexis Symbols', ui-monospace, Menlo, monospace; font-size: 14px; }
  .send { display: flex; gap: 8px; align-items: end; }
  .send textarea { flex: 1; font-size: 16px; }
  .row { display: flex; gap: 10px; align-items: end; flex-wrap: wrap; }
  .small { font-size: 13px; }
  .act { margin-top: 16px; }
  button:disabled { opacity: 0.5; }
</style>
