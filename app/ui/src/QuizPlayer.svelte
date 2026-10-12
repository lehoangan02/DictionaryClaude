<script lang="ts">
  import { call } from './lib/api';
  import { app, bump } from './lib/store.svelte';
  import { pct, passageParts } from './lib/format';
  import { enter, sweep, open as reveal } from './lib/motion';

  let { session, ondone, onretry }: { session: any; ondone: () => void; onretry: (id: string) => void } = $props();

  const items: any[] = $derived(session.items);
  let i = $state(0);
  let picks = $state<(string | null)[]>([]);
  // "Pick two" / "select all that apply" questions collect several answers from one list.
  let multi = $state<string[]>([]);
  let result = $state<any>(null);
  let score = $state(0);
  let finished = $state(false);
  let missed = $state<{ prompt: string; expected: string[]; explanation?: string }[]>([]);
  let shownAt = Date.now();
  let busy = $state(false);
  let error = $state('');
  const t0 = Date.now();

  $effect(() => {
    app.focus = true;
    // The phone's Back button pauses the quiz (the session is kept) instead of dropping you out of the app.
    const back = () => ondone();
    window.addEventListener('lexis:back', back);
    return () => { app.focus = false; window.removeEventListener('lexis:back', back); };
  });

  const item = $derived(items[i]);
  const blanks = $derived<number>(item?.blanks ?? 1);
  const select = $derived<number>(item?.select ?? 1);

  // Resuming: jump to the first question not answered yet and carry over the score so far.
  let resumed = false;
  $effect(() => {
    if (resumed) return;
    resumed = true;
    const done = new Set<string>((session.answered ?? []).map((a: any) => a.item_id));
    if (!done.size) return;
    score = (session.answered ?? []).filter((a: any) => a.correct).length;
    missed = session.missed ?? [];
    const next = items.findIndex((it: any) => !done.has(it.id));
    i = next < 0 ? items.length - 1 : next;
  });

  $effect(() => { void i; picks = Array((items[i]?.blanks ?? 1)).fill(null); multi = []; result = null; shownAt = Date.now(); });

  const ready = $derived(select > 1 ? multi.length === select : picks.length > 0 && picks.every((p) => p !== null));

  async function submit() {
    if (!ready || result || busy) return;
    busy = true;
    try {
      const given = select > 1 ? multi : blanks > 1 ? picks : picks[0];
      result = await call('quiz_answer', { session_id: session.id, item_id: item.id, given, ms: Date.now() - shownAt });
      if (result.correct) score++;
      else missed.push({ prompt: item.prompt, expected: result.expected, explanation: result.explanation });
    } catch (e) { error = (e as Error).message; }
    finally { busy = false; }
  }

  function next() {
    if (i < items.length - 1) i++;
    else { finished = true; bump(); }
  }

  function toggle(c: string) {
    if (result) return;
    multi = multi.includes(c) ? multi.filter((x) => x !== c) : multi.length < select ? [...multi, c] : [...multi.slice(1), c];
  }

  const isExpected = (c: string) => !!result && result.expected.some((e: string) => e.toLowerCase() === c.toLowerCase());

  function pick(b: number, c: string) {
    if (select > 1) { toggle(c); return; }
    if (result) return;
    // Choosing only selects; the answer counts when it is checked, so it can be changed first.
    picks[b] = c;
  }

  function keys(e: KeyboardEvent) {
    if ((e.target as HTMLElement).tagName === 'INPUT') return;
    if (result && (e.key === 'Enter' || e.key === ' ')) { e.preventDefault(); next(); return; }
    if (!result && blanks === 1 && /^[1-9]$/.test(e.key)) {
      const c = item.choices[0][Number(e.key) - 1];
      if (c) pick(0, c);
    } else if (!result && e.key === 'Enter') submit();
  }

  // Splits a prompt on its blank markers so they render as gaps.
  const parts = (s: string) => s.split(/(_{2,}|\(i+\) ?_{2,})/);
  const isBlank = (p: string) => /^_{2,}$/.test(p);
  const mins = $derived(Math.max(1, Math.round((Date.now() - t0) / 60000)));
  const roman = ['i', 'ii', 'iii', 'iv'];
</script>

<svelte:window onkeydown={keys} />

