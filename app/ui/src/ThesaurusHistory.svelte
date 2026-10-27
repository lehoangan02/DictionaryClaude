<script lang="ts">
  import Icon from './Icon.svelte';
  import { call } from './lib/api';
  import { app, bump } from './lib/store.svelte';
  import { flip } from 'svelte/animate';
  import { item as leave, flipDur } from './lib/motion';

  // The Thesaurus tab's own history: words searched there, apart from the Dictionary's History tab.
  // `seen` changes after every search so the list re-reads; `pick` searches a word again.
  let { pick, seen }: { pick: (w: string) => void; seen: number } = $props();

  type H = { id: string; ts: number; key: string; word: string; starred: boolean };
  type M = H & { ids: string[] };
  let q = $state('');
  let starred = $state(false);
  let rows = $state<H[]>([]);
  let error = $state('');
  let confirmClear = $state(false);

  async function load() {
    try { rows = await call('history_list', { q, starred, limit: 300, scope: 'thesaurus' }); error = ''; }
    catch (e) { error = (e as Error).message; }
  }
  $effect(() => { void q; void starred; void seen; void app.version; void app.historyVersion; load(); });

  async function star(h: M) { for (const id of h.ids) await call('history_star', { id, starred: h.starred ? 0 : 1 }); await load(); }
  async function del(h: M) { for (const id of h.ids) await call('history_delete', { id }); await load(); }
  async function clear() { await call('history_clear', { scope: 'thesaurus' }); confirmClear = false; bump(); await load(); }

  // One row per run of the same word.
  const items = $derived.by(() => {
    const out: M[] = [];
    for (const h of rows) {
      const prev = out.at(-1);
      if (prev && prev.key === h.key) { prev.ids.push(h.id); prev.starred = prev.starred || h.starred; }
      else out.push({ ...h, ids: [h.id] });
    }
    return out;
  });
  const stamp = (ts: number) => new Date(ts).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });
</script>

<section class="hist" aria-label="Thesaurus history">
  <h2>History <span class="muted">· Thesaurus</span></h2>
  <div class="tools">
    <input type="search" placeholder="Search part of a word…" bind:value={q} aria-label="Filter thesaurus history" />
    <label class="chk"><input type="checkbox" bind:checked={starred} /> Starred</label>
    {#if confirmClear}
      <button class="btn small" onclick={clear}>Clear unstarred — sure?</button>
      <button class="btn small ghost" onclick={() => (confirmClear = false)}>Cancel</button>
    {:else}
      <button class="btn small ghost" onclick={() => (confirmClear = true)}>Clear</button>
    {/if}
  </div>
  {#if error}<p class="err">{error}</p>{/if}
  <ul class="list">
    {#each items as h (h.id)}
      <li animate:flip={{ duration: flipDur() }} out:leave>
        <button class="w" onclick={() => pick(h.word)}>{h.word}</button>
        <span class="muted t">{stamp(h.ts)}</span>
        <button class="ic" class:on={h.starred} onclick={() => star(h)} aria-label={h.starred ? 'Unstar' : 'Star'}><Icon name={h.starred ? 'star' : 'star-o'} size={22} /></button>
        <button class="ic" onclick={() => del(h)} aria-label="Remove"><Icon name="close" size={20} /></button>
      </li>
    {:else}
      <li class="empty">{q || starred ? 'Nothing matches.' : 'Words you search in the Thesaurus will appear here.'}</li>
    {/each}
  </ul>
</section>

<style>
  .hist { margin-top: 28px; }
  .tools { display: flex; gap: 12px; align-items: center; flex-wrap: wrap; }
  .tools input[type='search'] { flex: 1; min-width: 160px; }
  .chk { display: flex; gap: 6px; align-items: center; }
  .list { list-style: none; margin: 8px 0 0; padding: 0; }
  .list li { display: flex; align-items: center; gap: 10px; border-bottom: 1px solid var(--line); padding: 4px 0; }
  .list li.empty { display: block; border: 0; color: var(--muted); }
  .w { background: none; border: 0; font-size: 18px; color: var(--blue); padding: 8px 0; text-align: left; flex: 1; }
  .t { font-size: 13px; }
  .ic { background: none; border: 0; color: var(--muted); min-width: 40px; min-height: 40px; }
  .ic.on { color: var(--accent); }
</style>
