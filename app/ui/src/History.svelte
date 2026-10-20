<script lang="ts">
  import Icon from './Icon.svelte';
  import { call } from './lib/api';
  import { app, openFromHistory, bump } from './lib/store.svelte';
  import { when } from './lib/format';
  import { flip } from 'svelte/animate';
  import { item as leave, flipDur } from './lib/motion';

  type H = { id: string; ts: number; key: string; word: string; source: string; starred: boolean };
  // One row per run of the same word, however many dictionaries it was opened in.
  type M = H & { ids: string[]; sources: string[] };
  let q = $state('');
  let starred = $state(false);
  let rows = $state<H[]>([]);
  let error = $state('');

  async function load() {
    try { rows = await call('history_list', { q, starred, limit: 300 }); error = ''; }
    catch (e) { error = (e as Error).message; }
  }
  $effect(() => { void q; void starred; void app.version; void app.historyVersion; load(); });

  async function star(h: M) { for (const id of h.ids) await call('history_star', { id, starred: h.starred ? 0 : 1 }); await load(); }
  async function del(h: M) { for (const id of h.ids) await call('history_delete', { id }); await load(); }
  let confirmClear = $state(false);
  async function clear() { await call('history_clear'); confirmClear = false; bump(); await load(); }

  // Paint the typed letters wherever they occur in the word (case-insensitive).
  function parts(word: string, part: string): { t: string; hit: boolean }[] {
    const p = part.trim().toLowerCase();
    const i = p ? word.toLowerCase().indexOf(p) : -1;
    if (i < 0 || word.toLowerCase().length !== word.length) return [{ t: word, hit: false }];
    return [{ t: word.slice(0, i), hit: false }, { t: word.slice(i, i + p.length), hit: true }, { t: word.slice(i + p.length), hit: false }];
  }

  // Group by day like a browser history.
  const groups = $derived.by(() => {
    const out: { day: string; items: M[] }[] = [];
    for (const h of rows) {
      const day = new Date(h.ts).toLocaleDateString(undefined, { weekday: 'long', month: 'long', day: 'numeric' });
      if (out.at(-1)?.day !== day) out.push({ day, items: [] });
      const items = out.at(-1)!.items;
      const prev = items.at(-1);
      if (prev && prev.key === h.key) {
        prev.ids.push(h.id);
        if (!prev.sources.includes(h.source)) prev.sources.push(h.source);
        prev.starred = prev.starred || h.starred;
      } else items.push({ ...h, ids: [h.id], sources: [h.source] });
    }
    return out;
  });
</script>

<h1>History</h1>
<div class="tools">
  <input type="search" placeholder="Search part of a word…" bind:value={q} aria-label="Filter history" />
  <label class="chk"><input type="checkbox" bind:checked={starred} /> Starred</label>
  {#if confirmClear}
    <button class="btn small" onclick={clear}>Clear unstarred — sure?</button>
    <button class="btn small ghost" onclick={() => (confirmClear = false)}>Cancel</button>
  {:else}
    <button class="btn small ghost" onclick={() => (confirmClear = true)}>Clear</button>
  {/if}
</div>

{#if error}<p class="err">{error}</p>{/if}
{#each groups as g, gi (g.day)}
  <h2 class="rise" style="--i:{gi}">{g.day}</h2>
  <ul class="list">
    {#each g.items as h (h.id)}
      <li animate:flip={{ duration: flipDur() }} out:leave>
        <button class="w" onclick={() => openFromHistory(h.word)}>{#each parts(h.word, q) as p}{#if p.hit}<mark class="hl">{p.t}</mark>{:else}{p.t}{/if}{/each}</button>
        <span class="muted t">{new Date(h.ts).toLocaleTimeString(undefined, { timeStyle: 'short' })} · {h.sources.join(', ')}</span>
        <button class="ic" class:on={h.starred} onclick={() => star(h)} aria-label={h.starred ? 'Unstar' : 'Star'}><Icon name={h.starred ? 'star' : 'star-o'} size={22} /></button>
        <button class="ic" onclick={() => del(h)} aria-label="Remove"><Icon name="close" size={20} /></button>
      </li>
    {/each}
  </ul>
{:else}
  <p class="empty">{q || starred ? 'Nothing matches.' : 'Words you look up will appear here.'}</p>
{/each}

<style>
  h1 { margin: 0 0 12px; font-size: 28px; }
  .tools { display: flex; gap: 12px; align-items: center; flex-wrap: wrap; }
  .tools input[type='search'] { flex: 1; min-width: 160px; }
  .chk { display: flex; gap: 6px; align-items: center; }
  .list { list-style: none; margin: 0; padding: 0; }
  .list li { display: flex; align-items: center; gap: 10px; border-bottom: 1px solid var(--line); padding: 4px 0; }
  .w { background: none; border: 0; font-size: 18px; color: var(--blue); padding: 8px 0; text-align: left; flex: 1; }
  .t { font-size: 13px; }
  mark.hl { background: color-mix(in srgb, var(--accent) 35%, transparent); color: inherit; border-radius: 2px; }
  .ic { background: none; border: 0; font-size: 20px; color: var(--muted); min-width: 40px; min-height: 40px; }
  .ic.on { color: var(--accent); }
</style>
