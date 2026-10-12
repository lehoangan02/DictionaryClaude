<script lang="ts">
  import { call } from './lib/api';
  import { app } from './lib/store.svelte';
  import { pct, when } from './lib/format';
  import QuizPlayer from './QuizPlayer.svelte';
  import { flip } from 'svelte/animate';
  import { item as leave, flipDur } from './lib/motion';

  type Sess = { id: string; kind: string; title: string; started: number; ended: number | null; total: number; correct: number; answered: number };
  let cat = $state<any>(null);
  let sessions = $state<Sess[]>([]);
  let q = $state('');
  let error = $state('');
  let busy = $state(false);
  // What is on screen: the hub, a live session, or a past session's review.
  let live = $state<any>(null);
  let review = $state<any>(null);
  let unit = $state<number | null>(app.quizUnit);
  $effect(() => { if (app.quizUnit) { unit = app.quizUnit; app.quizUnit = null; } });

  // Quick practice form
  let mode = $state('def_to_word');
  let scope = $state('all');
  let uFrom = $state(1);
  let uTo = $state(10);
  let count = $state(15);
  // Notes form
  let noteDate = $state('');
  let noteCount = $state(12);

  async function loadCat() { cat = await call('quiz_catalog'); }
  async function loadSessions() { sessions = await call('sessions_list', { q: q.trim() || undefined, limit: 40 }); }
  $effect(() => { void app.version; loadCat().catch((e) => (error = e.message)); });
  let timer: ReturnType<typeof setTimeout>;
  $effect(() => { void q; void live; void review; clearTimeout(timer); timer = setTimeout(() => loadSessions().catch((e) => (error = e.message)), 180); });

  async function start(req: Record<string, unknown>) {
    busy = true; error = '';
    try { live = await call('quiz_start', req); review = null; }
    catch (e) { error = (e as Error).message; }
    finally { busy = false; }
  }
  const startWords = () => start({
    kind: 'words', mode, count,
    ...(scope === 'high' ? { high_freq: true } : scope === 'range' ? { unit_from: uFrom, unit_to: uTo } : {}),
  });
  const startNotes = () => start({ kind: 'notes', count: noteCount, session_date: noteDate || undefined });
  const startUnit = (ids: string[]) => start({ kind: 'barrons', quiz_ids: ids });
  async function resume(id: string) {
    busy = true; error = '';
    try { live = await call('quiz_resume', { session_id: id }); review = null; }
    catch (e) { error = (e as Error).message; }
    finally { busy = false; }
  }
  async function openReview(id: string) { review = await call('session_get', { id }); live = null; }
  async function remove(id: string) { await call('session_delete', { id }); await loadSessions(); }
  function done() { live = null; loadSessions(); }

  const KIND: Record<string, string> = { matching: 'Matching', fill_in: 'Fill-ins', sense: 'Sense or nonsense', applying: 'Applying your knowledge' };
  const quizzesOf = (u: number) => cat?.units.find((x: any) => x.unit === u)?.quizzes ?? [];
</script>

