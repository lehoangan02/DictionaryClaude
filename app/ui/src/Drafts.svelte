<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from './Icon.svelte';
  import { call } from './lib/api';
  import { app, bump, openDetail, closeDetail } from './lib/store.svelte';
  import { shrink } from './lib/image';
  import DraftReview from './DraftReview.svelte';
  import Camera from './Camera.svelte';
  import { flip } from 'svelte/animate';
  import { grow, flipDur } from './lib/motion';

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
    // Syncing is what carries the photo up and the reading back; do not wait for the five-minute rhythm.
    let syncing = false;
    const t = setInterval(async () => {
      if (!working || syncing) return;
      syncing = true;
      try { await call('sync_now'); } catch { /* offline or not paired: the next try will do */ }
      syncing = false;
      load().catch(() => {});
    }, 4000);
    return () => clearInterval(t);
  });

  // Phones open the camera from a file input; a computer needs a camera of its own.
  const desktopCamera = !matchMedia('(pointer: coarse)').matches && !!navigator.mediaDevices?.getUserMedia;
  let camera = $state(false);

  async function addFiles(files: File[]) {
    error = '';
    try {
      for (const f of files) picked.push({ url: await shrink(f) });
    } catch { error = 'That photo could not be read. Try a JPEG or PNG.'; }
  }
  async function add(e: Event) {
    const input = e.target as HTMLInputElement;
    await addFiles(Array.from(input.files ?? []));
    input.value = '';
  }
  // Dragging images onto the screen adds them like "Add photos" does.
  let dragging = $state(false);
  const hasFiles = (e: DragEvent) => !!e.dataTransfer?.types.includes('Files');
  function dragOver(e: DragEvent) { if (hasFiles(e)) { e.preventDefault(); if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy'; dragging = true; } }
  function dragLeave(e: DragEvent) { if (!(e.currentTarget as Node).contains(e.relatedTarget as Node | null)) dragging = false; }
  async function drop(e: DragEvent) {
    if (!hasFiles(e)) return;
    e.preventDefault(); dragging = false;
    const images = Array.from(e.dataTransfer?.files ?? []).filter((f) => f.type.startsWith('image/'));
    if (!images.length) error = 'Only images can be dropped here.';
    else await addFiles(images);
  }

  async function send() {
    busy = true; error = '';
    try {
      await call('draft_create', { photos: picked.map((p) => p.url), session_date: date });
      picked = [];
      bump(); await load();
      call('sync_now').then(load).catch(() => {});
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

{#if camera}
  <Camera onshot={(f) => addFiles([f])} onclose={() => closeDetail()} />
{/if}

{#if open}
  <DraftReview id={open} onclose={() => closeDetail()} />
{:else}
  <div class="drops" class:drag={dragging} role="region" aria-label="Photograph your notes" ondragover={dragOver} ondragleave={dragLeave} ondrop={drop}>
  <section class="card rise">
    <h3>Photograph your notes</h3>
    <p class="muted small">Take, choose or drop photos of handwritten notes. The server reads them; you check the result before anything is saved.</p>
    <div class="row">
      {#if desktopCamera}
        <button class="btn" onclick={() => { camera = true; openDetail(() => (camera = false)); }}><Icon name="camera" size={20} /> Take photo</button>
      {:else}
        <label class="btn"><Icon name="camera" size={20} /> Take photo<input type="file" accept="image/*" capture="environment" onchange={add} hidden /></label>
      {/if}
      <label class="btn"><Icon name="image" size={20} /> Add photos<input type="file" accept="image/*" multiple onchange={add} hidden /></label>
    </div>
    {#if picked.length}
      <div class="thumbs">
        {#each picked as p, i (p.url)}
          <div class="th" in:grow out:grow animate:flip={{ duration: flipDur() }}><img src={p.url} alt="Photo {i + 1}" /><button aria-label="Remove photo {i + 1}" onclick={() => picked.splice(i, 1)}><Icon name="close" size={16} /></button></div>
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
      <button class="draft card rise" style="--i:{drafts.indexOf(d)}" onclick={() => { open = d.id; openDetail(() => { open = null; load(); }); }}>
        <span class="st" class:ready={d.status === 'needs_review'} class:bad={d.status === 'failed'}>{label(d)}</span>
        <span class="meta">{d.photos.length} photo{d.photos.length > 1 ? 's' : ''} · {d.session_date || 'undated'}</span>
        <span class="pv">{(d.text || d.ocr_text || '').split('\n')[0]}</span>
      </button>
    {/each}
  {/if}
  </div>
{/if}

<style>
  .drops { min-height: 60vh; border-radius: 12px; }
  .drops.drag { outline: 2px dashed var(--blue); outline-offset: -4px; background: var(--blue-soft); }
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
