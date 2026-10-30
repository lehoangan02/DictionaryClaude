<script lang="ts">
  import { open } from './lib/store.svelte';
  import { REL } from './lib/format';
  import Icon from './Icon.svelte';

  // `alone`: the Thesaurus tab, where an empty answer is said out loud instead of hiding the panel,
  // `pick` decides what a tapped word does (the Dictionary opens it; the tab searches it again), and `lookup`, when
  // given, puts a second button beside each word that opens it in the main dictionary.
  let { data, word, alone = false, pick = open, lookup }: { data: any; word: string; alone?: boolean; pick?: (w: string) => void; lookup?: (w: string) => void } = $props();

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
  type Sense = { pos: string; gloss: string; synonyms: string[]; similar: string[]; broader: string[]; narrower: string[]; opposite: string[] };
  const senses: Sense[] = $derived(data?.senses ?? []);
  const GROUPS = [
    ['synonyms', 'Same meaning'],
    ['similar', 'Similar'],
    ['opposite', 'Opposite'],
    ['broader', 'Broader'],
    ['narrower', 'Narrower'],
  ] as const;
  // Dictionary synonyms the meanings above do not already list.
  const inSenses = $derived(new Set(senses.flatMap((s) => [...s.synonyms, ...s.similar])));
  const moreSyn = $derived(syn.filter((w) => !inSenses.has(w)));
  const via: { word: string; form: string } | null = $derived(data?.via ?? null);
  const greSet = $derived(new Set<string>(data?.gre_words ?? []));
  let showMoby = $state(false);
</script>

