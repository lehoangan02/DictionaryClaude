<script lang="ts">
  import { canBack, canForward, backWord, forwardWord, goBack, goForward } from './lib/store.svelte';
  import Icon from './Icon.svelte';
</script>

<!-- Back and forward through the words you have looked up, like a browser. -->
{#if canBack() || canForward()}
  <div class="hn fadein" role="navigation" aria-label="Lookup history">
    <button disabled={!canBack()} onclick={goBack} aria-label="Previous word: {backWord() ?? ''}">
      <Icon name="prev" size={26} /><span>{backWord() ?? ''}</span>
    </button>
    <button class="r" disabled={!canForward()} onclick={goForward} aria-label="Next word: {forwardWord() ?? ''}">
      <span>{forwardWord() ?? ''}</span><Icon name="next" size={26} />
    </button>
  </div>
{/if}

<style>
  .hn { display: flex; justify-content: space-between; gap: 8px; background: var(--panel); border-radius: 12px; padding: 4px 6px; margin-bottom: 16px; }
  button { display: flex; align-items: center; gap: 10px; background: none; border: 0; color: var(--head); min-height: 44px; padding: 4px 10px; border-radius: 10px; max-width: 48%; }
  button span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 15px; }
  button:hover:not(:disabled) { background: var(--panel-2); }
  button:disabled { opacity: 0.28; cursor: default; }
  .r { margin-left: auto; }
</style>
