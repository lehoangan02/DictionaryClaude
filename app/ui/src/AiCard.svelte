<script lang="ts">
  import { call } from './lib/api';
  import { app } from './lib/store.svelte';

  let st = $state<any>(null);
  let test = $state<any>(null);
  let err = $state('');
  let busy = $state(false);

  $effect(() => { void app.version; call('ai_status').then((r) => (st = r)).catch(() => {}); });
  async function run() {
    busy = true; err = ''; test = null;
    try { test = await call('ai_test'); st = await call('ai_status'); } catch (e) { err = (e as Error).message; }
    busy = false;
  }
</script>

<section class="card rise" style="--i:4">
  <h3>On-device AI</h3>
  <p class="muted small">A small model on this device lets quiz-session search find things by meaning. It runs on the Neural Engine (Mac) or the phone's NPU (Android) when there is one, then the GPU, then the CPU.</p>
  {#if st?.ready}
    <p>Running on <b>{st.backend_name}</b>.</p>
    {#each st.skipped ?? [] as s}<p class="muted small">{s.backend} was not used: {s.why}</p>{/each}
    <button class="btn small" disabled={busy} onclick={run}>{busy ? 'Testing…' : 'Test it'}</button>
    {#if test}<p class="muted small">Ran on {test.backend_name}: first run {test.first_ms} ms, then {test.per_run_ms} ms per batch.</p>{/if}
  {:else if st}
    <p class="muted">{st.why === 'not downloaded' ? 'Not installed. Download “Meaning search (AI model)” above.' : `Not available: ${st.why}`}</p>
  {/if}
  {#if err}<p class="msg">{err}</p>{/if}
</section>
