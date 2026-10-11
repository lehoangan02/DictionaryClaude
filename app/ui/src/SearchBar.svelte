<script lang="ts">
  import { call } from './lib/api';
  import { app, open } from './lib/store.svelte';

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
        const r = await call('search', { q, source: app.source });
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

  function key(e: KeyboardEvent) {
    if (e.key === 'ArrowDown') { idx = Math.min(idx + 1, sugg.length - 1); e.preventDefault(); }
    else if (e.key === 'ArrowUp') { idx = Math.max(idx - 1, -1); e.preventDefault(); }
    else if (e.key === 'Enter') { const w = idx >= 0 ? sugg[idx].word : q.trim(); if (w) go(w); }
    else if (e.key === 'Escape') { focused = false; }
  }

  // Clicking a word elsewhere updates the box too.
  $effect(() => { if (app.word && app.word !== q) { q = app.word; sugg = []; } });
</script>

<div class="sb">
  <input
    type="search" placeholder="Search a word…" autocomplete="off" autocapitalize="off" spellcheck="false"
    aria-label="Search" bind:value={q} oninput={onInput} onkeydown={key}
    onfocus={() => (focused = true)} onblur={() => setTimeout(() => (focused = false), 140)}
  />
  {#if focused && (sugg.length || did.length)}
    <ul class="dd" role="listbox">
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
  input {
    background: #fff; color: #1c2430; border: 0; border-radius: 999px; padding: 10px 18px; font-size: 17px;
    box-shadow: inset 0 0 0 2px transparent;
  }
  input:focus { outline: none; box-shadow: inset 0 0 0 2px var(--accent); }
  .dd {
    position: absolute; left: 0; right: 0; top: calc(100% + 6px); margin: 0; padding: 6px 0; list-style: none;
    background: var(--bg); color: var(--text); border: 1px solid var(--line); border-radius: 12px;
    box-shadow: var(--shadow); max-height: 60vh; overflow: auto; z-index: 30;
  }
  .dd button {
    width: 100%; text-align: left; background: none; border: 0; padding: 8px 18px; display: flex;
    justify-content: space-between; align-items: center; font-size: 16px;
  }
  .dd button:hover, .dd button.sel { background: var(--panel-2); }
  em {
    font-style: normal; font-size: 11px; font-weight: 700; color: var(--accent); border: 1px solid var(--accent);
    border-radius: 4px; padding: 0 5px;
  }
  .hint { padding: 6px 18px 2px; font-size: 12px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); }
</style>
