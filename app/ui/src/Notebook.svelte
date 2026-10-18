<script lang="ts">
  import { open } from './lib/store.svelte';
  import { REL } from './lib/format';
  import { sweep } from './lib/motion';

  type N = { id: string; session_date: string; tags: string; body: string; updated: number };
  let { notes, onedit }: { notes: N[]; onedit: (n: N) => void } = $props();

  // The notes read like pages of one notebook, oldest first; the newest page opens first.
  const pages = $derived([...notes].sort((a, b) => (a.session_date || '').localeCompare(b.session_date || '') || a.updated - b.updated));
  let at = $state<number | null>(null);
  let dir = $state<1 | -1>(1);
  let toc = $state(false);
  const cur = $derived(Math.min(Math.max(at ?? pages.length - 1, 0), Math.max(pages.length - 1, 0)));
  const page = $derived(pages[cur]);

  function go(i: number) { if (i < 0 || i >= pages.length) return; dir = i >= cur ? 1 : -1; at = i; toc = false; }

  const SYM: Record<string, keyof typeof REL> = { '=': 'same', '≈': 'similar', '><': 'opposite', '⍯': 'different', '≠': 'different' };
  type Part = { word: string } | { rel: keyof typeof REL };
  // A line like "abate = lessen ≈ subside" becomes word chips joined by relation marks; anything else stays handwriting.
  function parse(line: string): Part[] | null {
    const bits = line.replaceAll('~=', '≈').replaceAll('=/=', '⍯').replaceAll('!=', '⍯').split(/\s*(><|≈|⍯|≠|=)\s*/);
    if (bits.length < 3) return null;
    const out: Part[] = [];
    for (const [i, b] of bits.entries()) {
      if (i % 2) out.push({ rel: SYM[b] });
      else if (b.trim()) out.push({ word: b.trim() });
      else return null;
    }
    return out;
  }
  const lines = $derived((page?.body ?? '').split('\n').map((l) => ({ text: l, parts: parse(l) })));
  const tagsOf = (t: string) => t.split(',').map((x) => x.trim()).filter(Boolean);
  const pretty = (d: string) => {
    const t = Date.parse(d);
    return Number.isNaN(t) ? d || 'Undated' : new Date(t).toLocaleDateString(undefined, { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' });
  };
  const words = (n: N) => n.body.split('\n').filter((l) => l.trim()).length;
</script>

{#if !pages.length}
  <p class="empty">No notes yet. Scanned or typed notes will appear here as pages.</p>
{:else}
  <div class="book">
    <div class="bar">
      <button class="btn small ghost" onclick={() => go(cur - 1)} disabled={cur === 0} aria-label="Previous page">‹</button>
      <button class="btn small ghost cnt" onclick={() => (toc = !toc)} aria-expanded={toc}>Page {cur + 1} of {pages.length} ▾</button>
      <button class="btn small ghost" onclick={() => go(cur + 1)} disabled={cur === pages.length - 1} aria-label="Next page">›</button>
    </div>
    {#if toc}
      <ol class="toc card">
        {#each pages as p, i (p.id)}
          <li><button class:on={i === cur} onclick={() => go(i)}><b>{p.session_date || 'Undated'}</b><span class="muted">{tagsOf(p.tags).join(', ') || `${words(p)} lines`}</span></button></li>
        {/each}
      </ol>
    {/if}
    {#key page.id}
      <article class="sheet" in:sweep={{ dir }}>
        <div class="holes" aria-hidden="true"><i></i><i></i><i></i></div>
        <header>
          <h2>{pretty(page.session_date)}</h2>
          {#if tagsOf(page.tags).length}<div class="tags">{#each tagsOf(page.tags) as t}<span class="stamp">{t}</span>{/each}</div>{/if}
        </header>
        <div class="ruled">
          {#each lines as l}
            {#if l.parts}
              <p class="rel">
                {#each l.parts as p}
                  {#if 'word' in p}<button class="w" onclick={() => open(p.word)}>{p.word}</button>
                  {:else}<b class="mark {p.rel}" title={REL[p.rel].label}>{REL[p.rel].sym}</b>{/if}
                {/each}
              </p>
            {:else}
              <p class="hand">{l.text || ' '}</p>
            {/if}
          {/each}
        </div>
        <footer>
          <button class="btn small ghost" onclick={() => onedit(page)}>Edit this page</button>
          <span class="pg">{cur + 1}</span>
        </footer>
      </article>
    {/key}
  </div>
{/if}

<style>
  .book { margin: 10px 0; }
  .bar { display: flex; justify-content: space-between; align-items: center; gap: 8px; margin-bottom: 8px; }
  .cnt { flex: 1; }
  .toc { list-style: none; margin: 0 0 10px; padding: 6px; max-height: 40vh; overflow: auto; }
  .toc button { display: flex; justify-content: space-between; gap: 10px; width: 100%; background: none; border: 0; padding: 7px 10px; border-radius: 8px; color: var(--text); text-align: left; }
  .toc button.on { background: var(--blue-soft); }
  .sheet {
    --ln: 34px;
    position: relative; padding: 18px 20px 14px 52px; border-radius: 6px 14px 14px 6px;
    background: #fffdf6; color: #26303f; border: 1px solid #e8e0c8;
    box-shadow: var(--lift), inset 6px 0 10px -8px rgba(0, 0, 0, 0.25);
    background-image: linear-gradient(90deg, transparent 40px, #f0a79a 40px, #f0a79a 41px, transparent 41px);
  }
  @media (prefers-color-scheme: dark) { :global(:root:not([data-theme='light'])) .sheet { background-color: #1d2433; color: #e6ebf3; border-color: #2b3550; } }
  :global(:root[data-theme='dark']) .sheet { background-color: #1d2433; color: #e6ebf3; border-color: #2b3550; }
  .holes { position: absolute; left: 12px; top: 0; bottom: 0; display: flex; flex-direction: column; justify-content: space-around; padding: 30px 0; }
  .holes i { width: 14px; height: 14px; border-radius: 50%; background: var(--bg); box-shadow: inset 0 1px 3px rgba(0, 0, 0, 0.35); }
  header { margin-bottom: 8px; }
  h2 { margin: 0; font-family: Georgia, 'Times New Roman', serif; font-size: 22px; font-weight: 600; color: var(--navy); }
  :global(:root[data-theme='dark']) h2 { color: #dce8ff; }
  .tags { display: flex; gap: 6px; flex-wrap: wrap; margin-top: 6px; }
  .stamp { border: 1.5px solid var(--accent); color: var(--accent); border-radius: 5px; padding: 0 7px; font-size: 11px; letter-spacing: 0.06em; text-transform: uppercase; transform: rotate(-1.5deg); }
  .ruled { background-image: repeating-linear-gradient(transparent 0 calc(var(--ln) - 1px), #c9d8ee calc(var(--ln) - 1px) var(--ln)); background-position: 0 6px; }
  .ruled p { margin: 0; min-height: var(--ln); line-height: var(--ln); font-size: 18px; }
  .hand { font-family: 'Segoe Print', 'Bradley Hand', 'Comic Sans MS', cursive, 'Lexis Symbols'; white-space: pre-wrap; }
  .rel { display: flex; flex-wrap: wrap; align-items: center; gap: 0 6px; }
  .w { background: none; border: 0; padding: 0 2px; font: inherit; font-family: Georgia, serif; font-weight: 600; color: var(--blue); border-bottom: 2px solid color-mix(in srgb, var(--blue) 35%, transparent); line-height: 1.2; }
  .mark { display: inline-block; min-width: 22px; text-align: center; border-radius: 999px; font-size: 14px; line-height: 22px; color: #fff; background: var(--navy); }
  .mark.same { background: #1b7f3b; }
  .mark.similar { background: #0a63b0; }
  .mark.opposite { background: #c0392b; }
  .mark.different { background: #e8590c; }
  footer { display: flex; justify-content: space-between; align-items: center; margin-top: 10px; }
  .pg { font-family: Georgia, serif; color: var(--muted); font-size: 14px; }
  button:disabled { opacity: 0.4; }
</style>
