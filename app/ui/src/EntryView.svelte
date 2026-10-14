<script lang="ts">
  let noRec = $state(false);
  let noRecWhy = $state('');
  import { call } from './lib/api';
  import { app, open, goTab, setSource, orderedSources, markMissing, isLookup, syncTrail } from './lib/store.svelte';
  import { highlight, posLabel, pronunciations, speak, lastSpeakError } from './lib/format';
  import Thesaurus from './Thesaurus.svelte';
  import HistoryNav from './HistoryNav.svelte';
  import Icon from './Icon.svelte';
  import Disclosure from './Disclosure.svelte';
  import { enter } from './lib/motion';

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
    const record = isLookup(word, source);
    call('entry', { word, source, record })
      .then(async (d) => {
        if (mine !== seq) return;
        data = d; did = [];
        const missing = !d.entry && !d.found_in?.length && !d.mw_error;
        markMissing(word, missing);
        if (record && !missing) await syncTrail(word);
        if (mine !== seq) return;
        if (!d.entry) did = (await call('search', { q: word, source: source === 'mw' ? undefined : source })).did_you_mean;
      })
      .catch((e) => { if (mine === seq) error = e.message; })
      .finally(() => { if (mine === seq) loading = false; });
  });

  const entry = $derived(data?.entry);
  const prons = $derived(entry ? pronunciations(entry.blocks) : []);
  // Packs built before the fix can hold a placeholder origin (a row of dots): only text with letters counts.
  const etyms = $derived(
    entry ? [...new Set<string>(entry.blocks.map((b: any) => b.etymology).filter((e: any) => e && /\p{L}/u.test(e)))] : [],
  );
  const tabs = $derived((void app.orderVersion, orderedSources().visible));
  // A dictionary without the word is greyed out and cannot be opened; Merriam-Webster is a live lookup, so it is always allowed.
  const has = (id: string) => id === 'mw' || !!data?.found_in.includes(id);
  const shownId = $derived<string>(data?.source?.id ?? app.source);

  // Short, modern sentences first; dated quotations stay one click away.
  type Ex = { text: string; by?: string };
  function split(all: Ex[]) {
    const plain = all.filter((e) => !e.by).sort((a, b) => a.text.length - b.text.length);
    const quotes = all.filter((e) => e.by).sort((a, b) => a.text.length - b.text.length);
    const ordered = [...plain, ...quotes];
    return { shown: ordered.slice(0, 2), more: ordered.slice(2) };
  }
  const accentColor = (a: string) => (a === 'US' ? 'us' : 'uk');
  // Content below the dictionary switch is re-entered whenever the word or dictionary changes.
  const contentKey = $derived(`${data?.key}|${data?.source?.id}`);
</script>

