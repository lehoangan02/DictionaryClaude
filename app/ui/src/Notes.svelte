<script lang="ts">
  import { call } from './lib/api';
  import { app, open, bump } from './lib/store.svelte';
  import { REL } from './lib/format';
  import Drafts from './Drafts.svelte';
  import Notebook from './Notebook.svelte';
  import { grow } from './lib/motion';

  type N = { id: string; session_date: string; tags: string; body: string; updated: number };
  let notes = $state<N[]>([]);
  let editing = $state<string | null>(null);
  let body = $state('');
  let date = $state(new Date().toISOString().slice(0, 10));
  let tags = $state('');
  let edges = $state<{ a: string; b: string; rel: keyof typeof REL }[]>([]);
  let error = $state('');
  let ta = $state<HTMLTextAreaElement | null>(null);
  let reviewing = $state(false);
  let timer: ReturnType<typeof setTimeout>;

  async function load() { notes = await call('notes_list'); }
  $effect(() => { void app.version; load().catch((e) => (error = e.message)); });

  // Phone keyboards cannot type ≈ or ⍯, so common ASCII stand-ins convert as you type.
  function autoconvert() {
    const swaps: [string, string][] = [['~=', '≈'], ['!=', '⍯'], ['=/=', '⍯']];
    for (const [from, to] of swaps) if (body.includes(from)) body = body.replaceAll(from, to);
    clearTimeout(timer);
    timer = setTimeout(async () => { edges = await call('note_parse', { body }); }, 150);
  }

  function insert(sym: string) {
    if (!ta) return;
    const s = ta.selectionStart, e = ta.selectionEnd;
    body = body.slice(0, s) + ` ${sym} ` + body.slice(e);
    queueMicrotask(() => { if (!ta) return; ta.focus(); ta.selectionStart = ta.selectionEnd = s + sym.length + 2; autoconvert(); });
  }

  function edit(n: N) { editing = n.id; body = n.body; date = n.session_date; tags = n.tags; autoconvert(); window.scrollTo({ top: 0, behavior: 'smooth' }); }
  function fresh() { editing = null; body = ''; tags = ''; edges = []; }

  async function save() {
    if (!body.trim()) return;
    try {
      const r = await call('note_save', { id: editing ?? undefined, body, session_date: date, tags });
      error = '';
      fresh(); bump(); await load();
      return r;
    } catch (e) { error = (e as Error).message; }
  }
  async function del(n: N) { await call('note_delete', { id: n.id }); if (editing === n.id) fresh(); bump(); await load(); }

  // Search runs on the notes already loaded: every word typed must appear in the note's text, tags or date.
  let notebook = $state(false);
  let query = $state('');
  const shown = $derived.by(() => {
    const words = query.toLowerCase().split(/\s+/).filter(Boolean);
    if (!words.length) return notes;
    return notes.filter((n) => {
      const hay = `${n.body}\n${n.tags}\n${n.session_date}`.toLowerCase();
      return words.every((w) => hay.includes(w));
    });
  });

  const lineCount =$derived(body.split('\n').filter((l) => l.trim()).length);
</script>