{#if finished}
  <section class="done" in:enter={{ y: 16, duration: 320 }}>
    <div class="ring ringfill" style="--p:{pct(score, items.length)}"><span>{pct(score, items.length)}%</span></div>
    <h1>{score} / {items.length}</h1>
    <p class="muted">{session.title} · about {mins} min</p>
    <div class="act">
      {#if missed.length}<button class="btn primary" onclick={() => onretry(session.id)}>Retry {missed.length} missed</button>{/if}
      <button class="btn" onclick={ondone}>Done</button>
    </div>
    {#if missed.length}
      <h2>To review</h2>
      <ul class="miss">
        {#each missed as m}
          <li class="rise" style="--i:{missed.indexOf(m)}"><p class="pr">{m.prompt}</p><p class="e">Answer: {m.expected.join(' · ')}</p>
            {#if m.explanation}<p class="x">{m.explanation}</p>{/if}</li>
        {/each}
      </ul>
    {:else}
      <p class="empty">Perfect round. 🎉</p>
    {/if}
  </section>
{:else if item}
  <section class="play">
    <header>
      <button class="btn small ghost" onclick={ondone}>⏸ Pause &amp; exit</button>
      <span class="muted">{item.group ?? session.title}</span>
      <span class="muted">{i + 1} / {items.length}</span>
    </header>
    <div class="bar"><div style="width:{(100 * (i + (result ? 1 : 0))) / items.length}%"></div></div>

    {#key i}
    <div class="qa" in:sweep={{ dir: 1 }}>
    {#if item.passage}
      <details class="passage" open>
        <summary>Passage</summary>
        <p class="pass">{#each passageParts(item.passage) as p}{#if p.n}<sup class="ln">{p.n}</sup>{/if}{p.text}{/each}</p>
      </details>
    {/if}

    <div class="card q">
      <p class="prompt">
        {#each parts(item.prompt) as p, k (k)}{#if isBlank(p)}<span class="gap">&nbsp;</span>{:else}{p}{/if}{/each}
      </p>
    </div>

    {#if select > 1}<p class="hint">Select {select} answers</p>{/if}
    {#each item.choices as list, b (b)}
      {#if blanks > 1}<h3>Blank ({roman[b]})</h3>{/if}
      <div class="choices" class:chips={item.kind === 'fill_in' && list.every((c: string) => c.length <= 16)}>
        {#each list as c, ci (c)}
          <button class="ch rise" style="--i:{ci}" class:sel={select > 1 ? multi.includes(c) : picks[b] === c}
            class:right={select > 1 ? isExpected(c) : result && result.expected[b]?.toLowerCase() === c.toLowerCase()}
            class:wrong={select > 1 ? !!result && multi.includes(c) && !isExpected(c) : result && picks[b] === c && result.expected[b]?.toLowerCase() !== c.toLowerCase()}
            aria-pressed={select > 1 ? multi.includes(c) : undefined}
            disabled={!!result && select === 1} onclick={() => pick(b, c)}>
            {#if blanks === 1 && select === 1 && ci < 9}<kbd>{ci + 1}</kbd>{/if}{c}
          </button>
        {/each}
      </div>
    {/each}
    </div>
    {/key}

    {#if error}<p class="err">{error}</p>{/if}
    {#if !result}
      <button class="btn primary go" disabled={!ready || busy} onclick={submit}>Check answer</button>
    {/if}
    {#if result}
      <div class="fb" class:ok={result.correct} transition:reveal>
        <b>{result.correct ? 'Correct' : 'Not quite'}</b>
        {#if !result.correct}<span> — {result.expected.join(' · ')}</span>{/if}
        {#if result.explanation}<p>{result.explanation}</p>{/if}
      </div>
      <button class="btn primary go" onclick={next}>{i < items.length - 1 ? 'Next' : 'See results'}</button>
    {/if}
  </section>
{/if}

<style>
  header { display: flex; justify-content: space-between; align-items: center; gap: 8px; }
  .bar { height: 6px; background: var(--panel-2); border-radius: 3px; margin: 8px 0 14px; overflow: hidden; }
  .bar div { height: 100%; background: var(--accent); transition: width 0.25s; }
  .q { padding: 18px; }
  .prompt { font-size: 19px; margin: 0; line-height: 1.55; }
  .gap { display: inline-block; min-width: 90px; border-bottom: 3px solid var(--accent); margin: 0 3px; }
  h3 { margin: 14px 0 6px; font-size: 13px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); }
  .choices { display: grid; gap: 8px; margin-top: 14px; }
  .choices.chips { grid-template-columns: repeat(auto-fill, minmax(130px, 1fr)); }
  .ch {
    text-align: left; border: 1.5px solid var(--line); background: var(--bg); border-radius: 12px; padding: 12px 14px;
    font-size: 16px; min-height: 48px; display: flex; gap: 10px; align-items: baseline;
  }
  .ch:hover:not(:disabled) { border-color: var(--blue); box-shadow: var(--lift); }
  .ch.sel { border-color: var(--blue); background: var(--panel-2); }
  .ch.right { border-color: var(--good); background: var(--good-bg); }
  .ch.wrong { border-color: var(--bad); background: var(--bad-bg); }
  kbd { font: 12px ui-monospace, monospace; color: var(--muted); border: 1px solid var(--line); border-radius: 4px; padding: 0 5px; }
  .fb { margin: 14px 0 4px; padding: 10px 14px; border-radius: 10px; background: var(--bad-bg); color: var(--text); }
  .fb.ok { background: var(--good-bg); }
  .fb p { margin: 4px 0 0; font-size: 14px; color: var(--muted); }
  .go { margin-top: 12px; width: 100%; }
  .hint { margin: 12px 0 0; color: var(--muted); font-size: 14px; }
  .passage { background: var(--panel); border: 1px solid var(--line); border-radius: 10px; padding: 8px 14px; margin-bottom: 12px; }
  .passage summary { cursor: pointer; font-weight: 600; }
  .pass { font: 16px/1.65 Georgia, 'Times New Roman', serif; margin: 8px 0 2px; max-height: 42vh; overflow: auto; }
  .ln { color: var(--accent); font: 700 11px system-ui, sans-serif; margin-right: 3px; }
  .done { text-align: center; }
  .done h1 { margin: 6px 0 0; font-size: 34px; }
  .ring {
    --size: 150px; width: var(--size); height: var(--size); border-radius: 50%; margin: 10px auto 8px; display: grid; place-items: center;
    background: conic-gradient(var(--good) calc(var(--p) * 1%), var(--panel-2) 0);
  }
  .ring span { width: calc(var(--size) - 28px); height: calc(var(--size) - 28px); border-radius: 50%; background: var(--bg); display: grid; place-items: center; font-size: 30px; font-weight: 700; }
  .act { display: flex; gap: 10px; justify-content: center; margin: 14px 0; flex-wrap: wrap; }
  .miss { list-style: none; padding: 0; margin: 0; text-align: left; }
  .miss li { border-top: 1px solid var(--line); padding: 10px 0; }
  .miss p { margin: 0 0 2px; } .pr { font-weight: 600; } .e { color: var(--good); } .x { color: var(--muted); font-size: 14px; }
  button:disabled { opacity: 0.55; }
</style>