{#if live}
  <QuizPlayer session={live} ondone={done} onretry={(id: string) => start({ kind: 'retry', session_id: id })} />
{:else if review}
  <section class="fadein">
    <button class="btn small ghost" onclick={() => (review = null)}>← Back</button>
    <h1>{review.title}</h1>
    <p class="muted">{when(review.started)} · {review.correct}/{review.total} correct ({pct(review.correct, review.total)}%)</p>
    <div class="act">
      <button class="btn primary" onclick={() => start({ kind: 'retry', session_id: review.id })}
        disabled={review.review.every((r: any) => r.correct !== false)}>Retry missed</button>
    </div>
    <ol class="rv">
      {#each review.review as r, ri (r.item.id)}
        <li class="rise" style="--i:{ri}" class:ok={r.correct === true} class:no={r.correct === false}>
          <span class="mark">{r.correct === true ? '✓' : r.correct === false ? '✗' : '–'}</span>
          <div>
            <p class="pr">{r.item.prompt}</p>
            {#if r.correct === true}
              <p class="e">{r.expected.join(' · ')}</p>
            {:else}
              {#if r.given}<p class="g">You: {r.given}</p>{:else}<p class="muted">Not answered</p>{/if}
              <p class="e">Answer: {r.expected.join(' · ')}</p>
            {/if}
            {#if r.explanation}<p class="x">{r.explanation}</p>{/if}
          </div>
        </li>
      {/each}
    </ol>
  </section>
{:else}
  <h1>Quizzes</h1>
  {#if error}<p class="err">{error}</p>{/if}

  <section class="card rise" style="--i:0">
    <h3>Past sessions</h3>
    <input type="search" bind:value={q} placeholder="Search sessions — “stubborn”, “words I got wrong”, “unit 12”…" aria-label="Search sessions" />
    {#each sessions as s (s.id)}
      <div class="srow" animate:flip={{ duration: flipDur() }} out:leave>
        <button class="st" onclick={() => openReview(s.id)}>
          <b>{s.title}</b>
          <span class="muted">{when(s.started)}{s.ended ? '' : ` · paused at ${s.answered} of ${s.total}`}</span>
        </button>
        {#if !s.ended}<button class="btn small primary" disabled={busy} onclick={() => resume(s.id)}>Resume</button>{/if}
        <span class="score" class:perfect={s.ended && s.correct === s.total}>{s.correct}/{s.total}<small>{pct(s.correct, s.total)}%</small></span>
        <button class="ic" onclick={() => remove(s.id)} aria-label="Delete session">✕</button>
      </div>
    {:else}
      <p class="empty">{q ? 'No sessions match.' : 'Finished quizzes will be listed here.'}</p>
    {/each}
  </section>

  <section class="card rise" style="--i:1">
    <h3>Quick practice</h3>
    {#if cat && !cat.barrons}
      <p class="muted">Barron's isn't on this device yet. Pair with your sync server in Settings and it arrives with the first sync.</p>
    {:else}
      <div class="grid">
        <label>Style
          <select bind:value={mode}>
            <option value="def_to_word">Meaning → word</option>
            <option value="word_to_def">Word → meaning</option>
            <option value="cloze">Fill the blank (sentences)</option>
          </select></label>
        <label>Words
          <select bind:value={scope}>
            <option value="all">All {cat?.words ?? 800} words</option>
            <option value="high">High-frequency (300)</option>
            <option value="range">Unit range…</option>
          </select></label>
        {#if scope === 'range'}
          <label>From unit <input type="number" min="1" max="80" bind:value={uFrom} /></label>
          <label>To unit <input type="number" min="1" max="80" bind:value={uTo} /></label>
        {/if}
        <label>Questions <input type="number" min="5" max="100" bind:value={count} /></label>
      </div>
      <button class="btn primary" onclick={startWords} disabled={busy}>Start</button>
    {/if}
  </section>

  <section class="card rise" style="--i:2">
    <h3>My tutor notes</h3>
    {#if (cat?.notes?.relations ?? 0) === 0}
      <p class="muted">Write relations in Notes (like <code>abate = lessen</code>) to practise them here.</p>
    {:else}
      <p class="muted">{cat.notes.relations} relations from your notes.</p>
      <div class="grid">
        <label>Session
          <select bind:value={noteDate}>
            <option value="">All notes</option>
            {#each cat.notes.dates as d (d)}<option value={d}>{d}</option>{/each}
          </select></label>
        <label>Questions <input type="number" min="3" max="60" bind:value={noteCount} /></label>
      </div>
      <button class="btn primary" onclick={startNotes} disabled={busy}>Quiz my notes</button>
    {/if}
  </section>

  {#if cat?.barrons}
    <section class="card rise" style="--i:3">
      <h3>Barron's exercises</h3>
      <p class="muted">Every review from the book: matching, fill-ins, sense or nonsense, and the applying-your-knowledge questions.</p>
      <div class="units" role="group" aria-label="Units">
        {#each cat.units as u (u.unit)}
          <button class:on={unit === u.unit} class:big={u.quizzes.some((x: any) => x.kind === 'applying')}
            onclick={() => (unit = unit === u.unit ? null : u.unit)}>{u.unit}</button>
        {/each}
      </div>
      {#if unit}
        <div class="up">
          <b>Unit {unit}</b>
          <button class="btn small primary" onclick={() => startUnit(quizzesOf(unit!).filter((x: any) => x.kind !== 'applying').map((x: any) => x.id))}>Whole review</button>
          {#each quizzesOf(unit) as z (z.id)}
            <button class="btn small" onclick={() => startUnit([z.id])}>{KIND[z.kind] ?? z.kind} · {z.count}</button>
          {/each}
        </div>
      {/if}
    </section>
  {/if}

  {#if cat?.extras}
    <section class="card rise" style="--i:4">
      <h3>Reviews</h3>
      <p class="muted">The two 50-question cumulative reviews at the back of the vocabulary section.</p>
      <div class="up">
        {#each cat.extras.reviews as z (z.id)}
          <button class="btn small" onclick={() => startUnit([z.id])}>{z.title.replace('Review: ', '')} · {z.count}</button>
        {/each}
      </div>
    </section>

    <section class="card rise" style="--i:5">
      <h3>Word roots</h3>
      <p class="muted">Root Work (match words to meanings) and the Root Roundup reviews.</p>
      <div class="units" role="group" aria-label="Root Work">
        {#each cat.extras.roots.filter((z: any) => z.id.startsWith('rw-')) as z (z.id)}
          <button onclick={() => startUnit([z.id])} title={z.title}>{Number(z.id.slice(3))}</button>
        {/each}
      </div>
      <div class="up">
        {#each cat.extras.roots.filter((z: any) => z.id.startsWith('rr-')) as z (z.id)}
          <button class="btn small" onclick={() => startUnit([z.id])}>Roundup {z.id.slice(3).replace('-', '–')}</button>
        {/each}
      </div>
    </section>
  {/if}
{/if}

<style>
  h1 { margin: 0 0 12px; font-size: 28px; }
  h3 { margin: 0 0 8px; font-size: 14px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); }
  section.card { margin: 14px 0; }
  .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 10px; margin-bottom: 12px; }
  .grid label { display: flex; flex-direction: column; font-size: 13px; color: var(--muted); gap: 3px; }
  .srow { display: flex; align-items: center; gap: 10px; border-top: 1px solid var(--line); margin-top: 8px; padding-top: 6px; }
  .st { flex: 1; text-align: left; background: none; border: 0; display: flex; flex-direction: column; padding: 4px 0; min-width: 0; }
  .st b { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .score { font-weight: 700; text-align: right; }
  .score small { display: block; font-weight: 400; color: var(--muted); font-size: 12px; }
  .score.perfect { color: var(--good); }
  .ic { background: none; border: 0; color: var(--muted); min-width: 36px; min-height: 36px; }
  .units { display: grid; grid-template-columns: repeat(auto-fill, minmax(42px, 1fr)); gap: 6px; }
  .units button { border: 1px solid var(--line); background: var(--bg); border-radius: 8px; min-height: 40px; }
  .units button.big { border-color: var(--accent); }
  .units button.on { background: var(--blue); color: #fff; border-color: var(--blue); }
  .up { display: flex; gap: 8px; flex-wrap: wrap; align-items: center; margin-top: 12px; }
  .act { margin: 8px 0 12px; }
  .rv { list-style: none; padding: 0; margin: 0; }
  .rv li { display: flex; gap: 12px; padding: 10px 0; border-top: 1px solid var(--line); }
  .mark { font-weight: 800; width: 22px; }
  .ok .mark { color: var(--good); } .no .mark { color: var(--bad); }
  .rv p { margin: 0 0 2px; }
  .pr { font-weight: 600; } .g { color: var(--bad); } .e { color: var(--good); } .x { color: var(--muted); font-size: 14px; }
  button:disabled { opacity: 0.5; }
</style>