{#snippet wordChip(w: string)}
  <span class="pair">
    <button class="chip" onclick={() => pick(w)}>{w}{#if greSet.has(w)}<i class="gdot" title="GRE word (Barron's)"></i>{/if}</button>{#if lookup}<button class="go" onclick={() => lookup(w)} title="Look up “{w}” in the dictionary" aria-label="Look up {w} in the dictionary"><Icon name="book" size={15} /></button>{/if}
  </span>
{/snippet}

{#if hasNotes || gre.length || senses.length || syn.length || ant.length || moby.length}
  <section class="th rise" class:alone style="--i:2">
    <h2>{alone ? word : 'GRE thesaurus'} <span class="muted">· {alone ? 'GRE thesaurus' : word}</span></h2>
    {#if via}<p class="muted via">“{via.form}” is a form of <button class="lnk" onclick={() => pick(via.word)}>{via.word}</button>; this is its thesaurus.</p>{/if}

    {#if hasNotes}
      <div class="box mine rise" style="--i:0">
        <h3>From your notes</h3>
        {#each noteGroups as g (g.k)}
          <p class="row">
            <span class="sym" title={REL[g.k].label}>{REL[g.k].sym}</span>
            {#each g.items as w (w)}{@render wordChip(w)}{/each}
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
              <span class="wg"><button class="w" onclick={() => pick(g.word)}>{g.word}</button>
              {#if lookup}<button class="go" onclick={() => lookup(g.word)} title="Look up “{g.word}” in the dictionary" aria-label="Look up {g.word} in the dictionary"><Icon name="book" size={15} /></button>{/if}</span>
              <span class="u">unit {g.unit}</span>
              <span class="d">{g.definition}</span>
            </li>
          {/each}
        </ul>
      </div>
    {/if}

    {#if senses.length}
      <div class="box rise" style="--i:2">
        <h3>By meaning <span class="muted">· WordNet</span></h3>
        {#each senses as s, i (i)}
          <div class="sense">
            <p class="gloss"><span class="pos">{s.pos}</span> {s.gloss}</p>
            {#each GROUPS as [k, label] (k)}
              {#if s[k].length}
                <p class="row"><span class="lab">{label}</span>{#each s[k] as w (w)}{@render wordChip(w)}{/each}</p>
              {/if}
            {/each}
          </div>
        {/each}
      </div>
    {/if}

    {#if moreSyn.length || ant.length}
      <div class="box rise" style="--i:2">
        <h3>{senses.length ? 'More from the dictionary' : 'Dictionary synonyms'}</h3>
        {#if moreSyn.length}<p class="row"><span class="sym" title="Synonyms">≈</span>
          {#each moreSyn as w (w)}{@render wordChip(w)}{/each}</p>{/if}
        {#if ant.length}<p class="row"><span class="sym" title="Opposite">&gt;&lt;</span>
          {#each ant as w (w)}{@render wordChip(w)}{/each}</p>{/if}
      </div>
    {/if}

    {#if moby.length}
      <div class="box rise" style="--i:3">
        <h3>Loosely related <span class="muted">· Moby thesaurus, best matches first</span></h3>
        <p class="row">
          {#each showMoby ? moby : moby.slice(0, 12) as w (w)}{@render wordChip(w)}{/each}
          {#if !showMoby && moby.length > 12}<button class="btn small" onclick={() => (showMoby = true)}>+{moby.length - 12}</button>{/if}
        </p>
      </div>
    {/if}
  </section>
{:else if alone}
  <p class="muted">No synonyms, opposites or related GRE words found for “{word}”.</p>
{/if}

<style>
  .th { margin-top: 28px; }
  .th.alone { margin-top: 12px; }
  h2 { margin-bottom: 8px; }
  h3 { margin: 0 0 6px; font-size: 14px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); }
  .box { background: var(--panel); border: 1px solid var(--line); border-radius: 10px; padding: 10px 14px; margin: 10px 0; }
  .box.mine { border-left: 5px solid var(--accent); }
  .row { margin: 4px 0; display: flex; flex-wrap: wrap; align-items: center; }
  .sym {
    display: inline-grid; place-items: center; min-width: 34px; height: 28px; margin-right: 8px; border-radius: 6px;
    background: var(--navy); color: #fff; font-weight: 700; font-size: 14px;
  }
  .sense { padding: 6px 0; border-top: 1px solid var(--line); }
  .sense:first-of-type { border-top: 0; }
  .gloss { margin: 0 0 2px; font-size: 14px; }
  .pos { font-size: 12px; font-style: italic; color: var(--muted); margin-right: 4px; }
  .lab { min-width: 78px; margin-right: 8px; font-size: 12px; color: var(--muted); text-transform: uppercase; letter-spacing: 0.05em; }
  .via { margin: 0 0 6px; font-size: 14px; }
  .lnk { background: none; border: 0; padding: 0; font: inherit; font-weight: 700; color: var(--blue); }
  .gdot { display: inline-block; width: 7px; height: 7px; margin-left: 6px; border-radius: 50%; background: var(--accent); vertical-align: middle; }
  .pair { display: inline-flex; align-items: center; margin: 2px 4px 2px 0; }
  .pair .chip { margin: 0; }
  .pair .chip:not(:last-child) { border-top-right-radius: 0; border-bottom-right-radius: 0; border-right: 0; }
  .go { display: inline-grid; place-items: center; width: 28px; height: 28px; padding: 0; background: var(--bg); color: var(--muted); border: 1px solid var(--line); border-radius: 0 999px 999px 0; }
  .go:hover { background: var(--panel-2); color: var(--blue); }
  .wg { display: inline-flex; align-items: center; gap: 8px; }
  .wg .go { border-radius: 50%; }
  .gl { list-style: none; margin: 0; padding: 0; }
  .gl li { padding: 5px 0; border-top: 1px solid var(--line); display: grid; grid-template-columns: auto auto 1fr; gap: 4px 10px; align-items: baseline; }
  .gl li:first-child { border-top: 0; }
  .w { background: none; border: 0; padding: 0; font-weight: 700; color: var(--blue); font-size: 16px; }
  .u { font-size: 12px; color: var(--muted); }
  .d { color: var(--text); grid-column: 1 / -1; font-size: 14px; }
  @media (min-width: 640px) { .d { grid-column: auto; } }
</style>
