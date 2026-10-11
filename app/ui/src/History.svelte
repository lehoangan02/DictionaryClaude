<script lang="ts">
  import { call } from './lib/api';
  import { app, open, bump } from './lib/store.svelte';
  import { when } from './lib/format';

  type H = { id: string; ts: number; key: string; word: string; source: string; starred: boolean };
  let q = $state('');
  let starred = $state(false);
  let rows = $state<H[]>([]);
  let error = $state('');

  async function load() {
    try { rows = await call('history_list', { q, starred, limit: 300 }); error = ''; }
    catch (e) { error = (e as Error).message; }
  }
  $effect(() => { void q; void starred; void app.version; load(); });

  async function star(h: H) { await call('history_star', { id: h.id, starred: h.starred ? 0 : 1 }); await load(); }
  async function del(h: H) { await call('history_delete', { id: h.id }); await load(); }
  let confirmClear = $state(false);
  async function clear() { await call('history_clear'); confirmClear = false; bump(); await load(); }

  // Group by day like a browser history.
  const groups = $derived.by(() => {
    const out: { day: string; items: H[] }[] = [];
    for (const h of rows) {
      const day = new Date(h.ts).toLocaleDateString(undefined, { weekday: 'long', month: 'long', day: 'numeric' });
      if (out.at(-1)?.day === day) out.at(-1)!.items.push(h); else out.push({ day, items: [h] });
    }
    return out;
  });
</script>

<h1>History</h1>
<div class="tools">
  <input type="search" placeholder="Filter by word…" bind:value={q} aria-label="Filter history" />
  <label class="chk"><input type="checkbox" bind:checked={starred} /> Starred</label>
  {#if confirmClear}
    <button class="btn small" onclick={clear}>Clear unstarred — sure?</button>
    <button class="btn small ghost" onclick={() => (confirmClear = false)}>Cancel</button>
  {:else}
    <button class="btn small ghost" onclick={() => (confirmClear = true)}>Clear</button>
  {/if}
</div>

{#if error}<p class="err">{error}</p>{/if}
{#each groups as g (g.day)}
  <h2>{g.day}</h2>
  <ul class="list">
    {#each g.items as h (h.id)}
      <li>
        <button class="w" onclick={() => open(h.word)}>{h.word}</button>
        <span class="muted t">{new Date(h.ts).toLocaleTimeString(undefined, { timeStyle: 'short' })} · {h.source}</span>
        <button class="ic" class:on={h.starred} onclick={() => star(h)} aria-label={h.starred ? 'Unstar' : 'Star'}>{h.starred ? '★' : '☆'}</button>
        <button class="ic" onclick={() => del(h)} aria-label="Remove">✕</button>
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
  .ic { background: none; border: 0; font-size: 20px; color: var(--muted); min-width: 40px; min-height: 40px; }
  .ic.on { color: var(--accent); }
</style>