{#if !reviewing}
<h1>Notes</h1>
<p class="muted">Write what your tutor says. Relations feed the thesaurus and a quiz of your own notes.</p>

{/if}
<Drafts bind:reviewing />
{#if !reviewing}

<section class="ed card rise">
  <div class="meta">
    <label>Session <input type="date" bind:value={date} /></label>
    <label>Tags <input type="text" placeholder="tutor, roots…" bind:value={tags} /></label>
  </div>
  <div class="syms" role="toolbar" aria-label="Insert symbol">
    {#each Object.entries(REL) as [k, r] (k)}
      <button class="btn small" onclick={() => insert(r.sym)} title={r.label}><b>{r.sym}</b> <span class="muted">{r.label}</span></button>
    {/each}
  </div>
  <textarea bind:this={ta} rows="7" bind:value={body} oninput={autoconvert} spellcheck="false" autocapitalize="off"
    placeholder={'abate = lessen = subside\nfrugal ≈ thrifty\nlucid >< opaque\nimbibition ⍯ osmosis'}></textarea>
  {#if edges.length}
    <div class="pv" aria-label="Parsed relations">
      {#each edges as e (e.a + e.rel + e.b)}
        <span class="pill" in:grow><button onclick={() => open(e.a)}>{e.a}</button><b>{REL[e.rel].sym}</b><button onclick={() => open(e.b)}>{e.b}</button></span>
      {/each}
    </div>
  {:else if lineCount}
    <p class="muted small">No relations found yet. Use = ≈ &gt;&lt; ⍯ between words.</p>
  {/if}
  {#if error}<p class="err">{error}</p>{/if}
  <div class="act">
    <button class="btn primary" onclick={save} disabled={!body.trim()}>{editing ? 'Save changes' : 'Save note'}</button>
    {#if editing}<button class="btn ghost" onclick={fresh}>Cancel</button>{/if}
  </div>
</section>

<div class="views" role="tablist" aria-label="How to view saved notes">
  <h2>Saved notes</h2>
  <span class="sp"></span>
  <button class="btn small" class:primary={!notebook} role="tab" aria-selected={!notebook} onclick={() => (notebook = false)}>List</button>
  <button class="btn small" class:primary={notebook} role="tab" aria-selected={notebook} onclick={() => (notebook = true)}>Notebook</button>
</div>
{#if notebook}
  <Notebook {notes} onedit={(n) => { notebook = false; edit(n); }} />
{:else}
<input class="find" type="search" placeholder="Search your notes by content, tag or date…" aria-label="Search notes" bind:value={query} />
{#if query.trim()}<p class="muted small">{shown.length} of {notes.length} notes match.</p>{/if}
{#each shown as n, ni (n.id)}
  <article class="card nt rise" style="--i:{ni}">
    <header>
      <b>{n.session_date || 'Undated'}</b>
      {#each n.tags.split(',').map((t) => t.trim()).filter(Boolean) as t}<span class="tag">{t}</span>{/each}
      <span class="sp"></span>
      <button class="btn small ghost" onclick={() => edit(n)}>Edit</button>
      <button class="btn small ghost" onclick={() => del(n)}>Delete</button>
    </header>
    <pre>{n.body}</pre>
  </article>
{:else}
  <p class="empty">{query.trim() ? 'No notes match your search.' : 'No notes yet.'}</p>
{/each}
{/if}

{/if}

<style>
  h1 { margin: 0 0 4px; font-size: 28px; }
  .ed { margin-top: 10px; }
  .meta { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; margin-bottom: 10px; }
  .meta label { display: flex; flex-direction: column; font-size: 13px; color: var(--muted); gap: 3px; }
  .syms { display: flex; gap: 6px; flex-wrap: wrap; margin-bottom: 8px; }
  textarea { font-family: 'Lexis Symbols', ui-monospace, 'SF Mono', Menlo, Consolas, monospace; font-size: 17px; line-height: 1.6; resize: vertical; }
  .pv { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 10px; }
  .pill { display: inline-flex; align-items: center; background: var(--bg); border: 1px solid var(--line); border-radius: 999px; overflow: hidden; }
  .pill button { background: none; border: 0; padding: 3px 10px; color: var(--blue); }
  .pill b { background: var(--navy); color: #fff; padding: 3px 9px; font-size: 13px; }
  .small { font-size: 13px; }
  .act { display: flex; gap: 10px; margin-top: 12px; }
  .find { width: 100%; box-sizing: border-box; margin-bottom: 6px; }
  .nt { margin: 10px 0; }
  .nt header { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .sp { flex: 1; }
  .views { display: flex; align-items: center; gap: 6px; }
  .views h2 { margin: 12px 0 8px; }
  .tag { background: var(--panel-2); border-radius: 999px; padding: 0 9px; font-size: 12px; }
  pre { white-space: pre-wrap; margin: 6px 0 0; font-family: 'Lexis Symbols', ui-monospace, Menlo, Consolas, monospace; font-size: 15px; }
  button:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
