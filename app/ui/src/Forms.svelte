<script lang="ts">
  import { call } from './lib/api';
  import { open } from './lib/store.svelte';
  import Icon from './Icon.svelte';
  import { drop } from './lib/motion';

  type S = { word: string; key: string; barrons: boolean };
  let q = $state('');
  let data = $state<any>(null);
  let sugg = $state<S[]>([]);
  let idx = $state(-1);
  let focused = $state(false);
  let error = $state('');
  let timer: ReturnType<typeof setTimeout>;
  let seq = 0;

  const posName: Record<string, string> = { verb: 'Verb', noun: 'Noun', adj: 'Adjective', adv: 'Adverb' };

  function onInput() {
    clearTimeout(timer);
    const mine = ++seq;
    timer = setTimeout(async () => {
      if (!q.trim()) { sugg = []; return; }
      try {
        const r = await call('search', { q, source: 'wiktionary' });
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
      data = await call('forms', { word: w });
      error = '';
    } catch (e) { error = (e as Error).message; data = null; }
  }

  function clear() { q = ''; sugg = []; seq++; document.getElementById('lexis-fq')?.focus(); }

  function key(e: KeyboardEvent) {
    if (e.key === 'ArrowDown') { idx = Math.min(idx + 1, sugg.length - 1); e.preventDefault(); }
    else if (e.key === 'ArrowUp') { idx = Math.max(idx - 1, -1); e.preventDefault(); }
    else if (e.key === 'Enter') go(idx >= 0 ? sugg[idx].word : q);
    else if (e.key === 'Escape') focused = false;
  }

  const single = (f: string) => !/\s/.test(f);
</script>

<h1>Word forms</h1>
<p class="muted">Type a word, in any of its forms, to see all of its forms laid out: plurals, tenses, participles and degrees of comparison.</p>

<div class="sb">
  <span class="lens"><Icon name="search" size={20} /></span>
  <input
    id="lexis-fq" type="search" placeholder="Show the forms of…" autocomplete="off" autocapitalize="off" spellcheck="false"
    aria-label="Word forms search" bind:value={q} oninput={onInput} onkeydown={key}
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
  {#each data.groups as g (g.lemma + g.via)}
    <section class="grp">
      <div class="gh">
        <h2>{g.lemma}</h2>
        {#if g.via}<span class="muted">“{g.via}” is a form of “{g.lemma}”</span>{/if}
        <button class="btn small ghost" onclick={() => open(g.lemma)}>Dictionary</button>
      </div>
      {#each g.tables as t}
        <div class="box">
          <h3>{posName[t.pos] ?? t.pos}</h3>
          {#if t.head}<p class="head muted">{t.head}</p>{/if}
          <dl>
            {#each t.rows as r}
              <dt>{r.label}</dt>
              <dd>
                {#each r.forms as c}
                  <span class="cell" class:hit={c.form.toLowerCase() === data.key}>
                    {#if single(c.form)}<button class="w" onclick={() => go(c.form)}>{c.form}</button>{:else}{c.form}{/if}{#if c.note}<em class="n">({c.note})</em>{/if}
                  </span>
                {:else}<span class="muted">—</span>{/each}
              </dd>
            {/each}
          </dl>
          {#if t.note}<p class="muted note">{t.note}</p>{/if}
        </div>
      {/each}
    </section>
  {:else}
    <p class="muted">No forms found for “{data.word}”.</p>
    {#if data.did_you_mean?.length}
      <p>Did you mean {#each data.did_you_mean as w, i (w)}{#if i}, {/if}<button class="w" onclick={() => go(w)}>{w}</button>{/each}?</p>
    {/if}
  {/each}
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
  .dd em { font-style: normal; font-size: 11px; font-weight: 700; color: var(--accent); border: 1px solid var(--accent); border-radius: 4px; padding: 0 5px; }

  .grp { margin-top: 24px; }
  .gh { display: flex; flex-wrap: wrap; align-items: baseline; gap: 4px 14px; }
  .gh h2 { margin: 0; }
  .gh .btn { margin-left: auto; align-self: center; }
  h3 { margin: 0 0 4px; font-size: 14px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); }
  .box { background: var(--panel); border: 1px solid var(--line); border-radius: 10px; padding: 10px 14px; margin: 10px 0; }
  .head { margin: 0 0 8px; font-size: 13px; }
  dl { margin: 0; display: grid; grid-template-columns: minmax(104px, 11.5em) 1fr; gap: 0 18px; }
  dt, dd { margin: 0; padding: 7px 0; border-top: 1px solid var(--line); }
  dt { color: var(--muted); font-size: 14px; }
  dd { display: flex; flex-wrap: wrap; gap: 2px 16px; font-size: 17px; }
  .cell.hit { background: var(--blue-soft); border-radius: 6px; padding: 0 6px; margin: 0 -6px; }
  .w { background: none; border: 0; padding: 0; font-weight: 700; color: var(--blue); font-size: inherit; }
  .n { font-style: normal; font-size: 12px; color: var(--muted); margin-left: 3px; }
  .note { margin: 8px 0 0; font-size: 13px; }
</style>