{#if !app.word}
  <HistoryNav />
  <section class="home">
    <h1>Look up any English word</h1>
    <p class="muted">
      {app.sources.filter((s) => s.entries != null).map((s) => `${s.name} (${s.entries!.toLocaleString()})`).join(' · ')} — fully offline{app.sources.some((s) => s.entries == null) ? '; Merriam-Webster looks words up online and keeps them' : ''}.
    </p>
    <p class="muted">Try <button class="chip" onclick={() => open('loquacious')}>loquacious</button>
      <button class="chip" onclick={() => open('abate')}>abate</button>
      <button class="chip" onclick={() => open('taciturn')}>taciturn</button></p>
  </section>
{:else if error}
  <p class="err">{error}</p>
{:else if data}
  <HistoryNav />
  <article class:dim={loading}>
    <div class="srcs" role="tablist" aria-label="Dictionary source">
      {#each tabs as s (s.id)}
        <button role="tab" aria-selected={shownId === s.id} class:on={shownId === s.id}
          class:missing={!has(s.id)} disabled={!has(s.id)} onclick={() => setSource(s.id)}
          title={has(s.id) ? '' : `${s.name} does not have this word`}>{s.name}</button>
      {/each}
    </div>

    {#key contentKey}
    <div class="body" in:enter={{ y: 12, duration: 280 }}>
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
      <header class="head">
        <div class="hwrow">
          <h1 class="hw">{data.syl ?? entry.word}</h1>
          <span class="posrow">
            {#each [...new Set<string>(entry.blocks.map((b: any) => b.pos))] as p, i (p)}{i ? ', ' : ''}{posLabel(p)}{/each}
          </span>
        </div>
        <div class="prons">
          {#each prons as p (p.accent + p.ipa)}
            <button class="pron {accentColor(p.accent)}" onclick={async () => { noRec = (await speak(entry.word, p.audio, p.accent)) === 'voice' && !!p.audio?.startsWith('oald:'); noRecWhy = lastSpeakError; }} aria-label="Pronounce{p.accent ? ' (' + p.accent + ')' : ''}">
              <Icon name="speaker" size={26} />
              {#if p.accent}<b>{p.accent}</b>{/if}
              {#if p.ipa}<span class="ipa">{p.ipa.startsWith('/') || p.ipa.startsWith('\\') ? p.ipa : '/' + p.ipa + '/'}</span>{/if}
            </button>
          {/each}
          {#if !prons.length}
            <button class="pron uk" onclick={() => speak(entry.word)} aria-label="Pronounce">
              <Icon name="speaker" size={26} />
              {#if data.ipa_fallback}<b>US</b><span class="ipa">{data.ipa_fallback}</span>{/if}
            </button>
          {/if}
        </div>
        {#if noRec}<p class="muted small">This is the system voice: the Oxford recording could not be played{noRecWhy ? ` (${noRecWhy})` : ''}. If the recordings are not on this device, download "OALD recordings" in Settings → Downloads.</p>{/if}
      </header>

      {#if data.barrons}
        <aside class="gre rise">
          <div class="gre-h"><b>Barron's Essential Words for the GRE</b>
            {#if data.barrons[0].high_freq}<span class="hf">High-frequency</span>{/if}
            <span class="muted">Unit {data.barrons[0].unit}</span></div>
          {#each data.barrons as b}
            <p><i>{posLabel(b.pos.replace('.', ''))}</i> {b.definition}</p>
            {#each (b.examples ?? []).slice(0, 2) as ex}<p class="ex">{@render hl(ex, entry.word)}</p>{/each}
          {/each}
          <button class="btn small" onclick={() => { app.quizUnit = data.barrons[0].unit; goTab('quiz'); }}>Practice unit {data.barrons[0].unit} →</button>
        </aside>
      {/if}

      {#each data.roots ?? [] as r}
        <aside class="roots rise" style="--i:1">
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
          {#if entry.blocks.length > 1 || b.word}
            <h2 class="bh">{b.word ?? entry.word} <span class="bpos">{posLabel(b.pos)}</span></h2>
          {/if}
          {#if b.head}<p class="head-line">{b.head}</p>{/if}
          <ol class="senses" class:single={b.senses.length === 1}>
            {#each b.senses as s, si}
              <li class="rise" style="--i:{si}">
                {#if s.tags?.length}<p class="lab">{s.tags.slice(0, 3).join(', ')}</p>{/if}
                <p class="def">
                  {#each s.parents ?? [] as par}<span class="parent">{par} ›</span>{/each}
                  {s.gloss}
                  {#if s.form_of}<button class="chip" onclick={() => open(s.form_of)}>{s.form_of} →</button>{/if}
                </p>
                {#if s.synonyms?.length}
                  <p class="rel"><span class="k">SYNONYM{s.synonyms.length > 1 ? 'S' : ''}</span>
                    {#each s.synonyms.slice(0, 8) as w, i}{i ? ', ' : ''}<button class="lk" onclick={() => open(w)}>{w}</button>{/each}</p>
                {/if}
                {#if s.antonyms?.length}
                  <p class="rel"><span class="k">OPPOSITE</span>
                    {#each s.antonyms.slice(0, 6) as w, i}{i ? ', ' : ''}<button class="lk" onclick={() => open(w)}>{w}</button>{/each}</p>
                {/if}
                {#if s.examples?.length}
                  {@const ex = split(s.examples)}
                  <ul class="exs">
                    {#each ex.shown as e}<li>{@render hl(e.text, entry.word)}{#if e.by}<span class="by"> — {e.by}</span>{/if}</li>{/each}
                  </ul>
                  {#if ex.more.length}
                    <div class="quotes">
                      <Disclosure label="{ex.more.length} more example{ex.more.length > 1 ? 's' : ''}">
                        <ul class="exs">
                          {#each ex.more as e}<li>{@render hl(e.text, entry.word)}{#if e.by}<span class="by"> — {e.by}</span>{/if}</li>{/each}
                        </ul>
                      </Disclosure>
                    </div>
                  {/if}
                {/if}
              </li>
            {/each}
          </ol>
          {#if b.forms?.length && bi === 0}
            <p class="forms">{#each b.forms.slice(0, 8) as f}<span>{f.form}{#if f.tags?.length} <em>({f.tags.join(', ')})</em>{/if}</span>{/each}</p>
          {/if}
        </section>
      {/each}

      {#if etyms.length}
        <div class="origin">
          <Disclosure label="Word Origin" boxed>
            <div class="ob">
              {#each showAllEtym ? etyms : etyms.slice(0, 1) as e}<p>{e}</p>{/each}
              {#if etyms.length > 1 && !showAllEtym}<button class="btn small" onclick={() => (showAllEtym = true)}>Show {etyms.length - 1} more</button>{/if}
            </div>
          </Disclosure>
        </div>
      {/if}

      {#if data.source.id === 'mw'}
        <p class="attr">
          <img class="mwl light" src="./mw-logo-light.png" alt="Merriam-Webster" width="50" height="50" />
          <img class="mwl dark" src="./mw-logo-dark.png" alt="" aria-hidden="true" width="50" height="50" />
          Definitions from <b>Merriam-Webster's Collegiate® Dictionary</b> and <b>Thesaurus</b>, © Merriam-Webster, Inc.
        </p>
      {/if}

      <Thesaurus data={data.thesaurus} word={entry.word} />
    {/if}
    </div>
    {/key}
  </article>
{:else if loading}
  <HistoryNav />
  <div aria-busy="true" aria-label="Looking up {app.word}">
    <div class="skel" style="height:40px;width:50%;margin:10px 0 14px"></div>
    <div class="skel" style="height:26px;width:35%;margin:0 0 22px"></div>
    <div class="skel" style="height:20px;width:92%;margin:0 0 10px"></div>
    <div class="skel" style="height:20px;width:78%;margin:0 0 10px"></div>
    <div class="skel" style="height:20px;width:64%"></div>
  </div>
{/if}

{#snippet hl(text: string, word: string)}{#each highlight(text, word) as p}{#if p.hit}<mark class="hl">{p.t}</mark>{:else}{p.t}{/if}{/each}{/snippet}

<style>
  .hl { background: color-mix(in srgb, var(--accent) 28%, transparent); color: inherit; border-radius: 3px; padding: 0 2px; font-weight: 600; }
  .home { text-align: center; padding: 56px 0 24px; }
  h1 { font-size: 30px; margin: 0 0 8px; }
  .dim { opacity: 0.55; transition: opacity 0.15s; }

  /* Dictionary switch: a small floating segmented control. */
  .srcs { display: inline-flex; gap: 2px; background: var(--surface); border: 1px solid var(--line); border-radius: 999px; padding: 3px; box-shadow: var(--lift); margin: 0 0 18px; max-width: 100%; overflow-x: auto; }
  .srcs button { background: none; border: 0; padding: 6px 14px; border-radius: 999px; color: var(--muted); white-space: nowrap; font-size: 14px; }
  .srcs button:hover { color: var(--blue); }
  .srcs button.on { background: var(--blue); color: #fff; font-weight: 600; }
  .srcs button.missing { opacity: 0.38; cursor: not-allowed; text-decoration: line-through; text-decoration-color: color-mix(in srgb, var(--muted) 50%, transparent); }
  .srcs button.missing:hover { color: var(--muted); }
  .note { background: var(--panel); border-left: 4px solid var(--accent); padding: 8px 12px; border-radius: 4px; }

  .head { margin-bottom: 18px; }
  .hwrow { display: flex; align-items: baseline; gap: 14px; flex-wrap: wrap; }
  .hw { font-size: 44px; color: var(--head); margin: 0; line-height: 1.12; word-break: break-word; font-weight: 700; letter-spacing: 0.2px; }
  .posrow { font-style: italic; font-size: 24px; color: var(--text); }
  .prons { display: flex; flex-direction: column; gap: 4px; margin: 12px 0 4px; align-items: flex-start; }
  .pron { display: inline-flex; gap: 10px; align-items: center; background: none; border: 0; padding: 4px 6px 4px 0; border-radius: 10px; color: var(--text); min-height: 44px; }
  .pron:hover { background: var(--panel); }
  .pron.uk :global(svg) { color: #1f3a93; } .pron.us :global(svg) { color: #c8285a; }
  :global([data-theme='dark']) .pron.uk :global(svg) { color: #7da2ff; }
  .pron b { font-size: 12px; color: var(--muted); font-weight: 600; }
  .ipa { font-family: 'Charis SIL', 'Gentium Plus', 'DejaVu Serif', Georgia, 'Times New Roman', serif; font-size: 22px; }

  .gre { background: var(--salmon); border-left: 5px solid var(--accent); border-radius: 6px; padding: 10px 16px; margin: 10px 0 18px; }
  .gre-h { display: flex; gap: 10px; align-items: baseline; flex-wrap: wrap; margin-bottom: 4px; }
  .hf { font-size: 11px; font-weight: 700; color: #fff; background: var(--accent); border-radius: 4px; padding: 1px 7px; }
  .gre p { margin: 4px 0; }
  .ex { font-style: italic; color: var(--muted); }
  .roots { background: var(--blue-soft); border-left: 5px solid var(--blue); border-radius: 6px; padding: 8px 16px; margin: 0 0 14px; }
  .rh { display: flex; gap: 10px; align-items: baseline; flex-wrap: wrap; }
  .rf { font-weight: 700; color: var(--blue); letter-spacing: 0.04em; }
  .rw { margin-top: 4px; }
  .chip.me { background: var(--blue); color: #fff; border-color: var(--blue); }

  .block { margin-top: 26px; }
  .bh { margin: 0 0 4px; font-size: 24px; color: var(--head); }
  .bpos { font-style: italic; font-weight: 400; font-size: 18px; color: var(--muted); margin-left: 6px; }
  .head-line { color: var(--muted); margin: 0 0 8px; font-style: italic; }

  .senses { padding-left: 1.5em; margin: 0; }
  .senses.single { list-style: none; padding-left: 0; }
  .senses > li { margin: 0 0 22px; padding-left: 4px; }
  .senses > li::marker { font-weight: 700; color: var(--head); }
  .lab { margin: 0 0 2px; color: var(--muted); font-style: italic; font-size: 17px; }
  .def { margin: 0; font-size: 19px; line-height: 1.5; }
  .parent { color: var(--muted); font-size: 15px; margin-right: 6px; }
  .rel { margin: 8px 0 0; font-size: 17px; }
  .k { font-size: 13px; letter-spacing: 0.02em; color: var(--text); margin-right: 8px; }
  .lk { background: none; border: 0; padding: 0; color: var(--blue); font-weight: 700; font-size: 18px; }
  .lk:hover { text-decoration: underline; }
  .exs { list-style: none; margin: 8px 0 0; padding: 0; }
  .exs li { position: relative; padding-left: 20px; margin: 5px 0; font-style: italic; color: var(--text); font-size: 18px; }
  .exs li::before { content: ''; position: absolute; left: 6px; top: 0.7em; width: 6px; height: 6px; border-radius: 50%; background: var(--muted); }
  .by { font-style: normal; font-size: 13px; color: var(--muted); }
  .quotes { margin: 2px 0 0 0; font-size: 14px; }
  .forms { color: var(--muted); font-size: 14px; display: flex; flex-wrap: wrap; gap: 4px 16px; }
  .forms em { font-style: normal; font-size: 12px; }

  /* "Word Origin": the salmon box with the orange bar, as in the Oxford app. */
  .origin { margin: 26px 0; }
  .ob { padding: 0 16px 12px 44px; }
  .ob p { margin: 0 0 8px; }

  .attr { display: flex; gap: 10px; align-items: center; flex-wrap: wrap; color: var(--muted); font-size: 13px; margin: 18px 0 0; }
  .mwl.dark { display: none; }
  @media (prefers-color-scheme: dark) { :global(:root:not([data-theme='light'])) .mwl.light { display: none; } :global(:root:not([data-theme='light'])) .mwl.dark { display: inline; } }
  :global(:root[data-theme='dark']) .mwl.light { display: none; }
  :global(:root[data-theme='dark']) .mwl.dark { display: inline; }
  @media (max-width: 720px) { .hw { font-size: 36px; } .posrow { font-size: 20px; } .def { font-size: 18px; } }
</style>
