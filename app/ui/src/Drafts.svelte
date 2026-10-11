<script lang="ts">
  import { onMount } from 'svelte';
  import { call } from './lib/api';
  import { app, bump } from './lib/store.svelte';
  import { shrink } from './lib/image';
  import DraftReview from './DraftReview.svelte';

  let { reviewing = $bindable(false) }: { reviewing?: boolean } = $props();
  let drafts = $state<any[]>([]);
  let picked = $state<{ url: string }[]>([]);
  let date = $state(new Date().toISOString().slice(0, 10));
  let busy = $state(false);
  let error = $state('');
  let open = $state<string | null>(null);
  $effect(() => { reviewing = open !== null; });

  async function load() { drafts = await call('drafts_list'); }
  $effect(() => { void app.version; load().catch(() => {}); });

  // While something is being read, look again soon; syncing brings the result.
  const working = $derived(drafts.some((d) => ['queued', 'processing'].includes(d.status) || ['requested', 'running'].includes(d.agent_status)));
  onMount(() => {
    const t = setInterval(() => { if (working) load().catch(() => {}); }, 4000);
    return () => clearInterval(t);
  });

  async function add(e: Event) {
    const input = e.target as HTMLInputElement;
    error = '';
    try {
      for (const f of Array.from(input.files ?? [])) picked.push({ url: await shrink(f) });
    } catch { error = 'That photo could not be read. Try a JPEG or PNG.'; }
    input.value = '';
  }

  async function send() {
    busy = true; error = '';
    try {
      await call('draft_create', { photos: picked.map((p) => p.url), session_date: date });
      picked = [];
      bump(); await load();
    } catch (e) { error = (e as Error).message; }
    busy = false;
  }

  const LABEL: Record<string, string> = {
    queued: 'Waiting to be read', processing: 'Reading…', needs_review: 'Ready to review', failed: 'Failed',
  };
  const label = (d: any) =>
    d.agent_status === 'running' || d.agent_status === 'requested' ? 'Agents reading…'
    : d.agent_status === 'done' && d.status === 'needs_review' ? 'Agents finished' : LABEL[d.status] ?? d.status;
</script>

{#if open}
  <DraftReview id={open} onclose={() => { open = null; load(); }} />
{:else}
  <section class="card">
    <h3>Photograph your notes</h3>
    <p class="muted small">Take or choose photos of handwritten notes. The server reads them; you check the result before anything is saved.</p>
    <div class="row">
      <label class="btn">📷 Take photo<input type="file" accept="image/*" capture="environment" onchange={add} hidden /></label>
      <label class="btn">🖼️ Add photos<input type="file" accept="image/*" multiple onchange={add} hidden /></label>
    </div>
    {#if picked.length}
      <div class="thumbs">
        {#each picked as p, i (i)}
          <div class="th"><img src={p.url} alt="Photo {i + 1}" /><button aria-label="Remove photo {i + 1}" onclick={() => picked.splice(i, 1)}>✕</button></div>
        {/each}
      </div>
      <div class="row">
        <label class="f">Session <input type="date" bind:value={date} /></label>
        <button class="btn primary" disabled={busy} onclick={send}>{busy ? 'Sending…' : `Send ${picked.length} photo${picked.length > 1 ? 's' : ''} for reading`}</button>
      </div>
    {/if}
    {#if error}<p class="err">{error}</p>{/if}
  </section>

  {#if drafts.length}
    <h2>Waiting for review</h2>
    {#each drafts as d (d.id)}
      <button class="draft card" onclick={() => (open = d.id)}>
        <span class="st" class:ready={d.status === 'needs_review'} class:bad={d.status === 'failed'}>{label(d)}</span>
        <span class="meta">{d.photos.length} photo{d.photos.length > 1 ? 's' : ''} · {d.session_date || 'undated'}</span>
        <span class="pv">{(d.text || d.ocr_text || '').split('\n')[0]}</span>
      </button>
    {/each}
  {/if}
{/if}

<style>
  h3 { margin: 0 0 6px; font-size: 14px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); }
  .small { font-size: 13px; }
  .row { display: flex; gap: 10px; align-items: end; flex-wrap: wrap; margin-top: 10px; }
  .f { display: flex; flex-direction: column; font-size: 13px; color: var(--muted); gap: 3px; }
  label.btn { display: inline-flex; align-items: center; gap: 6px; }
  .thumbs { display: flex; gap: 8px; flex-wrap: wrap; margin-top: 12px; }
  .th { position: relative; }
  .th img { height: 96px; border-radius: 8px; border: 1px solid var(--line); display: block; }
  .th button { position: absolute; top: 3px; right: 3px; border: 0; border-radius: 50%; width: 24px; height: 24px; background: rgba(0, 0, 0, 0.6); color: #fff; }
  h2 { margin-bottom: 8px; }
  .draft { display: grid; grid-template-columns: auto 1fr; gap: 2px 12px; width: 100%; text-align: left; margin: 8px 0; align-items: baseline; }
  .st { font-weight: 600; font-size: 13px; padding: 1px 9px; border-radius: 999px; background: var(--panel-2); justify-self: start; }
  .st.ready { background: var(--good-bg); color: var(--good); } .st.bad { background: var(--bad-bg); color: var(--bad); }
  .meta { color: var(--muted); font-size: 13px; }
  .pv { grid-column: 1 / -1; font-family: ui-monospace, Menlo, monospace; font-size: 14px; color: var(--muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  button:disabled { opacity: 0.5; }
</style>
