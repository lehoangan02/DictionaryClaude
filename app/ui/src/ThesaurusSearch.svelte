<script lang="ts">
  import { untrack } from 'svelte';
  import { call } from './lib/api';
  import { app, open } from './lib/store.svelte';
  import Icon from './Icon.svelte';
  import Thesaurus from './Thesaurus.svelte';
  import { drop } from './lib/motion';

  type S = { word: string; key: string; barrons: boolean };
  let q = $state('');
  let word = $state('');
  let data = $state<any>(null);
  let sugg = $state<S[]>([]);
  let idx = $state(-1);
  let focused = $state(false);
  let error = $state('');
  let timer: ReturnType<typeof setTimeout>;
  let seq = 0;

  function onInput() {
    clearTimeout(timer);
    const mine = ++seq;
    timer = setTimeout(async () => {
      if (!q.trim()) { sugg = []; return; }
      try {
        const r = await call('search', { q });
        if (mine !== seq) return; // a newer keystroke superseded this answer
        sugg = r.suggestions; idx = -1;
      } catch { /* suggestions are a nicety */ }
    }, 60);
  }

  async function go(w: string) {
    w = w.trim();
    if (!w) return;
    seq++;
    q = w; sugg = []; focused = false; idx = -1;
    (document.activeElement as HTMLElement | null)?.blur();
    try {
      data = await call('thesaurus', { word: w });
      word = w; error = '';
    } catch (e) { error = (e as Error).message; }
  }

  function clear() { q = ''; sugg = []; seq++; document.getElementById('lexis-tq')?.focus(); }

  function key(e: KeyboardEvent) {
    if (e.key === 'ArrowDown') { idx = Math.min(idx + 1, sugg.length - 1); e.preventDefault(); }
    else if (e.key === 'ArrowUp') { idx = Math.max(idx - 1, -1); e.preventDefault(); }
    else if (e.key === 'Enter') go(idx >= 0 ? sugg[idx].word : q);
    else if (e.key === 'Escape') focused = false;
  }

  // Notes edited or synced while the tab is open change what the answer holds.
  $effect(() => { void app.version; untrack(() => { if (word) reload(); }); });
  async function reload() {
    try { data = await call('thesaurus', { word }); } catch { /* keep what is shown */ }
  }
</script>

<h1>Thesaurus</h1>
<p class="muted">Type a word to see its synonyms, opposites and related GRE words, without the definitions.</p>

<div class="sb">
  <span class="lens"><Icon name="search" size={20} /></span>
  <input
    id="lexis-tq" type="search" placeholder="Find synonyms for…" autocomplete="off" autocapitalize="off" spellcheck="false"
    aria-label="Thesaurus search" bind:value={q} oninput={onInput} onkeydown={key}
    onfocus={() => (focused = true)} onblur={() => setTimeout(() => (focused = false), 140)}
  />
  {#if q}<button class="x fadein" onpointerdown={(e) => { e.preventDefault(); clear(); }} onclick={clear} aria-label="Clear"><Icon name="close" size={18} /></button>{/if}
  {#if focused && sugg.length}
    <ul class="dd" role="listbox" transition:drop>
      {#each sugg as s, i (s.key)}
        <li role="option" aria-selected={i === idx}>
          <button class:sel={i === idx} onmousedown={() => go(s.word)}>
            <span>{s.word}</span>{#if s.barrons}<em title="In Barron's Essential Words">GRE</em>{/if}
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

{#if error}<p class="err">{error}</p>{/if}
{#if data}
  <Thesaurus {data} {word} alone pick={go} />
  <p><button class="btn small ghost" onclick={() => open(word)}>Open “{word}” in the dictionary</button></p>
{/if}

<style>
  .sb { position: relative; margin: 14px 0 4px; }
  .lens { position: absolute; left: 14px; top: 50%; transform: translateY(-50%); color: var(--muted); display: flex; pointer-events: none; }
  input {
    background: var(--panel); color: var(--text); border: 1px solid var(--line); border-radius: 999px;
    padding: 10px 44px 10px 44px; font-size: 17px; box-shadow: inset 0 1px 2px rgba(16, 40, 80, 0.06);
    transition: box-shadow 0.15s, border-color 0.15s, background 0.15s;
  }
  input:focus { outline: none; background: var(--surface); border-color: var(--blue); box-shadow: 0 0 0 3px color-mix(in srgb, var(--blue) 25%, transparent), var(--lift); }
  input::-webkit-search-cancel-button { display: none; }
  .x { position: absolute; right: 8px; top: 50%; transform: translateY(-50%); width: 32px; height: 32px; border: 0; border-radius: 50%; background: none; color: var(--muted); display: grid; place-items: center; }
  .x:hover { background: var(--panel-2); color: var(--text); }
  .dd {
    position: absolute; left: 0; right: 0; top: calc(100% + 8px); margin: 0; padding: 6px 0; list-style: none;
    background: var(--surface); color: var(--text); border: 1px solid var(--line); border-radius: 16px;
    box-shadow: var(--shadow); max-height: 60vh; overflow: auto; z-index: 30;
  }
  .dd button { width: 100%; text-align: left; background: none; border: 0; padding: 9px 20px; display: flex; justify-content: space-between; align-items: center; font-size: 16px; color: var(--text); }
  .dd button:hover, .dd button.sel { background: var(--blue-soft); }
  em { font-style: normal; font-size: 11px; font-weight: 700; color: var(--accent); border: 1px solid var(--accent); border-radius: 4px; padding: 0 5px; }
</style>
