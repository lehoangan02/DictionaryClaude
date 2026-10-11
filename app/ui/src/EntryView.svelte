<script lang="ts">
  import { call } from './lib/api';
  import { app, open, setSource, bump } from './lib/store.svelte';
  import { posLabel, pronunciations, speak } from './lib/format';
  import Thesaurus from './Thesaurus.svelte';

  let data = $state<any>(null);
  let loading = $state(false);
  let error = $state('');
  let did = $state<string[]>([]);
  let showAllEtym = $state(false);
  let seq = 0;

  $effect(() => {
    const word = app.word, source = app.source;
    void app.version; // refresh when notes change (the thesaurus panel reads them)
    if (!word) { data = null; return; }
    const mine = ++seq;
    loading = true; error = '';
    call('entry', { word, source, record: true })
      .then(async (d) => {
        if (mine !== seq) return;
        data = d; did = [];
        if (!d.entry) did = (await call('search', { q: word, source })).did_you_mean;
      })
      .catch((e) => { if (mine === seq) error = e.message; })
      .finally(() => { if (mine === seq) loading = false; });
  });

  const entry = $derived(data?.entry);
  const prons = $derived(entry ? pronunciations(entry.blocks) : []);
  const etyms = $derived(
    entry ? [...new Set<string>(entry.blocks.map((b: any) => b.etymology).filter(Boolean))] : [],
  );
  const best = (id: string) => data?.found_in.includes(id);

  // Short, modern sentences first; dated quotations stay one click away.
  type Ex = { text: string; by?: string };
  function split(all: Ex[]) {
    const plain = all.filter((e) => !e.by).sort((a, b) => a.text.length - b.text.length);
    const quotes = all.filter((e) => e.by).sort((a, b) => a.text.length - b.text.length);
    const ordered = [...plain, ...quotes];
    return { shown: ordered.slice(0, 2), more: ordered.slice(2) };
  }
</script>

