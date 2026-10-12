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
  let mode = $state('both');
  let effort = $state('medium');
  let confirmAsk = $state(false);
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
    const t = setInterval(() => { if (d && (['queued', 'processing'].includes(d.status) || ['requested', 'running'].includes(d.agent_status))) load().catch(() => {}); }, 4000);
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

  async function ask() {
    busy = true; error = ''; confirmAsk = false;
    try { await call('draft_update', { id, text }); d = await call('draft_agents', { id, mode, effort }); bump(); }
    catch (e) { error = (e as Error).message; }
    busy = false;
  }

  // The merged reading, built from what both agents agree on and the choices made for conflicts.
  let picks = $state<Record<number, string>>({});
  function useMerged() {
    const m = d.agents.merged;
    text = m.lines.map((l: any, i: number) => (l.status === 'conflict' ? picks[i] ?? l.options[0].text : l.text)).join('\n');
    edited = true; parse();
  }
  const agentsRunning = $derived(['requested', 'running'].includes(d?.agent_status));
  const merged = $derived(d?.agents?.merged);
  const conflicts = $derived(merged?.lines?.filter((l: any) => l.status === 'conflict').length ?? 0);
  const who = (n: string) => (n === 'claude' ? 'Claude' : n === 'agy' ? 'agy' : 'Both agree');
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
    {#if agentsRunning}
      <p>The agents are reading the photos{d.agents?.request?.mode === 'both' ? ' and will compare notes if they differ' : ''}… this can take a minute or two.</p>
    {:else}
      <p class="muted small">Not happy with the reading? A stronger model can read the photos again (slower; it replaces the text above).</p>
      <div class="row">
        {#if confirmReread}
          <button class="btn primary" disabled={busy} onclick={reread}>Yes, replace the text</button>
          <button class="btn ghost" onclick={() => (confirmReread = false)}>Cancel</button>
        {:else}
          <button class="btn" disabled={busy || d.status === 'queued'} onclick={() => (confirmReread = true)}>Read again carefully</button>
        {/if}
      </div>
      <p class="muted small">Ask Claude and/or agy to read the photos. This uses your agent quota, so it only happens when you press the button.</p>
      <div class="row">
        <label class="f">Who <select bind:value={mode}>
          <option value="both">Both, then compare</option><option value="claude">Claude only</option><option value="agy">agy only</option></select></label>
        <label class="f">Effort <select bind:value={effort}><option>low</option><option>medium</option><option>high</option></select></label>
        {#if confirmAsk}
          <button class="btn primary" disabled={busy} onclick={ask}>Yes, use agent quota</button>
          <button class="btn ghost" onclick={() => (confirmAsk = false)}>Cancel</button>
        {:else}
          <button class="btn" disabled={busy || d.status === 'queued'} onclick={() => (confirmAsk = true)}>Ask agents…</button>
        {/if}
      </div>
    {/if}

    {#if d.agent_status === 'failed'}<p class="err">{d.error}</p>{/if}
    {#if d.agents?.claude || d.agents?.agy}
      <div class="versions">
        {#each ['claude', 'agy'] as n}
          {#if d.agents[n]}
            <div class="ver rise"><b>{who(n)}</b>
              {#if d.agents[n].error}<p class="err small">{d.agents[n].error}</p>
              {:else}
                {#each d.agents[n].lines as l}<div class="ln" class:unsure={l.unsure} title={l.reason}>{l.text}{#if l.unsure} <i>?</i>{/if}</div>{/each}
              {/if}
            </div>
          {/if}
        {/each}
      </div>
      {#if d.agents.rounds?.length}<p class="muted small">They disagreed, so each looked again after seeing the other's reading ({d.agents.usage.calls} calls in all).</p>{/if}
      {#if merged?.lines?.length}
        <h3>Compare line by line</h3>
        {#each merged.lines as l, i}
          {#if l.status === 'conflict'}
            <div class="conf">
              {#each l.options as o}
                <label><input type="radio" name="c{i}" checked={(picks[i] ?? l.options[0].text) === o.text} onchange={() => (picks[i] = o.text)} /> <span class="who">{who(o.from)}</span> {o.text}</label>
              {/each}
            </div>
          {:else}<div class="ok">✓ {l.text}</div>{/if}
        {/each}
        <button class="btn small" onclick={useMerged}>Use this reading ({conflicts} to decide)</button>
      {:else if merged?.alternatives?.length}
        <p class="muted small">The readings have different numbers of lines, so they can't be compared line by line. Pick one to start from:</p>
        {#each merged.alternatives as a}<button class="btn small" onclick={() => { text = a.text; edited = true; parse(); }}>Use {who(a.from)}'s version</button>{/each}
      {/if}
    {/if}
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
  .agents { margin-top: 16px; }
  .row { display: flex; gap: 10px; align-items: end; flex-wrap: wrap; }
  .small { font-size: 13px; }
  .versions { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; margin-top: 12px; }
  @media (max-width: 720px) { .versions { grid-template-columns: 1fr; } }
  .ver { background: var(--bg); border: 1px solid var(--line); border-radius: 8px; padding: 8px 12px; }
  .ln { font-family: 'Lexis Symbols', ui-monospace, Menlo, monospace; font-size: 14px; }
  .ln.unsure { background: color-mix(in srgb, var(--accent) 16%, transparent); }
  .ln i { color: var(--accent); font-style: normal; font-weight: 700; }
  .conf { background: var(--bg); border: 1px solid var(--accent); border-radius: 8px; padding: 6px 12px; margin: 6px 0; display: grid; gap: 4px; }
  .conf label { font-family: 'Lexis Symbols', ui-monospace, Menlo, monospace; font-size: 14px; }
  .who { font-family: system-ui; font-size: 12px; color: var(--muted); margin: 0 4px; }
  .ok { font-family: 'Lexis Symbols', ui-monospace, Menlo, monospace; font-size: 14px; color: var(--muted); }
  .act { margin-top: 16px; }
  button:disabled { opacity: 0.5; }
</style>
