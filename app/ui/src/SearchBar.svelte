<script lang="ts">
  import { call } from './lib/api';
  import { app, open } from './lib/store.svelte';
  import Icon from './Icon.svelte';
  import { drop } from './lib/motion';

  type S = { word: string; key: string; barrons: boolean };
  let q = $state('');
  let sugg = $state<S[]>([]);
  let did = $state<string[]>([]);
  let idx = $state(-1);
  let focused = $state(false);
  let timer: ReturnType<typeof setTimeout>;
  let seq = 0;

  function onInput() {
    clearTimeout(timer);
    const mine = ++seq;
    timer = setTimeout(async () => {
      if (!q.trim()) { sugg = []; did = []; return; }
      try {
        const r = await call('search', { q, source: app.source === 'mw' ? undefined : app.source });
        if (mine !== seq) return; // a newer keystroke superseded this answer
        sugg = r.suggestions; did = r.did_you_mean; idx = -1;
      } catch { /* the entry view shows errors */ }
    }, 60);
  }

  function go(w: string) {
    seq++;
    q = w; sugg = []; did = []; focused = false; idx = -1;
    open(w);
    (document.activeElement as HTMLElement | null)?.blur();
  }

  function clear() { q = ''; sugg = []; did = []; seq++; document.getElementById('lexis-q')?.focus(); }

  function key(e: KeyboardEvent) {
    if (e.key === 'ArrowDown') { idx = Math.min(idx + 1, sugg.length - 1); e.preventDefault(); }
    else if (e.key === 'ArrowUp') { idx = Math.max(idx - 1, -1); e.preventDefault(); }
    else if (e.key === 'Enter') { const w = idx >= 0 ? sugg[idx].word : q.trim(); if (w) go(w); }
    else if (e.key === 'Escape') { focused = false; }
  }

  // Clicking a word elsewhere updates the box too.
  $effect(() => { if (app.word && app.word !== q) { q = app.word; sugg = []; } else if (!app.word && !focused) { q = ''; } });
</script>

<div class="sb">
  <span class="lens"><Icon name="search" size={20} /></span>
  <input
    id="lexis-q" type="search" placeholder="Search a word…" autocomplete="off" autocapitalize="off" spellcheck="false"
    aria-label="Search" bind:value={q} oninput={onInput} onkeydown={key}
    onfocus={() => (focused = true)} onblur={() => setTimeout(() => (focused = false), 140)}
  />
  {#if q}<button class="x fadein" onmousedown={(e) => { e.preventDefault(); clear(); }} aria-label="Clear"><Icon name="close" size={18} /></button>{/if}
  {#if focused && (sugg.length || did.length)}
    <ul class="dd" role="listbox" transition:drop>
      {#each sugg as s, i (s.key)}
        <li role="option" aria-selected={i === idx}>
          <button class:sel={i === idx} onmousedown={() => go(s.word)}>
            <span>{s.word}</span>{#if s.barrons}<em title="In Barron's Essential Words">GRE</em>{/if}
          </button>
        </li>
      {/each}
      {#if did.length}
        <li class="hint">Did you mean</li>
        {#each did as w (w)}
          <li><button onmousedown={() => go(w)}>{w}</button></li>
        {/each}
      {/if}
    </ul>
  {/if}
</div>

<style>
  .sb { position: relative; }
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
  .hint { padding: 6px 20px 2px; font-size: 12px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); }
</style>