{#if !app.word}
  <section class="home">
    <h1>Look up any English word</h1>
    <p class="muted">
      {app.sources.map((s) => `${s.name} (${s.entries.toLocaleString()})`).join(' · ')} — fully offline.
    </p>
    <p class="muted">Try <button class="chip" onclick={() => open('loquacious')}>loquacious</button>
      <button class="chip" onclick={() => open('abate')}>abate</button>
      <button class="chip" onclick={() => open('taciturn')}>taciturn</button></p>
  </section>
{:else if error}
  <p class="err">{error}</p>
{:else if data}
  <article class:dim={loading}>
    <div class="srcs" role="tablist" aria-label="Dictionary source">
      {#each app.sources as s (s.id)}
        <button role="tab" aria-selected={app.source === s.id} class:on={app.source === s.id}
          class:missing={!best(s.id)} onclick={() => setSource(s.id)}
          title={best(s.id) ? '' : 'Not in this dictionary'}>{s.name}</button>
      {/each}
    </div>

    {#if !entry}
      <h1 class="hw">{app.word}</h1>
      {#if data.mw_error}
        <p class="err">{data.mw_error}</p>
      {:else}
        <p>No entry found for “{app.word}”.</p>
      {/if}
      {#if data.did_you_mean?.length}
        <p>Did you mean {#each data.did_you_mean as w (w)}<button class="chip" onclick={() => open(w)}>{w}</button>{/each}</p>
      {/if}
      {#if did.length}
        <p>Did you mean {#each did as w (w)}<button class="chip" onclick={() => open(w)}>{w}</button>{/each}</p>
      {/if}
    {:else}
      {#if data.source.id !== app.source}
        <p class="note">Not in {app.sources.find((s) => s.id === app.source)?.name}; showing {data.source.name}.</p>
      {/if}
      <header class="head">
        <h1 class="hw">{entry.word}</h1>
        <div class="posrow">
          {#each [...new Set<string>(entry.blocks.map((b: any) => b.pos))] as p (p)}
            <span class="pos">{posLabel(p)}</span>
          {/each}
        </div>
        <div class="prons">
          {#each prons as p (p.accent + p.ipa)}
            <button class="pron" onclick={() => speak(entry.word, p.audio)} aria-label="Pronounce {p.accent}">
              <span class="spk" aria-hidden="true">🔊</span>
              {#if p.accent}<b>{p.accent}</b>{/if}
              {#if p.ipa}<span class="ipa">{p.ipa}</span>{/if}
            </button>
          {/each}
          {#if !prons.length}
            <button class="pron" onclick={() => speak(entry.word)} aria-label="Pronounce">
              <span class="spk" aria-hidden="true">🔊</span>
              {#if data.ipa_fallback}<b>US</b><span class="ipa">{data.ipa_fallback}</span>{/if}
            </button>
          {/if}
        </div>
      </header>

      {#if data.barrons}
        <aside class="gre">
          <div class="gre-h"><b>Barron's Essential Words for the GRE</b>
            {#if data.barrons[0].high_freq}<span class="hf">High-frequency</span>{/if}
            <span class="muted">Unit {data.barrons[0].unit}</span></div>
          {#each data.barrons as b}
            <p><i>{posLabel(b.pos.replace('.', ''))}</i> {b.definition}</p>
            {#each (b.examples ?? []).slice(0, 2) as ex}<p class="ex">{ex}</p>{/each}
          {/each}
          <button class="btn small" onclick={() => { app.quizUnit = data.barrons[0].unit; app.tab = 'quiz'; }}>Practice unit {data.barrons[0].unit} →</button>
        </aside>
      {/if}

      {#each data.roots ?? [] as r}
        <aside class="roots">
          <div class="rh"><b>Word root</b> <span class="rf">{r.forms}</span> <span class="muted">“{r.meaning}” · {r.lang}</span></div>
          <div class="rw">
            {#each r.words as w (w.word)}
              <button class="chip" class:me={w.key === data.key} onclick={() => open(w.word)} title={w.definition}>{w.word}</button>
            {/each}
          </div>
        </aside>
      {/each}

      {#each entry.blocks as b, bi}
        <section class="block">
          <h2 class="bh">
            {b.word ?? entry.word} <span class="pos">{posLabel(b.pos)}</span>
          </h2>
          {#if b.head}<p class="head-line">{b.head}</p>{/if}
          <ol class="senses">
            {#each b.senses as s}
              <li>
                {#each s.parents ?? [] as par}<span class="parent">{par}</span>{/each}
                {#if s.tags?.length}<span class="tags">({s.tags.slice(0, 3).join(', ')})</span>{/if}
                <span class="def">{s.gloss}</span>
                {#if s.form_of}<button class="chip" onclick={() => open(s.form_of)}>{s.form_of} →</button>{/if}
                {#if s.examples?.length}
                  {@const ex = split(s.examples)}
                  <ul class="exs">
                    {#each ex.shown as e}<li>{e.text}{#if e.by}<span class="by"> — {e.by}</span>{/if}</li>{/each}
                  </ul>
                  {#if ex.more.length}
                    <details class="quotes">
                      <summary>{ex.more.length} more example{ex.more.length > 1 ? 's' : ''}</summary>
                      <ul class="exs">
                        {#each ex.more as e}<li>{e.text}{#if e.by}<span class="by"> — {e.by}</span>{/if}</li>{/each}
                      </ul>
                    </details>
                  {/if}
                {/if}
                {#if s.synonyms?.length}
                  <p class="rel"><b>Synonyms</b>
                    {#each s.synonyms.slice(0, 8) as w}<button class="chip" onclick={() => open(w)}>{w}</button>{/each}</p>
                {/if}
                {#if s.antonyms?.length}
                  <p class="rel"><b>Opposite</b>
                    {#each s.antonyms.slice(0, 6) as w}<button class="chip" onclick={() => open(w)}>{w}</button>{/each}</p>
                {/if}
              </li>
            {/each}
          </ol>
          {#if b.forms?.length && bi === 0}
            <p class="forms">{#each b.forms.slice(0, 8) as f}<span>{f.form}{#if f.tags?.length} <em>({f.tags.join(', ')})</em>{/if}</span>{/each}</p>
          {/if}
        </section>
      {/each}

      {#if data.source.id === 'mw'}
        <p class="attr">
          <img src="./mw-logo.png" alt="Merriam-Webster" height="28" onerror={(e) => ((e.currentTarget as HTMLElement).style.display = 'none')} />
          Definitions from <b>Merriam-Webster's Collegiate® Dictionary</b> and <b>Thesaurus</b>, © Merriam-Webster, Inc.
        </p>
      {/if}

      <Thesaurus data={data.thesaurus} word={entry.word} />

      {#if etyms.length}
        <details class="origin">
          <summary>Word origin</summary>
          {#each showAllEtym ? etyms : etyms.slice(0, 1) as e}<p>{e}</p>{/each}
          {#if etyms.length > 1 && !showAllEtym}<button class="btn small" onclick={() => (showAllEtym = true)}>Show {etyms.length - 1} more</button>{/if}
        </details>
      {/if}
    {/if}
  </article>
{:else if loading}
  <p class="empty">Looking up “{app.word}”…</p>
{/if}

<style>
  .home { text-align: center; padding: 56px 0 24px; }
  h1 { font-size: 30px; margin: 0 0 8px; }
  .dim { opacity: 0.55; transition: opacity 0.15s; }
  .srcs { display: flex; gap: 4px; border-bottom: 1px solid var(--line); margin-bottom: 14px; overflow-x: auto; }
  .srcs button {
    background: none; border: 0; padding: 8px 14px; border-bottom: 3px solid transparent; color: var(--muted); white-space: nowrap;
  }
  .srcs button.on { color: var(--blue); border-bottom-color: var(--blue); font-weight: 600; }
  .srcs button.missing:not(.on) { opacity: 0.5; }
  .note { background: var(--panel); border-left: 4px solid var(--accent); padding: 8px 12px; border-radius: 4px; }
  .hw { font-size: 40px; color: var(--blue); margin: 4px 0 0; line-height: 1.15; word-break: break-word; }
  .posrow { margin: 2px 0 6px; }
  .pos { font-style: italic; color: var(--muted); font-size: 15px; margin-right: 8px; }
  .prons { display: flex; gap: 10px; flex-wrap: wrap; margin: 6px 0 14px; }
  .pron {
    display: inline-flex; gap: 8px; align-items: center; background: var(--panel); border: 1px solid var(--line);
    border-radius: 999px; padding: 6px 14px; min-height: 40px;
  }
  .pron b { font-size: 12px; color: var(--muted); }
  .ipa { font-family: 'Noto Sans', 'DejaVu Sans', system-ui, sans-serif; }
  .gre { background: var(--panel); border-left: 5px solid var(--accent); border-radius: 6px; padding: 10px 16px; margin: 10px 0 18px; }
  .gre-h { display: flex; gap: 10px; align-items: baseline; flex-wrap: wrap; margin-bottom: 4px; }
  .hf { font-size: 11px; font-weight: 700; color: #fff; background: var(--accent); border-radius: 4px; padding: 1px 7px; }
  .gre p { margin: 4px 0; }
  .ex { font-style: italic; color: var(--muted); }
  .roots { background: var(--panel); border-left: 5px solid var(--blue); border-radius: 6px; padding: 8px 16px; margin: 0 0 14px; }
  .rh { display: flex; gap: 10px; align-items: baseline; flex-wrap: wrap; }
  .rf { font-weight: 700; color: var(--blue); letter-spacing: 0.04em; }
  .rw { margin-top: 4px; }
  .chip.me { background: var(--blue); color: #fff; border-color: var(--blue); }
  .block { margin-top: 22px; }
  .bh { margin: 0 0 4px; font-size: 24px; color: var(--navy); }
  :global([data-theme='dark']) .bh { color: var(--text); }
  .head-line { color: var(--muted); margin: 0 0 8px; }
  .senses { padding-left: 1.4em; margin: 0; }
  .senses > li { margin: 0 0 14px; padding-left: 4px; }
  .senses > li::marker { font-weight: 700; color: var(--blue); }
  .parent { color: var(--muted); font-size: 14px; margin-right: 6px; }
  .parent::after { content: ' ›'; }
  .tags { color: var(--muted); font-style: italic; margin-right: 6px; font-size: 14px; }
  .def { font-size: 17px; }
  .exs { list-style: none; margin: 4px 0 0; padding: 0; }
  .exs li { position: relative; padding-left: 16px; margin: 3px 0; font-style: italic; color: var(--muted); }
  .exs li::before { content: ''; position: absolute; left: 3px; top: 0.7em; width: 6px; height: 6px; border-radius: 50%; background: var(--blue); }
  .by { font-style: normal; font-size: 13px; }
  .quotes { margin: 2px 0 0 4px; font-size: 14px; }
  .quotes summary { cursor: pointer; color: var(--blue); }
  .rel { margin: 6px 0 0; font-size: 14px; }
  .rel b { color: var(--muted); font-size: 12px; text-transform: uppercase; letter-spacing: 0.05em; margin-right: 6px; }
  .forms { color: var(--muted); font-size: 14px; display: flex; flex-wrap: wrap; gap: 4px 16px; }
  .forms em { font-style: normal; font-size: 12px; }
  .attr { display: flex; gap: 10px; align-items: center; flex-wrap: wrap; color: var(--muted); font-size: 13px; margin: 18px 0 0; }
  .origin { margin: 24px 0; background: var(--panel); border-radius: 8px; padding: 10px 14px; }
  .origin summary { cursor: pointer; font-weight: 600; }
  @media (max-width: 720px) { .hw { font-size: 34px; } }
</style>
