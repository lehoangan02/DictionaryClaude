<script lang="ts">
  import { open } from './lib/store.svelte';
  import { REL } from './lib/format';

  let { data, word }: { data: any; word: string } = $props();

  const noteGroups = $derived(
    (['same', 'similar', 'opposite', 'different'] as const)
      .map((k) => ({ k, items: (data?.notes?.[k] ?? []) as string[] }))
      .filter((g) => g.items.length),
  );
  const hasNotes = $derived(noteGroups.length > 0);
  const gre: any[] = $derived(data?.barrons ?? []);
  const syn: string[] = $derived(data?.general?.synonyms ?? []);
  const ant: string[] = $derived(data?.general?.antonyms ?? []);
  const moby: string[] = $derived(data?.moby ?? []);
  let showMoby = $state(false);
</script>

{#if hasNotes || gre.length || syn.length || ant.length || moby.length}
  <section class="th rise" style="--i:2">
    <h2>GRE thesaurus <span class="muted">· {word}</span></h2>

    {#if hasNotes}
      <div class="box mine rise" style="--i:0">
        <h3>From your notes</h3>
        {#each noteGroups as g (g.k)}
          <p class="row">
            <span class="sym" title={REL[g.k].label}>{REL[g.k].sym}</span>
            {#each g.items as w (w)}<button class="chip" onclick={() => open(w)}>{w}</button>{/each}
          </p>
        {/each}
      </div>
    {/if}

    {#if gre.length}
      <div class="box rise" style="--i:1">
        <h3>Related GRE words <span class="muted">· Barron's</span></h3>
        <ul class="gl">
          {#each gre as g (g.key)}
            <li>
              <button class="w" onclick={() => open(g.word)}>{g.word}</button>
              <span class="u">unit {g.unit}</span>
              <span class="d">{g.definition}</span>
            </li>
          {/each}
        </ul>
      </div>
    {/if}

    {#if syn.length || ant.length}
      <div class="box rise" style="--i:2">
        <h3>Dictionary synonyms</h3>
        {#if syn.length}<p class="row"><span class="sym" title="Synonyms">≈</span>
          {#each syn as w (w)}<button class="chip" onclick={() => open(w)}>{w}</button>{/each}</p>{/if}
        {#if ant.length}<p class="row"><span class="sym" title="Opposite">&gt;&lt;</span>
          {#each ant as w (w)}<button class="chip" onclick={() => open(w)}>{w}</button>{/each}</p>{/if}
      </div>
    {/if}

    {#if moby.length}
      <div class="box rise" style="--i:3">
        <h3>More words <span class="muted">· Moby thesaurus</span></h3>
        <p class="row">
          {#each showMoby ? moby : moby.slice(0, 12) as w (w)}<button class="chip" onclick={() => open(w)}>{w}</button>{/each}
          {#if !showMoby && moby.length > 12}<button class="btn small" onclick={() => (showMoby = true)}>+{moby.length - 12}</button>{/if}
        </p>
      </div>
    {/if}
  </section>
{/if}

<style>
  .th { margin-top: 28px; }
  h2 { margin-bottom: 8px; }
  h3 { margin: 0 0 6px; font-size: 14px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); }
  .box { background: var(--panel); border: 1px solid var(--line); border-radius: 10px; padding: 10px 14px; margin: 10px 0; }
  .box.mine { border-left: 5px solid var(--accent); }
  .row { margin: 4px 0; display: flex; flex-wrap: wrap; align-items: center; }
  .sym {
    display: inline-grid; place-items: center; min-width: 34px; height: 28px; margin-right: 8px; border-radius: 6px;
    background: var(--navy); color: #fff; font-weight: 700; font-size: 14px;
  }
  .gl { list-style: none; margin: 0; padding: 0; }
  .gl li { padding: 5px 0; border-top: 1px solid var(--line); display: grid; grid-template-columns: auto auto 1fr; gap: 4px 10px; align-items: baseline; }
  .gl li:first-child { border-top: 0; }
  .w { background: none; border: 0; padding: 0; font-weight: 700; color: var(--blue); font-size: 16px; }
  .u { font-size: 12px; color: var(--muted); }
  .d { color: var(--text); grid-column: 1 / -1; font-size: 14px; }
  @media (min-width: 640px) { .d { grid-column: auto; } }
</style>
